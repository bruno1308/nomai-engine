# Agent Eval Harness Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a harness that launches Claude Code as an autonomous game developer, lets it build a breakout game from a GDD, then scores the result with the existing eval framework.

**Architecture:** A Python script (`run_agent_eval.py`) creates an isolated workdir, launches `claude -p` with the GDD + SDK reference as context, waits for it to finish, then independently runs and scores the agent's game script using `EvalRunner`. The agent uses snapshot-only self-verification (no eval framework access). Results feed into real `TaskResult` for the autonomy dimension.

**Tech Stack:** Python 3.13, subprocess (claude CLI), existing nomai eval framework, pytest

**Design doc:** `docs/plans/2026-03-14-agent-eval-harness-design.md`

---

### Task 1: Create the Breakout GDD Task Spec

**Files:**
- Create: `eval_tasks/breakout.md`

**Step 1: Create the eval_tasks directory and GDD file**

```markdown
# Task: Breakout Game

## Game Description
A classic breakout game. A paddle at the bottom of the screen, a ball
that bounces, and a grid of bricks at the top. The ball destroys bricks
on contact.

## Required Entities
- 1 paddle (type: "character", role: "paddle")
  - Position: bottom center (x=400, y=560)
  - Size: width=100, height=15
- 1 ball (type: "projectile", role: "ball")
  - Position: center area (x=400, y=300)
  - Size: 8x8
  - Initial velocity: dx=200, dy=-300
- 20 bricks (type: "destructible", role: "brick")
  - Layout: 4 rows x 5 columns
  - Each brick: width=60, height=20, spacing=10
  - Starting Y around 60, centered horizontally
- 3 boundary walls (type: "boundary")
  - wall_top (role: "wall_top"): top edge
  - wall_left (role: "wall_left"): left edge
  - wall_right (role: "wall_right"): right edge

## Game Area
- Width: 800, Height: 600

## Required Behaviors
- Ball bounces off paddle, walls, and bricks (restitution=1.0)
- Ball destroys brick on contact (despawn the brick entity)
- Ball velocity is preserved through bounces
- Paddle is kinematic (does not move from physics)
- Walls and bricks are static

## Physics Setup
- Use `init_physics()` before registering physics bodies
- Ball: dynamic body, circle collider (radius=8)
- Paddle: kinematic body, box collider
- Bricks: static body, box collider
- Walls: static body, box collider

## WASM Gameplay
- Load from: `gameplay/build/gameplay.wasm`

## Simulation
- Run for 300 ticks with `fixed_dt=1.0/60.0`
- During simulation, check each tick's manifest for collision events
- When ball hits a brick (collision event involving both), despawn the brick

## Success Criteria
After 300 ticks:
- All entity types exist with correct roles
- Ball has moved from starting position
- Physics collisions are working (ball bounces off walls/paddle)
- At least some bricks have been destroyed

## Output
Your script must be a single Python file at the path specified by the harness.
The script must:
1. Create the engine with `NomaiEngine(headless=True, fixed_dt=1.0/60.0)`
2. Register components, init physics, spawn all entities
3. Register physics bodies for all entities
4. Load the WASM gameplay
5. Run for 300 ticks with collision-based brick despawning
6. Print the final `engine.scene_snapshot().summary()` to stdout
7. Print "ENTITY_COUNT: <N>" as the last line of stdout

## Reference
- See `docs/ai/nomai-sdk-reference.md` for the full API reference
- See `run_eval_baseline.py` for a complete working example
```

**Step 2: Commit**

```bash
git add eval_tasks/breakout.md
git commit -m "feat: add breakout GDD task spec for agent eval"
```

---

### Task 2: Agent Harness — Core Launcher

**Files:**
- Create: `python/nomai-sdk/nomai/eval/agent_harness.py`
- Test: `python/nomai-sdk/tests/test_eval_agent_harness.py`

**Step 1: Write the failing test for AgentConfig and launch_agent**

