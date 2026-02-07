"""Week 3-4 Milestone Test: Python -> Engine -> Manifest end-to-end.

Validates that Python can:
1. Create an engine and register components
2. Spawn entities with components
3. Run ticks and receive typed TickManifest objects
4. Query entity index
5. Trace causal chains through manifest history
"""

import logging

import pytest
from nomai.engine import NomaiEngine
from nomai.manifest import TickManifest, EntityEntry, CausalChain

logger = logging.getLogger(__name__)


class TestMilestoneWeek34:
    """End-to-end integration tests for Python -> Rust engine."""

    def test_engine_creates_and_ticks(self) -> None:
        """Engine can be created and ticked without error."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")
        engine.register_component("health")
        manifest = engine.tick()
        assert isinstance(manifest, TickManifest)
        assert manifest.tick == 0
        assert engine.tick_count == 1

    def test_spawn_entities_appear_in_manifest(self) -> None:
        """Entities spawned from Python appear in the next tick's manifest."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")
        engine.register_component("health")

        engine.spawn_entity("unit", "warrior", {
            "position": {"x": 10.0, "y": 20.0},
            "health": 100,
        })
        engine.spawn_entity("unit", "mage", {
            "position": {"x": 30.0, "y": 40.0},
            "health": 80,
        })

        manifest = engine.tick()
        assert len(manifest.entity_spawns) == 2
        assert engine.entity_count == 2

    def test_run_ticks_returns_manifests(self) -> None:
        """run_ticks returns correct number of typed manifests."""
        engine = NomaiEngine(headless=True)
        engine.register_component("counter")

        manifests = engine.run_ticks(10)
        assert len(manifests) == 10
        for i, m in enumerate(manifests):
            assert isinstance(m, TickManifest)
            assert m.tick == i

    def test_entity_index_tracks_spawns(self) -> None:
        """Entity index tracks spawned entities with identity info."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")

        engine.spawn_entity("projectile", "bullet", {
            "position": {"x": 0.0, "y": 0.0},
        })
        engine.tick()

        index = engine.entity_index()
        assert len(index) >= 1
        bullet = [e for e in index if e.entity_type == "projectile"]
        assert len(bullet) == 1
        assert bullet[0].role == "bullet"
        assert bullet[0].alive is True
        assert bullet[0].tier == "Semantic"

    def test_despawn_reflected_in_manifest(self) -> None:
        """Despawned entities appear in manifest despawns list."""
        engine = NomaiEngine(headless=True)
        engine.register_component("health")

        engine.spawn_entity("unit", "target", {"health": 50})
        manifest_1 = engine.tick()
        assert len(manifest_1.entity_spawns) == 1
        entity_id = manifest_1.entity_spawns[0]

        engine.despawn_entity(entity_id)
        manifest_2 = engine.tick()
        assert entity_id in manifest_2.entity_despawns

    def test_set_component_produces_change(self) -> None:
        """set_component produces a ComponentChange in the manifest."""
        engine = NomaiEngine(headless=True)
        engine.register_component("score")

        engine.spawn_entity("player", "hero", {"score": 0})
        engine.tick()  # Apply spawn

        # Get entity ID from index.
        index = engine.entity_index()
        hero = [e for e in index if e.role == "hero"][0]

        engine.set_component(hero.entity_id, "score", 100)
        manifest = engine.tick()  # Apply set_component

        score_changes = [
            c for c in manifest.component_changes
            if c.component_type_name == "score"
        ]
        assert len(score_changes) >= 1

    def test_manifest_history_available(self) -> None:
        """All manifests within history window are accessible."""
        engine = NomaiEngine(headless=True)
        engine.register_component("counter")

        engine.run_ticks(20)

        history = engine.manifest_history()
        assert len(history) == 20

        # Each tick is accessible by number.
        for tick_num in range(20):
            m = engine.manifest_at_tick(tick_num)
            assert m is not None
            assert m.tick == tick_num

    def test_causal_chain_traced_through_manifest(self) -> None:
        """Causal chains can be traced from Python through the manifest."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")

        engine.spawn_entity("unit", "mover", {
            "position": {"x": 0.0, "y": 0.0},
        })
        engine.tick()  # Apply spawn (tick 0)

        # Get entity ID.
        index = engine.entity_index()
        mover = [e for e in index if e.role == "mover"][0]

        # Move the entity.
        engine.set_component(mover.entity_id, "position", {"x": 5.0, "y": 10.0})
        engine.tick()  # Apply set (tick 1)

        # Trace causality.
        chain = engine.trace_causality(mover.entity_id, "position", 1)
        assert chain is not None
        assert isinstance(chain, CausalChain)
        assert len(chain.steps) >= 1
        # First step should be the set from tick 1, second from spawn at tick 0.
        assert chain.steps[0].tick == 1
        assert chain.steps[0].reason_type == "SystemInternal"
        assert chain.steps[0].reason_detail == "python_set"

    def test_run_until_with_condition(self) -> None:
        """run_until stops when the condition is met."""
        engine = NomaiEngine(headless=True)
        engine.register_component("counter")

        manifests = engine.run_until(
            condition=lambda m: m.tick >= 5,
            max_ticks=100,
        )
        assert len(manifests) == 6  # ticks 0-5
        assert manifests[-1].tick == 5

    def test_100_ticks_end_to_end(self) -> None:
        """Full 100-tick simulation with spawns, mutations, and manifest queries."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")
        engine.register_component("health")
        engine.register_component("score")

        # Spawn 10 entities.
        for i in range(10):
            engine.spawn_entity("unit", f"soldier_{i}", {
                "position": {"x": float(i), "y": 0.0},
                "health": 100,
                "score": 0,
            })

        manifests = engine.run_ticks(100)
        assert len(manifests) == 100

        # Verify spawns appeared in first tick.
        assert len(manifests[0].entity_spawns) == 10

        # Verify entity index has all 10.
        index = engine.entity_index()
        soldiers = [e for e in index if e.entity_type == "unit"]
        assert len(soldiers) == 10

        # Verify manifests have correct tick numbers.
        for i, m in enumerate(manifests):
            assert m.tick == i

        # All manifests have valid aggregates.
        for m in manifests:
            assert m.aggregates.total_entity_count >= 0

        logger.info("Milestone PASS: 100 ticks, %d entities tracked", len(index))
