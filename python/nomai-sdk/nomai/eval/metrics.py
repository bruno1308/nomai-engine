"""Metric definitions and computation for the Nomai eval framework.

Every evaluation produces ``MetricResult`` instances grouped into
``DimensionScore`` objects.  All types are JSON-serializable for
regression storage and CI reporting.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from enum import Enum
from typing import Self

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Evaluation dimensions
# ---------------------------------------------------------------------------

class EvalDimension(Enum):
    """The six evaluation dimensions of the Nomai engine."""

    OBSERVABILITY = "observability"
    CONTROLLABILITY = "controllability"
    REPRODUCIBILITY = "reproducibility"
    VERIFICATION = "verification"
    AUTONOMY = "autonomy"
    EFFICIENCY = "efficiency"


# ---------------------------------------------------------------------------
# MetricResult
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class MetricResult:
    """The result of evaluating a single metric.

    Attributes:
        name: Machine-readable metric identifier.
        dimension: Which evaluation dimension this metric belongs to.
        value: Measured value (0.0-1.0 for rates, raw for counts/times).
        target: MVP target threshold (``None`` if tracking-only).
        passed: Whether *value* meets *target*.
        detail: Human-readable explanation of the result.
    """

    name: str
    dimension: EvalDimension
    value: float
    target: float | None
    passed: bool
    detail: str

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "name": self.name,
            "dimension": self.dimension.value,
            "value": self.value,
            "target": self.target,
            "passed": self.passed,
            "detail": self.detail,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_target = data.get("target")
        target: float | None = None
        if raw_target is not None:
            target = float(raw_target)  # type: ignore[arg-type]
        return cls(
            name=str(data["name"]),
            dimension=EvalDimension(str(data["dimension"])),
            value=float(data["value"]),  # type: ignore[arg-type]
            target=target,
            passed=bool(data["passed"]),
            detail=str(data["detail"]),
        )


# ---------------------------------------------------------------------------
# DimensionScore
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class DimensionScore:
    """Aggregated score for a single evaluation dimension.

    Attributes:
        dimension: The evaluation dimension.
        metrics: Individual metric results in this dimension.
        score: Fraction of metrics that passed (0.0-1.0).
        passed: True only if **all** metrics with a target passed.
    """

    dimension: EvalDimension
    metrics: list[MetricResult] = field(default_factory=list)
    score: float = 0.0
    passed: bool = False

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "dimension": self.dimension.value,
            "metrics": [m.to_dict() for m in self.metrics],
            "score": self.score,
            "passed": self.passed,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_metrics = data.get("metrics", [])
        metrics: list[MetricResult] = []
        if isinstance(raw_metrics, list):
            metrics = [MetricResult.from_dict(m) for m in raw_metrics]  # type: ignore[arg-type]
        return cls(
            dimension=EvalDimension(str(data["dimension"])),
            metrics=metrics,
            score=float(data.get("score", 0.0)),  # type: ignore[arg-type]
            passed=bool(data.get("passed", False)),
        )

    @classmethod
    def from_metrics(cls, dimension: EvalDimension, metrics: list[MetricResult]) -> Self:
        """Compute a dimension score from a list of metric results."""
        if not metrics:
            return cls(dimension=dimension, metrics=[], score=0.0, passed=True)
        targeted = [m for m in metrics if m.target is not None]
        passed_count = sum(1 for m in targeted if m.passed)
        score = passed_count / len(targeted) if targeted else 1.0
        all_passed = all(m.passed for m in targeted)
        return cls(
            dimension=dimension,
            metrics=list(metrics),
            score=score,
            passed=all_passed,
        )