```python
"""Tests for the agent eval harness."""

from __future__ import annotations

import os
import json
from pathlib import Path
from unittest.mock import patch, MagicMock

from nomai.eval.agent_harness import AgentConfig, launch_agent, AgentRun


class TestAgentConfig:
    def test_defaults(self) -> None:
        cfg = AgentConfig(task="breakout")
        assert cfg.task == "breakout"
        assert cfg.model == "sonnet"
        assert cfg.max_budget_usd == 5.0
        assert cfg.timeout_s == 600

    def test_custom(self) -> None:
        cfg = AgentConfig(task="breakout", model="opus", max_budget_usd=10.0)
        assert cfg.model == "opus"
        assert cfg.max_budget_usd == 10.0


class TestLaunchAgent:
    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_creates_workdir_and_launches(self, mock_run: MagicMock, tmp_path: Path) -> None:
        mock_run.return_value = MagicMock(returncode=0, stdout="done", stderr="")
        cfg = AgentConfig(task="breakout")
        # Create a minimal GDD
        task_dir = tmp_path / "eval_tasks"
        task_dir.mkdir()
        (task_dir / "breakout.md").write_text("# Breakout GDD")
        sdk_ref = tmp_path / "docs" / "ai"
        sdk_ref.mkdir(parents=True)
        (sdk_ref / "nomai-sdk-reference.md").write_text("# SDK Ref")

        run = launch_agent(cfg, project_root=tmp_path)
        assert isinstance(run, AgentRun)
        assert run.exit_code == 0
        assert run.workdir.exists()
        # Verify claude was called
        assert mock_run.called
        cmd = mock_run.call_args[0][0]
        assert cmd[0] == "claude"
        assert "-p" in cmd

    @patch("nomai.eval.agent_harness.subprocess.run")
    def test_strips_claudecode_env(self, mock_run: MagicMock, tmp_path: Path) -> None:
        mock_run.return_value = MagicMock(returncode=0, stdout="", stderr="")
        cfg = AgentConfig(task="breakout")
        task_dir = tmp_path / "eval_tasks"
        task_dir.mkdir()
        (task_dir / "breakout.md").write_text("# GDD")
        sdk_ref = tmp_path / "docs" / "ai"
        sdk_ref.mkdir(parents=True)
        (sdk_ref / "nomai-sdk-reference.md").write_text("# ref")

        with patch.dict(os.environ, {"CLAUDECODE": "1"}):
            launch_agent(cfg, project_root=tmp_path)

        env = mock_run.call_args[1]["env"]
        assert "CLAUDECODE" not in env
```

**Step 2: Run tests to verify they fail**

Run: `cd B:/Projects/Nomai && python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py -v`
Expected: FAIL (module not found)

**Step 3: Implement AgentConfig, AgentRun, and launch_agent**

