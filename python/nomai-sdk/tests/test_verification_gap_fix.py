"""Tests for verification gap fix: Tasks 2, 3, and 4.

Task 2: VALUE_RELATION expected type
Task 3: Fixed ENTITY_DESPAWNED specificity and COMPONENT_CHANGED delta check
Task 4: PhysicsSanityChecker detecting missing bounce
"""

from __future__ import annotations

from nomai.intents import (
    Expected,
    ExpectedType,
    IntentKind,
    IntentSpec,
    VerificationSuite,
    collision,
    component_changed,
    entity_despawned,
    event_emitted,
    tick_reached,
    value_relation,
)
from nomai.manifest import (
    Aggregates,
    ComponentChange,
    GameEvent,
    TickManifest,
)
from nomai.physics_sanity import PhysicsEntityInfo, PhysicsSanityChecker
from nomai.verify import (
    IntentResult,
    VerificationEngine,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _empty_aggregates(
    by_type: dict[str, int] | None = None,
    total: int | None = None,
) -> Aggregates:
    """Build an Aggregates with sensible defaults."""
    bt = by_type or {}
    return Aggregates(
        entity_count_by_tier={},
        entity_count_by_type=bt,
        total_entity_count=total if total is not None else sum(bt.values()),
    )


def _make_manifest(
    tick: int,
    changes: list[ComponentChange] | None = None,
    events: list[GameEvent] | None = None,
    despawns: list[int] | None = None,
    spawns: list[int] | None = None,
    aggregates: Aggregates | None = None,
) -> TickManifest:
    """Build a minimal TickManifest for testing."""
    return TickManifest(
        tick=tick,
        sim_time=tick / 60.0,
        entity_spawns=spawns or [],
        entity_despawns=despawns or [],
        component_changes=changes or [],
        events=events or [],
        aggregates=aggregates or _empty_aggregates(),
        systems_executed=["test_system"],
        commands_processed=0,
        commands_succeeded=0,
    )


def _make_change(
    entity_id: int = 0,
    component: str = "position",
    old_value: object = None,
    new_value: object = None,
    tick: int = 0,
    reason_type: str = "GameRule",
    reason_detail: str = "test",
) -> ComponentChange:
    """Build a ComponentChange for testing."""
    return ComponentChange(
        entity_id=entity_id,
        component_type_name=component,
        old_value=old_value,
        new_value=new_value,
        changed_by_system=1,
        reason_type=reason_type,
        reason_detail=reason_detail,
        command_index=0,
        tick=tick,
    )


def _make_event(
    event_type: str = "collision",
    description: str = "test event",
    involved: list[int] | None = None,
    tick: int = 0,
    reason_detail: str = "test",
) -> GameEvent:
    """Build a GameEvent for testing."""
    return GameEvent(
        event_type=event_type,
        description=description,
        involved_entities=involved or [],
        caused_by_system=1,
        reason_type="GameRule",
        reason_detail=reason_detail,
        tick=tick,
    )


# ===========================================================================
# Task 2: VALUE_RELATION expected type
# ===========================================================================


class TestValueRelationConstruction:
    """Tests for value_relation constructor and serialization."""

    def test_value_relation_construction(self) -> None:
        """value_relation() produces correct Expected with all params."""
        e = value_relation("ball", "velocity", "dy", "sign_flipped")
        assert e.type == ExpectedType.VALUE_RELATION
        assert e.params["entity"] == "ball"
        assert e.params["component"] == "velocity"
        assert e.params["field"] == "dy"
        assert e.params["relation"] == "sign_flipped"
        assert e.params["tolerance"] == 0.1

    def test_value_relation_custom_tolerance(self) -> None:
        """value_relation() accepts custom tolerance."""
        e = value_relation("ball", "velocity", "dy", "magnitude_preserved", tolerance=0.05)
        assert e.params["tolerance"] == 0.05

    def test_value_relation_to_dict_roundtrip(self) -> None:
        """value_relation Expected survives to_dict/from_dict round trip."""
        original = value_relation("ball", "velocity", "dy", "sign_flipped", tolerance=0.2)
        d = original.to_dict()
        restored = Expected.from_dict(d)
        assert restored.type == ExpectedType.VALUE_RELATION
        assert restored.params["entity"] == "ball"
        assert restored.params["component"] == "velocity"
        assert restored.params["field"] == "dy"
        assert restored.params["relation"] == "sign_flipped"
        assert restored.params["tolerance"] == 0.2

    def test_value_relation_frozen(self) -> None:
        """value_relation Expected is frozen."""
        e = value_relation("ball", "velocity", "dy", "sign_flipped")
        try:
            e.type = ExpectedType.ALL  # type: ignore[misc]
            assert False, "Should have raised FrozenInstanceError"
        except AttributeError:
            pass


class TestValueRelationSignFlipped:
    """Tests for sign_flipped relation in verification engine."""

    def test_sign_flipped_passes(self) -> None:
        """Sign flip detected: old positive, new negative."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "sign_flipped")
        assert engine._check_expected(e, manifest)

    def test_sign_flipped_negative_to_positive(self) -> None:
        """Sign flip: negative to positive."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 0.0, "dy": -5.0},
                new_value={"dx": 0.0, "dy": 5.0},
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "sign_flipped")
        assert engine._check_expected(e, manifest)

    def test_sign_flipped_fails_same_sign(self) -> None:
        """No sign flip when both values have the same sign."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": 3.0},
                new_value={"dx": 5.0, "dy": 4.0},
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "sign_flipped")
        assert not engine._check_expected(e, manifest)

    def test_sign_flipped_fails_zero(self) -> None:
        """No sign flip when one value is zero (0 * anything = 0, not < 0)."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": 0.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "sign_flipped")
        assert not engine._check_expected(e, manifest)


