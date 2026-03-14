"""Agent eval harness — launches Claude Code as an autonomous game developer.

Provides ``AgentConfig`` for task parameters, ``AgentRun`` for capturing
results, ``launch_agent`` to orchestrate the full pipeline, ``score_game``
to evaluate the agent's output, and ``run_agent_eval`` to run the complete
evaluation end-to-end.
"""

from __future__ import annotations

import hashlib
import json
import logging
import os
import re
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from datetime import datetime
from pathlib import Path

from nomai.eval.autonomy import TaskResult
from nomai.eval.llm_client import ClaudeCodeLLMClient
from nomai.eval.reasoning import (
    geval_all,
    generate_spatial_questions,
    multihop_spatial_accuracy,
)
from nomai.eval.scene_qa import generate_scene_questions, scene_qa_accuracy
from nomai.scene import SceneEntity, SceneSnapshot

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Module-level constants
# ---------------------------------------------------------------------------

PROJECT_ROOT: Path = Path(__file__).resolve().parents[4]

AGENT_SYSTEM_PROMPT: str = """\
You are an expert game developer using the Nomai engine.

CRITICAL: You MUST write ALL files to your current working directory. Do NOT \
use `cd` to navigate elsewhere. Do NOT write files to parent directories or \
any absolute path. Use only relative paths like `game.py`, `snapshot.json`, etc.

ARCHITECTURE: The Nomai engine has two layers:
- Python: engine setup, entity spawning, physics config, verification, snapshots.
- WASM (AssemblyScript): runtime game logic (collision responses, scoring, state machines).
Game logic written in Python only runs during headless simulation (engine.tick()). \
Game logic written in WASM runs in BOTH headless and visual mode (engine.run()). \
If you want your game to work visually, put runtime logic in WASM.

Your task:
1. Read the Game Design Document (GDD) provided in the prompt.
2. Read the SDK reference provided in the prompt.
3. Write an AssemblyScript gameplay module for collision/game logic.
   - Import host functions from "./host" (see gameplay/assembly/host.ts).
   - Export `tick()` and `on_collision(entityA: i64, entityB: i64)`.
   - Save the .ts file under gameplay/assembly/ in the project directory.
4. Compile the AssemblyScript to WASM using npx asc.
5. Write a Python script called `game.py` in the current directory.
   - Sets up the engine, spawns entities, registers physics, loads your WASM.
   - Runs the simulation (WASM handles collisions automatically).
   - Prints snapshot.summary() and saves snapshot.json.
6. Run game.py to verify it works.
7. Use snapshot.summary() to inspect game state — the engine is headless.
8. Fix any issues by iterating on the AssemblyScript and/or Python.
9. When satisfied, create DONE.txt. If stuck, create STUCK.txt.

Important notes:
- The coordinate system is Y-up: Y=0 is bottom of screen, Y=600 is top.
- Register all components before spawning entities.
- Call init_physics() before creating any physics bodies.
- Tick once after spawning entities to apply the spawns before registering physics bodies.
- Game logic in Python only runs headlessly. For logic that must work in both \
headless and visual mode, put it in WASM (e.g. collision responses, scoring).
- ALL output files (game.py, snapshot.json, DONE.txt) MUST be in the current directory.
"""


# ---------------------------------------------------------------------------
# Data classes
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class AgentConfig:
    """Immutable configuration for an agent evaluation run.

    Attributes:
        task: GDD task name (e.g. ``"breakout"``).
        model: Claude model alias (e.g. ``"sonnet"``, ``"opus"``).
        judge_model: Model alias used for LLM-judged scoring (e.g. ``"sonnet"``, ``"opus"``).
        max_budget_usd: Maximum spend for the agent session.
        timeout_s: Subprocess wall-clock timeout in seconds.
    """

    task: str
    model: str = "sonnet"
    judge_model: str | None = None
    max_budget_usd: float = 5.0
    timeout_s: int = 600


