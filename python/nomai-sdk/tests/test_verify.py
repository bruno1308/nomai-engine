"""Tests for nomai.verify -- verification engine.

Tests validate entity, behavior, metric, and invariant verification
against mock tick manifests, covering both pass and fail cases.
"""

from __future__ import annotations

from nomai.intents import (
    IntentKind,
    IntentSpec,
    VerificationSuite,
    aggregate_changed,
    aggregate_condition,
    all_,
    and_,
    any_,
    collision,
    component_changed,
    component_condition,
    entity_despawned,
    event_emitted,
    event_occurred,
    in_state,
    or_,
    tick_reached,
)
from nomai.manifest import (
    Aggregates,
    CausalChain,
    CausalStep,
    ComponentChange,
    GameEvent,
    TickManifest,
)
from nomai.verify import (
    IntentResult,
    VerificationEngine,
    VerificationReport,
)


# ---------------------------------------------------------------------------
# Helpers for building mock manifests
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
) -> GameEvent:
    """Build a GameEvent for testing."""
    return GameEvent(
        event_type=event_type,
        description=description,
        involved_entities=involved or [],
        caused_by_system=1,
        reason_type="GameRule",
        reason_detail="test",
        tick=tick,
    )


# ---------------------------------------------------------------------------
# Entity verification
# ---------------------------------------------------------------------------

class TestEntityVerification:
    """Tests for _verify_entity."""

    def test_entity_exists_passes(self) -> None:
        """Entity in index matches role -- passes."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="paddle_exists",
            kind=IntentKind.ENTITY,
            description="Paddle must exist",
            entity_type="character",
            entity_role="paddle",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        entity_index = {
            "paddle": {"entity_type": "character", "role": "paddle", "tier": "Semantic"},
        }
        manifests = [_make_manifest(tick=1)]

        # Act
        report = engine.verify(suite, manifests, entity_index)

        # Assert
        assert report.all_passed
        assert report.results[0].passed
        assert report.results[0].intent_name == "paddle_exists"

    def test_entity_missing_fails(self) -> None:
        """Entity not in index and not in manifests -- fails."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="paddle_exists",
            kind=IntentKind.ENTITY,
            description="Paddle must exist",
            entity_type="character",
            entity_role="paddle",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        manifests = [_make_manifest(tick=1)]

        # Act
        report = engine.verify(suite, manifests, entity_index={})

        # Assert
        assert not report.all_passed
        assert not report.results[0].passed
        assert "paddle" in report.results[0].failure_reason
        assert "spawn command" in report.results[0].suggestion

    def test_entity_found_in_manifest_identity(self) -> None:
        """Entity not in index but found via manifest identity change."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_exists",
            kind=IntentKind.ENTITY,
            description="Ball must exist",
            entity_type="projectile",
            entity_role="ball",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        change = _make_change(
            entity_id=1,
            component="identity",
            new_value={"role": "ball", "entity_type": "projectile"},
            tick=1,
        )
        manifests = [_make_manifest(tick=1, changes=[change])]

        # Act
        report = engine.verify(suite, manifests, entity_index={})

        # Assert
        assert report.all_passed
        assert len(report.results[0].evidence) == 1

    def test_entity_type_mismatch_fails(self) -> None:
        """Entity in index but type does not match -- fails."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="paddle_exists",
            kind=IntentKind.ENTITY,
            description="Paddle must be character type",
            entity_type="character",
            entity_role="paddle",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        entity_index = {
            "paddle": {"entity_type": "projectile", "role": "paddle"},
        }
        manifests = [_make_manifest(tick=1)]

        # Act
        report = engine.verify(suite, manifests, entity_index)

        # Assert
        assert not report.all_passed
        assert "does not match" in report.results[0].failure_reason


# ---------------------------------------------------------------------------
# Behavior verification
# ---------------------------------------------------------------------------