```python
"""Agent eval harness — launches Claude Code to build a game from a GDD.

Usage::

    from nomai.eval.agent_harness import AgentConfig, launch_agent, score_game
    cfg = AgentConfig(task="breakout", model="sonnet")
    run = launch_agent(cfg)
    result = score_game(run)
"""

from __future__ import annotations

import logging
import os
import subprocess
import time
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path

logger = logging.getLogger(__name__)

PROJECT_ROOT = Path(__file__).resolve().parents[4]  # nomai-sdk -> nomai -> python -> Nomai

AGENT_SYSTEM_PROMPT = """\
You are a game developer building a game with the Nomai engine.

Your job:
1. Read the Game Design Document (GDD) carefully
2. Read the SDK reference to learn the API
3. Write a Python script that creates the game
4. Run the script to verify it works
5. Read the SceneSnapshot output to verify the game state is correct
6. If something is wrong, fix and re-run

When you are satisfied the game is correct, create a file called DONE.txt
in the output directory with the word "done".

If you get stuck and cannot proceed, create a file called STUCK.txt
explaining what went wrong.

IMPORTANT:
- Always read run_eval_baseline.py first — it is a complete working example
- Use snapshot.summary() to verify your game state (not visual output)
- The engine is headless — there is no window or renderer
- You must register components BEFORE spawning entities
- You must init_physics() BEFORE registering physics bodies
- You must tick() once after spawning to apply the spawns
"""


@dataclass(frozen=True)
class AgentConfig:
    """Configuration for an agent eval run."""
    task: str
    model: str = "sonnet"
    max_budget_usd: float = 5.0
    timeout_s: int = 600


@dataclass
class AgentRun:
    """Result of launching the agent (before scoring)."""
    config: AgentConfig
    workdir: Path
    exit_code: int
    wall_time_s: float
    stdout: str
    stderr: str

    @property
    def game_script(self) -> Path:
        return self.workdir / "game.py"

    @property
    def game_exists(self) -> bool:
        return self.game_script.exists()

    @property
    def signal(self) -> str:
        if (self.workdir / "DONE.txt").exists():
            return "DONE"
        if (self.workdir / "STUCK.txt").exists():
            return "STUCK"
        return "TIMEOUT" if self.exit_code != 0 else "UNKNOWN"


def launch_agent(
    config: AgentConfig,
    *,
    project_root: Path | None = None,
) -> AgentRun:
    """Launch Claude Code to build a game from a GDD."""
    root = project_root or PROJECT_ROOT
    run_id = datetime.now().strftime("%Y%m%d_%H%M%S")
    workdir = root / "eval_workdir" / run_id
    workdir.mkdir(parents=True, exist_ok=True)

    # Load GDD and SDK reference
    gdd = (root / "eval_tasks" / f"{config.task}.md").read_text()
    sdk_ref = (root / "docs" / "ai" / "nomai-sdk-reference.md").read_text()

    prompt = (
        f"{gdd}\n\n"
        f"---\n\n"
        f"# SDK Reference\n\n{sdk_ref}\n\n"
        f"---\n\n"
        f"Output your game script to: {workdir}/game.py\n"
        f"When done, create: {workdir}/DONE.txt\n"
        f"If stuck, create: {workdir}/STUCK.txt\n"
    )

    cmd = [
        "claude", "-p", prompt,
        "--system-prompt", AGENT_SYSTEM_PROMPT,
        "--model", config.model,
        "--dangerously-skip-permissions",
        "--max-budget-usd", str(config.max_budget_usd),
        "--add-dir", str(workdir),
    ]

    env = os.environ.copy()
    env.pop("CLAUDECODE", None)

    logger.info("Launching agent: task=%s model=%s workdir=%s",
                config.task, config.model, workdir)

    t0 = time.monotonic()
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=config.timeout_s,
            env=env,
            cwd=str(root),
        )
        exit_code = result.returncode
        stdout = result.stdout
        stderr = result.stderr
    except subprocess.TimeoutExpired:
        exit_code = -1
        stdout = ""
        stderr = "Agent timed out"
    wall_time = time.monotonic() - t0

    logger.info("Agent finished: exit=%d wall=%.1fs signal=%s",
                exit_code, wall_time,
                "DONE" if (workdir / "DONE.txt").exists() else "?")

    return AgentRun(
        config=config,
        workdir=workdir,
        exit_code=exit_code,
        wall_time_s=wall_time,
        stdout=stdout,
        stderr=stderr,
    )
```

**Step 4: Run tests to verify they pass**

Run: `cd B:/Projects/Nomai && python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py -v`
Expected: PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/agent_harness.py python/nomai-sdk/tests/test_eval_agent_harness.py
git commit -m "feat: add agent eval harness core launcher"
```

---

### Task 3: Scoring Phase — Run and Evaluate the Agent's Game

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py`
- Test: `python/nomai-sdk/tests/test_eval_agent_harness.py`

**Step 1: Write the failing test for score_game**

Add to `test_eval_agent_harness.py`:

