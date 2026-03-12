"""Controllability dimension evaluation -- API effectiveness.

Answers: "Can AI effectively drive the engine to desired states?"

Metrics:
- ``command_semantic_reliability``: Do commands produce the intended
  manifest deltas?
- ``action_effect_latency``: How many ticks between command and
  observable effect?
- ``api_capability_coverage``: What fraction of required action
  primitives are exposed?
"""

from __future__ import annotations

import logging
import math
from dataclasses import dataclass

from nomai.eval.metrics import EvalDimension, MetricResult

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class CommandResult:
    """Outcome of a single command issued to the engine.

    Attributes:
        command_desc: Human-readable command description.
        expected_delta: What the manifest should reflect.
        actual_delta: What the manifest actually reflects.
        matched: Whether expected == actual (within tolerance).
    """

    command_desc: str
    expected_delta: str
    actual_delta: str
    matched: bool


@dataclass(frozen=True)
class LatencyObservation:
    """Observed latency between issuing a command and seeing its effect.

    Attributes:
        command_tick: Tick at which the command was issued.
        effect_tick: Tick at which the effect was observed in the manifest.
    """

    command_tick: int
    effect_tick: int

    @property
    def latency(self) -> int:
        """Ticks between command and effect."""
        return self.effect_tick - self.command_tick


def command_semantic_reliability(
    results: list[CommandResult],
) -> MetricResult:
    """Fraction of commands that produced the intended manifest delta.

    Args:
        results: Outcomes of commands issued during the eval session.

    Returns:
        MetricResult with reliability rate.  Target >= 0.99.
    """
    if not results:
        return MetricResult(
            name="command_semantic_reliability",
            dimension=EvalDimension.CONTROLLABILITY,
            value=1.0,
            target=0.99,
            passed=True,
            detail="No commands evaluated (vacuously correct).",
        )

    matched = sum(1 for r in results if r.matched)
    rate = matched / len(results)
    target = 0.99
    return MetricResult(
        name="command_semantic_reliability",
        dimension=EvalDimension.CONTROLLABILITY,
        value=rate,
        target=target,
        passed=rate >= target,
        detail=f"{matched}/{len(results)} commands produced expected manifest delta.",
    )


def action_effect_latency(
    observations: list[LatencyObservation],
) -> MetricResult:
    """P95 latency (in ticks) between command and observable effect.

    Args:
        observations: Latency observations from the eval session.

    Returns:
        MetricResult with P95 latency.  Target <= 1 tick.
    """
    if not observations:
        return MetricResult(
            name="action_effect_latency_p95",
            dimension=EvalDimension.CONTROLLABILITY,
            value=0.0,
            target=1.0,
            passed=True,
            detail="No latency observations (vacuously correct).",
        )

    latencies = sorted(o.latency for o in observations)
    idx = max(0, math.ceil(len(latencies) * 0.95) - 1)
    p95 = float(latencies[idx])
    target = 1.0
    return MetricResult(
        name="action_effect_latency_p95",
        dimension=EvalDimension.CONTROLLABILITY,
        value=p95,
        target=target,
        passed=p95 <= target,
        detail=f"P95 latency: {p95:.1f} ticks across {len(observations)} observations.",
    )


def api_capability_coverage(
    required: set[str],
    exposed: set[str],
) -> MetricResult:
    """Fraction of required action primitives that are exposed by the API.

    Args:
        required: Set of capability names the AI needs.
        exposed: Set of capability names the engine exposes.

    Returns:
        MetricResult with coverage rate.  Target >= 0.95.
    """
    if not required:
        return MetricResult(
            name="api_capability_coverage",
            dimension=EvalDimension.CONTROLLABILITY,
            value=1.0,
            target=0.95,
            passed=True,
            detail="No required capabilities specified (vacuously correct).",
        )

    covered = required & exposed
    missing = required - exposed
    rate = len(covered) / len(required)
    target = 0.95
    missing_str = f" Missing: {', '.join(sorted(missing))}." if missing else ""
    return MetricResult(
        name="api_capability_coverage",
        dimension=EvalDimension.CONTROLLABILITY,
        value=rate,
        target=target,
        passed=rate >= target,
        detail=f"{len(covered)}/{len(required)} required capabilities exposed.{missing_str}",
    )
