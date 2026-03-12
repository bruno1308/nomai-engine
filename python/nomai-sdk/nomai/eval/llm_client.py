"""LLM client abstraction for eval metrics that need language model calls.

Provides a base ``LLMClient`` class and a ``MockLLMClient`` for deterministic
testing of Tier 2 / Tier 3 metrics (Scene QA, Action Prediction, G-Eval,
Multi-hop Spatial) without requiring a live LLM endpoint.
"""

from __future__ import annotations

import logging
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