```python
class TestScoreGame:
    def test_missing_game_script_returns_failed(self, tmp_path: Path) -> None:
        from nomai.eval.agent_harness import score_game
        run = AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=tmp_path,
            exit_code=0,
            wall_time_s=10.0,
            stdout="",
            stderr="",
        )
        result = score_game(run)
        assert result["task_result"].succeeded is False

    def test_broken_script_returns_failed(self, tmp_path: Path) -> None:
        from nomai.eval.agent_harness import score_game
        (tmp_path / "game.py").write_text("raise RuntimeError('boom')")
        run = AgentRun(
            config=AgentConfig(task="breakout"),
            workdir=tmp_path,
            exit_code=0,
            wall_time_s=10.0,
            stdout="",
            stderr="",
        )
        result = score_game(run)
        assert result["task_result"].succeeded is False
        assert "boom" in result["task_result"].task_id or not result["task_result"].succeeded
```

**Step 2: Run test to verify it fails**

Run: `cd B:/Projects/Nomai && python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py::TestScoreGame -v`
Expected: FAIL

**Step 3: Implement score_game**

Add to `agent_harness.py`:

```python
import hashlib
import sys

from nomai.eval.autonomy import TaskResult
from nomai.eval.runner import EvalRunner


def _run_game_script(script_path: Path, timeout: int = 120) -> tuple[int, str, str]:
    """Run a game script as a subprocess, return (exit_code, stdout, stderr)."""
    result = subprocess.run(
        [sys.executable, str(script_path)],
        capture_output=True,
        text=True,
        timeout=timeout,
        cwd=str(script_path.parent),
    )
    return result.returncode, result.stdout, result.stderr


def score_game(run: AgentRun) -> dict:
    """Score the agent's game script against eval metrics.

    Returns a dict with keys:
        task_result: TaskResult for the autonomy dimension
        eval_report: EvalReport (or None if script didn't run)
        ground_truth: dict with comparison data
    """
    if not run.game_exists:
        return {
            "task_result": TaskResult(
                task_id=run.config.task,
                succeeded=False,
                iterations=1,
                human_interventions=0,
            ),
            "eval_report": None,
            "ground_truth": {"error": "game.py not found"},
        }

    # Run the game script twice for reproducibility check
    try:
        exit1, stdout1, stderr1 = _run_game_script(run.game_script)
    except subprocess.TimeoutExpired:
        return {
            "task_result": TaskResult(
                task_id=run.config.task,
                succeeded=False,
                iterations=1,
            ),
            "eval_report": None,
            "ground_truth": {"error": "game.py timed out"},
        }

    if exit1 != 0:
        return {
            "task_result": TaskResult(
                task_id=run.config.task,
                succeeded=False,
                iterations=1,
            ),
            "eval_report": None,
            "ground_truth": {"error": f"game.py crashed: {stderr1[:500]}"},
        }

    # Second run for determinism check
    try:
        exit2, stdout2, _ = _run_game_script(run.game_script)
        replay_deterministic = (exit2 == 0 and
                                hashlib.sha256(stdout1.encode()).hexdigest() ==
                                hashlib.sha256(stdout2.encode()).hexdigest())
    except (subprocess.TimeoutExpired, Exception):
        replay_deterministic = False

    # Parse entity count from stdout (last line: "ENTITY_COUNT: N")
    entity_count = 0
    for line in reversed(stdout1.strip().splitlines()):
        if line.startswith("ENTITY_COUNT:"):
            try:
                entity_count = int(line.split(":")[1].strip())
            except ValueError:
                pass
            break

    # Basic success: script ran and produced entities
    succeeded = exit1 == 0 and entity_count > 0

    task_result = TaskResult(
        task_id=run.config.task,
        succeeded=succeeded,
        complexity_weight=1.0,
        iterations=1,
        human_interventions=0,
        replay_deterministic=replay_deterministic,
        perf_gates_met=True,
    )

    return {
        "task_result": task_result,
        "eval_report": None,  # Full eval integration in Task 4
        "ground_truth": {
            "entity_count": entity_count,
            "script_exit_code": exit1,
            "replay_deterministic": replay_deterministic,
            "stdout_preview": stdout1[:1000],
        },
    }
```

