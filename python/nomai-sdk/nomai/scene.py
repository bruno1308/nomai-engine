"""Scene snapshot types — the text equivalent of a rendered frame.

A ``SceneSnapshot`` captures every entity's spatial data, identity,
and components at a single point in time. Combined with
``TickManifest`` (per-tick diffs), this gives AI full observability
without pixel-peeking.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field
from typing import Self

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class SceneEntity:
    """A single entity's state in the scene snapshot."""

    entity_id: int
    entity_type: str
    role: str
    tier: str
    position: tuple[float, float] | None
    size: tuple[float, float] | None
    velocity: tuple[float, float] | None
    visible: bool
    z_index: float
    components: dict[str, object] = field(default_factory=dict)

    def to_dict(self) -> dict[str, object]:
        return {
            "entity_id": self.entity_id,
            "entity_type": self.entity_type,
            "role": self.role,
            "tier": self.tier,
            "position": list(self.position) if self.position else None,
            "size": list(self.size) if self.size else None,
            "velocity": list(self.velocity) if self.velocity else None,
            "visible": self.visible,
            "z_index": self.z_index,
            "components": dict(self.components),
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        raw_pos = data.get("position")
        raw_size = data.get("size")
        raw_vel = data.get("velocity")
        return cls(
            entity_id=int(data["entity_id"]),  # type: ignore[arg-type]
            entity_type=str(data.get("entity_type", "unknown")),
            role=str(data.get("role", "unknown")),
            tier=str(data.get("tier", "Unknown")),
            position=tuple(raw_pos) if raw_pos else None,  # type: ignore[arg-type]
            size=tuple(raw_size) if raw_size else None,  # type: ignore[arg-type]
            velocity=tuple(raw_vel) if raw_vel else None,  # type: ignore[arg-type]
            visible=bool(data.get("visible", True)),
            z_index=float(data.get("z_index", 0.0)),  # type: ignore[arg-type]
            components=dict(data.get("components", {})),  # type: ignore[arg-type]
        )


@dataclass(frozen=True)
class SceneBounds:
    """Axis-aligned bounding box of the scene."""

    min_x: float
    min_y: float
    max_x: float
    max_y: float

    def to_dict(self) -> dict[str, float]:
        return {
            "min_x": self.min_x,
            "min_y": self.min_y,
            "max_x": self.max_x,
            "max_y": self.max_y,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        return cls(
            min_x=float(data.get("min_x", 0.0)),  # type: ignore[arg-type]
            min_y=float(data.get("min_y", 0.0)),  # type: ignore[arg-type]
            max_x=float(data.get("max_x", 0.0)),  # type: ignore[arg-type]
            max_y=float(data.get("max_y", 0.0)),  # type: ignore[arg-type]
        )


@dataclass(frozen=True)
class SceneSnapshot:
    """A complete snapshot of the game scene at a single tick.

    This is the text equivalent of a rendered frame. Combined with
    ``TickManifest`` (per-tick diffs), this gives AI full observability.
    """

    schema_version: int
    tick: int
    sim_time: float
    entities: list[SceneEntity]
    bounds: SceneBounds
    entity_count: int

    def to_dict(self) -> dict[str, object]:
        return {
            "schema_version": self.schema_version,
            "tick": self.tick,
            "sim_time": self.sim_time,
            "entities": [e.to_dict() for e in self.entities],
            "bounds": self.bounds.to_dict(),
            "entity_count": self.entity_count,
        }

    def to_json(self, indent: int | None = 2) -> str:
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        raw_entities = data.get("entities", [])
        entities = [SceneEntity.from_dict(e) for e in raw_entities]  # type: ignore[arg-type]
        raw_bounds = data.get("bounds", {})
        bounds = SceneBounds.from_dict(raw_bounds)  # type: ignore[arg-type]
        return cls(
            schema_version=int(data.get("schema_version", 1)),  # type: ignore[arg-type]
            tick=int(data.get("tick", 0)),  # type: ignore[arg-type]
            sim_time=float(data.get("sim_time", 0.0)),  # type: ignore[arg-type]
            entities=entities,
            bounds=bounds,
            entity_count=int(data.get("entity_count", len(entities))),  # type: ignore[arg-type]
        )

    def entity_by_role(self, role: str) -> SceneEntity | None:
        """Find the first entity with the given role."""
        for e in self.entities:
            if e.role == role:
                return e
        return None

    def entities_by_role(self, role: str) -> list[SceneEntity]:
        """Find all entities with the given role."""
        return [e for e in self.entities if e.role == role]

    def entities_by_type(self, entity_type: str) -> list[SceneEntity]:
        """Find all entities with the given type."""
        return [e for e in self.entities if e.entity_type == entity_type]

    def summary(self) -> str:
        """Human-readable summary of the scene."""
        lines = [
            f"Scene @ tick {self.tick} (t={self.sim_time:.3f}s)",
            f"  Entities: {self.entity_count}",
            f"  Bounds: ({self.bounds.min_x:.0f},{self.bounds.min_y:.0f}) to ({self.bounds.max_x:.0f},{self.bounds.max_y:.0f})",
        ]
        for e in self.entities:
            pos_str = f"({e.position[0]:.1f},{e.position[1]:.1f})" if e.position else "none"
            size_str = f"{e.size[0]:.0f}x{e.size[1]:.0f}" if e.size else "none"
            vel_str = f"v=({e.velocity[0]:.1f},{e.velocity[1]:.1f})" if e.velocity else ""
            vis = "" if e.visible else " [hidden]"
            lines.append(f"  [{e.entity_id}] {e.role} ({e.entity_type}) @ {pos_str} {size_str} {vel_str}{vis}")
        return "\n".join(lines)