class TestValueRelationMagnitudePreserved:
    """Tests for magnitude_preserved relation."""

    def test_magnitude_preserved_exact(self) -> None:
        """Magnitude preserved when abs(old) == abs(new)."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "magnitude_preserved")
        assert engine._check_expected(e, manifest)

    def test_magnitude_preserved_within_tolerance(self) -> None:
        """Magnitude preserved within 10% tolerance."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -10.0},
                new_value={"dx": 5.0, "dy": 10.5},  # 5% difference
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "magnitude_preserved")
        assert engine._check_expected(e, manifest)

    def test_magnitude_preserved_fails_large_change(self) -> None:
        """Magnitude not preserved when difference exceeds tolerance."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -10.0},
                new_value={"dx": 5.0, "dy": 5.0},  # 50% difference
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "magnitude_preserved")
        assert not engine._check_expected(e, manifest)

    def test_magnitude_preserved_custom_tolerance(self) -> None:
        """Tight tolerance catches small changes."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -10.0},
                new_value={"dx": 5.0, "dy": 10.5},  # 5% difference
                tick=1,
            )],
        )
        # With 1% tolerance, 5% diff should fail
        e = value_relation("ball", "velocity", "dy", "magnitude_preserved", tolerance=0.01)
        assert not engine._check_expected(e, manifest)

    def test_magnitude_preserved_zero_old_fails(self) -> None:
        """Magnitude preserved fails when old value is zero (division guard)."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dy": 0.0},
                new_value={"dy": 5.0},
                tick=1,
            )],
        )
        e = value_relation("ball", "velocity", "dy", "magnitude_preserved")
        assert not engine._check_expected(e, manifest)


class TestValueRelationIncreased:
    """Tests for increased relation."""

    def test_increased_passes(self) -> None:
        """New value is greater than old value."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="score",
                old_value={"value": 10},
                new_value={"value": 20},
                tick=1,
            )],
        )
        e = value_relation("player", "score", "value", "increased")
        assert engine._check_expected(e, manifest)

    def test_increased_fails_when_decreased(self) -> None:
        """Fails when new < old."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="score",
                old_value={"value": 20},
                new_value={"value": 10},
                tick=1,
            )],
        )
        e = value_relation("player", "score", "value", "increased")
        assert not engine._check_expected(e, manifest)

    def test_increased_fails_when_equal(self) -> None:
        """Fails when new == old."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="score",
                old_value={"value": 10},
                new_value={"value": 10},
                tick=1,
            )],
        )
        e = value_relation("player", "score", "value", "increased")
        assert not engine._check_expected(e, manifest)


class TestValueRelationDecreased:
    """Tests for decreased relation."""

    def test_decreased_passes(self) -> None:
        """New value is less than old value."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="health",
                old_value={"hp": 100},
                new_value={"hp": 50},
                tick=1,
            )],
        )
        e = value_relation("enemy", "health", "hp", "decreased")
        assert engine._check_expected(e, manifest)

    def test_decreased_fails_when_increased(self) -> None:
        """Fails when new > old."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="health",
                old_value={"hp": 50},
                new_value={"hp": 100},
                tick=1,
            )],
        )
        e = value_relation("enemy", "health", "hp", "decreased")
        assert not engine._check_expected(e, manifest)


