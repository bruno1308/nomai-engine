"""Reproducibility dimension evaluation -- determinism.

Answers: "Are simulations reliably reproducible?"

Metrics:
- ``replay_hash_match_rate``: Do replay checkpoints match originals?
- ``snapshot_fidelity``: Does restore-then-replay match the original
  trajectory?
"""

from __future__ import annotations

import logging
from dataclasses import dataclass

from nomai.eval.metrics import EvalDimension, MetricResult

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class HashCheckpoint:
    """A pair of expected vs actual state hashes at a checkpoint.

    Attributes:
        tick: Tick number of the checkpoint.
        expected_hash: Hash from the original run.
        actual_hash: Hash from the replay run.
    """

    tick: int
    expected_hash: str
    actual_hash: str

    @property
    def matches(self) -> bool:
        return self.expected_hash == self.actual_hash


def replay_hash_match_rate(
    checkpoints: list[HashCheckpoint],
) -> MetricResult:
    """Fraction of replay checkpoints whose hash matches the original.

    Args:
        checkpoints: Hash comparisons at each checkpoint tick.

    Returns:
        MetricResult with match rate.  Target = 1.0.
    """
    if not checkpoints:
        return MetricResult(
            name="replay_hash_match_rate",
            dimension=EvalDimension.REPRODUCIBILITY,
            value=1.0,
            target=1.0,
            passed=True,
            detail="No checkpoints to verify (vacuously correct).",
        )

    matched = sum(1 for c in checkpoints if c.matches)
    rate = matched / len(checkpoints)
    diverged = [c for c in checkpoints if not c.matches]
    diverge_detail = ""
    if diverged:
        first = diverged[0]
        diverge_detail = f" First divergence at tick {first.tick}."
    return MetricResult(
        name="replay_hash_match_rate",
        dimension=EvalDimension.REPRODUCIBILITY,
        value=rate,
        target=1.0,
        passed=rate >= 1.0,
        detail=f"{matched}/{len(checkpoints)} checkpoints match.{diverge_detail}",
    )


def snapshot_fidelity(
    pairs: list[tuple[str, str]],
) -> MetricResult:
    """Fraction of snapshot-restore-replay hashes matching the original.

    Args:
        pairs: List of ``(original_hash, restored_replay_hash)`` pairs.

    Returns:
        MetricResult with fidelity rate.  Target = 1.0.
    """
    if not pairs:
        return MetricResult(
            name="snapshot_fidelity",
            dimension=EvalDimension.REPRODUCIBILITY,
            value=1.0,
            target=1.0,
            passed=True,
            detail="No snapshot pairs to verify (vacuously correct).",
        )

    matched = sum(1 for orig, restored in pairs if orig == restored)
    rate = matched / len(pairs)
    return MetricResult(
        name="snapshot_fidelity",
        dimension=EvalDimension.REPRODUCIBILITY,
        value=rate,
        target=1.0,
        passed=rate >= 1.0,
        detail=f"{matched}/{len(pairs)} snapshot-restore-replay hashes match.",
    )
