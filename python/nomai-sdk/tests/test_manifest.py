"""Tests for nomai.manifest -- manifest data types and JSON parsing.

Tests validate construction, serialization round-trips, and parsing from
the exact JSON format that the Rust engine produces via ``serde_json``.
"""

from __future__ import annotations

import json

from nomai.manifest import (
    Aggregates,
    CausalChain,
    CausalStep,
    ComponentChange,
    EntityEntry,
    GameEvent,
    TickManifest,
)


# ---------------------------------------------------------------------------
# ComponentChange
# ---------------------------------------------------------------------------

class TestComponentChange:
    """Tests for ComponentChange dataclass."""

    def test_construction_modification(self) -> None:
        """A modification change has both old_value and new_value."""
        change = ComponentChange(
            entity_id=42,
            component_type_name="health",
            old_value=100,
            new_value=75,
            changed_by_system=1,
            reason_type="GameRule",
            reason_detail="damage_applied",
            command_index=0,
            tick=5,
        )
        assert change.entity_id == 42
        assert change.component_type_name == "health"
        assert change.old_value == 100
        assert change.new_value == 75
        assert change.changed_by_system == 1
        assert change.reason_type == "GameRule"
        assert change.reason_detail == "damage_applied"
        assert change.command_index == 0
        assert change.tick == 5

    def test_construction_spawn(self) -> None:
        """A spawn change has old_value=None."""
        change = ComponentChange(
            entity_id=0,
            component_type_name="position",
            old_value=None,
            new_value={"x": 10.0, "y": 20.0},
            changed_by_system=1,
            reason_type="GameRule",
            reason_detail="player_spawn",
            command_index=0,
            tick=1,
        )
        assert change.old_value is None
        assert change.new_value == {"x": 10.0, "y": 20.0}

    def test_construction_removal(self) -> None:
        """A removal/despawn change has new_value=None."""
        change = ComponentChange(
            entity_id=3,
            component_type_name="health",
            old_value=0,
            new_value=None,
            changed_by_system=100,
            reason_type="GameRule",
            reason_detail="entity_destroyed",
            command_index=0,
            tick=10,
        )
        assert change.old_value == 0
        assert change.new_value is None

    def test_frozen(self) -> None:
        """ComponentChange is immutable."""
        change = ComponentChange(
            entity_id=0,
            component_type_name="health",
            old_value=100,
            new_value=50,
            changed_by_system=0,
            reason_type="GameRule",
            reason_detail="test",
            command_index=0,
            tick=1,
        )
        try:
            change.tick = 99  # type: ignore[misc]
            assert False, "Should have raised FrozenInstanceError"
        except AttributeError:
            pass

    def test_to_dict_roundtrip(self) -> None:
        """to_dict -> from_dict produces an equivalent object."""
        original = ComponentChange(
            entity_id=42,
            component_type_name="position",
            old_value={"x": 0.0, "y": 0.0},
            new_value={"x": 1.0, "y": 0.0},
            changed_by_system=200,
            reason_type="PlayerInput",
            reason_detail="move_right",
            command_index=3,
            tick=7,
        )
        d = original.to_dict()
        restored = ComponentChange.from_dict(d)
        assert restored.entity_id == original.entity_id
        assert restored.component_type_name == original.component_type_name
        assert restored.changed_by_system == original.changed_by_system
        assert restored.reason_type == original.reason_type
        assert restored.reason_detail == original.reason_detail
        assert restored.command_index == original.command_index
        assert restored.tick == original.tick

    def test_from_rust_json(self) -> None:
        """Parse the exact JSON that Rust serde_json produces."""
        rust_json = {
            "entity_id": 42,
            "component_type_name": "health",
            "old_value": 100,
            "new_value": 75,
            "changed_by": 1,
            "reason": {"GameRule": "damage_applied"},
            "command_index": 0,
            "tick": 5,
        }
        change = ComponentChange.from_dict(rust_json)
        assert change.entity_id == 42
        assert change.component_type_name == "health"
        assert change.old_value == 100
        assert change.new_value == 75
        assert change.changed_by_system == 1
        assert change.reason_type == "GameRule"
        assert change.reason_detail == "damage_applied"

    def test_collision_response_reason(self) -> None:
        """Parse CollisionResponse causal reason (array payload)."""
        rust_json = {
            "entity_id": 0,
            "component_type_name": "health",
            "old_value": 100,
            "new_value": 90,
            "changed_by": 200,
            "reason": {"CollisionResponse": [0, 1]},
            "command_index": 1,
            "tick": 3,
        }
        change = ComponentChange.from_dict(rust_json)
        assert change.reason_type == "CollisionResponse"
        assert change.reason_detail == "[0,1]"

    def test_state_transition_reason(self) -> None:
        """Parse StateTransition causal reason (object payload)."""
        rust_json = {
            "entity_id": 5,
            "component_type_name": "state",
            "old_value": "grounded",
            "new_value": "airborne",
            "changed_by": 3,
            "reason": {"StateTransition": {"from": "grounded", "to": "airborne"}},
            "command_index": 2,
            "tick": 10,
        }
        change = ComponentChange.from_dict(rust_json)
        assert change.reason_type == "StateTransition"
        # Detail is JSON-encoded dict
        detail_parsed = json.loads(change.reason_detail)
        assert detail_parsed["from"] == "grounded"
        assert detail_parsed["to"] == "airborne"


