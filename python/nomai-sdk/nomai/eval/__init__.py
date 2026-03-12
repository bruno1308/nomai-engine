"""Nomai Engine evaluation framework.

Measures how well the engine enables autonomous AI game development
across five dimensions: Observability, Controllability, Reproducibility,
Verification, and Autonomy, with Efficiency as a cross-cutting constraint.

The north-star metric is CW-ZTVCR (Complexity-Weighted Zero-Touch
Verified Completion Rate).
"""

from nomai.eval.metrics import (
    DimensionScore,
    EvalDimension,
    MetricResult,
)
from nomai.eval.report import EvalReport
from nomai.eval.runner import EvalRunner

__all__ = [
    "DimensionScore",
    "EvalDimension",
    "EvalReport",
    "EvalRunner",
    "MetricResult",
]