class TestBehaviorVerification:
    """Tests for _verify_behavior."""

    def test_behavior_trigger_and_expected_passes(self) -> None:
        """Trigger fires, expected outcome met -- passes."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_bounces",
            kind=IntentKind.BEHAVIOR,
            description="Ball bounces on collision",
            trigger=event_occurred("collision"),
            expected=component_changed("ball", "velocity", field_name="dy"),
            timeout_ticks=10,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        # Tick 1: no events
        m1 = _make_manifest(tick=1)
        # Tick 2: collision event fires (trigger)
        m2 = _make_manifest(
            tick=2,
            events=[_make_event("collision", "ball hit paddle", [0, 1], tick=2)],
            changes=[
                _make_change(
                    entity_id=0,
                    component="velocity",
                    old_value={"dx": 5.0, "dy": -3.0},
                    new_value={"dx": 5.0, "dy": 3.0},
                    tick=2,
                ),
            ],
        )
        manifests = [m1, m2]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed
        assert report.results[0].trigger_tick == 2
        assert len(report.results[0].evidence) > 0

    def test_behavior_trigger_never_fires(self) -> None:
        """Trigger never fires across all ticks -- fails."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_bounces",
            kind=IntentKind.BEHAVIOR,
            description="Ball bounces on collision",
            trigger=event_occurred("collision"),
            expected=component_changed("ball", "velocity"),
            timeout_ticks=10,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        manifests = [_make_manifest(tick=i) for i in range(5)]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        assert "never fired" in report.results[0].failure_reason

    def test_behavior_expected_never_met(self) -> None:
        """Trigger fires but expected outcome not met within timeout -- fails."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_bounces",
            kind=IntentKind.BEHAVIOR,
            description="Ball bounces on collision",
            trigger=event_occurred("collision"),
            expected=component_changed("ball", "velocity", field_name="dy"),
            timeout_ticks=3,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        # Tick 0: collision event (trigger fires)
        m0 = _make_manifest(
            tick=0,
            events=[_make_event("collision", "ball hit paddle", tick=0)],
        )
        # Ticks 1-3: no velocity changes
        m1 = _make_manifest(tick=1)
        m2 = _make_manifest(tick=2)
        manifests = [m0, m1, m2]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        result = report.results[0]
        assert result.trigger_tick == 0
        assert "not met" in result.failure_reason
        assert "gameplay logic" in result.suggestion

    def test_behavior_tick_reached_trigger(self) -> None:
        """TICK_REACHED trigger fires at the correct tick."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="spawn_at_tick_5",
            kind=IntentKind.BEHAVIOR,
            description="Something spawns at tick 5",
            trigger=tick_reached(5),
            expected=event_emitted("spawn"),
            timeout_ticks=3,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(tick=i) for i in range(5)
        ] + [
            _make_manifest(
                tick=5,
                events=[_make_event("spawn", "entity spawned", tick=5)],
            ),
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed
        assert report.results[0].trigger_tick == 5

    def test_behavior_component_condition_trigger(self) -> None:
        """COMPONENT_CONDITION trigger fires on matching change."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="speed_triggers_boost",
            kind=IntentKind.BEHAVIOR,
            description="When speed > 10, boost event fires",
            trigger=component_condition("ball", "velocity", "dx", ">", 10),
            expected=event_emitted("boost"),
            timeout_ticks=5,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        m0 = _make_manifest(tick=0)
        m1 = _make_manifest(
            tick=1,
            changes=[
                _make_change(
                    component="velocity",
                    new_value={"dx": 15.0, "dy": 0.0},
                    tick=1,
                ),
            ],
        )
        m2 = _make_manifest(
            tick=2,
            events=[_make_event("boost", "speed boost activated", tick=2)],
        )
        manifests = [m0, m1, m2]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_behavior_aggregate_trigger(self) -> None:
        """AGGREGATE_CONDITION trigger fires when brick count reaches 0."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="level_complete",
            kind=IntentKind.BEHAVIOR,
            description="Level completes when all bricks destroyed",
            trigger=aggregate_condition("brick", "==", 0),
            expected=event_emitted("level_complete"),
            timeout_ticks=5,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        m0 = _make_manifest(tick=0, aggregates=_empty_aggregates({"brick": 3}))
        m1 = _make_manifest(tick=1, aggregates=_empty_aggregates({"brick": 1}))
        m2 = _make_manifest(
            tick=2,
            aggregates=_empty_aggregates({"brick": 0}),
            events=[_make_event("level_complete", "all bricks destroyed", tick=2)],
        )
        manifests = [m0, m1, m2]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_behavior_collision_trigger(self) -> None:
        """COLLISION trigger fires when collision event appears."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_brick_collision",
            kind=IntentKind.BEHAVIOR,
            description="Ball hits brick",
            trigger=collision("ball", "brick"),
            expected=entity_despawned("brick"),
            timeout_ticks=5,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        m0 = _make_manifest(tick=0)
        m1 = _make_manifest(
            tick=1,
            events=[_make_event("collision", "ball hit brick", [0, 1], tick=1)],
            despawns=[1],
        )
        manifests = [m0, m1]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_behavior_and_trigger(self) -> None:
        """AND trigger requires all children to match."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="complex_trigger",
            kind=IntentKind.BEHAVIOR,
            description="AND trigger test",
            trigger=and_(
                tick_reached(2),
                event_occurred("collision"),
            ),
            expected=event_emitted("response"),
            timeout_ticks=3,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        # Tick 1: collision but tick < 2
        m1 = _make_manifest(
            tick=1,
            events=[_make_event("collision", tick=1)],
        )
        # Tick 2: tick >= 2 but no collision
        m2 = _make_manifest(tick=2)
        # Tick 3: both conditions met
        m3 = _make_manifest(
            tick=3,
            events=[
                _make_event("collision", tick=3),
                _make_event("response", tick=3),
            ],
        )
        manifests = [m1, m2, m3]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_behavior_or_trigger(self) -> None:
        """OR trigger requires at least one child to match."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="or_trigger",
            kind=IntentKind.BEHAVIOR,
            description="OR trigger test",
            trigger=or_(
                event_occurred("timeout"),
                tick_reached(100),
            ),
            expected=event_emitted("game_over"),
            timeout_ticks=5,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        # Tick 50: timeout event fires (first OR branch)
        m = _make_manifest(
            tick=50,
            events=[
                _make_event("timeout", tick=50),
                _make_event("game_over", tick=50),
            ],
        )
        manifests = [_make_manifest(tick=0), m]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_behavior_no_trigger_defined(self) -> None:
        """Behavior intent with no trigger fails gracefully."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="no_trigger",
            kind=IntentKind.BEHAVIOR,
            description="Missing trigger",
            expected=event_emitted("something"),
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        manifests = [_make_manifest(tick=0)]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        assert "no trigger" in report.results[0].failure_reason.lower()


# ---------------------------------------------------------------------------
# Metric verification
# ---------------------------------------------------------------------------

class TestMetricVerification:
    """Tests for _verify_metric."""

    def test_metric_in_range_passes(self) -> None:
        """All values within range -- passes."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_speed_bounded",
            kind=IntentKind.METRIC,
            description="Ball speed stays in range",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="dx",
            metric_range=(-10.0, 10.0),
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(
                tick=i,
                changes=[
                    _make_change(
                        component="velocity",
                        new_value={"dx": float(i), "dy": 0.0},
                        tick=i,
                    ),
                ],
            )
            for i in range(5)
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_metric_out_of_range_fails(self) -> None:
        """Value exceeds range -- fails with correct tick."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_speed_bounded",
            kind=IntentKind.METRIC,
            description="Ball speed stays in range",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="dx",
            metric_range=(-10.0, 10.0),
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(
                tick=0,
                changes=[
                    _make_change(
                        component="velocity",
                        new_value={"dx": 5.0, "dy": 0.0},
                        tick=0,
                    ),
                ],
            ),
            _make_manifest(
                tick=1,
                changes=[
                    _make_change(
                        component="velocity",
                        new_value={"dx": 15.0, "dy": 0.0},
                        tick=1,
                    ),
                ],
            ),
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        result = report.results[0]
        assert result.trigger_tick == 1
        assert "15.0" in result.failure_reason
        assert "out of range" in result.failure_reason
        assert len(result.evidence) == 1

    def test_metric_no_range_fails(self) -> None:
        """Metric intent without range defined fails gracefully."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="no_range",
            kind=IntentKind.METRIC,
            description="Missing range",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="dx",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        manifests = [_make_manifest(tick=0)]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        assert "no metric_range" in report.results[0].failure_reason.lower()

    def test_metric_negative_range(self) -> None:
        """Metric with negative range boundaries works correctly."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="temperature_bounded",
            kind=IntentKind.METRIC,
            description="Temperature in valid range",
            metric_entity="env",
            metric_component="temperature",
            metric_field="value",
            metric_range=(-50.0, 50.0),
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(
                tick=0,
                changes=[
                    _make_change(
                        component="temperature",
                        new_value={"value": -60.0},
                        tick=0,
                    ),
                ],
            ),
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        assert "-60.0" in report.results[0].failure_reason


# ---------------------------------------------------------------------------
# Invariant verification
# ---------------------------------------------------------------------------

class TestInvariantVerification:
    """Tests for _verify_invariant."""

    def test_invariant_holds_passes(self) -> None:
        """Aggregate invariant holds on every tick -- passes."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="bricks_exist",
            kind=IntentKind.INVARIANT,
            description="There must always be bricks",
            condition="aggregate:brick > 0",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(tick=i, aggregates=_empty_aggregates({"brick": 10 - i}))
            for i in range(5)
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_invariant_violated_fails(self) -> None:
        """Aggregate invariant violated on some tick -- fails."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="bricks_exist",
            kind=IntentKind.INVARIANT,
            description="There must always be bricks",
            condition="aggregate:brick > 0",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(tick=0, aggregates=_empty_aggregates({"brick": 5})),
            _make_manifest(tick=1, aggregates=_empty_aggregates({"brick": 3})),
            _make_manifest(tick=2, aggregates=_empty_aggregates({"brick": 0})),
            _make_manifest(tick=3, aggregates=_empty_aggregates({"brick": 0})),
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        result = report.results[0]
        assert result.trigger_tick == 2
        assert "violated" in result.failure_reason

    def test_entity_count_invariant_holds(self) -> None:
        """Entity count invariant holds -- passes."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="entities_exist",
            kind=IntentKind.INVARIANT,
            description="Total entities must be positive",
            condition="entity_count > 0",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(tick=0, aggregates=_empty_aggregates(total=5)),
            _make_manifest(tick=1, aggregates=_empty_aggregates(total=3)),
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_entity_count_invariant_violated(self) -> None:
        """Entity count invariant violated -- fails."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="entities_exist",
            kind=IntentKind.INVARIANT,
            description="Total entities must be positive",
            condition="entity_count > 0",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        manifests = [
            _make_manifest(tick=0, aggregates=_empty_aggregates(total=5)),
            _make_manifest(tick=1, aggregates=_empty_aggregates(total=0)),
        ]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed
        assert report.results[0].trigger_tick == 1

    def test_freeform_invariant_passes_with_warning(self) -> None:
        """Free-form invariant passes trivially in the spike."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="ball_in_bounds",
            kind=IntentKind.INVARIANT,
            description="Ball stays in bounds",
            condition="entity('ball').position.x >= 0 and entity('ball').position.x <= 800",
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )
        manifests = [_make_manifest(tick=0)]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed
        assert "free-form" in report.results[0].suggestion.lower()


# ---------------------------------------------------------------------------
# VerificationReport
# ---------------------------------------------------------------------------

class TestVerificationReport:
    """Tests for VerificationReport structure and summary."""

    def test_verification_report_summary(self) -> None:
        """Report generates readable summary text."""
        # Arrange
        report = VerificationReport(
            suite_name="breakout_test",
            total_intents=3,
            passed=2,
            failed=1,
            results=[
                IntentResult(intent_name="paddle_exists", passed=True),
                IntentResult(intent_name="ball_exists", passed=True),
                IntentResult(
                    intent_name="ball_bounces",
                    passed=False,
                    failure_reason="Trigger never fired across 100 ticks",
                    suggestion="Check collision system",
                ),
            ],
            wall_time_ms=12.5,
            ticks_examined=100,
        )

        # Act
        summary = report.summary()

        # Assert
        assert "breakout_test" in summary
        assert "Passed: 2" in summary
        assert "Failed: 1" in summary
        assert "[PASS] paddle_exists" in summary
        assert "[PASS] ball_exists" in summary
        assert "[FAIL] ball_bounces" in summary
        assert "Trigger never fired" in summary
        assert "Check collision system" in summary
        assert "1 FAILED" in summary

    def test_all_passed_report(self) -> None:
        """Report with all intents passed."""
        # Arrange
        report = VerificationReport(
            suite_name="happy_path",
            total_intents=2,
            passed=2,
            failed=0,
            results=[
                IntentResult(intent_name="a", passed=True),
                IntentResult(intent_name="b", passed=True),
            ],
        )

        # Assert
        assert report.all_passed
        assert len(report.failures()) == 0
        assert "ALL PASSED" in report.summary()
        assert "All intents passed" in report.diagnosis()

    def test_failures_method(self) -> None:
        """failures() returns only failed results."""
        # Arrange
        report = VerificationReport(
            suite_name="mixed",
            total_intents=3,
            passed=1,
            failed=2,
            results=[
                IntentResult(intent_name="a", passed=True),
                IntentResult(intent_name="b", passed=False, failure_reason="reason_b"),
                IntentResult(intent_name="c", passed=False, failure_reason="reason_c"),
            ],
        )

        # Act
        failures = report.failures()

        # Assert
        assert len(failures) == 2
        assert failures[0].intent_name == "b"
        assert failures[1].intent_name == "c"

    def test_diagnosis_includes_evidence(self) -> None:
        """diagnosis() includes evidence and causal chain info."""
        # Arrange
        evidence = [
            _make_change(
                entity_id=42,
                component="health",
                old_value=100,
                new_value=0,
                tick=5,
                reason_type="GameRule",
                reason_detail="damage",
            ),
        ]
        chain = CausalChain(
            entity_id=42,
            component="health",
            steps=[
                CausalStep(
                    tick=5,
                    command_index=0,
                    system_id=1,
                    reason_type="GameRule",
                    reason_detail="damage",
                    description="Health reduced to 0",
                ),
            ],
        )
        report = VerificationReport(
            suite_name="diag_test",
            total_intents=1,
            passed=0,
            failed=1,
            results=[
                IntentResult(
                    intent_name="enemy_survives",
                    passed=False,
                    failure_reason="Enemy health reached 0",
                    evidence=evidence,
                    causal_chain=chain,
                    suggestion="Increase enemy health",
                ),
            ],
        )

        # Act
        diag = report.diagnosis()

        # Assert
        assert "VERIFICATION FAILED" in diag
        assert "enemy_survives" in diag
        assert "1 component change" in diag
        assert "entity 42 health" in diag
        assert "100 -> 0" in diag
        assert "Causal chain (1 steps)" in diag
        assert "Health reduced to 0" in diag
        assert "Increase enemy health" in diag

    def test_report_to_dict_roundtrip(self) -> None:
        """Report serializes to dict correctly."""
        # Arrange
        report = VerificationReport(
            suite_name="test_suite",
            total_intents=2,
            passed=1,
            failed=1,
            results=[
                IntentResult(intent_name="pass_intent", passed=True),
                IntentResult(
                    intent_name="fail_intent",
                    passed=False,
                    failure_reason="something broke",
                ),
            ],
            wall_time_ms=5.0,
            ticks_examined=10,
        )

        # Act
        d = report.to_dict()

        # Assert
        assert d["suite_name"] == "test_suite"
        assert d["total_intents"] == 2
        assert d["passed"] == 1
        assert d["failed"] == 1
        assert d["all_passed"] is False
        assert d["wall_time_ms"] == 5.0
        assert d["ticks_examined"] == 10
        assert len(d["results"]) == 2  # type: ignore[arg-type]


# ---------------------------------------------------------------------------
# IntentResult
# ---------------------------------------------------------------------------

class TestIntentResult:
    """Tests for IntentResult to_dict."""

    def test_to_dict_passing(self) -> None:
        """Passing result serializes correctly."""
        result = IntentResult(intent_name="test", passed=True)
        d = result.to_dict()
        assert d["intent_name"] == "test"
        assert d["passed"] is True
        assert d["failure_reason"] == ""
        assert d["trigger_tick"] is None
        assert d["evidence"] == []
        assert d["causal_chain"] is None

    def test_to_dict_failing_with_evidence(self) -> None:
        """Failing result with evidence serializes correctly."""
        evidence = [_make_change(entity_id=1, component="hp", tick=3)]
        result = IntentResult(
            intent_name="fail_test",
            passed=False,
            failure_reason="health dropped",
            trigger_tick=3,
            evidence=evidence,
            suggestion="Heal the entity",
        )
        d = result.to_dict()
        assert d["passed"] is False
        assert d["trigger_tick"] == 3
        assert len(d["evidence"]) == 1  # type: ignore[arg-type]
        assert d["suggestion"] == "Heal the entity"


# ---------------------------------------------------------------------------
# Full suite integration
# ---------------------------------------------------------------------------

class TestFullSuiteVerification:
    """Integration tests with mixed intent types in one suite."""

    def test_full_suite_mixed_results(self) -> None:
        """Suite with mix of pass/fail from different intent types."""
        # Arrange
        engine = VerificationEngine()
        suite = VerificationSuite(
            name="breakout_mixed",
            description="Mixed pass/fail suite",
            intents=[
                # Entity: passes (found in index)
                IntentSpec(
                    name="paddle_exists",
                    kind=IntentKind.ENTITY,
                    description="Paddle must exist",
                    entity_type="character",
                    entity_role="paddle",
                ),
                # Entity: fails (missing)
                IntentSpec(
                    name="powerup_exists",
                    kind=IntentKind.ENTITY,
                    description="Powerup must exist",
                    entity_type="item",
                    entity_role="powerup",
                ),
                # Behavior: passes
                IntentSpec(
                    name="bounce_behavior",
                    kind=IntentKind.BEHAVIOR,
                    description="Bounce on tick 2",
                    trigger=tick_reached(2),
                    expected=component_changed("ball", "velocity", field_name="dy"),
                    timeout_ticks=5,
                ),
                # Metric: passes
                IntentSpec(
                    name="speed_bounded",
                    kind=IntentKind.METRIC,
                    description="Speed in range",
                    metric_entity="ball",
                    metric_component="velocity",
                    metric_field="dx",
                    metric_range=(-20.0, 20.0),
                ),
                # Invariant: fails (brick count drops to 0)
                IntentSpec(
                    name="bricks_always_exist",
                    kind=IntentKind.INVARIANT,
                    description="Bricks should always exist",
                    condition="aggregate:brick > 0",
                ),
            ],
        )

        entity_index = {
            "paddle": {"entity_type": "character", "role": "paddle"},
        }

        manifests = [
            _make_manifest(
                tick=0,
                aggregates=_empty_aggregates({"brick": 5}),
            ),
            _make_manifest(
                tick=1,
                aggregates=_empty_aggregates({"brick": 3}),
                changes=[
                    _make_change(
                        component="velocity",
                        new_value={"dx": 5.0, "dy": 3.0},
                        tick=1,
                    ),
                ],
            ),
            _make_manifest(
                tick=2,
                aggregates=_empty_aggregates({"brick": 1}),
                changes=[
                    _make_change(
                        component="velocity",
                        new_value={"dx": 5.0, "dy": -3.0},
                        tick=2,
                    ),
                ],
            ),
            _make_manifest(
                tick=3,
                aggregates=_empty_aggregates({"brick": 0}),
            ),
        ]

        # Act
        report = engine.verify(suite, manifests, entity_index)

        # Assert
        assert not report.all_passed
        assert report.total_intents == 5
        assert report.passed == 3  # paddle_exists, bounce, speed_bounded
        assert report.failed == 2  # powerup_exists, bricks_always_exist
        assert report.ticks_examined == 4

        # Check specific results
        assert report.results[0].passed  # paddle_exists
        assert not report.results[1].passed  # powerup_exists
        assert report.results[2].passed  # bounce_behavior
        assert report.results[3].passed  # speed_bounded
        assert not report.results[4].passed  # bricks_always_exist

        # Verify summary contains key info
        summary = report.summary()
        assert "Passed: 3" in summary
        assert "Failed: 2" in summary
        assert "2 FAILED" in summary

    def test_full_suite_all_pass(self) -> None:
        """Suite where every intent passes."""
        # Arrange
        engine = VerificationEngine()
        suite = VerificationSuite(
            name="happy_suite",
            description="Everything works",
            intents=[
                IntentSpec(
                    name="paddle_exists",
                    kind=IntentKind.ENTITY,
                    description="Paddle exists",
                    entity_role="paddle",
                ),
                IntentSpec(
                    name="tick_trigger",
                    kind=IntentKind.BEHAVIOR,
                    description="Event after tick 0",
                    trigger=tick_reached(0),
                    expected=event_emitted("start"),
                    timeout_ticks=3,
                ),
                IntentSpec(
                    name="speed_ok",
                    kind=IntentKind.METRIC,
                    description="Speed bounded",
                    metric_component="velocity",
                    metric_field="dx",
                    metric_range=(-100.0, 100.0),
                ),
                IntentSpec(
                    name="entities_positive",
                    kind=IntentKind.INVARIANT,
                    description="Entity count > 0",
                    condition="entity_count > 0",
                ),
            ],
        )

        entity_index = {"paddle": {"role": "paddle"}}

        manifests = [
            _make_manifest(
                tick=0,
                events=[_make_event("start", tick=0)],
                changes=[
                    _make_change(component="velocity", new_value={"dx": 5.0}, tick=0),
                ],
                aggregates=_empty_aggregates(total=3),
            ),
            _make_manifest(
                tick=1,
                changes=[
                    _make_change(component="velocity", new_value={"dx": -3.0}, tick=1),
                ],
                aggregates=_empty_aggregates(total=3),
            ),
        ]

        # Act
        report = engine.verify(suite, manifests, entity_index)

        # Assert
        assert report.all_passed
        assert report.total_intents == 4
        assert report.passed == 4
        assert report.failed == 0
        assert "ALL PASSED" in report.summary()

    def test_empty_suite(self) -> None:
        """Empty suite produces a report with no results."""
        # Arrange
        engine = VerificationEngine()
        suite = VerificationSuite(
            name="empty",
            description="Nothing to verify",
        )
        manifests = [_make_manifest(tick=0)]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed
        assert report.total_intents == 0
        assert report.passed == 0
        assert report.failed == 0

    def test_empty_manifests(self) -> None:
        """Suite against empty manifest list: entity fails, trigger never fires."""
        # Arrange
        engine = VerificationEngine()
        suite = VerificationSuite(
            name="no_manifests",
            description="No simulation data",
            intents=[
                IntentSpec(
                    name="paddle_exists",
                    kind=IntentKind.ENTITY,
                    description="Paddle must exist",
                    entity_role="paddle",
                ),
                IntentSpec(
                    name="bounce",
                    kind=IntentKind.BEHAVIOR,
                    description="Bounce behavior",
                    trigger=event_occurred("collision"),
                    expected=event_emitted("bounce"),
                ),
            ],
        )

        # Act
        report = engine.verify(suite, [], entity_index={})

        # Assert
        assert not report.all_passed
        assert report.failed == 2


# ---------------------------------------------------------------------------
# Expected outcome: ALL / ANY composite
# ---------------------------------------------------------------------------

class TestCompositeExpected:
    """Tests for ALL and ANY expected outcome evaluation."""

    def test_all_expected_passes(self) -> None:
        """ALL expected: every child must match."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="multi_outcome",
            kind=IntentKind.BEHAVIOR,
            description="Multiple outcomes expected",
            trigger=tick_reached(1),
            expected=all_(
                component_changed("ball", "velocity", field_name="dy"),
                event_emitted("bounce"),
            ),
            timeout_ticks=5,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        m = _make_manifest(
            tick=1,
            changes=[
                _make_change(component="velocity", new_value={"dx": 5.0, "dy": 3.0}, tick=1),
            ],
            events=[_make_event("bounce", tick=1)],
        )
        manifests = [m]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed

    def test_all_expected_fails_if_one_missing(self) -> None:
        """ALL expected: fails if any child does not match."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="multi_outcome",
            kind=IntentKind.BEHAVIOR,
            description="Multiple outcomes expected",
            trigger=tick_reached(0),
            expected=all_(
                component_changed("ball", "velocity", field_name="dy"),
                event_emitted("bounce"),
            ),
            timeout_ticks=2,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        # Velocity changes but no bounce event
        m = _make_manifest(
            tick=0,
            changes=[
                _make_change(component="velocity", new_value={"dx": 5.0, "dy": 3.0}, tick=0),
            ],
        )
        manifests = [m]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert not report.all_passed

    def test_any_expected_passes_with_one(self) -> None:
        """ANY expected: passes if at least one child matches."""
        # Arrange
        engine = VerificationEngine()
        intent = IntentSpec(
            name="any_outcome",
            kind=IntentKind.BEHAVIOR,
            description="At least one outcome",
            trigger=tick_reached(0),
            expected=any_(
                event_emitted("bounce"),
                entity_despawned("brick"),
            ),
            timeout_ticks=3,
        )
        suite = VerificationSuite(
            name="test",
            description="test",
            intents=[intent],
        )

        # Only despawn, no bounce event
        m = _make_manifest(tick=0, despawns=[5])
        manifests = [m]

        # Act
        report = engine.verify(suite, manifests)

        # Assert
        assert report.all_passed


# ---------------------------------------------------------------------------
# Comparison helper
# ---------------------------------------------------------------------------

class TestCompareHelper:
    """Tests for the _compare numeric comparison helper."""

    def test_all_operators(self) -> None:
        """All comparison operators work correctly."""
        engine = VerificationEngine()
        assert engine._compare(5.0, "==", 5.0)
        assert not engine._compare(5.0, "==", 6.0)
        assert engine._compare(5.0, "!=", 6.0)
        assert not engine._compare(5.0, "!=", 5.0)
        assert engine._compare(5.0, "<", 6.0)
        assert not engine._compare(5.0, "<", 5.0)
        assert engine._compare(5.0, "<=", 5.0)
        assert engine._compare(5.0, "<=", 6.0)
        assert engine._compare(6.0, ">", 5.0)
        assert not engine._compare(5.0, ">", 5.0)
        assert engine._compare(5.0, ">=", 5.0)
        assert engine._compare(6.0, ">=", 5.0)

    def test_unknown_operator_returns_false(self) -> None:
        """Unknown operator returns False."""
        engine = VerificationEngine()
        assert not engine._compare(5.0, "~=", 5.0)