**Step 4: Run tests to verify they pass**

Run: `cd B:/Projects/Nomai && python -m pytest python/nomai-sdk/tests/test_eval_agent_harness.py -v`
Expected: PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/agent_harness.py python/nomai-sdk/tests/test_eval_agent_harness.py
git commit -m "feat: add score_game to agent eval harness"
```

---

### Task 4: Full Report — run_agent_eval Entry Point

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/agent_harness.py`
- Create: `run_agent_eval.py` (top-level script)

**Step 1: Add run_agent_eval function to agent_harness.py**

```python
import json


def run_agent_eval(
    task: str = "breakout",
    model: str = "sonnet",
    max_budget_usd: float = 5.0,
    *,
    project_root: Path | None = None,
) -> dict:
    """Full agent eval: launch agent, score result, save report.

    Returns the complete report dict.
    """
    config = AgentConfig(
        task=task,
        model=model,
        max_budget_usd=max_budget_usd,
    )

    print(f"\n{'='*60}")
    print(f"  AGENT EVAL: {task} (model={model}, budget=${max_budget_usd})")
    print(f"{'='*60}\n")

    # Launch the agent
    print("[1/3] Launching agent...")
    run = launch_agent(config, project_root=project_root)
    print(f"  Exit code: {run.exit_code}")
    print(f"  Wall time: {run.wall_time_s:.1f}s")
    print(f"  Signal: {run.signal}")
    print(f"  Game script exists: {run.game_exists}")

    # Score the result
    print("\n[2/3] Scoring agent output...")
    result = score_game(run)
    tr = result["task_result"]
    print(f"  Succeeded: {tr.succeeded}")
    print(f"  Fully succeeded: {tr.fully_succeeded}")
    print(f"  Replay deterministic: {tr.replay_deterministic}")

    # Build report
    report = {
        "agent_meta": {
            "model": config.model,
            "task": config.task,
            "wall_time_s": run.wall_time_s,
            "max_budget_usd": config.max_budget_usd,
            "exit_code": run.exit_code,
            "signal": run.signal,
            "game_script": str(run.game_script) if run.game_exists else None,
            "workdir": str(run.workdir),
        },
        "task_result": {
            "task_id": tr.task_id,
            "succeeded": tr.succeeded,
            "fully_succeeded": tr.fully_succeeded,
            "complexity_weight": tr.complexity_weight,
            "iterations": tr.iterations,
            "human_interventions": tr.human_interventions,
            "replay_deterministic": tr.replay_deterministic,
            "perf_gates_met": tr.perf_gates_met,
        },
        "ground_truth": result["ground_truth"],
    }

    # Save report
    root = project_root or PROJECT_ROOT
    report_path = root / "eval_agent_report.json"
    print(f"\n[3/3] Saving report to {report_path}")
    report_path.write_text(json.dumps(report, indent=2, default=str))

    # Verdict
    print(f"\n{'='*60}")
    if tr.fully_succeeded:
        print("  VERDICT: PASS - Agent fully succeeded")
    elif tr.succeeded:
        print("  VERDICT: PARTIAL - Game works but not fully autonomous")
    else:
        print("  VERDICT: FAIL - Agent did not produce a working game")
    print(f"{'='*60}\n")

    return report
```

**Step 2: Create the top-level run_agent_eval.py script**

