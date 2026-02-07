"""Manifest data types mirroring the Rust engine's serde JSON output.

Every type here corresponds to a Rust struct in ``nomai-manifest``. The
``from_json`` / ``from_dict`` class methods parse the exact JSON that
``serde_json`` produces for those Rust types.

Serialization conventions from the Rust side
--------------------------------------------
- ``EntityId`` is a newtype ``EntityId(u64)`` serialized as a plain integer.
- ``SystemId`` is a newtype ``SystemId(u32)`` serialized as a plain integer.
- ``CausalReason`` is an externally-tagged enum:
  ``{"GameRule": "reason"}``, ``{"PlayerInput": "input"}``,
  ``{"CollisionResponse": [entity_a, entity_b]}``,
  ``{"StateTransition": {"from": "a", "to": "b"}}``,
  ``{"Timer": "name"}``, ``{"SystemInternal": "detail"}``.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field
from typing import Self

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# Causal reason parsing helpers
# ---------------------------------------------------------------------------

def _parse_reason(raw: object) -> tuple[str, str]:
    """Parse a serde-serialized ``CausalReason`` into ``(reason_type, reason_detail)``.

    The Rust ``CausalReason`` enum serializes via serde's default
    externally-tagged representation.  Examples::

        {"GameRule": "brick_destroyed"}
        {"PlayerInput": "move_right"}
        {"CollisionResponse": [0, 1]}
        {"StateTransition": {"from": "grounded", "to": "airborne"}}
        {"Timer": "cooldown_expired"}
        {"SystemInternal": "physics_step"}

    Returns a ``(reason_type, reason_detail)`` tuple.  For compound
    payloads the detail is a JSON-encoded string.
    """
    if isinstance(raw, dict):
        for key, value in raw.items():
            if isinstance(value, str):
                return (key, value)
            if isinstance(value, (list, dict)):
                return (key, json.dumps(value, separators=(",", ":")))
            # Fallback: stringify
            return (key, str(value))
    # If it's already a string (shouldn't happen from serde but be safe)
    if isinstance(raw, str):
        return ("Unknown", raw)
    return ("Unknown", str(raw))


def _reason_to_dict(reason_type: str, reason_detail: str) -> dict[str, object]:
    """Reconstruct the serde JSON representation of a ``CausalReason``.

    Reverses ``_parse_reason``.
    """
    # Attempt to parse detail back to structured JSON for compound types.
    if reason_type in ("CollisionResponse", "StateTransition"):
        try:
            return {reason_type: json.loads(reason_detail)}
        except (json.JSONDecodeError, TypeError):
            pass
    return {reason_type: reason_detail}


def _parse_system_id(raw: object) -> int:
    """Parse a serde-serialized ``SystemId``.

    ``SystemId(u32)`` serializes as a plain integer when using the default
    serde derive on a newtype struct.  However, if ``#[serde(transparent)]``
    is *not* present it may appear as ``{"0": 100}``.  We handle both.
    """
    if isinstance(raw, int):
        return raw
    if isinstance(raw, dict):
        # {"0": value} format
        for value in raw.values():
            if isinstance(value, int):
                return value
    return int(raw)  # type: ignore[arg-type]


def _parse_entity_id(raw: object) -> int:
    """Parse a serde-serialized ``EntityId``.

    Same logic as ``_parse_system_id`` -- newtype wrapper over ``u64``.
    """
    if isinstance(raw, int):
        return raw
    if isinstance(raw, dict):
        for value in raw.values():
            if isinstance(value, int):
                return value
    return int(raw)  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# ComponentChange
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class ComponentChange:
    """A single component mutation with causality metadata.

    Mirrors ``nomai_manifest::journal::ComponentChange``.
    """
    entity_id: int
    component_type_name: str
    old_value: object
    new_value: object
    changed_by_system: int
    reason_type: str
    reason_detail: str
    command_index: int
    tick: int

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "entity_id": self.entity_id,
            "component_type_name": self.component_type_name,
            "old_value": self.old_value,
            "new_value": self.new_value,
            "changed_by": self.changed_by_system,
            "reason": _reason_to_dict(self.reason_type, self.reason_detail),
            "command_index": self.command_index,
            "tick": self.tick,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        reason_type, reason_detail = _parse_reason(data.get("reason", {}))
        return cls(
            entity_id=_parse_entity_id(data["entity_id"]),
            component_type_name=str(data["component_type_name"]),
            old_value=data.get("old_value"),
            new_value=data.get("new_value"),
            changed_by_system=_parse_system_id(data["changed_by"]),
            reason_type=reason_type,
            reason_detail=reason_detail,
            command_index=int(data["command_index"]),  # type: ignore[arg-type]
            tick=int(data["tick"]),  # type: ignore[arg-type]
        )


# ---------------------------------------------------------------------------
# GameEvent
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class GameEvent:
    """A game event with involved entities and causality.

    Mirrors ``nomai_manifest::manifest::GameEvent``.
    """
    event_type: str
    description: str
    involved_entities: list[int]
    caused_by_system: int
    reason_type: str
    reason_detail: str
    tick: int

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "event_type": self.event_type,
            "description": self.description,
            "involved_entities": self.involved_entities,
            "caused_by": self.caused_by_system,
            "reason": _reason_to_dict(self.reason_type, self.reason_detail),
            "tick": self.tick,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        reason_type, reason_detail = _parse_reason(data.get("reason", {}))
        raw_entities = data.get("involved_entities", [])
        entities: list[int] = []
        if isinstance(raw_entities, list):
            entities = [_parse_entity_id(e) for e in raw_entities]
        return cls(
            event_type=str(data["event_type"]),
            description=str(data["description"]),
            involved_entities=entities,
            caused_by_system=_parse_system_id(data["caused_by"]),
            reason_type=reason_type,
            reason_detail=reason_detail,
            tick=int(data["tick"]),  # type: ignore[arg-type]
        )


# ---------------------------------------------------------------------------
# Aggregates
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class Aggregates:
    """Aggregate statistics computed at end of tick.

    Mirrors ``nomai_manifest::manifest::Aggregates``.
    """
    entity_count_by_tier: dict[str, int]
    entity_count_by_type: dict[str, int]
    total_entity_count: int
    custom: dict[str, float] = field(default_factory=dict)

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "entity_count_by_tier": dict(self.entity_count_by_tier),
            "entity_count_by_type": dict(self.entity_count_by_type),
            "total_entity_count": self.total_entity_count,
            "custom": dict(self.custom),
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        raw_tier = data.get("entity_count_by_tier", {})
        raw_type = data.get("entity_count_by_type", {})
        raw_custom = data.get("custom", {})
        tier_counts: dict[str, int] = {}
        type_counts: dict[str, int] = {}
        custom: dict[str, float] = {}
        if isinstance(raw_tier, dict):
            tier_counts = {str(k): int(v) for k, v in raw_tier.items()}  # type: ignore[arg-type]
        if isinstance(raw_type, dict):
            type_counts = {str(k): int(v) for k, v in raw_type.items()}  # type: ignore[arg-type]
        if isinstance(raw_custom, dict):
            custom = {str(k): float(v) for k, v in raw_custom.items()}  # type: ignore[arg-type]
        return cls(
            entity_count_by_tier=tier_counts,
            entity_count_by_type=type_counts,
            total_entity_count=int(data.get("total_entity_count", 0)),  # type: ignore[arg-type]
            custom=custom,
        )


# ---------------------------------------------------------------------------
# CausalStep
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class CausalStep:
    """A single step in a causal chain.

    Mirrors ``nomai_manifest::manifest::CausalStep``.
    """
    tick: int
    command_index: int
    system_id: int
    reason_type: str
    reason_detail: str
    description: str

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "tick": self.tick,
            "command_index": self.command_index,
            "system_id": self.system_id,
            "reason": _reason_to_dict(self.reason_type, self.reason_detail),
            "description": self.description,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        reason_type, reason_detail = _parse_reason(data.get("reason", {}))
        return cls(
            tick=int(data["tick"]),  # type: ignore[arg-type]
            command_index=int(data["command_index"]),  # type: ignore[arg-type]
            system_id=_parse_system_id(data["system_id"]),
            reason_type=reason_type,
            reason_detail=reason_detail,
            description=str(data["description"]),
        )


# ---------------------------------------------------------------------------
# CausalChain
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class CausalChain:
    """A causal chain tracing a component change back to its root cause.

    Mirrors ``nomai_manifest::manifest::CausalChain``.
    """
    entity_id: int
    component: str
    steps: list[CausalStep]

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "entity_id": self.entity_id,
            "component": self.component,
            "steps": [s.to_dict() for s in self.steps],
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        raw_steps = data.get("steps", [])
        steps: list[CausalStep] = []
        if isinstance(raw_steps, list):
            steps = [CausalStep.from_dict(s) for s in raw_steps]  # type: ignore[arg-type]
        return cls(
            entity_id=_parse_entity_id(data["entity_id"]),
            component=str(data["component"]),
            steps=steps,
        )


# ---------------------------------------------------------------------------
# EntityEntry
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class EntityEntry:
    """A single entity entry in the entity index.

    Mirrors ``nomai_manifest::manifest::EntityEntry``.
    """
    entity_id: int
    tier: str
    entity_type: str
    role: str
    alive: bool
    spawned_at_tick: int
    despawned_at_tick: int | None

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "entity_id": self.entity_id,
            "tier": self.tier,
            "entity_type": self.entity_type,
            "role": self.role,
            "alive": self.alive,
            "spawned_at_tick": self.spawned_at_tick,
            "despawned_at_tick": self.despawned_at_tick,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        raw_despawn = data.get("despawned_at_tick")
        despawned: int | None = None
        if raw_despawn is not None:
            despawned = int(raw_despawn)  # type: ignore[arg-type]
        return cls(
            entity_id=_parse_entity_id(data["entity_id"]),
            tier=str(data["tier"]),
            entity_type=str(data["entity_type"]),
            role=str(data["role"]),
            alive=bool(data["alive"]),
            spawned_at_tick=int(data["spawned_at_tick"]),  # type: ignore[arg-type]
            despawned_at_tick=despawned,
        )


# ---------------------------------------------------------------------------
# TickManifest
# ---------------------------------------------------------------------------

@dataclass
class TickManifest:
    """The complete manifest for a single simulation tick.

    Mirrors ``nomai_manifest::manifest::TickManifest``.
    """
    tick: int
    sim_time: float
    entity_spawns: list[int]
    entity_despawns: list[int]
    component_changes: list[ComponentChange]
    events: list[GameEvent]
    aggregates: Aggregates
    systems_executed: list[str]
    commands_processed: int
    commands_succeeded: int

    def to_dict(self) -> dict[str, object]:
        """Serialize to a dict matching the Rust serde JSON layout."""
        return {
            "tick": self.tick,
            "sim_time": self.sim_time,
            "entity_spawns": self.entity_spawns,
            "entity_despawns": self.entity_despawns,
            "component_changes": [c.to_dict() for c in self.component_changes],
            "events": [e.to_dict() for e in self.events],
            "aggregates": self.aggregates.to_dict(),
            "systems_executed": self.systems_executed,
            "commands_processed": self.commands_processed,
            "commands_succeeded": self.commands_succeeded,
        }

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Parse from a dict matching the Rust serde JSON layout."""
        raw_spawns = data.get("entity_spawns", [])
        raw_despawns = data.get("entity_despawns", [])
        raw_changes = data.get("component_changes", [])
        raw_events = data.get("events", [])
        raw_agg = data.get("aggregates", {})
        raw_systems = data.get("systems_executed", [])

        spawns: list[int] = []
        if isinstance(raw_spawns, list):
            spawns = [_parse_entity_id(e) for e in raw_spawns]

        despawns: list[int] = []
        if isinstance(raw_despawns, list):
            despawns = [_parse_entity_id(e) for e in raw_despawns]

        changes: list[ComponentChange] = []
        if isinstance(raw_changes, list):
            changes = [ComponentChange.from_dict(c) for c in raw_changes]  # type: ignore[arg-type]

        events: list[GameEvent] = []
        if isinstance(raw_events, list):
            events = [GameEvent.from_dict(e) for e in raw_events]  # type: ignore[arg-type]

        aggregates = Aggregates(
            entity_count_by_tier={},
            entity_count_by_type={},
            total_entity_count=0,
        )
        if isinstance(raw_agg, dict):
            aggregates = Aggregates.from_dict(raw_agg)

        systems: list[str] = []
        if isinstance(raw_systems, list):
            systems = [str(s) for s in raw_systems]

        return cls(
            tick=int(data.get("tick", 0)),  # type: ignore[arg-type]
            sim_time=float(data.get("sim_time", 0.0)),  # type: ignore[arg-type]
            entity_spawns=spawns,
            entity_despawns=despawns,
            component_changes=changes,
            events=events,
            aggregates=aggregates,
            systems_executed=systems,
            commands_processed=int(data.get("commands_processed", 0)),  # type: ignore[arg-type]
            commands_succeeded=int(data.get("commands_succeeded", 0)),  # type: ignore[arg-type]
        )

    @classmethod
    def from_json(cls, data: dict[str, object]) -> Self:
        """Parse from a dict produced by Rust ``serde_json`` output.

        This is an alias for ``from_dict`` provided for clarity at call
        sites that receive raw JSON-deserialized dicts.
        """
        return cls.from_dict(data)