@dataclass
class AgentRun:
    """Captures the result of a single agent evaluation run.

    Attributes:
        config: The agent configuration used.
        workdir: Isolated working directory for this run.
        exit_code: Subprocess exit code (non-zero on failure/timeout).
        wall_time_s: Elapsed wall-clock time in seconds.
        stdout: Captured standard output from the agent.
        stderr: Captured standard error from the agent.
    """

    config: AgentConfig
    workdir: Path
    exit_code: int
    wall_time_s: float
    stdout: str
    stderr: str

    def _find_file(self, name: str) -> Path | None:
        """Search for *name* in workdir, then parent (eval_workdir/).

        The agent occasionally writes files to the parent directory instead
        of the timestamped workdir.  This fallback prevents false negatives.
        """
        candidate = self.workdir / name
        if candidate.exists():
            return candidate
        parent_candidate = self.workdir.parent / name
        if parent_candidate.exists():
            logger.warning(
                "File %s found in parent dir %s instead of workdir %s — "
                "agent likely navigated away from cwd",
                name, parent_candidate, self.workdir,
            )
            return parent_candidate
        return None

    @property
    def game_script(self) -> Path:
        """Path to the agent's output game script."""
        found = self._find_file("game.py")
        return found if found is not None else self.workdir / "game.py"

    @property
    def game_exists(self) -> bool:
        """Whether the agent produced a game.py file."""
        return self._find_file("game.py") is not None

    @property
    def signal(self) -> str:
        """Infer the agent's completion signal.

        Returns:
            ``"DONE"`` if DONE.txt exists, ``"STUCK"`` if STUCK.txt exists,
            ``"TIMEOUT"`` if the exit code is non-zero, else ``"UNKNOWN"``.
        """
        if self._find_file("DONE.txt") is not None:
            return "DONE"
        if self._find_file("STUCK.txt") is not None:
            return "STUCK"
        if self.exit_code != 0:
            return "TIMEOUT"
        return "UNKNOWN"


# ---------------------------------------------------------------------------
# launch_agent
# ---------------------------------------------------------------------------

def launch_agent(
    config: AgentConfig,
    *,
    project_root: Path | None = None,
) -> AgentRun:
    """Launch Claude Code as an autonomous agent to build a game from a GDD.

    Creates an isolated working directory, reads the GDD and SDK reference,
    composes a prompt, and invokes ``claude -p`` with full tools access and
    ``--dangerously-skip-permissions`` for sandboxed execution.

    Args:
        config: Agent configuration.
        project_root: Override for the project root directory.  Defaults to
            ``PROJECT_ROOT`` (auto-detected from this file's location).

    Returns:
        An ``AgentRun`` capturing the subprocess results.
    """
    root = project_root or PROJECT_ROOT

    # Create isolated workdir (microseconds to avoid collisions in batch runs)
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S_%f")
    workdir = root / "eval_workdir" / timestamp
    workdir.mkdir(parents=True, exist_ok=True)

    # Read GDD
    gdd_path = root / "eval_tasks" / f"{config.task}.md"
    if not gdd_path.exists():
        raise FileNotFoundError(
            f"GDD not found for task '{config.task}': {gdd_path}"
        )
    gdd_content = gdd_path.read_text(encoding="utf-8")

    # Read SDK reference
    sdk_ref_path = root / "docs" / "ai" / "nomai-sdk-reference.md"
    if not sdk_ref_path.exists():
        raise FileNotFoundError(
            f"SDK reference not found: {sdk_ref_path}"
        )
    sdk_ref_content = sdk_ref_path.read_text(encoding="utf-8")

    # Read host.ts so agent knows the WASM host API
    host_ts_path = root / "gameplay" / "assembly" / "host.ts"
    host_ts_content = ""
    if host_ts_path.exists():
        host_ts_content = host_ts_path.read_text(encoding="utf-8")

    # Build user prompt
    prompt = (
        f"## Game Design Document\n\n{gdd_content}\n\n"
        f"## SDK Reference\n\n{sdk_ref_content}\n\n"
        f"## WASM Host API (gameplay/assembly/host.ts)\n\n"
        f"```typescript\n{host_ts_content}\n```\n\n"
        f"## Output Instructions\n\n"
        f"1. Write your AssemblyScript gameplay module under gameplay/assembly/.\n"
        f"2. Compile it: cd gameplay && npx asc assembly/YOUR_FILE.ts "
        f"--outFile build/YOUR_FILE.wasm --optimize --exportRuntime\n"
        f"3. Write `game.py` in the working directory (setup + load WASM + run).\n"
        f"4. Create DONE.txt when finished, or STUCK.txt if you cannot proceed.\n"
    )

    # Build command
    cmd = [
        "claude",
        "-p",
        prompt,
        "--system-prompt", AGENT_SYSTEM_PROMPT,
        "--model", config.model,
        "--dangerously-skip-permissions",
        "--max-budget-usd", str(config.max_budget_usd),
        "--add-dir", str(root),
    ]

    # Prepare environment — strip CLAUDECODE to avoid nesting issues
    env = os.environ.copy()
    env.pop("CLAUDECODE", None)

    logger.info(
        "Launching agent for task=%s model=%s workdir=%s",
        config.task, config.model, workdir,
    )

    t0 = time.monotonic()
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=config.timeout_s,
            env=env,
            cwd=str(workdir),
        )
        wall_time = time.monotonic() - t0
        return AgentRun(
            config=config,
            workdir=workdir,
            exit_code=result.returncode,
            wall_time_s=wall_time,
            stdout=result.stdout,
            stderr=result.stderr,
        )
    except subprocess.TimeoutExpired as exc:
        wall_time = time.monotonic() - t0
        logger.warning(
            "Agent timed out after %.1fs for task=%s", wall_time, config.task
        )
        return AgentRun(
            config=config,
            workdir=workdir,
            exit_code=-1,
            wall_time_s=wall_time,
            stdout=(exc.output or b"").decode("utf-8", errors="replace")
                   if isinstance(exc.output, bytes) else (exc.output or ""),
            stderr=(exc.stderr or b"").decode("utf-8", errors="replace")
                   if isinstance(exc.stderr, bytes) else (exc.stderr or ""),
        )


