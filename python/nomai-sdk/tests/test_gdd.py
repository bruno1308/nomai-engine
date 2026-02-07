"""Tests for nomai.gdd -- Game Design Document data model.

Tests validate construction, to_dict/from_dict round-trips, JSON
serialization, and a complete breakout spec built from the GDD types.
"""

from __future__ import annotations

import json

import pytest

from nomai.gdd import (
    BoundsSpec,
    ClarificationQuestion,
    CompletenessChecker,
    DegenerateStateSpec,
    EntitySpec,
    GameDesignSpec,
    InteractionSpec,
    InvariantSpec,
    PlayAreaSpec,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_breakout_spec() -> GameDesignSpec:
    """Build a complete breakout GDD spec for testing.

    Contains 3 entities (paddle, ball, brick), 4 interactions,
    1 invariant, and 1 degenerate state.  Every dynamic/kinematic entity
    has bounds and speed_max, and every movable entity pair has an
    interaction defined.
    """
    return GameDesignSpec(
        title="Breakout",
        description="Classic breakout clone with paddle, ball, and bricks",
        play_area=PlayAreaSpec(width=800.0, height=600.0),
        entities=(
            EntitySpec(
                name="paddle",
                entity_type="character",
                role="paddle",
                body_type="kinematic",
                bounds=BoundsSpec(y_min=550.0, y_max=550.0),
                speed_max=8.0,
                required_components=("position", "size", "velocity"),
            ),
            EntitySpec(
                name="ball",
                entity_type="projectile",
                role="ball",
                body_type="dynamic",
                bounds=BoundsSpec(x_min=0.0, x_max=800.0, y_min=0.0, y_max=600.0),
                speed_max=10.0,
                required_components=("position", "velocity"),
            ),
            EntitySpec(
                name="brick",
                entity_type="obstacle",
                role="brick",
                body_type="static",
                required_components=("position", "size", "health"),
            ),
        ),
        interactions=(
            InteractionSpec(
                entity_a="ball",
                entity_b="paddle",
                behavior="bounce",
                description="Ball bounces off the paddle, reversing y-velocity",
            ),
            InteractionSpec(
                entity_a="ball",
                entity_b="brick",
                behavior="destroy",
                description="Ball destroys brick on contact",
            ),
            InteractionSpec(
                entity_a="ball",
                entity_b="wall",
                behavior="bounce",
                description="Ball bounces off walls",
            ),
            InteractionSpec(
                entity_a="paddle",
                entity_b="brick",
                behavior="none",
                description="Paddle and brick do not interact directly",
            ),
        ),
        invariants=(
            InvariantSpec(
                name="ball_in_bounds",
                entity="ball",
                component="position",
                field="x",
                condition=">= 0 and <= 800",
                description="Ball x-position must stay within the play area",
            ),
        ),
        degenerate_states=(
            DegenerateStateSpec(
                name="ball_stuck",
                entity="ball",
                component="velocity",
                field="dy",
                condition="== 0",
                description="Ball y-velocity should never be zero during play",
            ),
        ),
        win_condition="All bricks destroyed",
        lose_condition="Ball falls below paddle",
    )


# ---------------------------------------------------------------------------
# BoundsSpec
# ---------------------------------------------------------------------------

class TestBoundsSpec:
    """Tests for BoundsSpec construction and round-trip."""

    def test_roundtrip(self) -> None:
        """BoundsSpec survives to_dict/from_dict round-trip."""
        original = BoundsSpec(x_min=0.0, x_max=800.0, y_min=0.0, y_max=600.0)
        d = original.to_dict()
        restored = BoundsSpec.from_dict(d)
        assert restored == original

    def test_partial_bounds(self) -> None:
        """BoundsSpec with only some fields set round-trips correctly."""
        original = BoundsSpec(y_min=550.0, y_max=550.0)
        d = original.to_dict()
        restored = BoundsSpec.from_dict(d)
        assert restored == original
        assert restored.x_min is None
        assert restored.x_max is None

    def test_json_roundtrip(self) -> None:
        """BoundsSpec survives full JSON serialize/deserialize."""
        original = BoundsSpec(x_min=10.0, x_max=790.0)
        json_str = original.to_json()
        restored = BoundsSpec.from_json(json_str)
        assert restored == original

    def test_frozen(self) -> None:
        """BoundsSpec is immutable."""
        b = BoundsSpec(x_min=0.0)
        with pytest.raises(AttributeError):
            b.x_min = 5.0  # type: ignore[misc]


# ---------------------------------------------------------------------------
# PlayAreaSpec
# ---------------------------------------------------------------------------

class TestPlayAreaSpec:
    """Tests for PlayAreaSpec construction and round-trip."""

    def test_roundtrip(self) -> None:
        original = PlayAreaSpec(width=800.0, height=600.0)
        d = original.to_dict()
        restored = PlayAreaSpec.from_dict(d)
        assert restored == original

    def test_json_roundtrip(self) -> None:
        original = PlayAreaSpec(width=1024.0, height=768.0)
        json_str = original.to_json()
        restored = PlayAreaSpec.from_json(json_str)
        assert restored == original


# ---------------------------------------------------------------------------
# EntitySpec
# ---------------------------------------------------------------------------

class TestEntitySpec:
    """Tests for EntitySpec construction and round-trip."""

    def test_roundtrip(self) -> None:
        original = EntitySpec(
            name="paddle",
            entity_type="character",
            role="paddle",
            body_type="kinematic",
            bounds=BoundsSpec(y_min=550.0, y_max=550.0),
            speed_max=8.0,
            required_components=("position", "size", "velocity"),
        )
        d = original.to_dict()
        restored = EntitySpec.from_dict(d)
        assert restored == original

    def test_minimal_entity(self) -> None:
        """EntitySpec with no optional fields round-trips correctly."""
        original = EntitySpec(
            name="particle",
            entity_type="effect",
            role="vfx",
        )
        d = original.to_dict()
        restored = EntitySpec.from_dict(d)
        assert restored == original
        assert restored.body_type is None
        assert restored.bounds is None
        assert restored.speed_max is None
        assert restored.required_components == ()

    def test_json_roundtrip(self) -> None:
        original = EntitySpec(
            name="ball",
            entity_type="projectile",
            role="ball",
            body_type="dynamic",
            speed_max=10.0,
            required_components=("position", "velocity"),
        )
        json_str = original.to_json()
        restored = EntitySpec.from_json(json_str)
        assert restored == original

    def test_frozen(self) -> None:
        e = EntitySpec(name="x", entity_type="y", role="z")
        with pytest.raises(AttributeError):
            e.name = "changed"  # type: ignore[misc]


# ---------------------------------------------------------------------------
# InteractionSpec
# ---------------------------------------------------------------------------

class TestInteractionSpec:
    """Tests for InteractionSpec construction and round-trip."""

    def test_roundtrip(self) -> None:
        original = InteractionSpec(
            entity_a="ball",
            entity_b="paddle",
            behavior="bounce",
            description="Ball bounces off paddle",
        )
        d = original.to_dict()
        restored = InteractionSpec.from_dict(d)
        assert restored == original

    def test_minimal(self) -> None:
        """InteractionSpec with no description round-trips correctly."""
        original = InteractionSpec(
            entity_a="ball",
            entity_b="brick",
            behavior="destroy",
        )
        d = original.to_dict()
        restored = InteractionSpec.from_dict(d)
        assert restored == original
        assert restored.description == ""

    def test_json_roundtrip(self) -> None:
        original = InteractionSpec(
            entity_a="player",
            entity_b="enemy",
            behavior="damage",
            description="Player takes damage from enemy",
        )
        json_str = original.to_json()
        restored = InteractionSpec.from_json(json_str)
        assert restored == original


# ---------------------------------------------------------------------------
# InvariantSpec
# ---------------------------------------------------------------------------

class TestInvariantSpec:
    """Tests for InvariantSpec construction and round-trip."""

    def test_roundtrip(self) -> None:
        original = InvariantSpec(
            name="ball_in_bounds",
            entity="ball",
            component="position",
            field="x",
            condition=">= 0 and <= 800",
            description="Ball x-position within play area",
        )
        d = original.to_dict()
        restored = InvariantSpec.from_dict(d)
        assert restored == original

    def test_json_roundtrip(self) -> None:
        original = InvariantSpec(
            name="speed_cap",
            entity="ball",
            component="velocity",
            field="dx",
            condition="<= 10.0",
        )
        json_str = original.to_json()
        restored = InvariantSpec.from_json(json_str)
        assert restored == original


# ---------------------------------------------------------------------------
# DegenerateStateSpec
# ---------------------------------------------------------------------------

class TestDegenerateStateSpec:
    """Tests for DegenerateStateSpec construction and round-trip."""

    def test_roundtrip(self) -> None:
        original = DegenerateStateSpec(
            name="ball_stuck",
            entity="ball",
            component="velocity",
            field="dy",
            condition="== 0",
            description="Ball y-velocity should never be zero",
        )
        d = original.to_dict()
        restored = DegenerateStateSpec.from_dict(d)
        assert restored == original

    def test_json_roundtrip(self) -> None:
        original = DegenerateStateSpec(
            name="paddle_offscreen",
            entity="paddle",
            component="position",
            field="x",
            condition="< 0 or > 800",
        )
        json_str = original.to_json()
        restored = DegenerateStateSpec.from_json(json_str)
        assert restored == original


# ---------------------------------------------------------------------------
# ClarificationQuestion
# ---------------------------------------------------------------------------

class TestClarificationQuestion:
    """Tests for ClarificationQuestion construction and round-trip."""

    def test_roundtrip(self) -> None:
        original = ClarificationQuestion(
            question="What happens when the ball hits the top wall?",
            category="physics",
            severity="medium",
            context="No wall bounce behavior specified for top boundary",
        )
        d = original.to_dict()
        restored = ClarificationQuestion.from_dict(d)
        assert restored == original

    def test_minimal(self) -> None:
        """ClarificationQuestion with no context round-trips correctly."""
        original = ClarificationQuestion(
            question="How fast should the paddle move?",
            category="gameplay",
            severity="low",
        )
        d = original.to_dict()
        restored = ClarificationQuestion.from_dict(d)
        assert restored == original
        assert restored.context == ""

    def test_json_roundtrip(self) -> None:
        original = ClarificationQuestion(
            question="What is the win condition?",
            category="game_flow",
            severity="high",
            context="No win condition defined",
        )
        json_str = original.to_json()
        restored = ClarificationQuestion.from_json(json_str)
        assert restored == original


# ---------------------------------------------------------------------------
# GameDesignSpec
# ---------------------------------------------------------------------------

class TestGameDesignSpec:
    """Tests for GameDesignSpec construction, round-trip, and the breakout helper."""

    def test_full_roundtrip(self) -> None:
        """Complete breakout GameDesignSpec survives to_dict/from_dict round-trip."""
        original = _make_breakout_spec()
        d = original.to_dict()
        restored = GameDesignSpec.from_dict(d)
        assert restored == original

    def test_full_json_roundtrip(self) -> None:
        """Complete breakout GameDesignSpec survives JSON round-trip."""
        original = _make_breakout_spec()
        json_str = original.to_json()
        restored = GameDesignSpec.from_json(json_str)
        assert restored == original

    def test_json_is_valid(self) -> None:
        """GameDesignSpec serializes to valid, parseable JSON."""
        spec = _make_breakout_spec()
        json_str = spec.to_json()
        parsed = json.loads(json_str)
        assert isinstance(parsed, dict)
        assert parsed["title"] == "Breakout"
        assert len(parsed["entities"]) == 3
        assert len(parsed["interactions"]) == 4
        assert len(parsed["invariants"]) == 1
        assert len(parsed["degenerate_states"]) == 1

    def test_minimal_spec(self) -> None:
        """GameDesignSpec with only required fields round-trips correctly."""
        original = GameDesignSpec(title="Minimal Game")
        d = original.to_dict()
        restored = GameDesignSpec.from_dict(d)
        assert restored == original
        assert restored.description == ""
        assert restored.play_area is None
        assert restored.entities == ()
        assert restored.interactions == ()
        assert restored.invariants == ()
        assert restored.degenerate_states == ()
        assert restored.win_condition == ""
        assert restored.lose_condition == ""

    def test_breakout_spec_entity_count(self) -> None:
        """Breakout spec has exactly 3 entities."""
        spec = _make_breakout_spec()
        assert len(spec.entities) == 3
        names = tuple(e.name for e in spec.entities)
        assert names == ("paddle", "ball", "brick")

    def test_breakout_spec_interaction_count(self) -> None:
        """Breakout spec has exactly 4 interactions."""
        spec = _make_breakout_spec()
        assert len(spec.interactions) == 4

    def test_breakout_spec_invariant_count(self) -> None:
        """Breakout spec has exactly 1 invariant."""
        spec = _make_breakout_spec()
        assert len(spec.invariants) == 1

    def test_breakout_spec_degenerate_count(self) -> None:
        """Breakout spec has exactly 1 degenerate state."""
        spec = _make_breakout_spec()
        assert len(spec.degenerate_states) == 1

    def test_frozen(self) -> None:
        """GameDesignSpec is immutable."""
        spec = GameDesignSpec(title="Test")
        with pytest.raises(AttributeError):
            spec.title = "Changed"  # type: ignore[misc]


# ---------------------------------------------------------------------------
# Edge cases: list normalization, null handling, malformed payloads
# ---------------------------------------------------------------------------

class TestListNormalization:
    """Verify that list inputs are coerced to tuples via __post_init__."""

    def test_entity_spec_list_components(self) -> None:
        """EntitySpec accepts list for required_components and normalizes to tuple."""
        e = EntitySpec(
            name="x",
            entity_type="y",
            role="z",
            required_components=["a", "b"],  # type: ignore[arg-type]
        )
        assert isinstance(e.required_components, tuple)
        assert e.required_components == ("a", "b")

    def test_entity_spec_list_roundtrip_equality(self) -> None:
        """EntitySpec created with list equals one created with tuple."""
        from_list = EntitySpec(
            name="x", entity_type="y", role="z",
            required_components=["a", "b"],  # type: ignore[arg-type]
        )
        from_tuple = EntitySpec(
            name="x", entity_type="y", role="z",
            required_components=("a", "b"),
        )
        assert from_list == from_tuple

    def test_game_design_spec_list_entities(self) -> None:
        """GameDesignSpec accepts lists for collection fields and normalizes to tuples."""
        entity = EntitySpec(name="e", entity_type="t", role="r")
        interaction = InteractionSpec(entity_a="a", entity_b="b", behavior="c")
        invariant = InvariantSpec(
            name="i", entity="e", component="c", field="f", condition="x",
        )
        degenerate = DegenerateStateSpec(
            name="d", entity="e", component="c", field="f", condition="y",
        )
        spec = GameDesignSpec(
            title="Test",
            entities=[entity],  # type: ignore[arg-type]
            interactions=[interaction],  # type: ignore[arg-type]
            invariants=[invariant],  # type: ignore[arg-type]
            degenerate_states=[degenerate],  # type: ignore[arg-type]
        )
        assert isinstance(spec.entities, tuple)
        assert isinstance(spec.interactions, tuple)
        assert isinstance(spec.invariants, tuple)
        assert isinstance(spec.degenerate_states, tuple)

    def test_game_design_spec_list_roundtrip_equality(self) -> None:
        """GameDesignSpec with list inputs equals same spec with tuple inputs."""
        entity = EntitySpec(name="e", entity_type="t", role="r")
        from_list = GameDesignSpec(
            title="Test",
            entities=[entity],  # type: ignore[arg-type]
        )
        from_tuple = GameDesignSpec(
            title="Test",
            entities=(entity,),
        )
        assert from_list == from_tuple

    def test_entity_from_dict_tuple_components(self) -> None:
        """EntitySpec.from_dict accepts tuple-valued required_components."""
        data = {
            "name": "x", "entity_type": "y", "role": "z",
            "required_components": ("a", "b"),
        }
        spec = EntitySpec.from_dict(data)
        assert spec.required_components == ("a", "b")

    def test_game_design_spec_from_dict_tuple_collections(self) -> None:
        """GameDesignSpec.from_dict accepts tuple-valued collection fields."""
        entity_dict = {"name": "e", "entity_type": "t", "role": "r"}
        data = {
            "title": "Test",
            "entities": (entity_dict,),
        }
        spec = GameDesignSpec.from_dict(data)
        assert len(spec.entities) == 1
        assert spec.entities[0].name == "e"


class TestNullHandling:
    """Verify that JSON null values do not become the string 'None'."""

    def test_interaction_null_description(self) -> None:
        """InteractionSpec with null description deserializes to empty string."""
        data = {
            "entity_a": "a", "entity_b": "b",
            "behavior": "bounce", "description": None,
        }
        spec = InteractionSpec.from_dict(data)
        assert spec.description == ""
        assert spec.description != "None"

    def test_invariant_null_description(self) -> None:
        """InvariantSpec with null description deserializes to empty string."""
        data = {
            "name": "n", "entity": "e", "component": "c",
            "field": "f", "condition": "x", "description": None,
        }
        spec = InvariantSpec.from_dict(data)
        assert spec.description == ""

    def test_degenerate_null_description(self) -> None:
        """DegenerateStateSpec with null description deserializes to empty string."""
        data = {
            "name": "n", "entity": "e", "component": "c",
            "field": "f", "condition": "x", "description": None,
        }
        spec = DegenerateStateSpec.from_dict(data)
        assert spec.description == ""

    def test_clarification_null_context(self) -> None:
        """ClarificationQuestion with null context deserializes to empty string."""
        data = {
            "question": "q", "category": "c",
            "severity": "s", "context": None,
        }
        spec = ClarificationQuestion.from_dict(data)
        assert spec.context == ""

    def test_game_design_spec_null_strings(self) -> None:
        """GameDesignSpec with null optional string fields deserializes to empty string."""
        data = {
            "title": "Test",
            "description": None,
            "win_condition": None,
            "lose_condition": None,
        }
        spec = GameDesignSpec.from_dict(data)
        assert spec.description == ""
        assert spec.win_condition == ""
        assert spec.lose_condition == ""

    def test_entity_null_body_type_stays_none(self) -> None:
        """EntitySpec with null body_type deserializes to None (not 'None' string)."""
        data = {
            "name": "n", "entity_type": "t", "role": "r",
            "body_type": None,
        }
        spec = EntitySpec.from_dict(data)
        assert spec.body_type is None

    def test_json_null_roundtrip(self) -> None:
        """JSON null values survive full JSON round-trip correctly."""
        json_str = json.dumps({
            "entity_a": "a", "entity_b": "b",
            "behavior": "bounce", "description": None,
        })
        spec = InteractionSpec.from_json(json_str)
        assert spec.description == ""


# ---------------------------------------------------------------------------
# CompletenessChecker
# ---------------------------------------------------------------------------

class TestCompletenessChecker:
    """Tests for CompletenessChecker gap detection."""

    def test_complete_spec_has_no_questions(self) -> None:
        """A fully specified breakout spec produces zero clarification questions."""
        spec = _make_breakout_spec()
        checker = CompletenessChecker()
        questions = checker.check(spec)
        assert len(questions) == 0

    def test_missing_bounds_on_dynamic_entity(self) -> None:
        """Dynamic entity without bounds triggers a bounds question."""
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=PlayAreaSpec(width=800.0, height=600.0),
            entities=(
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic", required_components=("position",)),
            ),
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        bound_qs = [q for q in questions if q.category == "bounds"]
        assert len(bound_qs) >= 1
        assert "ball" in bound_qs[0].question.lower()

    def test_missing_interaction_pair(self) -> None:
        """Two dynamic/kinematic entities with no interaction spec."""
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=PlayAreaSpec(width=800.0, height=600.0),
            entities=(
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic", required_components=("position",)),
                EntitySpec(name="paddle", entity_type="character", role="paddle",
                           body_type="kinematic", required_components=("position",)),
            ),
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        interaction_qs = [q for q in questions if q.category == "interaction"]
        assert len(interaction_qs) >= 1

    def test_missing_speed_limit(self) -> None:
        """Dynamic entity without speed_max."""
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=PlayAreaSpec(width=800.0, height=600.0),
            entities=(
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic",
                           bounds=BoundsSpec(x_min=0, x_max=800, y_min=0, y_max=600),
                           required_components=("position",)),
            ),
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        speed_qs = [q for q in questions if q.category == "invariant"]
        assert any("speed" in q.question.lower() for q in speed_qs)

    def test_no_play_area_asks_question(self) -> None:
        """Missing play area triggers a bounds question about play area."""
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=None,
            entities=(
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic", required_components=("position",)),
            ),
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        area_qs = [q for q in questions if q.category == "bounds"]
        assert any("play area" in q.question.lower() for q in area_qs)

    def test_no_degenerate_states_asks_question(self) -> None:
        """Empty degenerate_states triggers a degenerate question."""
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=PlayAreaSpec(width=800.0, height=600.0),
            entities=(
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic",
                           bounds=BoundsSpec(x_min=0, x_max=800, y_min=0, y_max=600),
                           speed_max=500.0,
                           required_components=("position",)),
            ),
            degenerate_states=(),
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        degen_qs = [q for q in questions if q.category == "degenerate"]
        assert len(degen_qs) >= 1
