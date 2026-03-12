"""Autonomy dimension evaluation -- end-to-end capability.

Answers: "Can AI go from GDD to verified game without human help?"

Metrics:
- ``zero_touch_completion_rate``: Weighted success rate across GDD tasks
  (this IS the CW-ZTVCR north-star metric).
- ``convergence_median``: How many write-verify-fix iterations to green?
- ``human_intervention_count``: How often does a human need to step in?
"""

from __future__ import annotations

import logging
import statistics
from dataclasses import dataclass

from nomai.eval.metrics import EvalDimension, MetricResult

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class TaskResult:
    """Outcome of a single GDD-to-verified-game task.

    Attributes:
        task_id: Unique identifier for the GDD task.
        succeeded: Whether the task completed with all intents passing.
        complexity_weight: Weight for CW-ZTVCR computation (higher =
            more complex game).
        iterations: Number of write-verify-fix loop iterations.
        human_interventions: Number of times a human had to step in.
        replay_deterministic: Whether replay hashes matched.
        perf_gates_met: Whether manifest overhead was within budget.
    """

    task_id: str
    succeeded: bool
    complexity_weight: float = 1.0
    iterations: int = 0
    human_interventions: int = 0
    replay_deterministic: bool = True
    perf_gates_met: bool = True

    @property
    def fully_succeeded(self) -> bool:
        """A task fully succeeds only if ALL conditions hold."""
        return (
            self.succeeded
            and self.human_interventions == 0
            and self.replay_deterministic
            and self.perf_gates_met
        )


def zero_touch_completion_rate(
    results: list[TaskResult],
) -> MetricResult:
    """Complexity-Weighted Zero-Touch Verified Completion Rate (CW-ZTVCR).

    A task counts as success only if: intents pass, replay is
    deterministic, zero human intervention, and perf gates are met.

    Formula: ``sum(w_i * success_i) / sum(w_i)``

    Args:
        results: Outcomes of GDD-to-game tasks.

    Returns:
        MetricResult with the CW-ZTVCR value.  No fixed target (tracked
        by complexity tier).
    """
    if not results:
        return MetricResult(
            name="cw_ztvcr",
            dimension=EvalDimension.AUTONOMY,
            value=0.0,
            target=None,
            passed=True,
            detail="No tasks evaluated.",
        )

    total_weight = sum(r.complexity_weight for r in results)
    if total_weight == 0:
        return MetricResult(
            name="cw_ztvcr",
            dimension=EvalDimension.AUTONOMY,
            value=0.0,
            target=None,
            passed=True,
            detail="All tasks have zero weight.",
        )

    weighted_success = sum(
        r.complexity_weight for r in results if r.fully_succeeded
    )
    rate = weighted_success / total_weight
    succeeded = sum(1 for r in results if r.fully_succeeded)
    return MetricResult(
        name="cw_ztvcr",
        dimension=EvalDimension.AUTONOMY,
        value=rate,
        target=None,
        passed=True,  # Tracking metric, no fixed gate.
        detail=f"{succeeded}/{len(results)} tasks fully succeeded (weighted: {rate:.3f}).",
    )


def convergence_median(
    results: list[TaskResult],
) -> MetricResult:
    """Median number of write-verify-fix iterations to green.

    Only considers tasks that succeeded.

    Args:
        results: Task outcomes.

    Returns:
        MetricResult with median iterations.  Target <= 5.
    """
    succeeded = [r for r in results if r.succeeded]
    if not succeeded:
        return MetricResult(
            name="convergence_median",
            dimension=EvalDimension.AUTONOMY,
            value=0.0,
            target=5.0,
            passed=True,
            detail="No succeeded tasks to measure convergence.",
        )

    iters = [r.iterations for r in succeeded]
    med = statistics.median(iters)
    target = 5.0
    return MetricResult(
        name="convergence_median",
        dimension=EvalDimension.AUTONOMY,
        value=med,
        target=target,
        passed=med <= target,
        detail=f"Median {med:.1f} iterations across {len(succeeded)} succeeded tasks.",
    )


def human_intervention_count(
    results: list[TaskResult],
) -> MetricResult:
    """Mean number of human interventions per run.

    Args:
        results: Task outcomes.

    Returns:
        MetricResult with mean intervention count.  Target = 0.
    """
    if not results:
        return MetricResult(
            name="human_intervention_count",
            dimension=EvalDimension.AUTONOMY,
            value=0.0,
            target=0.0,
            passed=True,
            detail="No tasks evaluated.",
        )

    total = sum(r.human_interventions for r in results)
    mean = total / len(results)
    return MetricResult(
        name="human_intervention_count",
        dimension=EvalDimension.AUTONOMY,
        value=mean,
        target=0.0,
        passed=mean <= 0.0,
        detail=f"{total} total interventions across {len(results)} tasks (mean {mean:.2f}).",
    )