# ---------------------------------------------------------------------------
# Helper: run game script
# ---------------------------------------------------------------------------

def _run_game_script(
    game_script: Path,
    *,
    project_root: Path,
    timeout: int = 120,
) -> subprocess.CompletedProcess[str]:
    """Run a game.py script in a subprocess.

    Args:
        game_script: Path to the game.py file.
        project_root: Project root directory (used as cwd so nomai imports work).
        timeout: Wall-clock timeout in seconds.

    Returns:
        CompletedProcess with captured stdout/stderr.

    Raises:
        subprocess.TimeoutExpired: If the script exceeds *timeout* seconds.
    """
    return subprocess.run(
        [sys.executable, str(game_script)],
        capture_output=True,
        text=True,
        timeout=timeout,
        cwd=str(project_root),
    )


def _parse_entity_count(stdout: str) -> int:
    """Extract ENTITY_COUNT from the last lines of stdout.

    Looks for a line matching ``ENTITY_COUNT: <N>`` and returns the integer.
    Returns 0 if no match is found.
    """
    for line in reversed(stdout.splitlines()):
        m = re.search(r"ENTITY_COUNT:\s*(\d+)", line)
        if m:
            return int(m.group(1))
    return 0


# ---------------------------------------------------------------------------
# Structural validation (Tier 1 — no LLM, fast, deterministic)
# ---------------------------------------------------------------------------

@dataclass
class SnapshotValidation:
    """Result of structural validation against GDD requirements."""

    checks: dict[str, bool]
    details: dict[str, str]

    @property
    def passed(self) -> bool:
        return all(self.checks.values())

    @property
    def failures(self) -> list[str]:
        return [name for name, ok in self.checks.items() if not ok]


