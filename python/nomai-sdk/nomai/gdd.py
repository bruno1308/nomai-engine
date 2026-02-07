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

from nomai.intents import (
    IntentKind,
    IntentSpec,
    VerificationSuite,
    collision,
    component_changed,
    entity_despawned,
)

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
# CompletenessChecker
# ---------------------------------------------------------------------------

class CompletenessChecker:
    """Validates a GameDesignSpec for gaps and missing constraints.

    Analyzes a game design specification and returns a list of
    :class:`ClarificationQuestion` instances for every gap found.

    Checks:
    - Every dynamic/kinematic entity has bounds defined
    - Every movable entity pair has an interaction specified
    - Every dynamic entity has a speed limit
    - Play area is defined
    - At least one degenerate state is identified
    """

    def check(self, spec: GameDesignSpec) -> list[ClarificationQuestion]:
        """Run all completeness checks and return questions for gaps found.

        Args:
            spec: The game design specification to validate.

        Returns:
            A list of clarification questions, one per gap detected.
            An empty list indicates the spec is complete.
        """
        questions: list[ClarificationQuestion] = []
        questions.extend(self._check_play_area(spec))
        questions.extend(self._check_entity_bounds(spec))
        questions.extend(self._check_interaction_matrix(spec))
        questions.extend(self._check_speed_limits(spec))
        questions.extend(self._check_degenerate_states(spec))
        return questions

    def _check_play_area(self, spec: GameDesignSpec) -> list[ClarificationQuestion]:
        """Check that the play area dimensions are defined."""
        if spec.play_area is None:
            logger.debug("CompletenessChecker: play area is not defined")
            return [
                ClarificationQuestion(
                    question="What are the play area dimensions?",
                    category="bounds",
                    severity="high",
                    context="No play area defined in the game design spec.",
                ),
            ]
        return []

    def _check_entity_bounds(
        self, spec: GameDesignSpec,
    ) -> list[ClarificationQuestion]:
        """Check that every dynamic/kinematic entity has bounds defined."""
        questions: list[ClarificationQuestion] = []
        for entity in spec.entities:
            if entity.body_type in ("dynamic", "kinematic") and entity.bounds is None:
                logger.debug(
                    "CompletenessChecker: entity %r (%s) has no bounds",
                    entity.name, entity.body_type,
                )
                questions.append(
                    ClarificationQuestion(
                        question=(
                            f"What are the bounds for the {entity.name} entity? "
                            f"It is {entity.body_type} but has no bounds defined."
                        ),
                        category="bounds",
                        severity="high",
                        context=(
                            f"Entity '{entity.name}' has body_type='{entity.body_type}' "
                            f"but no BoundsSpec is set."
                        ),
                    ),
                )
        return questions

    def _check_interaction_matrix(
        self, spec: GameDesignSpec,
    ) -> list[ClarificationQuestion]:
        """Check that every pair of movable entities has an interaction defined.

        A pair requires an interaction if at least one entity in the pair
        has body_type 'dynamic' or 'kinematic'.  Interactions are checked
        in both directions (entity_a/entity_b is order-independent).
        """
        movable_types = {"dynamic", "kinematic"}

        # Build a set of interaction pairs (order-independent)
        interaction_pairs: set[frozenset[str]] = set()
        for interaction in spec.interactions:
            interaction_pairs.add(frozenset((interaction.entity_a, interaction.entity_b)))

        questions: list[ClarificationQuestion] = []
        entities = list(spec.entities)
        for i in range(len(entities)):
            for j in range(i + 1, len(entities)):
                a = entities[i]
                b = entities[j]
                a_movable = a.body_type in movable_types
                b_movable = b.body_type in movable_types
                if not (a_movable or b_movable):
                    continue
                pair = frozenset((a.name, b.name))
                if pair not in interaction_pairs:
                    logger.debug(
                        "CompletenessChecker: no interaction between %r and %r",
                        a.name, b.name,
                    )
                    questions.append(
                        ClarificationQuestion(
                            question=(
                                f"What happens when {a.name} and {b.name} interact? "
                                f"No interaction is specified for this pair."
                            ),
                            category="interaction",
                            severity="medium",
                            context=(
                                f"Entities '{a.name}' (body_type='{a.body_type}') and "
                                f"'{b.name}' (body_type='{b.body_type}') have no "
                                f"InteractionSpec defined."
                            ),
                        ),
                    )
        return questions

    def _check_speed_limits(
        self, spec: GameDesignSpec,
    ) -> list[ClarificationQuestion]:
        """Check that every dynamic entity has a speed limit defined."""
        questions: list[ClarificationQuestion] = []
        for entity in spec.entities:
            if entity.body_type == "dynamic" and entity.speed_max is None:
                logger.debug(
                    "CompletenessChecker: dynamic entity %r has no speed limit",
                    entity.name,
                )
                questions.append(
                    ClarificationQuestion(
                        question=(
                            f"What is the maximum speed for the {entity.name} entity? "
                            f"A speed limit is needed to prevent degenerate behavior."
                        ),
                        category="invariant",
                        severity="medium",
                        context=(
                            f"Entity '{entity.name}' has body_type='dynamic' "
                            f"but no speed_max is set."
                        ),
                    ),
                )
        return questions

    def _check_degenerate_states(
        self, spec: GameDesignSpec,
    ) -> list[ClarificationQuestion]:
        """Check that at least one degenerate state is identified."""
        if len(spec.degenerate_states) == 0:
            logger.debug("CompletenessChecker: no degenerate states defined")
            return [
                ClarificationQuestion(
                    question=(
                        "What degenerate states should be avoided? "
                        "No degenerate states are defined in the spec."
                    ),
                    category="degenerate",
                    severity="medium",
                    context="The spec has no DegenerateStateSpec entries.",
                ),
            ]
        return []