class TestValueRelationInBehaviorIntent:
    """Integration: value_relation in a full behavior intent verification."""

    def test_behavior_with_value_relation_passes(self) -> None:
        """Full behavior verification using value_relation passes."""
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_reflects_on_paddle",
            kind=IntentKind.BEHAVIOR,
            description="Ball velocity.dy flips sign on paddle collision",
            trigger=collision("ball", "paddle"),
            expected=value_relation("ball", "velocity", "dy", "sign_flipped"),
            timeout_ticks=10,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        m0 = _make_manifest(tick=0)
        m1 = _make_manifest(
            tick=1,
            events=[_make_event(
                "collision", "ball hit paddle", [1, 0],
                tick=1, reason_detail="ball:paddle",
            )],
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
            )],
        )
        manifests = [m0, m1]

        report = engine.verify(suite, manifests)
        assert report.all_passed

    def test_behavior_with_value_relation_fails(self) -> None:
        """Behavior verification fails when sign is not flipped."""
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_reflects_on_paddle",
            kind=IntentKind.BEHAVIOR,
            description="Ball velocity.dy flips sign on paddle collision",
            trigger=collision("ball", "paddle"),
            expected=value_relation("ball", "velocity", "dy", "sign_flipped"),
            timeout_ticks=3,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "ball hit paddle", [1, 0],
                tick=0, reason_detail="ball:paddle",
            )],
            # Velocity change but no sign flip on dy
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": 5.0, "dy": -2.0},
                tick=0,
            )],
        )
        manifests = [m0]

        report = engine.verify(suite, manifests)
        assert not report.all_passed
        assert "not met" in report.results[0].failure_reason


# ===========================================================================
# Task 3: Fixed ENTITY_DESPAWNED specificity
# ===========================================================================


class TestEntityDespawnedSpecificity:
    """Tests for entity_despawned matching the entity param."""

    def test_matches_via_event_reason_detail(self) -> None:
        """Matches when event reason_detail contains entity name."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            despawns=[42],
            events=[_make_event(
                "collision", "ball hits brick", [1, 42],
                tick=1, reason_detail="ball:brick",
            )],
        )
        e = entity_despawned("brick")
        assert engine._check_expected(e, manifest)

    def test_matches_via_event_description(self) -> None:
        """Matches when event description contains entity name."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            despawns=[42],
            events=[_make_event(
                "despawn", "brick destroyed", [42],
                tick=1, reason_detail="",
            )],
        )
        e = entity_despawned("brick")
        assert engine._check_expected(e, manifest)

    def test_matches_via_change_reason_detail(self) -> None:
        """Matches when component change on despawned entity has name in reason."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            despawns=[42],
            changes=[_make_change(
                entity_id=42,
                component="health",
                old_value=1,
                new_value=0,
                tick=1,
                reason_detail="brick:destroyed",
            )],
        )
        e = entity_despawned("brick")
        assert engine._check_expected(e, manifest)

    def test_matches_via_identity_component(self) -> None:
        """Matches when identity component has matching role."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            despawns=[42],
            changes=[_make_change(
                entity_id=42,
                component="identity",
                new_value={"role": "brick", "entity_type": "destructible"},
                tick=1,
            )],
        )
        e = entity_despawned("brick")
        assert engine._check_expected(e, manifest)

    def test_matches_via_entity_id_string(self) -> None:
        """Matches when entity param is the entity ID as a string."""
        engine = VerificationEngine()
        manifest = _make_manifest(tick=1, despawns=[42])
        e = entity_despawned("42")
        assert engine._check_expected(e, manifest)

    def test_rejects_wrong_entity_name(self) -> None:
        """Fails when despawned entity does not match the expected name."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            despawns=[42],
            events=[_make_event(
                "collision", "ball hits wall", [1, 42],
                tick=1, reason_detail="ball:wall",
            )],
        )
        e = entity_despawned("brick")
        assert not engine._check_expected(e, manifest)

    def test_rejects_no_despawns(self) -> None:
        """Fails when no entities are despawned."""
        engine = VerificationEngine()
        manifest = _make_manifest(tick=1)
        e = entity_despawned("brick")
        assert not engine._check_expected(e, manifest)


# ===========================================================================
# Task 3: Fixed COMPONENT_CHANGED delta check
# ===========================================================================


class TestComponentChangedDelta:
    """Tests for the delta check on COMPONENT_CHANGED."""

    def test_passes_when_field_value_actually_changed(self) -> None:
        """Passes when the field has different old and new values."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
            )],
        )
        e = component_changed("ball", "velocity", field_name="dy")
        assert engine._check_expected(e, manifest)

    def test_fails_when_field_value_unchanged(self) -> None:
        """Fails when old and new field values are identical."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value={"dx": 5.0, "dy": 3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
            )],
        )
        e = component_changed("ball", "velocity", field_name="dy")
        assert not engine._check_expected(e, manifest)

    def test_passes_when_old_value_is_none(self) -> None:
        """Passes when old_value is None (initial set, not a delta)."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="velocity",
                old_value=None,
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
            )],
        )
        e = component_changed("ball", "velocity", field_name="dy")
        assert engine._check_expected(e, manifest)

    def test_passes_when_no_field_specified_and_values_differ(self) -> None:
        """Passes when no field specified and old != new."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="state",
                old_value="idle",
                new_value="running",
                tick=1,
            )],
        )
        e = component_changed("player", "state")
        assert engine._check_expected(e, manifest)

    def test_fails_when_no_field_and_values_same(self) -> None:
        """Fails when no field specified and old == new."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="state",
                old_value="idle",
                new_value="idle",
                tick=1,
            )],
        )
        e = component_changed("player", "state")
        assert not engine._check_expected(e, manifest)

    def test_passes_with_expected_value_match(self) -> None:
        """Passes when expected_value matches new_value."""
        engine = VerificationEngine()
        manifest = _make_manifest(
            tick=1,
            changes=[_make_change(
                component="score",
                old_value={"value": 0},
                new_value={"value": 10},
                tick=1,
            )],
        )
        e = component_changed("player", "score", field_name="value", expected_value=10)
        assert engine._check_expected(e, manifest)