def validate_breakout_snapshot(snapshot: SceneSnapshot) -> SnapshotValidation:
    """Validate a breakout game snapshot against GDD success criteria.

    Checks are derived directly from eval_tasks/breakout.md:
    - Correct entity types and roles
    - Ball moved from starting position
    - Ball is within game bounds (or close)
    - At least some bricks were destroyed
    - Walls exist with expected count
    """
    checks: dict[str, bool] = {}
    details: dict[str, str] = {}

    entities = snapshot.entities
    roles: dict[str, list[SceneEntity]] = {}
    for e in entities:
        roles.setdefault(e.role, []).append(e)

    # Check 1: Required entity types exist
    has_paddle = len(roles.get("paddle", [])) == 1
    checks["has_paddle"] = has_paddle
    details["has_paddle"] = f"paddle count: {len(roles.get('paddle', []))}" + (" (expected 1)" if not has_paddle else "")

    has_ball = len(roles.get("ball", [])) == 1
    checks["has_ball"] = has_ball
    details["has_ball"] = f"ball count: {len(roles.get('ball', []))}" + (" (expected 1)" if not has_ball else "")

    brick_count = len(roles.get("brick", []))
    has_bricks = brick_count > 0
    checks["has_bricks"] = has_bricks
    details["has_bricks"] = f"brick count: {brick_count}"

    wall_roles = [r for r in roles if r.startswith("wall_")]
    has_walls = len(wall_roles) >= 3
    checks["has_walls"] = has_walls
    details["has_walls"] = f"wall count: {len(wall_roles)} (expected >=3)"

    # Check 2: Ball moved from starting position (400, 300)
    if has_ball:
        ball = roles["ball"][0]
        if ball.position is not None:
            bx, by = ball.position
            ball_moved = abs(bx - 400.0) > 1.0 or abs(by - 300.0) > 1.0
            checks["ball_moved"] = ball_moved
            details["ball_moved"] = f"ball position: ({bx:.1f}, {by:.1f}), start was (400, 300)"
        else:
            checks["ball_moved"] = False
            details["ball_moved"] = "ball has no position"
    else:
        checks["ball_moved"] = False
        details["ball_moved"] = "no ball entity"

    # Check 3: Ball is within game bounds (with margin for wall thickness)
    GAME_W, GAME_H = 800.0, 600.0
    MARGIN = 50.0  # generous margin for wall overshoot
    if has_ball and roles["ball"][0].position is not None:
        bx, by = roles["ball"][0].position
        ball_in_bounds = (
            -MARGIN <= bx <= GAME_W + MARGIN
            and -MARGIN <= by <= GAME_H + MARGIN
        )
        checks["ball_in_bounds"] = ball_in_bounds
        details["ball_in_bounds"] = (
            f"ball at ({bx:.1f}, {by:.1f}), "
            f"bounds: (0-{GAME_W}, 0-{GAME_H}), margin: {MARGIN}"
        )
    else:
        checks["ball_in_bounds"] = False
        details["ball_in_bounds"] = "no ball position to check"

    # Check 4: Some bricks were destroyed (started with 20)
    INITIAL_BRICKS = 20
    bricks_destroyed = INITIAL_BRICKS - brick_count
    some_destroyed = bricks_destroyed > 0
    checks["bricks_destroyed"] = some_destroyed
    details["bricks_destroyed"] = (
        f"{bricks_destroyed}/{INITIAL_BRICKS} bricks destroyed"
        + ("" if some_destroyed else " — ball may not be colliding with bricks")
    )

    return SnapshotValidation(checks=checks, details=details)


# ---------------------------------------------------------------------------
# score_game
# ---------------------------------------------------------------------------

