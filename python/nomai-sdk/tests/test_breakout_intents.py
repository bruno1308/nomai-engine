"""Tests for nomai.breakout_intents -- canonical breakout verification suite.

Validates the suite construction, intent counts per kind, JSON roundtrip,
and validation (no warnings on evaluable intents).
"""

from __future__ import annotations

import json

from nomai.breakout_intents import build_breakout_suite
from nomai.intents import IntentKind, VerificationSuite


class TestBuildBreakoutSuite:
    """Tests for build_breakout_suite()."""

    def test_suite_returns_verification_suite(self) -> None:
        """build_breakout_suite returns a VerificationSuite."""
        suite = build_breakout_suite()
        assert isinstance(suite, VerificationSuite)

    def test_suite_name_and_description(self) -> None:
        """Suite has expected name and non-empty description."""
        suite = build_breakout_suite()
        assert suite.name == "breakout_verification"
        assert len(suite.description) > 0

    def test_total_intent_count(self) -> None:
        """Suite contains exactly 13 intents."""
        suite = build_breakout_suite()
        assert len(suite.intents) == 13

    def test_entity_intent_count(self) -> None:
        """Suite has exactly 3 entity intents."""
        suite = build_breakout_suite()
        entities = [i for i in suite.intents if i.kind == IntentKind.ENTITY]
        assert len(entities) == 3

    def test_behavior_intent_count(self) -> None:
        """Suite has exactly 6 behavior intents."""
        suite = build_breakout_suite()
        behaviors = [i for i in suite.intents if i.kind == IntentKind.BEHAVIOR]
        assert len(behaviors) == 6

    def test_metric_intent_count(self) -> None:
        """Suite has exactly 2 metric intents."""
        suite = build_breakout_suite()
        metrics = [i for i in suite.intents if i.kind == IntentKind.METRIC]
        assert len(metrics) == 2

    def test_invariant_intent_count(self) -> None:
        """Suite has exactly 2 invariant intents."""
        suite = build_breakout_suite()
        invariants = [i for i in suite.intents if i.kind == IntentKind.INVARIANT]
        assert len(invariants) == 2

    def test_entity_intent_names(self) -> None:
        """Entity intents have expected names."""
        suite = build_breakout_suite()
        entities = [i for i in suite.intents if i.kind == IntentKind.ENTITY]
        names = {i.name for i in entities}
        assert names == {"paddle_exists", "ball_exists", "bricks_exist"}

    def test_behavior_intent_names(self) -> None:
        """Behavior intents have expected names."""
        suite = build_breakout_suite()
        behaviors = [i for i in suite.intents if i.kind == IntentKind.BEHAVIOR]
        names = {i.name for i in behaviors}
        assert names == {
            "ball_bounces_off_walls",
            "ball_bounces_off_paddle",
            "brick_destroyed_on_hit",
            "ball_reflects_on_brick_collision",
            "ball_reflects_on_wall_collision",
            "game_won_when_no_bricks",
        }

    def test_paddle_entity_details(self) -> None:
        """Paddle entity has correct type, role, and components."""
        suite = build_breakout_suite()
        paddle = next(i for i in suite.intents if i.name == "paddle_exists")
        assert paddle.entity_type == "character"
        assert paddle.entity_role == "paddle"
        assert paddle.must_exist is True
        assert "position" in paddle.required_components
        assert "size" in paddle.required_components

    def test_ball_entity_details(self) -> None:
        """Ball entity has correct type, role, and components."""
        suite = build_breakout_suite()
        ball = next(i for i in suite.intents if i.name == "ball_exists")
        assert ball.entity_type == "projectile"
        assert ball.entity_role == "ball"
        assert "position" in ball.required_components
        assert "velocity" in ball.required_components

    def test_bricks_entity_details(self) -> None:
        """Bricks entity has correct type and role."""
        suite = build_breakout_suite()
        bricks = next(i for i in suite.intents if i.name == "bricks_exist")
        assert bricks.entity_type == "destructible"
        assert bricks.entity_role == "brick"

    def test_behavior_intents_have_triggers(self) -> None:
        """All behavior intents have non-None triggers."""
        suite = build_breakout_suite()
        behaviors = [i for i in suite.intents if i.kind == IntentKind.BEHAVIOR]
        for b in behaviors:
            assert b.trigger is not None, f"{b.name} has no trigger"

    def test_behavior_intents_have_expected(self) -> None:
        """All behavior intents have non-None expected outcomes."""
        suite = build_breakout_suite()
        behaviors = [i for i in suite.intents if i.kind == IntentKind.BEHAVIOR]
        for b in behaviors:
            assert b.expected is not None, f"{b.name} has no expected"

    def test_metric_intents_have_ranges(self) -> None:
        """All metric intents have valid ranges."""
        suite = build_breakout_suite()
        metrics = [i for i in suite.intents if i.kind == IntentKind.METRIC]
        for m in metrics:
            assert m.metric_range is not None, f"{m.name} has no range"
            assert m.metric_range[0] <= m.metric_range[1], (
                f"{m.name} range is inverted"
            )

    def test_invariant_intents_have_conditions(self) -> None:
        """All invariant intents have non-empty conditions."""
        suite = build_breakout_suite()
        invariants = [i for i in suite.intents if i.kind == IntentKind.INVARIANT]
        for inv in invariants:
            assert inv.condition, f"{inv.name} has no condition"

    def test_suite_validates_without_warnings(self) -> None:
        """Suite.validate() produces no warnings."""
        suite = build_breakout_suite()
        warnings = suite.validate()
        assert warnings == [], f"Unexpected warnings: {warnings}"

    def test_json_roundtrip_preserves_all_intents(self) -> None:
        """Suite survives JSON serialization and deserialization."""
        suite = build_breakout_suite()
        json_str = suite.to_json()
        restored = VerificationSuite.from_json(json_str)

        assert restored.name == suite.name
        assert len(restored.intents) == len(suite.intents)
        for orig, rest in zip(suite.intents, restored.intents):
            assert orig.name == rest.name
            assert orig.kind == rest.kind

    def test_json_roundtrip_preserves_triggers(self) -> None:
        """Behavior trigger details survive JSON roundtrip."""
        suite = build_breakout_suite()
        json_str = suite.to_json()
        restored = VerificationSuite.from_json(json_str)

        behaviors = [i for i in restored.intents if i.kind == IntentKind.BEHAVIOR]
        paddle_bounce = next(
            i for i in behaviors if i.name == "ball_bounces_off_paddle"
        )
        assert paddle_bounce.trigger is not None
        assert paddle_bounce.trigger.params["entity_a"] == "ball"
        assert paddle_bounce.trigger.params["entity_b"] == "paddle"
        assert paddle_bounce.expected is not None

    def test_json_is_valid_json(self) -> None:
        """Serialized suite is valid JSON with correct structure."""
        suite = build_breakout_suite()
        json_str = suite.to_json()
        parsed = json.loads(json_str)
        assert isinstance(parsed, dict)
        assert "name" in parsed
        assert "intents" in parsed
        assert len(parsed["intents"]) == 13

    def test_each_intent_has_description(self) -> None:
        """Every intent in the suite has a non-empty description."""
        suite = build_breakout_suite()
        for intent in suite.intents:
            assert intent.description, f"{intent.name} has empty description"
