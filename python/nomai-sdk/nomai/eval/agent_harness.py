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

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Module-level constants
# ---------------------------------------------------------------------------

PROJECT_ROOT: Path = Path(__file__).resolve().parents[4]

AGENT_SYSTEM_PROMPT: str = """\
You are an expert game developer using the Nomai engine.

Your task:
1. Read the Game Design Document (GDD) provided in the prompt.
2. Read the SDK reference provided in the prompt.
3. Write a Python script called game.py that implements the game.
4. Run the script to verify it works.
5. Use snapshot.summary() to inspect the game state visually.
6. Fix any issues you find by iterating on the script.
7. Save the final snapshot as JSON for scoring:
   import json
   with open("snapshot.json", "w") as f:
       json.dump(snapshot.to_dict(), f)
8. When you are satisfied the game works correctly, create a file called DONE.txt.
9. If you are stuck and cannot make further progress, create a file called STUCK.txt \
with a description of what went wrong.

Important notes:
- Read run_eval_baseline.py first to understand the engine patterns.
- Use snapshot.summary() to inspect game state — the engine is headless, there is no GUI.
- Register all components before spawning entities.
- Call init_physics() before creating any physics bodies.
- Tick once after spawning entities to apply the spawns before registering physics bodies.
- The output script must be saved as game.py in your working directory.
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
    judge_model: str = "sonnet"
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

    @property
    def game_script(self) -> Path:
        """Path to the agent's output game script."""
        return self.workdir / "game.py"

    @property
    def game_exists(self) -> bool:
        """Whether the agent produced a game.py file."""
        return self.game_script.exists()

    @property
    def signal(self) -> str:
        """Infer the agent's completion signal.

        Returns:
            ``"DONE"`` if DONE.txt exists, ``"STUCK"`` if STUCK.txt exists,
            ``"TIMEOUT"`` if the exit code is non-zero, else ``"UNKNOWN"``.
        """
        if (self.workdir / "DONE.txt").exists():
            return "DONE"
        if (self.workdir / "STUCK.txt").exists():
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

    # Build user prompt
    prompt = (
        f"## Game Design Document\n\n{gdd_content}\n\n"
        f"## SDK Reference\n\n{sdk_ref_content}\n\n"
        f"## Output Instructions\n\n"
        f"Write your game script as `game.py` in the working directory.\n"
        f"Create DONE.txt when finished, or STUCK.txt if you cannot proceed.\n"
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
# score_game
# ---------------------------------------------------------------------------

def score_game(run: AgentRun, *, project_root: Path | None = None) -> dict:
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
        }

    # Second run — determinism check
    try:
        result2 = _run_game_script(run.game_script, project_root=root)
    except subprocess.TimeoutExpired:
        result2 = None

    if result2 is not None:
        hash1 = hashlib.sha256(result1.stdout.encode()).hexdigest()
        hash2 = hashlib.sha256(result2.stdout.encode()).hexdigest()
        replay_deterministic = hash1 == hash2
    else:
        replay_deterministic = False

    entity_count = _parse_entity_count(result1.stdout)
    succeeded = result1.returncode == 0 and entity_count > 0

    task_result = TaskResult(
        task_id=run.config.task,
        succeeded=succeeded,
        replay_deterministic=replay_deterministic,
    )

    return {
        "task_result": task_result,
        "eval_report": None,
        "ground_truth": {
            "entity_count": entity_count,
            "replay_deterministic": replay_deterministic,
            "stdout_hash": hashlib.sha256(result1.stdout.encode()).hexdigest(),
        },
    }


# ---------------------------------------------------------------------------
# run_agent_eval
# ---------------------------------------------------------------------------

def run_agent_eval(
    task: str = "breakout",
    model: str = "sonnet",
    max_budget_usd: float = 5.0,
    *,
    project_root: Path | None = None,
) -> dict:
    """Run a complete agent evaluation: launch, score, and report.

    Args:
        task: GDD task name.
        model: Claude model alias.
        max_budget_usd: Maximum spend.
        project_root: Override for the project root directory.

    Returns:
        Report dict with ``agent_meta``, ``task_result``, and ``ground_truth``.
    """
    root = project_root or PROJECT_ROOT

    config = AgentConfig(
        task=task,
        model=model,
        max_budget_usd=max_budget_usd,
    )

    # --- Header ---
    print("=" * 60)
    print(f"  Agent Eval: task={config.task}  model={config.model}")
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
    score = score_game(agent_run, project_root=root)
    task_result: TaskResult = score["task_result"]
    print(f"  succeeded: {task_result.succeeded}")
    print(f"  replay_deterministic: {task_result.replay_deterministic}")
    if "entity_count" in score["ground_truth"]:
        print(f"  entity_count: {score['ground_truth']['entity_count']}")

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
    }

    # --- Save report ---
    report_path = root / "eval_agent_report.json"
    report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")

    # --- Verdict ---
    print("\n[3/3] Verdict")
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