def score_game(run: AgentRun, *, project_root: Path | None = None, judge_model: str | None = None) -> dict:
    """Run the agent's game.py and produce a scored TaskResult.

    Executes the game script twice: the first run checks for basic correctness
    (zero exit code, positive entity count); the second run compares stdout
    hashes to verify replay determinism.

    Args:
        run: The AgentRun from ``launch_agent``.
        project_root: Override for project root.  Defaults to ``PROJECT_ROOT``.

    Returns:
        A dict with keys ``task_result`` (TaskResult), ``eval_report`` (None),
        and ``ground_truth`` (dict with run metadata).
    """
    root = project_root or PROJECT_ROOT

    # No game script produced — immediate failure
    if not run.game_exists:
        return {
            "task_result": TaskResult(
                task_id=run.config.task,
                succeeded=False,
            ),
            "eval_report": None,
            "ground_truth": {"error": "game.py not found"},
            "llm_scores": None,
        }

    # First run
    try:
        result1 = _run_game_script(run.game_script, project_root=root)
    except subprocess.TimeoutExpired:
        return {
            "task_result": TaskResult(
                task_id=run.config.task,
                succeeded=False,
            ),
            "eval_report": None,
            "ground_truth": {"error": "game.py timed out"},
            "llm_scores": None,
        }

    if result1.returncode != 0:
        return {
            "task_result": TaskResult(
                task_id=run.config.task,
                succeeded=False,
            ),
            "eval_report": None,
            "ground_truth": {
                "error": "game.py crashed",
                "exit_code": result1.returncode,
                "stderr": result1.stderr,
            },
            "llm_scores": None,
        }

    # Second run — determinism check.
    # Compare snapshot.json content (game state) rather than stdout text,
    # since stdout may contain non-deterministic formatting (e.g. dict
    # iteration order) that doesn't reflect actual game state differences.

    # Capture run 1's snapshot before run 2 overwrites it.
    snap_path = run._find_file("snapshot.json")
    snap1_content: str | None = None
    if snap_path is not None and snap_path.exists():
        snap1_content = snap_path.read_text(encoding="utf-8")

    try:
        result2 = _run_game_script(run.game_script, project_root=root)
    except subprocess.TimeoutExpired:
        result2 = None

    if result2 is not None:
        if snap1_content is not None and snap_path is not None and snap_path.exists():
            snap2_content = snap_path.read_text(encoding="utf-8")
            try:
                snap1_norm = json.dumps(json.loads(snap1_content), sort_keys=True)
                snap2_norm = json.dumps(json.loads(snap2_content), sort_keys=True)
                replay_deterministic = snap1_norm == snap2_norm
            except (json.JSONDecodeError, ValueError):
                hash1 = hashlib.sha256(result1.stdout.encode()).hexdigest()
                hash2 = hashlib.sha256(result2.stdout.encode()).hexdigest()
                replay_deterministic = hash1 == hash2
        else:
            hash1 = hashlib.sha256(result1.stdout.encode()).hexdigest()
            hash2 = hashlib.sha256(result2.stdout.encode()).hexdigest()
            replay_deterministic = hash1 == hash2
    else:
        replay_deterministic = False

    entity_count = _parse_entity_count(result1.stdout)

    # --- Structural validation (Tier 1) ---
    snapshot_found = run._find_file("snapshot.json")
    snapshot_path = snapshot_found if snapshot_found is not None else run.workdir / "snapshot.json"

    validation = None
    snapshot: SceneSnapshot | None = None
    if snapshot_path.exists():
        try:
            snap_data = json.loads(snapshot_path.read_text(encoding="utf-8"))
            snapshot = SceneSnapshot.from_dict(snap_data)
            validation = validate_breakout_snapshot(snapshot)
            if not validation.passed:
                logger.warning(
                    "Snapshot validation failed: %s",
                    ", ".join(validation.failures),
                )
        except Exception:
            logger.exception("Snapshot validation error — falling back to basic check")

    if validation is not None:
        succeeded = validation.passed and entity_count > 0
    else:
        # Fallback: no snapshot available, use basic check
        succeeded = result1.returncode == 0 and entity_count > 0

    task_result = TaskResult(
        task_id=run.config.task,
        succeeded=succeeded,
        replay_deterministic=replay_deterministic,
    )

    # --- LLM-judged deep scoring ---
    llm_scores = None
    if snapshot is not None and judge_model:
        assert snapshot is not None  # narrow type for pyright
        try:
            llm = ClaudeCodeLLMClient(model=judge_model)

            # Scene QA (Tier 2)
            scene_questions = generate_scene_questions(snapshot)
            scene_qa_result = scene_qa_accuracy(snapshot, scene_questions, llm)

            # G-Eval (Tier 3)
            geval_results = geval_all(snapshot, llm)

            # Multi-hop spatial (Tier 3)
            spatial_questions = generate_spatial_questions(snapshot)
            multihop_result = multihop_spatial_accuracy(snapshot, spatial_questions, llm)

            llm_scores = {
                "judge_model": judge_model,
                "scene_qa_accuracy": scene_qa_result.value,
            }
            for gr in geval_results:
                llm_scores[gr.name] = gr.value
            llm_scores["multihop_spatial_accuracy"] = multihop_result.value

        except Exception:
            logger.exception("Deep scoring failed — continuing with basic score")

    ground_truth: dict[str, object] = {
        "entity_count": entity_count,
        "replay_deterministic": replay_deterministic,
        "stdout_hash": hashlib.sha256(result1.stdout.encode()).hexdigest(),
    }
    if validation is not None:
        ground_truth["validation_passed"] = validation.passed
        ground_truth["validation_checks"] = validation.checks
        ground_truth["validation_details"] = validation.details

    return {
        "task_result": task_result,
        "eval_report": None,
        "ground_truth": ground_truth,
        "llm_scores": llm_scores,
    }