```python
#!/usr/bin/env python3
"""Run the agent eval harness.

Launches Claude Code as an autonomous game developer, lets it build a
breakout game from a GDD, then scores the result.

Usage::

    python run_agent_eval.py
    python run_agent_eval.py --model opus --budget 10
    python run_agent_eval.py --task breakout --model haiku
"""

from __future__ import annotations

import argparse
import sys

from nomai.eval.agent_harness import run_agent_eval


def main() -> int:
    parser = argparse.ArgumentParser(description="Run agent eval harness")
    parser.add_argument("--task", default="breakout", help="GDD task name")
    parser.add_argument("--model", default="sonnet", help="Claude model")
    parser.add_argument("--budget", type=float, default=5.0,
                        help="Max budget in USD")
    args = parser.parse_args()

    report = run_agent_eval(
        task=args.task,
        model=args.model,
        max_budget_usd=args.budget,
    )

    tr = report["task_result"]
    return 0 if tr["fully_succeeded"] else 1


if __name__ == "__main__":
    sys.exit(main())
```

**Step 3: Run full test suite to verify nothing broke**

Run: `cd B:/Projects/Nomai && python -m pytest python/nomai-sdk/tests/ -q`
Expected: All tests pass

**Step 4: Commit**

```bash
git add python/nomai-sdk/nomai/eval/agent_harness.py run_agent_eval.py
git commit -m "feat: add run_agent_eval.py entry point for agent eval harness"
```

---

### Task 5: Wire into eval __init__.py and Add .gitignore for Workdir

**Files:**
- Modify: `python/nomai-sdk/nomai/eval/__init__.py`
- Create: `eval_workdir/.gitignore`
- Create: `eval_tasks/.gitkeep`

**Step 1: Add exports to __init__.py**

Add to the imports:
```python
from nomai.eval.agent_harness import AgentConfig, AgentRun, launch_agent, score_game, run_agent_eval
```

Add to `__all__`:
```python
"AgentConfig",
"AgentRun",
"launch_agent",
"score_game",
"run_agent_eval",
```

**Step 2: Create eval_workdir/.gitignore**

```
# Ignore agent eval outputs (regenerated each run)
*
!.gitignore
```

**Step 3: Create eval_tasks/.gitkeep** (directory already has breakout.md from Task 1)

No action needed — directory already exists with breakout.md.

**Step 4: Run full test suite**

Run: `cd B:/Projects/Nomai && python -m pytest python/nomai-sdk/tests/ -q`
Expected: All tests pass

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/eval/__init__.py eval_workdir/.gitignore
git commit -m "feat: wire agent harness into eval exports, add workdir gitignore"
```

---

### Task 6: Dry Run — Verify the Full Pipeline (Manual)

This task is a manual verification that the full pipeline works end-to-end.

**Step 1: Verify all files exist**

```bash
ls eval_tasks/breakout.md
ls docs/ai/nomai-sdk-reference.md
ls run_agent_eval.py
ls python/nomai-sdk/nomai/eval/agent_harness.py
```

**Step 2: Run the harness with minimal budget to test plumbing**

```bash
python run_agent_eval.py --model haiku --budget 1.0
```

**Step 3: Check output**

- Does `eval_workdir/<timestamp>/` exist?
- Does `eval_agent_report.json` exist?
- Did the agent create `game.py`?
- Did scoring run without errors?

**Step 4: Review the report**

```bash
cat eval_agent_report.json
```

**Step 5: Commit any fixes needed**

```bash
git add -A
git commit -m "fix: agent eval harness dry-run fixes"
```

---

## Files Summary

| File | Action | Purpose |
|------|--------|---------|
| `eval_tasks/breakout.md` | Create | GDD task spec |
| `docs/ai/nomai-sdk-reference.md` | Already created | SDK reference for agent |
| `python/nomai-sdk/nomai/eval/agent_harness.py` | Create | Harness core: launch + score |
| `python/nomai-sdk/tests/test_eval_agent_harness.py` | Create | Tests for harness |
| `run_agent_eval.py` | Create | Top-level entry point |
| `python/nomai-sdk/nomai/eval/__init__.py` | Modify | Add exports |
| `eval_workdir/.gitignore` | Create | Ignore generated outputs |
