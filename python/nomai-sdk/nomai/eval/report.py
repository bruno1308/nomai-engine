"""Structured evaluation report for the Nomai engine.

An ``EvalReport`` is the top-level output of an evaluation run.  It
aggregates per-dimension scores, computes the north-star CW-ZTVCR
metric, and serializes to JSON for storage and CI consumption.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field
from typing import Self

from nomai.eval.metrics import DimensionScore, MetricResult

logger = logging.getLogger(__name__)


@dataclass
class EvalReport:
    """Complete evaluation report for a single eval run.

    Attributes:
        timestamp: ISO-8601 timestamp of the run.
        engine_version: Git SHA or version tag of the engine under test.
        dimensions: Per-dimension aggregated scores keyed by dimension name.
        cw_ztvcr: North-star metric (Complexity-Weighted Zero-Touch
            Verified Completion Rate).  ``-1.0`` if autonomy was not evaluated.
        metrics: Flat list of every individual metric result.
        complexity_tier: Game complexity tier used for the run
            (e.g. ``"breakout"``, ``"puzzle"``, ``"platformer"``).
    """

    timestamp: str
    engine_version: str
    dimensions: dict[str, DimensionScore] = field(default_factory=dict)
    cw_ztvcr: float = -1.0
    metrics: list[MetricResult] = field(default_factory=list)
    complexity_tier: str = "breakout"

    # -- Serialization -------------------------------------------------------

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "timestamp": self.timestamp,
            "engine_version": self.engine_version,
            "dimensions": {k: v.to_dict() for k, v in self.dimensions.items()},
            "cw_ztvcr": self.cw_ztvcr,
            "metrics": [m.to_dict() for m in self.metrics],
            "complexity_tier": self.complexity_tier,
        }

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_dims = data.get("dimensions", {})
        dims: dict[str, DimensionScore] = {}
        if isinstance(raw_dims, dict):
            dims = {
                str(k): DimensionScore.from_dict(v)  # type: ignore[arg-type]
                for k, v in raw_dims.items()
            }
        raw_metrics = data.get("metrics", [])
        metrics: list[MetricResult] = []
        if isinstance(raw_metrics, list):
            metrics = [MetricResult.from_dict(m) for m in raw_metrics]  # type: ignore[arg-type]
        return cls(
            timestamp=str(data.get("timestamp", "")),
            engine_version=str(data.get("engine_version", "")),
            dimensions=dims,
            cw_ztvcr=float(data.get("cw_ztvcr", -1.0)),  # type: ignore[arg-type]
            metrics=metrics,
            complexity_tier=str(data.get("complexity_tier", "breakout")),
        )

    # -- Summary -------------------------------------------------------------

    def summary(self) -> str:
        """Produce a human-readable summary of the evaluation."""
        lines: list[str] = []
        lines.append(f"Nomai Eval Report  [{self.timestamp}]")
        lines.append(f"Engine: {self.engine_version}  Tier: {self.complexity_tier}")
        lines.append(f"CW-ZTVCR: {self.cw_ztvcr:.3f}")
        lines.append("")
        for name, dim in self.dimensions.items():
            status = "PASS" if dim.passed else "FAIL"
            lines.append(f"  {name}: {dim.score:.1%} [{status}]")
            for m in dim.metrics:
                flag = "ok" if m.passed else "FAIL"
                target_str = f" (target {m.target})" if m.target is not None else ""
                lines.append(f"    {m.name}: {m.value:.4f}{target_str} [{flag}]")
        lines.append("")
        total_pass = sum(1 for m in self.metrics if m.passed)
        total_target = sum(1 for m in self.metrics if m.target is not None)
        lines.append(f"Total: {total_pass}/{total_target} metrics passing")
        return "\n".join(lines)