# ---------------------------------------------------------------------------
# GameEvent
# ---------------------------------------------------------------------------

class TestGameEvent:
    """Tests for GameEvent dataclass."""

    def test_construction(self) -> None:
        """Construct a game event."""
        event = GameEvent(
            event_type="collision",
            description="Ball hit paddle",
            involved_entities=[0, 1],
            caused_by_system=200,
            reason_type="CollisionResponse",
            reason_detail="[0,1]",
            tick=15,
        )
        assert event.event_type == "collision"
        assert event.description == "Ball hit paddle"
        assert event.involved_entities == [0, 1]
        assert event.caused_by_system == 200
        assert event.tick == 15

    def test_frozen(self) -> None:
        """GameEvent is immutable."""
        event = GameEvent(
            event_type="test",
            description="test",
            involved_entities=[],
            caused_by_system=0,
            reason_type="SystemInternal",
            reason_detail="test",
            tick=1,
        )
        try:
            event.tick = 99  # type: ignore[misc]
            assert False, "Should have raised FrozenInstanceError"
        except AttributeError:
            pass

    def test_to_dict_roundtrip(self) -> None:
        """to_dict -> from_dict produces an equivalent object."""
        original = GameEvent(
            event_type="score_change",
            description="Score increased by 100",
            involved_entities=[0],
            caused_by_system=100,
            reason_type="GameRule",
            reason_detail="score_on_hit",
            tick=20,
        )
        d = original.to_dict()
        restored = GameEvent.from_dict(d)
        assert restored.event_type == original.event_type
        assert restored.description == original.description
        assert restored.involved_entities == original.involved_entities
        assert restored.caused_by_system == original.caused_by_system
        assert restored.reason_type == original.reason_type
        assert restored.reason_detail == original.reason_detail
        assert restored.tick == original.tick

    def test_from_rust_json(self) -> None:
        """Parse the exact JSON that Rust serde_json produces."""
        rust_json = {
            "event_type": "collision",
            "description": "Player collided with enemy",
            "involved_entities": [0, 1],
            "caused_by": 200,
            "reason": {"CollisionResponse": [0, 1]},
            "tick": 1,
        }
        event = GameEvent.from_dict(rust_json)
        assert event.event_type == "collision"
        assert event.involved_entities == [0, 1]
        assert event.caused_by_system == 200
        assert event.reason_type == "CollisionResponse"


# ---------------------------------------------------------------------------
# Aggregates
# ---------------------------------------------------------------------------