# ===========================================================================
# Task 4: PhysicsSanityChecker
# ===========================================================================


class TestPhysicsSanityCheckerConstruction:
    """Tests for PhysicsSanityChecker and PhysicsEntityInfo."""

    def test_physics_entity_info_construction(self) -> None:
        """PhysicsEntityInfo constructs with correct fields."""
        info = PhysicsEntityInfo(
            entity_id=1,
            body_type="dynamic",
            restitution=1.0,
            collider_shape="circle",
        )
        assert info.entity_id == 1
        assert info.body_type == "dynamic"
        assert info.restitution == 1.0
        assert info.collider_shape == "circle"

    def test_physics_entity_info_frozen(self) -> None:
        """PhysicsEntityInfo is frozen."""
        info = PhysicsEntityInfo(1, "dynamic", 1.0, "circle")
        try:
            info.body_type = "static"  # type: ignore[misc]
            assert False, "Should have raised FrozenInstanceError"
        except AttributeError:
            pass

    def test_checker_construction(self) -> None:
        """PhysicsSanityChecker accepts a registry dict."""
        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)
        assert len(checker.registry) == 2


class TestPhysicsSanityCheckerBounceDetection:
    """Tests for check_collision_responses detecting missing bounces."""

    def test_no_collision_no_failures(self) -> None:
        """No collision events means no failures."""
        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
        }
        checker = PhysicsSanityChecker(registry)
        manifests = [_make_manifest(tick=0), _make_manifest(tick=1)]

        results = checker.check_collision_responses(manifests)
        assert results == []

    def test_collision_with_bounce_passes(self) -> None:
        """Collision followed by velocity sign flip produces no failure."""
        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "ball hits wall", [1, 2], tick=0,
            )],
            changes=[_make_change(
                entity_id=1,
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=0,
            )],
        )
        m1 = _make_manifest(tick=1)
        manifests = [m0, m1]

        results = checker.check_collision_responses(manifests)
        assert results == []

    def test_collision_without_bounce_fails(self) -> None:
        """Collision with dynamic entity but no velocity sign flip fails."""
        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "ball hits wall", [1, 2], tick=0,
            )],
            # No velocity change
        )
        m1 = _make_manifest(tick=1)
        m2 = _make_manifest(tick=2)
        m3 = _make_manifest(tick=3)
        manifests = [m0, m1, m2, m3]

        results = checker.check_collision_responses(manifests)
        assert len(results) == 1
        assert not results[0].passed
        assert results[0].trigger_tick == 0
        assert "entity 1" in results[0].failure_reason
        assert "restitution=1.0" in results[0].failure_reason
        assert "no velocity sign flip" in results[0].failure_reason
        assert "deferred_unregister" in results[0].suggestion

    def test_collision_with_delayed_bounce_passes(self) -> None:
        """Velocity sign flip within 3 ticks after collision passes."""
        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 0.8, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "ball hits wall", [1, 2], tick=0,
            )],
        )
        m1 = _make_manifest(tick=1)
        # Bounce happens at tick 2 (within 3-tick window)
        m2 = _make_manifest(
            tick=2,
            changes=[_make_change(
                entity_id=1,
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": -5.0, "dy": -3.0},
                tick=2,
            )],
        )
        m3 = _make_manifest(tick=3)
        manifests = [m0, m1, m2, m3]

        results = checker.check_collision_responses(manifests)
        assert results == []

    def test_static_entity_ignored(self) -> None:
        """Static entities in collisions are not checked for bounce."""
        registry = {
            1: PhysicsEntityInfo(1, "static", 0.0, "box"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "wall hits wall", [1, 2], tick=0,
            )],
        )
        manifests = [m0]

        results = checker.check_collision_responses(manifests)
        assert results == []

    def test_zero_restitution_ignored(self) -> None:
        """Dynamic entity with restitution=0 is not checked for bounce."""
        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 0.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "ball hits wall", [1, 2], tick=0,
            )],
        )
        manifests = [m0]

        results = checker.check_collision_responses(manifests)
        assert results == []

    def test_unknown_entity_ignored(self) -> None:
        """Entities not in registry are silently ignored."""
        registry: dict[int, PhysicsEntityInfo] = {}
        checker = PhysicsSanityChecker(registry)

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "unknown hit", [99, 100], tick=0,
            )],
        )
        manifests = [m0]

        results = checker.check_collision_responses(manifests)
        assert results == []

    def test_multiple_collisions_multiple_failures(self) -> None:
        """Multiple missing bounces produce multiple failure results."""
        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }
        checker = PhysicsSanityChecker(registry)

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "ball hits wall", [1, 2], tick=0,
            )],
        )
        m1 = _make_manifest(tick=1)
        m2 = _make_manifest(tick=2)
        m3 = _make_manifest(tick=3)
        m4 = _make_manifest(
            tick=4,
            events=[_make_event(
                "collision", "ball hits wall again", [1, 2], tick=4,
            )],
        )
        m5 = _make_manifest(tick=5)
        m6 = _make_manifest(tick=6)
        m7 = _make_manifest(tick=7)
        manifests = [m0, m1, m2, m3, m4, m5, m6, m7]

        results = checker.check_collision_responses(manifests)
        assert len(results) == 2
        assert results[0].trigger_tick == 0
        assert results[1].trigger_tick == 4


