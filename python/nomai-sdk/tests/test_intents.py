"""Tests for nomai.intents -- intent spec DSL and serialization.

Tests validate construction, to_dict/from_dict round-trips, and the
complete breakout verification suite expressed using the intent DSL.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from nomai.intents import (
    Expected,
    ExpectedType,
    IntentKind,
    IntentSpec,
    Trigger,
    TriggerType,
    VerificationSuite,
    after,
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
    state_transition,
    tick_reached,
)


# ---------------------------------------------------------------------------
# Trigger construction and round-trip
# ---------------------------------------------------------------------------

class TestTrigger:
    """Tests for Trigger and its constructor functions."""

    def test_collision(self) -> None:
        t = collision("ball", "paddle")
        assert t.type == TriggerType.COLLISION
        assert t.params["entity_a"] == "ball"
        assert t.params["entity_b"] == "paddle"
        assert t.children == []

    def test_state_transition(self) -> None:
        t = state_transition("player", "alive", "dead")
        assert t.type == TriggerType.STATE_TRANSITION
        assert t.params["entity"] == "player"
        assert t.params["from_state"] == "alive"
        assert t.params["to_state"] == "dead"

    def test_aggregate_condition(self) -> None:
        t = aggregate_condition("brick", "==", 0)
        assert t.type == TriggerType.AGGREGATE_CONDITION
        assert t.params["entity_type"] == "brick"
        assert t.params["comparison"] == "=="
        assert t.params["value"] == 0

    def test_component_condition(self) -> None:
        t = component_condition("ball", "velocity", "dx", ">", 0)
        assert t.type == TriggerType.COMPONENT_CONDITION
        assert t.params["entity"] == "ball"
        assert t.params["component"] == "velocity"
        assert t.params["field"] == "dx"

    def test_event_occurred(self) -> None:
        t = event_occurred("collision", involving=["ball", "brick"])
        assert t.type == TriggerType.EVENT_OCCURRED
        assert t.params["event_type"] == "collision"
        assert t.params["involving"] == ["ball", "brick"]

    def test_event_occurred_no_involving(self) -> None:
        t = event_occurred("level_up")
        assert t.type == TriggerType.EVENT_OCCURRED
        assert t.params["event_type"] == "level_up"
        assert "involving" not in t.params

    def test_tick_reached(self) -> None:
        t = tick_reached(100)
        assert t.type == TriggerType.TICK_REACHED
        assert t.params["tick"] == 100

    def test_and_composite(self) -> None:
        t = and_(
            collision("ball", "paddle"),
            component_condition("ball", "velocity", "dy", "<", 0),
        )
        assert t.type == TriggerType.AND
        assert len(t.children) == 2
        assert t.children[0].type == TriggerType.COLLISION
        assert t.children[1].type == TriggerType.COMPONENT_CONDITION

    def test_or_composite(self) -> None:
        t = or_(
            event_occurred("game_over"),
            tick_reached(6000),
        )
        assert t.type == TriggerType.OR
        assert len(t.children) == 2

    def test_to_dict_roundtrip_leaf(self) -> None:
        """Leaf trigger round-trips through to_dict/from_dict."""
        original = collision("ball", "brick")
        d = original.to_dict()
        restored = Trigger.from_dict(d)
        assert restored.type == original.type
        assert restored.params == original.params
        assert restored.children == []

    def test_to_dict_roundtrip_composite(self) -> None:
        """Composite trigger round-trips through to_dict/from_dict."""
        original = and_(
            collision("ball", "paddle"),
            or_(
                event_occurred("power_up"),
                tick_reached(500),
            ),
        )
        d = original.to_dict()
        restored = Trigger.from_dict(d)
        assert restored.type == TriggerType.AND
        assert len(restored.children) == 2
        assert restored.children[0].type == TriggerType.COLLISION
        assert restored.children[1].type == TriggerType.OR
        assert len(restored.children[1].children) == 2

    def test_json_roundtrip(self) -> None:
        """Trigger survives full JSON serialize/deserialize."""
        original = and_(
            collision("ball", "paddle"),
            component_condition("ball", "position", "y", ">", 100),
        )
        json_str = json.dumps(original.to_dict())
        data = json.loads(json_str)
        restored = Trigger.from_dict(data)
        assert restored.type == original.type
        assert len(restored.children) == len(original.children)

    def test_after_trigger(self) -> None:
        """After trigger wraps a child trigger with a tick delay."""
        inner = collision("ball", "paddle")
        t = after(inner, delay_ticks=5)
        assert t.type == TriggerType.AFTER
        assert t.params["delay_ticks"] == 5
        assert len(t.children) == 1
        assert t.children[0] == inner

    def test_after_trigger_round_trip(self) -> None:
        """After trigger survives to_dict/from_dict round trip."""
        inner = collision("ball", "paddle")
        t = after(inner, delay_ticks=5)
        d = t.to_dict()
        restored = Trigger.from_dict(d)
        assert restored == t

    def test_frozen(self) -> None:
        """Trigger is immutable."""
        t = collision("a", "b")
        try:
            t.type = TriggerType.AND  # type: ignore[misc]
            assert False, "Should have raised FrozenInstanceError"
        except AttributeError:
            pass


# ---------------------------------------------------------------------------
# Expected construction and round-trip
# ---------------------------------------------------------------------------

class TestExpected:
    """Tests for Expected and its constructor functions."""

    def test_component_changed(self) -> None:
        e = component_changed("ball", "position", field_name="y")
        assert e.type == ExpectedType.COMPONENT_CHANGED
        assert e.params["entity"] == "ball"
        assert e.params["component"] == "position"
        assert e.params["field"] == "y"

    def test_component_changed_with_value(self) -> None:
        e = component_changed("score", "value", expected_value=100)
        assert e.params["expected_value"] == 100

    def test_entity_despawned(self) -> None:
        e = entity_despawned("brick_0")
        assert e.type == ExpectedType.ENTITY_DESPAWNED
        assert e.params["entity"] == "brick_0"

    def test_aggregate_changed(self) -> None:
        e = aggregate_changed("brick", "<", 10)
        assert e.type == ExpectedType.AGGREGATE_CHANGED
        assert e.params["entity_type"] == "brick"
        assert e.params["comparison"] == "<"
        assert e.params["value"] == 10

    def test_in_state(self) -> None:
        e = in_state("player", "state", "alive")
        assert e.type == ExpectedType.IN_STATE
        assert e.params["entity"] == "player"
        assert e.params["component"] == "state"
        assert e.params["state"] == "alive"

    def test_event_emitted(self) -> None:
        e = event_emitted("score_change", involving=["player"])
        assert e.type == ExpectedType.EVENT_EMITTED
        assert e.params["event_type"] == "score_change"
        assert e.params["involving"] == ["player"]

    def test_event_emitted_no_involving(self) -> None:
        e = event_emitted("game_over")
        assert e.type == ExpectedType.EVENT_EMITTED
        assert "involving" not in e.params

    def test_all_composite(self) -> None:
        e = all_(
            component_changed("ball", "position", field_name="y"),
            event_emitted("bounce"),
        )
        assert e.type == ExpectedType.ALL
        assert len(e.children) == 2

    def test_any_composite(self) -> None:
        e = any_(
            entity_despawned("brick"),
            aggregate_changed("brick", "<", 5),
        )
        assert e.type == ExpectedType.ANY
        assert len(e.children) == 2

    def test_to_dict_roundtrip_leaf(self) -> None:
        original = component_changed("ball", "velocity", field_name="dy")
        d = original.to_dict()
        restored = Expected.from_dict(d)
        assert restored.type == original.type
        assert restored.params == original.params

    def test_to_dict_roundtrip_composite(self) -> None:
        original = all_(
            component_changed("ball", "position"),
            any_(
                event_emitted("bounce"),
                entity_despawned("brick"),
            ),
        )
        d = original.to_dict()
        restored = Expected.from_dict(d)
        assert restored.type == ExpectedType.ALL
        assert len(restored.children) == 2
        assert restored.children[1].type == ExpectedType.ANY

    def test_json_roundtrip(self) -> None:
        original = all_(
            component_changed("ball", "position", field_name="y"),
            entity_despawned("brick_0"),
        )
        json_str = json.dumps(original.to_dict())
        data = json.loads(json_str)
        restored = Expected.from_dict(data)
        assert restored.type == original.type
        assert len(restored.children) == 2

    def test_frozen(self) -> None:
        e = entity_despawned("brick")
        try:
            e.type = ExpectedType.ALL  # type: ignore[misc]
            assert False, "Should have raised FrozenInstanceError"
        except AttributeError:
            pass


# ---------------------------------------------------------------------------
# IntentSpec construction and round-trip
# ---------------------------------------------------------------------------

class TestIntentSpec:
    """Tests for IntentSpec with each kind."""

    def test_entity_intent(self) -> None:
        """Construct an entity intent spec."""
        spec = IntentSpec(
            name="paddle_exists",
            kind=IntentKind.ENTITY,
            description="The paddle entity must exist",
            entity_type="character",
            entity_role="paddle",
            must_exist=True,
            must_be_visible=True,
            required_components=["position", "size"],
        )
        assert spec.name == "paddle_exists"
        assert spec.kind == IntentKind.ENTITY
        assert spec.entity_type == "character"
        assert spec.entity_role == "paddle"
        assert spec.required_components == ["position", "size"]

    def test_behavior_intent(self) -> None:
        """Construct a behavior intent spec."""
        spec = IntentSpec(
            name="ball_bounces_off_paddle",
            kind=IntentKind.BEHAVIOR,
            description="When the ball collides with the paddle, the ball's y-velocity reverses",
            trigger=collision("ball", "paddle"),
            expected=component_changed("ball", "velocity", field_name="dy"),
            timeout_ticks=120,
        )
        assert spec.kind == IntentKind.BEHAVIOR
        assert spec.trigger is not None
        assert spec.trigger.type == TriggerType.COLLISION
        assert spec.expected is not None
        assert spec.expected.type == ExpectedType.COMPONENT_CHANGED
        assert spec.timeout_ticks == 120

    def test_metric_intent(self) -> None:
        """Construct a metric intent spec."""
        spec = IntentSpec(
            name="ball_speed_bounded",
            kind=IntentKind.METRIC,
            description="Ball horizontal speed must stay in range",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="dx",
            metric_range=(-10.0, 10.0),
        )
        assert spec.kind == IntentKind.METRIC
        assert spec.metric_entity == "ball"
        assert spec.metric_component == "velocity"
        assert spec.metric_field == "dx"
        assert spec.metric_range == (-10.0, 10.0)

    def test_invariant_intent(self) -> None:
        """Construct an invariant intent spec."""
        spec = IntentSpec(
            name="ball_in_bounds",
            kind=IntentKind.INVARIANT,
            description="Ball position must stay within game bounds every tick",
            condition="entity('ball').position.x >= 0 and entity('ball').position.x <= 800",
        )
        assert spec.kind == IntentKind.INVARIANT
        assert spec.condition is not None
        assert "ball" in spec.condition

    def test_entity_to_dict_roundtrip(self) -> None:
        original = IntentSpec(
            name="paddle_exists",
            kind=IntentKind.ENTITY,
            description="Paddle must exist",
            entity_type="character",
            entity_role="paddle",
            must_exist=True,
            must_be_visible=True,
            required_components=["position", "size"],
        )
        d = original.to_dict()
        restored = IntentSpec.from_dict(d)
        assert restored.name == original.name
        assert restored.kind == original.kind
        assert restored.entity_type == original.entity_type
        assert restored.entity_role == original.entity_role
        assert restored.must_exist == original.must_exist
        assert restored.must_be_visible == original.must_be_visible
        assert restored.required_components == original.required_components

    def test_behavior_to_dict_roundtrip(self) -> None:
        original = IntentSpec(
            name="ball_bounces",
            kind=IntentKind.BEHAVIOR,
            description="Ball bounces off paddle",
            trigger=collision("ball", "paddle"),
            expected=component_changed("ball", "position", field_name="y"),
            timeout_ticks=120,
        )
        d = original.to_dict()
        restored = IntentSpec.from_dict(d)
        assert restored.name == original.name
        assert restored.kind == original.kind
        assert restored.trigger is not None
        assert restored.trigger.type == TriggerType.COLLISION
        assert restored.expected is not None
        assert restored.expected.type == ExpectedType.COMPONENT_CHANGED
        assert restored.timeout_ticks == 120

    def test_metric_to_dict_roundtrip(self) -> None:
        original = IntentSpec(
            name="ball_speed",
            kind=IntentKind.METRIC,
            description="Ball speed bounded",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="dx",
            metric_range=(-10.0, 10.0),
        )
        d = original.to_dict()
        restored = IntentSpec.from_dict(d)
        assert restored.name == original.name
        assert restored.kind == original.kind
        assert restored.metric_entity == original.metric_entity
        assert restored.metric_component == original.metric_component
        assert restored.metric_field == original.metric_field
        assert restored.metric_range is not None
        assert abs(restored.metric_range[0] - (-10.0)) < 1e-10
        assert abs(restored.metric_range[1] - 10.0) < 1e-10

    def test_invariant_to_dict_roundtrip(self) -> None:
        original = IntentSpec(
            name="ball_in_bounds",
            kind=IntentKind.INVARIANT,
            description="Ball stays in bounds",
            condition="entity('ball').position.x >= 0",
        )
        d = original.to_dict()
        restored = IntentSpec.from_dict(d)
        assert restored.name == original.name
        assert restored.kind == original.kind
        assert restored.condition == original.condition

    def test_to_json_from_json_roundtrip(self) -> None:
        original = IntentSpec(
            name="test_intent",
            kind=IntentKind.BEHAVIOR,
            description="Test behavior",
            trigger=tick_reached(100),
            expected=entity_despawned("enemy"),
            timeout_ticks=200,
        )
        json_str = original.to_json()
        restored = IntentSpec.from_json(json_str)
        assert restored.name == original.name
        assert restored.kind == original.kind
        assert restored.trigger is not None
        assert restored.trigger.type == TriggerType.TICK_REACHED
        assert restored.expected is not None
        assert restored.expected.type == ExpectedType.ENTITY_DESPAWNED


# ---------------------------------------------------------------------------
# VerificationSuite
# ---------------------------------------------------------------------------

class TestVerificationSuite:
    """Tests for VerificationSuite serialization."""

    def test_construction(self) -> None:
        suite = VerificationSuite(
            name="breakout_basic",
            description="Basic breakout game verification",
            intents=[
                IntentSpec(
                    name="paddle_exists",
                    kind=IntentKind.ENTITY,
                    description="Paddle must exist",
                    entity_type="character",
                    entity_role="paddle",
                ),
            ],
        )
        assert suite.name == "breakout_basic"
        assert len(suite.intents) == 1

    def test_json_roundtrip(self) -> None:
        """Full JSON serialization round-trip of a multi-intent suite."""
        suite = VerificationSuite(
            name="breakout_full",
            description="Complete breakout verification",
            intents=[
                IntentSpec(
                    name="paddle_exists",
                    kind=IntentKind.ENTITY,
                    description="Paddle exists with correct role",
                    entity_type="character",
                    entity_role="paddle",
                    required_components=["position", "size"],
                ),
                IntentSpec(
                    name="ball_exists",
                    kind=IntentKind.ENTITY,
                    description="Ball exists with correct role",
                    entity_type="projectile",
                    entity_role="ball",
                    required_components=["position", "velocity"],
                ),
                IntentSpec(
                    name="ball_bounces",
                    kind=IntentKind.BEHAVIOR,
                    description="Ball bounces off paddle",
                    trigger=collision("ball", "paddle"),
                    expected=component_changed("ball", "position", field_name="y"),
                    timeout_ticks=120,
                ),
                IntentSpec(
                    name="ball_speed_bounded",
                    kind=IntentKind.METRIC,
                    description="Ball speed stays in range",
                    metric_entity="ball",
                    metric_component="velocity",
                    metric_field="dx",
                    metric_range=(-10.0, 10.0),
                ),
                IntentSpec(
                    name="ball_in_bounds",
                    kind=IntentKind.INVARIANT,
                    description="Ball position within bounds every tick",
                    condition="entity('ball').position.x >= 0 and entity('ball').position.x <= 800 and entity('ball').position.y >= 0 and entity('ball').position.y <= 600",
                ),
            ],
        )

        json_str = suite.to_json()
        restored = VerificationSuite.from_json(json_str)

        assert restored.name == suite.name
        assert restored.description == suite.description
        assert len(restored.intents) == 5

        # Check each intent kind was preserved
        assert restored.intents[0].kind == IntentKind.ENTITY
        assert restored.intents[0].entity_role == "paddle"

        assert restored.intents[1].kind == IntentKind.ENTITY
        assert restored.intents[1].entity_role == "ball"

        assert restored.intents[2].kind == IntentKind.BEHAVIOR
        assert restored.intents[2].trigger is not None
        assert restored.intents[2].trigger.type == TriggerType.COLLISION
        assert restored.intents[2].expected is not None

        assert restored.intents[3].kind == IntentKind.METRIC
        assert restored.intents[3].metric_range is not None

        assert restored.intents[4].kind == IntentKind.INVARIANT
        assert restored.intents[4].condition is not None

    def test_empty_suite(self) -> None:
        suite = VerificationSuite(
            name="empty",
            description="No intents",
        )
        json_str = suite.to_json()
        restored = VerificationSuite.from_json(json_str)
        assert restored.name == "empty"
        assert len(restored.intents) == 0

    def test_to_dict_roundtrip(self) -> None:
        suite = VerificationSuite(
            name="test",
            description="Test suite",
            intents=[
                IntentSpec(
                    name="inv1",
                    kind=IntentKind.INVARIANT,
                    description="Some invariant",
                    condition="true",
                ),
            ],
        )
        d = suite.to_dict()
        restored = VerificationSuite.from_dict(d)
        assert restored.name == "test"
        assert len(restored.intents) == 1
        assert restored.intents[0].condition == "true"


# ---------------------------------------------------------------------------
# Breakout verification assertions
# ---------------------------------------------------------------------------

class TestBreakoutIntents:
    """Express breakout game verification using the intent DSL.

    These tests don't run against a game engine -- they verify that the
    intent specs can be constructed and serialized correctly for the
    specific breakout game requirements.
    """

    def test_paddle_entity_intent(self) -> None:
        """Entity: paddle exists with role 'paddle'."""
        spec = IntentSpec(
            name="paddle_exists",
            kind=IntentKind.ENTITY,
            description="The paddle entity must exist with the 'paddle' role",
            entity_type="character",
            entity_role="paddle",
            must_exist=True,
            must_be_visible=True,
            required_components=["position", "size"],
        )
        assert spec.entity_role == "paddle"
        assert spec.must_exist is True

        # Verify it survives JSON roundtrip
        json_str = spec.to_json()
        restored = IntentSpec.from_json(json_str)
        assert restored.entity_role == "paddle"
        assert restored.required_components == ["position", "size"]

    def test_ball_entity_intent(self) -> None:
        """Entity: ball exists with role 'ball'."""
        spec = IntentSpec(
            name="ball_exists",
            kind=IntentKind.ENTITY,
            description="The ball entity must exist with the 'ball' role",
            entity_type="projectile",
            entity_role="ball",
            must_exist=True,
            must_be_visible=True,
            required_components=["position", "velocity"],
        )
        assert spec.entity_role == "ball"

        json_str = spec.to_json()
        restored = IntentSpec.from_json(json_str)
        assert restored.entity_role == "ball"

    def test_ball_paddle_collision_behavior(self) -> None:
        """Behavior: when collision(ball, paddle) -> ball position.y changes."""
        spec = IntentSpec(
            name="ball_bounces_off_paddle",
            kind=IntentKind.BEHAVIOR,
            description="When the ball collides with the paddle, the ball's y-position changes",
            trigger=collision("ball", "paddle"),
            expected=component_changed("ball", "position", field_name="y"),
            timeout_ticks=120,
        )
        assert spec.trigger is not None
        assert spec.trigger.type == TriggerType.COLLISION
        assert spec.trigger.params["entity_a"] == "ball"
        assert spec.trigger.params["entity_b"] == "paddle"
        assert spec.expected is not None
        assert spec.expected.type == ExpectedType.COMPONENT_CHANGED
        assert spec.expected.params["entity"] == "ball"
        assert spec.expected.params["component"] == "position"
        assert spec.expected.params["field"] == "y"

        # JSON roundtrip
        json_str = spec.to_json()
        restored = IntentSpec.from_json(json_str)
        assert restored.trigger is not None
        assert restored.trigger.params["entity_a"] == "ball"
        assert restored.expected is not None
        assert restored.expected.params["field"] == "y"

    def test_ball_speed_metric(self) -> None:
        """Metric: ball speed.x in range [-10, 10]."""
        spec = IntentSpec(
            name="ball_speed_x_bounded",
            kind=IntentKind.METRIC,
            description="Ball horizontal speed must stay within [-10, 10]",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="dx",
            metric_range=(-10.0, 10.0),
        )
        assert spec.metric_range == (-10.0, 10.0)

        json_str = spec.to_json()
        restored = IntentSpec.from_json(json_str)
        assert restored.metric_range is not None
        assert restored.metric_range[0] == -10.0
        assert restored.metric_range[1] == 10.0

    def test_ball_in_bounds_invariant(self) -> None:
        """Invariant: ball position within bounds every tick."""
        spec = IntentSpec(
            name="ball_in_bounds",
            kind=IntentKind.INVARIANT,
            description="Ball position must stay within game bounds (0-800 x, 0-600 y) every tick",
            condition=(
                "entity('ball').position.x >= 0 and "
                "entity('ball').position.x <= 800 and "
                "entity('ball').position.y >= 0 and "
                "entity('ball').position.y <= 600"
            ),
        )
        assert "position.x >= 0" in (spec.condition or "")
        assert "position.y <= 600" in (spec.condition or "")

        json_str = spec.to_json()
        restored = IntentSpec.from_json(json_str)
        assert restored.condition is not None
        assert "800" in restored.condition

    def test_complete_breakout_suite(self) -> None:
        """Build and serialize a complete breakout verification suite."""
        suite = VerificationSuite(
            name="breakout_verification",
            description="Complete verification suite for the breakout clone",
            intents=[
                # Entity intents
                IntentSpec(
                    name="paddle_exists",
                    kind=IntentKind.ENTITY,
                    description="Paddle entity must exist",
                    entity_type="character",
                    entity_role="paddle",
                    required_components=["position", "size"],
                ),
                IntentSpec(
                    name="ball_exists",
                    kind=IntentKind.ENTITY,
                    description="Ball entity must exist",
                    entity_type="projectile",
                    entity_role="ball",
                    required_components=["position", "velocity"],
                ),
                # Behavior intent
                IntentSpec(
                    name="ball_bounces_off_paddle",
                    kind=IntentKind.BEHAVIOR,
                    description="Ball bounces when hitting paddle",
                    trigger=collision("ball", "paddle"),
                    expected=component_changed("ball", "position", field_name="y"),
                    timeout_ticks=120,
                ),
                # Metric intent
                IntentSpec(
                    name="ball_speed_x_bounded",
                    kind=IntentKind.METRIC,
                    description="Ball speed.x in [-10, 10]",
                    metric_entity="ball",
                    metric_component="velocity",
                    metric_field="dx",
                    metric_range=(-10.0, 10.0),
                ),
                # Invariant intent
                IntentSpec(
                    name="ball_in_bounds",
                    kind=IntentKind.INVARIANT,
                    description="Ball stays in game bounds",
                    condition=(
                        "entity('ball').position.x >= 0 and "
                        "entity('ball').position.x <= 800 and "
                        "entity('ball').position.y >= 0 and "
                        "entity('ball').position.y <= 600"
                    ),
                ),
            ],
        )

        # Serialize and restore
        json_str = suite.to_json()
        restored = VerificationSuite.from_json(json_str)

        assert restored.name == "breakout_verification"
        assert len(restored.intents) == 5

        # Verify each kind
        kinds = [i.kind for i in restored.intents]
        assert kinds == [
            IntentKind.ENTITY,
            IntentKind.ENTITY,
            IntentKind.BEHAVIOR,
            IntentKind.METRIC,
            IntentKind.INVARIANT,
        ]

        # Verify the JSON is valid and parseable
        parsed = json.loads(json_str)
        assert isinstance(parsed, dict)
        assert len(parsed["intents"]) == 5


# ---------------------------------------------------------------------------
# Intent spec validation
# ---------------------------------------------------------------------------

class TestIntentValidation:
    """Tests for intent spec validation."""

    def test_behavior_intent_missing_trigger_warns(self) -> None:
        """Behavior intent without a trigger produces a validation warning."""
        spec = IntentSpec(
            name="no-trigger",
            kind=IntentKind.BEHAVIOR,
            description="Missing trigger",
            expected=component_changed("ball", "velocity"),
        )
        warnings = spec.validate()
        assert any("trigger" in w.lower() for w in warnings)

    def test_behavior_intent_missing_expected_warns(self) -> None:
        """Behavior intent without expected produces a validation warning."""
        spec = IntentSpec(
            name="no-expected",
            kind=IntentKind.BEHAVIOR,
            description="Missing expected",
            trigger=collision("a", "b"),
        )
        warnings = spec.validate()
        assert any("expected" in w.lower() for w in warnings)

    def test_metric_intent_missing_range_warns(self) -> None:
        """Metric intent without range produces a validation warning."""
        spec = IntentSpec(
            name="no-range",
            kind=IntentKind.METRIC,
            description="Missing range",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="speed",
        )
        warnings = spec.validate()
        assert any("range" in w.lower() for w in warnings)

    def test_metric_intent_inverted_range_warns(self) -> None:
        """Metric intent with min > max produces a validation warning."""
        spec = IntentSpec(
            name="bad-range",
            kind=IntentKind.METRIC,
            description="Inverted range",
            metric_entity="ball",
            metric_component="velocity",
            metric_field="speed",
            metric_range=(100.0, 0.0),
        )
        warnings = spec.validate()
        assert any("range" in w.lower() for w in warnings)

    def test_entity_intent_missing_role_warns(self) -> None:
        """Entity intent without role produces a validation warning."""
        spec = IntentSpec(
            name="no-role",
            kind=IntentKind.ENTITY,
            description="Missing role",
        )
        warnings = spec.validate()
        assert any("role" in w.lower() for w in warnings)

    def test_invariant_intent_missing_condition_warns(self) -> None:
        """Invariant intent without condition produces a validation warning."""
        spec = IntentSpec(
            name="no-cond",
            kind=IntentKind.INVARIANT,
            description="Missing condition",
        )
        warnings = spec.validate()
        assert any("condition" in w.lower() for w in warnings)

    def test_valid_behavior_intent_no_warnings(self) -> None:
        """Well-formed behavior intent produces no warnings."""
        spec = IntentSpec(
            name="valid",
            kind=IntentKind.BEHAVIOR,
            description="Valid behavior",
            trigger=collision("ball", "paddle"),
            expected=component_changed("ball", "velocity"),
        )
        warnings = spec.validate()
        assert warnings == []

    def test_suite_validate_aggregates_warnings(self) -> None:
        """Suite validation collects warnings from all intents."""
        bad1 = IntentSpec(name="a", kind=IntentKind.BEHAVIOR, description="a")
        bad2 = IntentSpec(name="b", kind=IntentKind.INVARIANT, description="b")
        suite = VerificationSuite(name="test", description="test", intents=[bad1, bad2])
        warnings = suite.validate()
        assert len(warnings) >= 2  # At least one per bad intent

    def test_after_trigger_zero_delay_warns(self) -> None:
        """After trigger with delay_ticks <= 0 produces a validation warning."""
        spec = IntentSpec(
            name="bad-after",
            kind=IntentKind.BEHAVIOR,
            description="Zero delay after",
            trigger=after(collision("a", "b"), delay_ticks=0),
            expected=component_changed("ball", "velocity"),
        )
        warnings = spec.validate()
        assert any("delay" in w.lower() for w in warnings)

    def test_empty_and_trigger_warns(self) -> None:
        """AND trigger with no children produces a validation warning."""
        spec = IntentSpec(
            name="empty-and",
            kind=IntentKind.BEHAVIOR,
            description="Empty AND",
            trigger=and_(),
            expected=component_changed("ball", "velocity"),
        )
        warnings = spec.validate()
        assert any("children" in w.lower() or "empty" in w.lower() for w in warnings)


# ---------------------------------------------------------------------------
# Suite file I/O
# ---------------------------------------------------------------------------

class TestSuiteFileIO:
    """Tests for suite file save/load."""

    def test_save_and_load_file(self, tmp_path: Path) -> None:
        """Suite can be saved to and loaded from a JSON file."""
        suite = VerificationSuite(
            name="test-suite",
            description="A test suite",
            intents=[
                IntentSpec(
                    name="entity-test",
                    kind=IntentKind.ENTITY,
                    description="Entity exists",
                    entity_type="character",
                    entity_role="player",
                ),
            ],
        )
        filepath = tmp_path / "suite.json"
        suite.save(filepath)
        assert filepath.exists()

        loaded = VerificationSuite.load(filepath)
        assert loaded.name == suite.name
        assert loaded.description == suite.description
        assert len(loaded.intents) == 1
        assert loaded.intents[0].name == "entity-test"

    def test_load_nonexistent_file_raises(self, tmp_path: Path) -> None:
        """Loading from a nonexistent file raises FileNotFoundError."""
        with pytest.raises(FileNotFoundError):
            VerificationSuite.load(tmp_path / "nope.json")

    def test_save_creates_parent_dirs(self, tmp_path: Path) -> None:
        """Save creates parent directories if they don't exist."""
        suite = VerificationSuite(name="test", description="test")
        filepath = tmp_path / "sub" / "dir" / "suite.json"
        suite.save(filepath)
        assert filepath.exists()