class TestAggregates:
    """Tests for Aggregates dataclass."""

    def test_construction(self) -> None:
        agg = Aggregates(
            entity_count_by_tier={"Semantic": 2, "Pooled": 5},
            entity_count_by_type={"character": 2, "brick": 5},
            total_entity_count=7,
        )
        assert agg.total_entity_count == 7
        assert agg.entity_count_by_tier["Semantic"] == 2

    def test_to_dict_roundtrip(self) -> None:
        original = Aggregates(
            entity_count_by_tier={"Semantic": 3, "Pooled": 10},
            entity_count_by_type={"character": 1, "enemy": 2, "brick": 10},
            total_entity_count=13,
        )
        d = original.to_dict()
        restored = Aggregates.from_dict(d)
        assert restored.entity_count_by_tier == original.entity_count_by_tier
        assert restored.entity_count_by_type == original.entity_count_by_type
        assert restored.total_entity_count == original.total_entity_count


# ---------------------------------------------------------------------------
# CausalStep / CausalChain
# ---------------------------------------------------------------------------

class TestCausalStep:
    """Tests for CausalStep dataclass."""

    def test_construction(self) -> None:
        step = CausalStep(
            tick=5,
            command_index=2,
            system_id=200,
            reason_type="PlayerInput",
            reason_detail="move_right",
            description="System 200 changed position on entity 0",
        )
        assert step.tick == 5
        assert step.system_id == 200

    def test_to_dict_roundtrip(self) -> None:
        original = CausalStep(
            tick=10,
            command_index=0,
            system_id=1,
            reason_type="GameRule",
            reason_detail="spawn",
            description="Player spawned",
        )
        d = original.to_dict()
        restored = CausalStep.from_dict(d)
        assert restored.tick == original.tick
        assert restored.command_index == original.command_index
        assert restored.system_id == original.system_id
        assert restored.reason_type == original.reason_type
        assert restored.reason_detail == original.reason_detail
        assert restored.description == original.description


class TestCausalChain:
    """Tests for CausalChain dataclass."""

    def test_construction(self) -> None:
        chain = CausalChain(
            entity_id=42,
            component="position",
            steps=[
                CausalStep(
                    tick=5,
                    command_index=0,
                    system_id=200,
                    reason_type="PlayerInput",
                    reason_detail="move_right",
                    description="Position changed at tick 5",
                ),
                CausalStep(
                    tick=4,
                    command_index=0,
                    system_id=200,
                    reason_type="PlayerInput",
                    reason_detail="move_right",
                    description="Position changed at tick 4",
                ),
            ],
        )
        assert chain.entity_id == 42
        assert chain.component == "position"
        assert len(chain.steps) == 2

    def test_to_dict_roundtrip(self) -> None:
        original = CausalChain(
            entity_id=7,
            component="health",
            steps=[
                CausalStep(
                    tick=3,
                    command_index=1,
                    system_id=100,
                    reason_type="GameRule",
                    reason_detail="damage",
                    description="Health reduced by game rule",
                ),
            ],
        )
        d = original.to_dict()
        restored = CausalChain.from_dict(d)
        assert restored.entity_id == original.entity_id
        assert restored.component == original.component
        assert len(restored.steps) == 1
        assert restored.steps[0].tick == 3


# ---------------------------------------------------------------------------
# EntityEntry
# ---------------------------------------------------------------------------

class TestEntityEntry:
    """Tests for EntityEntry dataclass."""

    def test_alive_entity(self) -> None:
        entry = EntityEntry(
            entity_id=0,
            tier="Semantic",
            entity_type="character",
            role="player",
            alive=True,
            spawned_at_tick=1,
            despawned_at_tick=None,
        )
        assert entry.alive
        assert entry.despawned_at_tick is None

    def test_despawned_entity(self) -> None:
        entry = EntityEntry(
            entity_id=1,
            tier="Pooled",
            entity_type="destructible",
            role="brick",
            alive=False,
            spawned_at_tick=1,
            despawned_at_tick=10,
        )
        assert not entry.alive
        assert entry.despawned_at_tick == 10

    def test_to_dict_roundtrip(self) -> None:
        original = EntityEntry(
            entity_id=5,
            tier="Semantic",
            entity_type="character",
            role="enemy",
            alive=True,
            spawned_at_tick=3,
            despawned_at_tick=None,
        )
        d = original.to_dict()
        restored = EntityEntry.from_dict(d)
        assert restored.entity_id == original.entity_id
        assert restored.tier == original.tier
        assert restored.entity_type == original.entity_type
        assert restored.role == original.role
        assert restored.alive == original.alive
        assert restored.spawned_at_tick == original.spawned_at_tick
        assert restored.despawned_at_tick == original.despawned_at_tick