# ---------------------------------------------------------------------------
# run_agent_eval
# ---------------------------------------------------------------------------

def run_agent_eval(
    task: str = "breakout",
    model: str = "sonnet",
    max_budget_usd: float = 5.0,
    *,
    judge_model: str | None = None,
    project_root: Path | None = None,
) -> dict:
    """Run a complete agent evaluation: launch, score, and report.

    Args:
        task: GDD task name.
        model: Claude model alias.
        max_budget_usd: Maximum spend.
        judge_model: Model alias used for LLM-judged scoring.
        project_root: Override for the project root directory.

    Returns:
        Report dict with ``agent_meta``, ``task_result``, ``ground_truth``,
        and ``llm_scores``.
    """
    root = project_root or PROJECT_ROOT

    config = AgentConfig(
        task=task,
        model=model,
        judge_model=judge_model,
        max_budget_usd=max_budget_usd,
    )

    # --- Header ---
    print("=" * 60)
    print(f"  Agent Eval: task={config.task}  model={config.model}")
    print(f"  judge={config.judge_model or 'none (deep scoring off)'}")
    print(f"  budget=${config.max_budget_usd:.2f}  timeout={config.timeout_s}s")
    print("=" * 60)

    # --- Launch ---
    print("\n[1/3] Launching agent...")
    agent_run = launch_agent(config, project_root=root)
    print(f"  Agent finished: exit_code={agent_run.exit_code}  "
          f"wall_time={agent_run.wall_time_s:.1f}s  signal={agent_run.signal}")
    print(f"  game.py exists: {agent_run.game_exists}")

    # --- Save agent logs ---
    (agent_run.workdir / "agent_stdout.log").write_text(
        agent_run.stdout, encoding="utf-8"
    )
    (agent_run.workdir / "agent_stderr.log").write_text(
        agent_run.stderr, encoding="utf-8"
    )

    # --- Score ---
    print("\n[2/3] Scoring game...")
    score = score_game(agent_run, project_root=root, judge_model=config.judge_model)
    task_result: TaskResult = score["task_result"]
    print(f"  succeeded: {task_result.succeeded}")
    print(f"  replay_deterministic: {task_result.replay_deterministic}")
    if "entity_count" in score["ground_truth"]:
        print(f"  entity_count: {score['ground_truth']['entity_count']}")

    # Print structural validation results
    gt = score["ground_truth"]
    if "validation_checks" in gt:
        print("\n  Structural validation:")
        for check_name, passed in gt["validation_checks"].items():
            status = "PASS" if passed else "FAIL"
            detail = gt["validation_details"].get(check_name, "")
            print(f"    [{status}] {check_name}: {detail}")

    # --- Build report ---
    report = {
        "agent_meta": {
            "task": config.task,
            "model": config.model,
            "max_budget_usd": config.max_budget_usd,
            "wall_time_s": agent_run.wall_time_s,
            "exit_code": agent_run.exit_code,
            "signal": agent_run.signal,
        },
        "task_result": asdict(task_result),
        "ground_truth": score["ground_truth"],
        "llm_scores": score.get("llm_scores"),
    }

    # --- Save report ---
    report_path = root / "eval_agent_report.json"
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")

    # --- Verdict ---
    print("\n[3/3] Verdict")
    llm_scores = score.get("llm_scores")
    if llm_scores:
        print(f"\n  LLM Scores (judge={llm_scores['judge_model']}):")
        for key, val in llm_scores.items():
            if key != "judge_model":
                print(f"    {key}: {val:.2f}")
    if task_result.fully_succeeded:
        verdict = "PASS"
    elif task_result.succeeded:
        verdict = "PARTIAL"
    else:
        verdict = "FAIL"
    print(f"  >>> {verdict} <<<")
    print(f"  Report saved to: {report_path}")
    print("=" * 60)

    return report
