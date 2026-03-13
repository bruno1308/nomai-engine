"""Agent eval harness — launches Claude Code as an autonomous game developer.

Provides ``AgentConfig`` for task parameters, ``AgentRun`` for capturing
results, and ``launch_agent`` to orchestrate the full pipeline: create an
isolated working directory, compose a prompt from the GDD + SDK reference,
and invoke ``claude -p`` with full tools access.
"""

from __future__ import annotations

import logging
import os
import subprocess
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

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
7. When you are satisfied the game works correctly, create a file called DONE.txt.
8. If you are stuck and cannot make further progress, create a file called STUCK.txt \
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
        max_budget_usd: Maximum spend for the agent session.
        timeout_s: Subprocess wall-clock timeout in seconds.
    """

    task: str
    model: str = "sonnet"
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

    # Create isolated workdir
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")
    workdir = root / "eval_workdir" / timestamp
    workdir.mkdir(parents=True, exist_ok=True)

    # Read GDD
    gdd_path = root / "eval_tasks" / f"{config.task}.md"
    gdd_content = gdd_path.read_text(encoding="utf-8")

    # Read SDK reference
    sdk_ref_path = root / "docs" / "ai" / "nomai-sdk-reference.md"
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
        "--add-dir", str(workdir),
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
            cwd=str(root),
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