class TestPhysicsSanityIntegration:
    """Integration: physics_registry parameter on VerificationEngine.verify()."""

    def test_physics_registry_appends_sanity_results(self) -> None:
        """Sanity check failures are appended to verification results."""
        engine = VerificationEngine()
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[
                IntentSpec(
                    name="tick-check",
                    kind=IntentKind.BEHAVIOR,
                    description="Event at tick 0",
                    trigger=tick_reached(0),
                    expected=event_emitted("start"),
                    timeout_ticks=3,
                ),
            ],
        )

        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }

        m0 = _make_manifest(
            tick=0,
            events=[
                _make_event("start", "game starts", tick=0),
                _make_event("collision", "ball hits wall", [1, 2], tick=0),
            ],
        )
        m1 = _make_manifest(tick=1)
        m2 = _make_manifest(tick=2)
        m3 = _make_manifest(tick=3)
        manifests = [m0, m1, m2, m3]

        report = engine.verify(suite, manifests, physics_registry=registry)

        # Intent check passes, but physics sanity fails (no bounce)
        assert not report.all_passed
        assert report.total_intents == 2  # 1 intent + 1 sanity failure
        assert report.passed == 1
        assert report.failed == 1

        # The sanity failure
        sanity_result = report.results[1]
        assert "physics_sanity" in sanity_result.intent_name
        assert not sanity_result.passed

    def test_physics_registry_none_skips_checks(self) -> None:
        """No physics checks run when registry is None (default)."""
        engine = VerificationEngine()
        suite = VerificationSuite(name="test", description="test")
        manifests = [_make_manifest(tick=0)]

        report = engine.verify(suite, manifests)
        assert report.all_passed
        assert report.total_intents == 0

    def test_physics_registry_all_passing(self) -> None:
        """No extra failures when all collisions have proper bounces."""
        engine = VerificationEngine()
        suite = VerificationSuite(name="test", description="test")

        registry = {
            1: PhysicsEntityInfo(1, "dynamic", 1.0, "circle"),
            2: PhysicsEntityInfo(2, "static", 0.0, "box"),
        }

        m0 = _make_manifest(
            tick=0,
            events=[_make_event(
                "collision", "ball hits wall", [1, 2], tick=0,
            )],
            changes=[_make_change(
                entity_id=1,
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": -5.0, "dy": -3.0},
                tick=0,
            )],
        )
        manifests = [m0]

        report = engine.verify(suite, manifests, physics_registry=registry)
        assert report.all_passed
        assert report.total_intents == 0  # No intents, sanity passed (not reported)
