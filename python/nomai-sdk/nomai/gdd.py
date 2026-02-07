"""Game Design Document data model for the Nomai verification engine.

Provides structured types representing a game design specification that
can be analyzed to auto-generate verification intents.  A ``GameDesignSpec``
captures entities, interactions, invariants, degenerate states, and
win/lose conditions in a format suitable for both human and AI consumption.

All types are frozen dataclasses with ``to_dict`` / ``from_dict`` /
``to_json`` / ``from_json`` for round-trip serialization.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass
from typing import Self

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# BoundsSpec
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class BoundsSpec:
    """Axis-aligned spatial bounds for an entity.

    Each bound is optional; ``None`` means unconstrained on that axis/side.
    """
    x_min: float | None = None
    x_max: float | None = None
    y_min: float | None = None
    y_max: float | None = None

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "x_min": self.x_min,
            "x_max": self.x_max,
            "y_min": self.y_min,
            "y_max": self.y_max,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        return cls(
            x_min=_opt_float(data.get("x_min")),
            x_max=_opt_float(data.get("x_max")),
            y_min=_opt_float(data.get("y_min")),
            y_max=_opt_float(data.get("y_max")),
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# PlayAreaSpec
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class PlayAreaSpec:
    """Dimensions of the game play area."""
    width: float
    height: float

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "width": self.width,
            "height": self.height,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        return cls(
            width=float(data["width"]),  # type: ignore[arg-type]
            height=float(data["height"]),  # type: ignore[arg-type]
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# EntitySpec
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class EntitySpec:
    """Specification for a game entity described in a design document.

    Attributes:
        name: Human-readable name for the entity (e.g. ``"paddle"``).
        entity_type: The type category (e.g. ``"character"``, ``"projectile"``).
        role: The semantic role for manifest identity (e.g. ``"paddle"``).
        body_type: Optional physics body type (``"static"``, ``"dynamic"``,
            ``"kinematic"``).
        bounds: Optional spatial bounds constraining the entity.
        speed_max: Optional maximum speed for the entity.
        required_components: Component types the entity must have.
    """
    name: str
    entity_type: str
    role: str
    body_type: str | None = None
    bounds: BoundsSpec | None = None
    speed_max: float | None = None
    required_components: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if not isinstance(self.required_components, tuple):
            object.__setattr__(
                self, "required_components", tuple(self.required_components)
            )

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "name": self.name,
            "entity_type": self.entity_type,
            "role": self.role,
            "body_type": self.body_type,
            "bounds": self.bounds.to_dict() if self.bounds is not None else None,
            "speed_max": self.speed_max,
            "required_components": list(self.required_components),
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_bounds = data.get("bounds")
        bounds: BoundsSpec | None = None
        if isinstance(raw_bounds, dict):
            bounds = BoundsSpec.from_dict(raw_bounds)

        raw_comps = data.get("required_components", ())
        comps: tuple[str, ...] = ()
        if isinstance(raw_comps, (list, tuple)):
            comps = tuple(str(c) for c in raw_comps)

        return cls(
            name=str(data["name"]),
            entity_type=str(data["entity_type"]),
            role=str(data["role"]),
            body_type=_opt_str(data.get("body_type")),
            bounds=bounds,
            speed_max=_opt_float(data.get("speed_max")),
            required_components=comps,
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# InteractionSpec
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class InteractionSpec:
    """Specification for an interaction between two entities.

    Attributes:
        entity_a: Name of the first entity involved.
        entity_b: Name of the second entity involved.
        behavior: The interaction behavior (e.g. ``"bounce"``, ``"destroy"``).
        description: Human-readable explanation of the interaction.
    """
    entity_a: str
    entity_b: str
    behavior: str
    description: str = ""

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "entity_a": self.entity_a,
            "entity_b": self.entity_b,
            "behavior": self.behavior,
            "description": self.description,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        return cls(
            entity_a=str(data["entity_a"]),
            entity_b=str(data["entity_b"]),
            behavior=str(data["behavior"]),
            description=_str_or_empty(data.get("description")),
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# InvariantSpec
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class InvariantSpec:
    """A game invariant that must hold every tick.

    Attributes:
        name: Short identifier for the invariant.
        entity: Entity the invariant applies to.
        component: Component type to check.
        field: Field within the component.
        condition: Condition expression (e.g. ``">= 0 and <= 800"``).
        description: Human-readable explanation.
    """
    name: str
    entity: str
    component: str
    field: str
    condition: str
    description: str = ""

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "name": self.name,
            "entity": self.entity,
            "component": self.component,
            "field": self.field,
            "condition": self.condition,
            "description": self.description,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        return cls(
            name=str(data["name"]),
            entity=str(data["entity"]),
            component=str(data["component"]),
            field=str(data["field"]),
            condition=str(data["condition"]),
            description=_str_or_empty(data.get("description")),
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# DegenerateStateSpec
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class DegenerateStateSpec:
    """A degenerate state the game should avoid.

    Degenerate states are conditions that are technically valid but indicate
    the game is in an unplayable or undesirable situation.

    Attributes:
        name: Short identifier for the degenerate state.
        entity: Entity the degenerate state applies to.
        component: Component type to check.
        field: Field within the component.
        condition: Condition expression that describes the degenerate state.
        description: Human-readable explanation.
    """
    name: str
    entity: str
    component: str
    field: str
    condition: str
    description: str = ""

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "name": self.name,
            "entity": self.entity,
            "component": self.component,
            "field": self.field,
            "condition": self.condition,
            "description": self.description,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        return cls(
            name=str(data["name"]),
            entity=str(data["entity"]),
            component=str(data["component"]),
            field=str(data["field"]),
            condition=str(data["condition"]),
            description=_str_or_empty(data.get("description")),
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# ClarificationQuestion
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class ClarificationQuestion:
    """A question generated during GDD analysis that needs clarification.

    When a game design document is ambiguous or incomplete, the analyzer
    produces clarification questions so the AI (or human) can resolve them
    before generating verification intents.

    Attributes:
        question: The question text.
        category: Category of the question (e.g. ``"physics"``,
            ``"gameplay"``, ``"game_flow"``).
        severity: How important the clarification is
            (``"high"``, ``"medium"``, ``"low"``).
        context: Additional context explaining why the question was raised.
    """
    question: str
    category: str
    severity: str
    context: str = ""

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "question": self.question,
            "category": self.category,
            "severity": self.severity,
            "context": self.context,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        return cls(
            question=str(data["question"]),
            category=str(data["category"]),
            severity=str(data["severity"]),
            context=_str_or_empty(data.get("context")),
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# GameDesignSpec
# ---------------------------------------------------------------------------

@dataclass(frozen=True)
class GameDesignSpec:
    """A structured game design specification.

    Captures the complete design of a game in a format that can be analyzed
    to auto-generate verification intents.  The spec is game-agnostic and
    describes entities, their interactions, invariants, degenerate states,
    and win/lose conditions.

    Attributes:
        title: The name of the game.
        description: A brief description of the game.
        play_area: Dimensions of the play area.
        entities: Entity specifications.
        interactions: Interaction specifications between entities.
        invariants: Game invariants that must hold every tick.
        degenerate_states: Degenerate states the game should avoid.
        win_condition: Description of the win condition.
        lose_condition: Description of the lose condition.
    """
    title: str
    description: str = ""
    play_area: PlayAreaSpec | None = None
    entities: tuple[EntitySpec, ...] = ()
    interactions: tuple[InteractionSpec, ...] = ()
    invariants: tuple[InvariantSpec, ...] = ()
    degenerate_states: tuple[DegenerateStateSpec, ...] = ()
    win_condition: str = ""
    lose_condition: str = ""

    def __post_init__(self) -> None:
        for attr in ("entities", "interactions", "invariants", "degenerate_states"):
            val = getattr(self, attr)
            if not isinstance(val, tuple):
                object.__setattr__(self, attr, tuple(val))

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "title": self.title,
            "description": self.description,
            "play_area": self.play_area.to_dict() if self.play_area is not None else None,
            "entities": [e.to_dict() for e in self.entities],
            "interactions": [i.to_dict() for i in self.interactions],
            "invariants": [i.to_dict() for i in self.invariants],
            "degenerate_states": [d.to_dict() for d in self.degenerate_states],
            "win_condition": self.win_condition,
            "lose_condition": self.lose_condition,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_play_area = data.get("play_area")
        play_area: PlayAreaSpec | None = None
        if isinstance(raw_play_area, dict):
            play_area = PlayAreaSpec.from_dict(raw_play_area)

        raw_entities = data.get("entities", ())
        entities: tuple[EntitySpec, ...] = ()
        if isinstance(raw_entities, (list, tuple)):
            entities = tuple(
                EntitySpec.from_dict(e) for e in raw_entities  # type: ignore[arg-type]
            )

        raw_interactions = data.get("interactions", ())
        interactions: tuple[InteractionSpec, ...] = ()
        if isinstance(raw_interactions, (list, tuple)):
            interactions = tuple(
                InteractionSpec.from_dict(i) for i in raw_interactions  # type: ignore[arg-type]
            )

        raw_invariants = data.get("invariants", ())
        invariants: tuple[InvariantSpec, ...] = ()
        if isinstance(raw_invariants, (list, tuple)):
            invariants = tuple(
                InvariantSpec.from_dict(i) for i in raw_invariants  # type: ignore[arg-type]
            )

        raw_degenerate = data.get("degenerate_states", ())
        degenerate_states: tuple[DegenerateStateSpec, ...] = ()
        if isinstance(raw_degenerate, (list, tuple)):
            degenerate_states = tuple(
                DegenerateStateSpec.from_dict(d) for d in raw_degenerate  # type: ignore[arg-type]
            )

        return cls(
            title=str(data["title"]),
            description=_str_or_empty(data.get("description")),
            play_area=play_area,
            entities=entities,
            interactions=interactions,
            invariants=invariants,
            degenerate_states=degenerate_states,
            win_condition=_str_or_empty(data.get("win_condition")),
            lose_condition=_str_or_empty(data.get("lose_condition")),
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)


# ---------------------------------------------------------------------------
# Private helpers
# ---------------------------------------------------------------------------

def _opt_float(raw: object) -> float | None:
    """Convert a value to float, returning None if the input is None."""
    if raw is None:
        return None
    return float(raw)  # type: ignore[arg-type]


def _opt_str(raw: object) -> str | None:
    """Convert a value to str, returning None if the input is None."""
    if raw is None:
        return None
    return str(raw)


def _str_or_empty(raw: object) -> str:
    """Convert a value to str, returning empty string if None."""
    if raw is None:
        return ""
    return str(raw)