# ---------------------------------------------------------------------------
# IntentGenerator
# ---------------------------------------------------------------------------

class IntentGenerator:
    """Compiles a GameDesignSpec into a VerificationSuite.

    Generates one atomic IntentSpec per constraint for maximum
    failure localization in the verification report.
    """

    def generate(self, spec: GameDesignSpec) -> VerificationSuite:
        """Generate a complete verification suite from a game design spec.

        Args:
            spec: The game design specification to compile.

        Returns:
            A VerificationSuite containing one IntentSpec per constraint.
        """
        intents: list[IntentSpec] = []
        intents.extend(self._entity_intents(spec))
        intents.extend(self._bounds_invariants(spec))
        intents.extend(self._speed_metrics(spec))
        intents.extend(self._interaction_behaviors(spec))
        intents.extend(self._degenerate_invariants(spec))
        return VerificationSuite(
            name=f"{spec.title.lower().replace(' ', '_')}_verification",
            description=f"Auto-generated verification suite for {spec.title}",
            intents=intents,
        )

    def _entity_intents(self, spec: GameDesignSpec) -> list[IntentSpec]:
        """Generate one ENTITY intent per entity in the spec."""
        intents: list[IntentSpec] = []
        for entity in spec.entities:
            intents.append(
                IntentSpec(
                    name=f"{entity.name}_exists",
                    kind=IntentKind.ENTITY,
                    description=f"Entity '{entity.name}' must exist with required components",
                    entity_type=entity.entity_type,
                    entity_role=entity.role,
                    must_exist=True,
                    must_be_visible=True,
                    required_components=list(entity.required_components),
                )
            )
        return intents

    def _bounds_invariants(self, spec: GameDesignSpec) -> list[IntentSpec]:
        """Generate INVARIANT intents for entity spatial bounds.

        One invariant per axis that has both min and max defined.
        """
        intents: list[IntentSpec] = []
        for entity in spec.entities:
            if entity.bounds is None:
                continue
            bounds = entity.bounds
            if bounds.x_min is not None and bounds.x_max is not None:
                intents.append(
                    IntentSpec(
                        name=f"{entity.name}_x_bounds",
                        kind=IntentKind.INVARIANT,
                        description=(
                            f"Entity '{entity.name}' x-position must stay within "
                            f"[{bounds.x_min}, {bounds.x_max}]"
                        ),
                        condition=(
                            f"component_range:{entity.name}.position.x "
                            f"in [{bounds.x_min}, {bounds.x_max}]"
                        ),
                    )
                )
            if bounds.y_min is not None and bounds.y_max is not None:
                intents.append(
                    IntentSpec(
                        name=f"{entity.name}_y_bounds",
                        kind=IntentKind.INVARIANT,
                        description=(
                            f"Entity '{entity.name}' y-position must stay within "
                            f"[{bounds.y_min}, {bounds.y_max}]"
                        ),
                        condition=(
                            f"component_range:{entity.name}.position.y "
                            f"in [{bounds.y_min}, {bounds.y_max}]"
                        ),
                    )
                )
        return intents

    def _speed_metrics(self, spec: GameDesignSpec) -> list[IntentSpec]:
        """Generate METRIC intents for entity speed limits.

        One metric per velocity axis (dx, dy) for each entity with speed_max.
        """
        intents: list[IntentSpec] = []
        for entity in spec.entities:
            if entity.speed_max is None:
                continue
            for axis in ("dx", "dy"):
                intents.append(
                    IntentSpec(
                        name=f"{entity.name}_speed_{axis}",
                        kind=IntentKind.METRIC,
                        description=(
                            f"Entity '{entity.name}' velocity.{axis} must stay within "
                            f"[{-entity.speed_max}, {entity.speed_max}]"
                        ),
                        metric_entity=entity.name,
                        metric_component="velocity",
                        metric_field=axis,
                        metric_range=(-entity.speed_max, entity.speed_max),
                    )
                )
        return intents

    def _interaction_behaviors(self, spec: GameDesignSpec) -> list[IntentSpec]:
        """Generate BEHAVIOR intents for entity interactions.

        Skips interactions with behavior 'none'.
        """
        intents: list[IntentSpec] = []
        _BOUNCE_BEHAVIORS = {"bounce", "reflect"}
        _DESTROY_BEHAVIORS = {"destroy", "reflect_and_destroy"}

        for interaction in spec.interactions:
            behavior = interaction.behavior.lower()
            if behavior == "none":
                continue

            trigger = collision(interaction.entity_a, interaction.entity_b)

            if behavior in _BOUNCE_BEHAVIORS:
                expected = component_changed(interaction.entity_a, "velocity")
            elif behavior in _DESTROY_BEHAVIORS:
                expected = entity_despawned(interaction.entity_b)
            else:
                # Unknown behavior -- generate with component_changed as fallback
                expected = component_changed(interaction.entity_a, "velocity")

            name = (
                f"{interaction.entity_a}_{interaction.entity_b}"
                f"_{interaction.behavior}"
            )
            description = interaction.description or (
                f"When {interaction.entity_a} collides with "
                f"{interaction.entity_b}: {interaction.behavior}"
            )

            intents.append(
                IntentSpec(
                    name=name,
                    kind=IntentKind.BEHAVIOR,
                    description=description,
                    trigger=trigger,
                    expected=expected,
                    timeout_ticks=600,
                )
            )
        return intents

    def _degenerate_invariants(self, spec: GameDesignSpec) -> list[IntentSpec]:
        """Generate INVARIANT intents for degenerate state guards."""
        intents: list[IntentSpec] = []
        for degen in spec.degenerate_states:
            intents.append(
                IntentSpec(
                    name=f"degenerate_{degen.name}",
                    kind=IntentKind.INVARIANT,
                    description=(
                        degen.description or
                        f"Degenerate guard: {degen.entity}.{degen.component}"
                        f".{degen.field} must not be {degen.condition}"
                    ),
                    condition=(
                        f"degenerate_guard:{degen.entity}.{degen.component}"
                        f".{degen.field} != 0"
                    ),
                )
            )
        return intents


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
