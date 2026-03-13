"""LLM client abstraction for eval metrics that need language model calls.

Provides a base ``LLMClient`` class, a ``MockLLMClient`` for deterministic
testing, and a ``ClaudeCodeLLMClient`` that shells out to the local
``claude`` CLI in print mode for real LLM completions.
"""

from __future__ import annotations

import logging
import os
import subprocess
from dataclasses import dataclass, field

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Base class
# ---------------------------------------------------------------------------

class LLMClient:
    """Base class for language-model completions used by eval metrics."""

    def complete(self, system: str, prompt: str) -> str:
        """Return a completion given a *system* message and a *prompt*.

        Subclasses must override this method.
        """
        raise NotImplementedError


# ---------------------------------------------------------------------------
# Mock implementation
# ---------------------------------------------------------------------------

@dataclass
class MockLLMClient(LLMClient):
    """Deterministic mock that cycles through pre-configured responses.

    Attributes:
        responses: Ordered list of responses to return.  Cycles when
            exhausted unless *strict* is ``True``.
        history: Recorded ``(system, prompt)`` pairs for every call.
        strict: When ``True``, raise ``IndexError`` once all responses
            have been consumed instead of cycling.
    """

    responses: list[str] = field(default_factory=lambda: ["mock response"])
    history: list[tuple[str, str]] = field(default_factory=list)
    strict: bool = False
    _call_index: int = field(default=0, repr=False)

    def complete(self, system: str, prompt: str) -> str:
        """Return the next configured response and record the call."""
        self.history.append((system, prompt))
        if self.strict and self._call_index >= len(self.responses):
            raise IndexError(
                f"MockLLMClient exhausted: {self._call_index} calls but only "
                f"{len(self.responses)} responses configured"
            )
        response = self.responses[self._call_index % len(self.responses)]
        self._call_index += 1
        return response


# ---------------------------------------------------------------------------
# Claude Code CLI implementation
# ---------------------------------------------------------------------------

@dataclass
class ClaudeCodeLLMClient(LLMClient):
    """LLM client that shells out to the local ``claude`` CLI in print mode.

    Requires the ``claude`` CLI to be installed and authenticated.
    Uses ``claude -p`` (non-interactive print mode) with ``--system-prompt``
    for the system message and the positional argument for the user prompt.

    Attributes:
        model: Model alias to pass via ``--model`` (e.g. ``"haiku"``,
            ``"sonnet"``, ``"opus"``).  Defaults to ``"sonnet"``.
        timeout: Subprocess timeout in seconds.  Defaults to 60.
        max_tokens: Not used by CLI but reserved for future use.
    """

    model: str = "sonnet"
    timeout: int = 60

    def complete(self, system: str, prompt: str) -> str:
        """Call ``claude -p`` and return its stdout as the completion."""
        cmd = [
            "claude",
            "-p",
            prompt,
            "--system-prompt", system,
            "--model", self.model,
            "--no-session-persistence",
            "--tools", "",
        ]

        env = os.environ.copy()
        # Allow calling claude from within a Claude Code session
        env.pop("CLAUDECODE", None)

        logger.debug("ClaudeCodeLLMClient calling: %s", " ".join(cmd[:6]))
        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=self.timeout,
                env=env,
            )
        except FileNotFoundError:
            raise RuntimeError(
                "claude CLI not found. Install Claude Code: "
                "https://docs.anthropic.com/en/docs/claude-code"
            )
        except subprocess.TimeoutExpired:
            raise RuntimeError(
                f"claude CLI timed out after {self.timeout}s"
            )

        if result.returncode != 0:
            stderr = result.stderr.strip()
            raise RuntimeError(
                f"claude CLI exited with code {result.returncode}: {stderr}"
            )

        return result.stdout.strip()