# ---------------------------------------------------------------------------
# TickManifest
# ---------------------------------------------------------------------------

class TestTickManifest:
    """Tests for TickManifest and its from_json factory method."""

    def test_construction(self) -> None:
        """Construct a minimal TickManifest."""
        manifest = TickManifest(
            tick=1,
            sim_time=1.0 / 60.0,
            entity_spawns=[0],
            entity_despawns=[],
            component_changes=[],
            events=[],
            aggregates=Aggregates(
                entity_count_by_tier={"Semantic": 1},
                entity_count_by_type={"character": 1},
                total_entity_count=1,
            ),
            systems_executed=["spawner"],
            commands_processed=1,
            commands_succeeded=1,
        )
        assert manifest.tick == 1
        assert manifest.entity_spawns == [0]
        assert manifest.aggregates.total_entity_count == 1

    def test_from_json_sample_rust_output(self) -> None:
        """Parse a sample JSON blob matching what the Rust engine produces.

        This JSON was constructed to match the serde_json output of the
        Rust ``TickManifest`` struct.
        """
        rust_json: dict[str, object] = {
            "tick": 5,
            "sim_time": 0.08333333333333333,
            "entity_spawns": [],
            "entity_despawns": [],
            "component_changes": [
                {
                    "entity_id": 0,
                    "component_type_name": "position",
                    "old_value": None,
                    "new_value": {"x": 25.0, "y": 0.0},
                    "changed_by": 200,
                    "reason": {"PlayerInput": "move_right"},
                    "command_index": 0,
                    "tick": 5,
                },
                {
                    "entity_id": 4294967296,
                    "component_type_name": "health",
                    "old_value": 50,
                    "new_value": 0,
                    "changed_by": 100,
                    "reason": {"GameRule": "take_damage"},
                    "command_index": 1,
                    "tick": 5,
                },
            ],
            "events": [
                {
                    "event_type": "collision",
                    "description": "Ball hit brick",
                    "involved_entities": [0, 4294967296],
                    "caused_by": 200,
                    "reason": {"CollisionResponse": [0, 4294967296]},
                    "tick": 5,
                },
            ],
            "aggregates": {
                "entity_count_by_tier": {"Semantic": 2, "Pooled": 3},
                "entity_count_by_type": {
                    "character": 2,
                    "destructible": 3,
                },
                "total_entity_count": 5,
            },
            "systems_executed": ["physics", "gameplay"],
            "commands_processed": 3,
            "commands_succeeded": 2,
        }

        manifest = TickManifest.from_json(rust_json)

        assert manifest.tick == 5
        assert abs(manifest.sim_time - 0.08333333333333333) < 1e-15
        assert manifest.entity_spawns == []
        assert manifest.entity_despawns == []

        assert len(manifest.component_changes) == 2
        change0 = manifest.component_changes[0]
        assert change0.entity_id == 0
        assert change0.component_type_name == "position"
        assert change0.old_value is None
        assert change0.reason_type == "PlayerInput"
        assert change0.reason_detail == "move_right"

        change1 = manifest.component_changes[1]
        assert change1.entity_id == 4294967296
        assert change1.component_type_name == "health"
        assert change1.old_value == 50
        assert change1.new_value == 0
        assert change1.reason_type == "GameRule"

        assert len(manifest.events) == 1
        event = manifest.events[0]
        assert event.event_type == "collision"
        assert event.involved_entities == [0, 4294967296]

        assert manifest.aggregates.total_entity_count == 5
        assert manifest.aggregates.entity_count_by_tier["Semantic"] == 2
        assert manifest.aggregates.entity_count_by_tier["Pooled"] == 3

        assert manifest.systems_executed == ["physics", "gameplay"]
        assert manifest.commands_processed == 3
        assert manifest.commands_succeeded == 2

    def test_to_json_roundtrip(self) -> None:
        """TickManifest can be serialized to JSON and parsed back."""
        original = TickManifest(
            tick=10,
            sim_time=10.0 / 60.0,
            entity_spawns=[100],
            entity_despawns=[50],
            component_changes=[
                ComponentChange(
                    entity_id=100,
                    component_type_name="position",
                    old_value=None,
                    new_value={"x": 0.0, "y": 0.0},
                    changed_by_system=1,
                    reason_type="GameRule",
                    reason_detail="coin_spawn",
                    command_index=0,
                    tick=10,
                ),
            ],
            events=[],
            aggregates=Aggregates(
                entity_count_by_tier={"Semantic": 1},
                entity_count_by_type={"collectible": 1},
                total_entity_count=1,
            ),
            systems_executed=["spawner", "physics"],
            commands_processed=2,
            commands_succeeded=2,
        )

        json_str = original.to_json()
        data = json.loads(json_str)
        restored = TickManifest.from_json(data)

        assert restored.tick == original.tick
        assert abs(restored.sim_time - original.sim_time) < 1e-15
        assert restored.entity_spawns == original.entity_spawns
        assert restored.entity_despawns == original.entity_despawns
        assert len(restored.component_changes) == 1
        assert restored.component_changes[0].entity_id == 100
        assert restored.aggregates.total_entity_count == 1
        assert restored.systems_executed == original.systems_executed
        assert restored.commands_processed == original.commands_processed
        assert restored.commands_succeeded == original.commands_succeeded

    def test_empty_manifest(self) -> None:
        """An empty tick manifest parses correctly."""
        rust_json: dict[str, object] = {
            "tick": 0,
            "sim_time": 0.0,
            "entity_spawns": [],
            "entity_despawns": [],
            "component_changes": [],
            "events": [],
            "aggregates": {
                "entity_count_by_tier": {},
                "entity_count_by_type": {},
                "total_entity_count": 0,
            },
            "systems_executed": [],
            "commands_processed": 0,
            "commands_succeeded": 0,
        }
        manifest = TickManifest.from_json(rust_json)
        assert manifest.tick == 0
        assert manifest.sim_time == 0.0
        assert manifest.component_changes == []
        assert manifest.events == []
        assert manifest.aggregates.total_entity_count == 0

    def test_system_id_dict_format(self) -> None:
        """Parse SystemId in the non-transparent serde format {"0": 100}."""
        rust_json = {
            "entity_id": 0,
            "component_type_name": "health",
            "old_value": 100,
            "new_value": 50,
            "changed_by": {"0": 100},
            "reason": {"GameRule": "damage"},
            "command_index": 0,
            "tick": 1,
        }
        change = ComponentChange.from_dict(rust_json)
        assert change.changed_by_system == 100

    def test_manifest_spawn_tick(self) -> None:
        """Manifest with spawn data from a tick where entities were created."""
        rust_json: dict[str, object] = {
            "tick": 1,
            "sim_time": 0.016666666666666666,
            "entity_spawns": [0, 4294967296, 8589934592],
            "entity_despawns": [],
            "component_changes": [
                {
                    "entity_id": 0,
                    "component_type_name": "position",
                    "old_value": None,
                    "new_value": {"x": 0.0, "y": 0.0},
                    "changed_by": 1,
                    "reason": {"GameRule": "player_spawn"},
                    "command_index": 0,
                    "tick": 1,
                },
            ],
            "events": [],
            "aggregates": {
                "entity_count_by_tier": {"Semantic": 2, "Pooled": 1},
                "entity_count_by_type": {
                    "character": 2,
                    "destructible": 1,
                },
                "total_entity_count": 3,
            },
            "systems_executed": ["spawner"],
            "commands_processed": 3,
            "commands_succeeded": 3,
        }
        manifest = TickManifest.from_json(rust_json)
        assert len(manifest.entity_spawns) == 3
        assert manifest.entity_spawns[0] == 0
        assert manifest.entity_spawns[1] == 4294967296
        assert manifest.entity_spawns[2] == 8589934592
        assert manifest.aggregates.total_entity_count == 3
