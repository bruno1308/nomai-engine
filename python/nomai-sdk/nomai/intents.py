"""Intent specification DSL for the Nomai verification engine.

Intent specs describe *what* a game should do, not *how* it does it.
The verification engine checks intent specs against tick manifests
produced by the engine.

All types are JSON-serializable for regression test storage.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field
from enum import Enum
from pathlib import Path
from typing import Self

logger = logging.getLogger(__name__)


# ---------------------------------------------------------------------------
# TriggerType / Trigger
# ---------------------------------------------------------------------------

class TriggerType(Enum):
    """Types of trigger expressions that start a behavior verification."""
    COLLISION = "collision"
    STATE_TRANSITION = "state_transition"
    AGGREGATE_CONDITION = "aggregate_condition"
    COMPONENT_CONDITION = "component_condition"
    EVENT_OCCURRED = "event_occurred"
    TICK_REACHED = "tick_reached"
    AND = "and"
    OR = "or"
    AFTER = "after"


@dataclass(frozen=True)
class Trigger:
    """A trigger expression describing when a behavior should be observed.

    Triggers can be leaf conditions (collision, state transition, etc.)
    or composite (AND, OR) combining child triggers.
    """
    type: TriggerType
    params: dict[str, object] = field(default_factory=dict)
    children: list[Trigger] = field(default_factory=list)

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        result: dict[str, object] = {
            "type": self.type.value,
            "params": dict(self.params),
        }
        if self.children:
            result["children"] = [c.to_dict() for c in self.children]
        else:
            result["children"] = []
        return result

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_type = data.get("type", "")
        trigger_type = TriggerType(str(raw_type))

        raw_params = data.get("params", {})
        params: dict[str, object] = {}
        if isinstance(raw_params, dict):
            params = dict(raw_params)

        raw_children = data.get("children", [])
        children: list[Trigger] = []
        if isinstance(raw_children, list):
            children = [Trigger.from_dict(c) for c in raw_children]  # type: ignore[arg-type]

        return cls(
            type=trigger_type,
            params=params,
            children=children,
        )


# -- Trigger constructor functions ------------------------------------------

def collision(
    entity_a: str,
    entity_b: str,
) -> Trigger:
    """Create a collision trigger between two entities."""
    return Trigger(
        type=TriggerType.COLLISION,
        params={"entity_a": entity_a, "entity_b": entity_b},
    )


def state_transition(
    entity: str,
    from_state: str,
    to_state: str,
) -> Trigger:
    """Create a state transition trigger."""
    return Trigger(
        type=TriggerType.STATE_TRANSITION,
        params={"entity": entity, "from_state": from_state, "to_state": to_state},
    )


def aggregate_condition(
    entity_type: str,
    comparison: str,
    value: int | float,
) -> Trigger:
    """Create an aggregate condition trigger (e.g., brick count == 0)."""
    return Trigger(
        type=TriggerType.AGGREGATE_CONDITION,
        params={
            "entity_type": entity_type,
            "comparison": comparison,
            "value": value,
        },
    )


def component_condition(
    entity: str,
    component: str,
    field_name: str,
    comparison: str,
    value: object,
) -> Trigger:
    """Create a component condition trigger."""
    return Trigger(
        type=TriggerType.COMPONENT_CONDITION,
        params={
            "entity": entity,
            "component": component,
            "field": field_name,
            "comparison": comparison,
            "value": value,
        },
    )


def event_occurred(
    event_type: str,
    involving: list[str] | None = None,
) -> Trigger:
    """Create an event-occurred trigger."""
    params: dict[str, object] = {"event_type": event_type}
    if involving is not None:
        params["involving"] = involving
    return Trigger(
        type=TriggerType.EVENT_OCCURRED,
        params=params,
    )


def tick_reached(tick: int) -> Trigger:
    """Create a tick-reached trigger."""
    return Trigger(
        type=TriggerType.TICK_REACHED,
        params={"tick": tick},
    )


def and_(*triggers: Trigger) -> Trigger:
    """Combine triggers with AND logic."""
    return Trigger(
        type=TriggerType.AND,
        children=list(triggers),
    )


def or_(*triggers: Trigger) -> Trigger:
    """Combine triggers with OR logic."""
    return Trigger(
        type=TriggerType.OR,
        children=list(triggers),
    )


def after(trigger: Trigger, delay_ticks: int) -> Trigger:
    """Create an After trigger: fires delay_ticks after the child trigger fires."""
    return Trigger(
        type=TriggerType.AFTER,
        params={"delay_ticks": delay_ticks},
        children=[trigger],
    )


# ---------------------------------------------------------------------------
# ExpectedType / Expected
# ---------------------------------------------------------------------------

class ExpectedType(Enum):
    """Types of expected outcomes that a behavior should produce."""
    COMPONENT_CHANGED = "component_changed"
    ENTITY_DESPAWNED = "entity_despawned"
    AGGREGATE_CHANGED = "aggregate_changed"
    IN_STATE = "in_state"
    EVENT_EMITTED = "event_emitted"
    VALUE_RELATION = "value_relation"
    ALL = "all"
    ANY = "any"


@dataclass(frozen=True)
class Expected:
    """An expected outcome describing what should happen after a trigger fires.

    Expected outcomes can be leaf conditions or composite (ALL, ANY)
    combining child outcomes.
    """
    type: ExpectedType
    params: dict[str, object] = field(default_factory=dict)
    children: list[Expected] = field(default_factory=list)

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        result: dict[str, object] = {
            "type": self.type.value,
            "params": dict(self.params),
        }
        if self.children:
            result["children"] = [c.to_dict() for c in self.children]
        else:
            result["children"] = []
        return result

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_type = data.get("type", "")
        expected_type = ExpectedType(str(raw_type))

        raw_params = data.get("params", {})
        params: dict[str, object] = {}
        if isinstance(raw_params, dict):
            params = dict(raw_params)

        raw_children = data.get("children", [])
        children: list[Expected] = []
        if isinstance(raw_children, list):
            children = [Expected.from_dict(c) for c in raw_children]  # type: ignore[arg-type]

        return cls(
            type=expected_type,
            params=params,
            children=children,
        )


# -- Expected constructor functions -----------------------------------------

def component_changed(
    entity: str,
    component: str,
    field_name: str | None = None,
    expected_value: object = None,
) -> Expected:
    """Expect a component value to have changed."""
    params: dict[str, object] = {
        "entity": entity,
        "component": component,
    }
    if field_name is not None:
        params["field"] = field_name
    if expected_value is not None:
        params["expected_value"] = expected_value
    return Expected(
        type=ExpectedType.COMPONENT_CHANGED,
        params=params,
    )


def entity_despawned(entity: str) -> Expected:
    """Expect an entity to be despawned."""
    return Expected(
        type=ExpectedType.ENTITY_DESPAWNED,
        params={"entity": entity},
    )


def aggregate_changed(
    entity_type: str,
    comparison: str,
    value: int | float,
) -> Expected:
    """Expect an aggregate value to match a condition."""
    return Expected(
        type=ExpectedType.AGGREGATE_CHANGED,
        params={
            "entity_type": entity_type,
            "comparison": comparison,
            "value": value,
        },
    )


def in_state(
    entity: str,
    component: str,
    state: str,
) -> Expected:
    """Expect an entity to be in a given state."""
    return Expected(
        type=ExpectedType.IN_STATE,
        params={
            "entity": entity,
            "component": component,
            "state": state,
        },
    )


def event_emitted(
    event_type: str,
    involving: list[str] | None = None,
) -> Expected:
    """Expect a game event to be emitted."""
    params: dict[str, object] = {"event_type": event_type}
    if involving is not None:
        params["involving"] = involving
    return Expected(
        type=ExpectedType.EVENT_EMITTED,
        params=params,
    )


def value_relation(
    entity: str,
    component: str,
    field_name: str,
    relation: str,
    tolerance: float = 0.1,
) -> Expected:
    """Expect a relational assertion between old and new component values.

    Relations:
    - ``"sign_flipped"``: old_value and new_value have opposite signs
    - ``"magnitude_preserved"``: ``abs(new) ~= abs(old)`` within tolerance
    - ``"increased"``: ``new > old``
    - ``"decreased"``: ``new < old``
    - ``"changed_by_more_than"``: ``abs(new - old) > tolerance``

    Args:
        entity: Name/role of the entity to check.
        component: Component type name (e.g. ``"velocity"``).
        field_name: Field within the component (e.g. ``"dy"``).
        relation: One of the relation strings above.
        tolerance: Fractional tolerance for ``"magnitude_preserved"``
            (default 0.1 = 10%), or absolute threshold for
            ``"changed_by_more_than"``.
    """
    return Expected(
        type=ExpectedType.VALUE_RELATION,
        params={
            "entity": entity,
            "component": component,
            "field": field_name,
            "relation": relation,
            "tolerance": tolerance,
        },
    )


def all_(*expectations: Expected) -> Expected:
    """Combine expected outcomes with ALL logic (all must pass)."""
    return Expected(
        type=ExpectedType.ALL,
        children=list(expectations),
    )


def any_(*expectations: Expected) -> Expected:
    """Combine expected outcomes with ANY logic (at least one must pass)."""
    return Expected(
        type=ExpectedType.ANY,
        children=list(expectations),
    )


# ---------------------------------------------------------------------------
# IntentKind
# ---------------------------------------------------------------------------

class IntentKind(Enum):
    """The kind of intent spec."""
    ENTITY = "entity"
    BEHAVIOR = "behavior"
    METRIC = "metric"
    INVARIANT = "invariant"


# ---------------------------------------------------------------------------
# IntentSpec
# ---------------------------------------------------------------------------

@dataclass
class IntentSpec:
    """A single verification intent.

    Each intent spec describes one thing the game should satisfy. The
    ``kind`` field determines which optional fields are relevant:

    - ``ENTITY``: ``entity_type``, ``entity_role``, ``must_exist``,
      ``must_be_visible``, ``required_components``.
    - ``BEHAVIOR``: ``trigger``, ``expected``, ``timeout_ticks``.
    - ``METRIC``: ``metric_entity``, ``metric_component``,
      ``metric_field``, ``metric_range``.
    - ``INVARIANT``: ``condition``.
    """
    name: str
    kind: IntentKind
    description: str

    # -- Entity intent fields -----------------------------------------------
    entity_type: str | None = None
    entity_role: str | None = None
    must_exist: bool = True
    must_be_visible: bool = True
    required_components: list[str] = field(default_factory=list)

    # -- Behavior intent fields ---------------------------------------------
    trigger: Trigger | None = None
    expected: Expected | None = None
    timeout_ticks: int = 600

    # -- Metric intent fields -----------------------------------------------
    metric_entity: str | None = None
    metric_component: str | None = None
    metric_field: str | None = None
    metric_range: tuple[float, float] | None = None

    # -- Invariant intent fields --------------------------------------------
    condition: str | None = None

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        result: dict[str, object] = {
            "name": self.name,
            "kind": self.kind.value,
            "description": self.description,
        }

        if self.kind == IntentKind.ENTITY:
            result["entity_type"] = self.entity_type
            result["entity_role"] = self.entity_role
            result["must_exist"] = self.must_exist
            result["must_be_visible"] = self.must_be_visible
            result["required_components"] = list(self.required_components)

        elif self.kind == IntentKind.BEHAVIOR:
            result["trigger"] = self.trigger.to_dict() if self.trigger else None
            result["expected"] = self.expected.to_dict() if self.expected else None
            result["timeout_ticks"] = self.timeout_ticks

        elif self.kind == IntentKind.METRIC:
            result["metric_entity"] = self.metric_entity
            result["metric_component"] = self.metric_component
            result["metric_field"] = self.metric_field
            if self.metric_range is not None:
                result["metric_range"] = list(self.metric_range)
            else:
                result["metric_range"] = None

        elif self.kind == IntentKind.INVARIANT:
            result["condition"] = self.condition

        return result

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        kind = IntentKind(str(data["kind"]))
        spec = cls(
            name=str(data["name"]),
            kind=kind,
            description=str(data["description"]),
        )

        if kind == IntentKind.ENTITY:
            raw_type = data.get("entity_type")
            spec.entity_type = str(raw_type) if raw_type is not None else None
            raw_role = data.get("entity_role")
            spec.entity_role = str(raw_role) if raw_role is not None else None
            spec.must_exist = bool(data.get("must_exist", True))
            spec.must_be_visible = bool(data.get("must_be_visible", True))
            raw_comps = data.get("required_components", [])
            if isinstance(raw_comps, list):
                spec.required_components = [str(c) for c in raw_comps]

        elif kind == IntentKind.BEHAVIOR:
            raw_trigger = data.get("trigger")
            if isinstance(raw_trigger, dict):
                spec.trigger = Trigger.from_dict(raw_trigger)
            raw_expected = data.get("expected")
            if isinstance(raw_expected, dict):
                spec.expected = Expected.from_dict(raw_expected)
            spec.timeout_ticks = int(data.get("timeout_ticks", 600))  # type: ignore[arg-type]

        elif kind == IntentKind.METRIC:
            raw_ent = data.get("metric_entity")
            spec.metric_entity = str(raw_ent) if raw_ent is not None else None
            raw_comp = data.get("metric_component")
            spec.metric_component = str(raw_comp) if raw_comp is not None else None
            raw_field = data.get("metric_field")
            spec.metric_field = str(raw_field) if raw_field is not None else None
            raw_range = data.get("metric_range")
            if isinstance(raw_range, list) and len(raw_range) == 2:
                spec.metric_range = (float(raw_range[0]), float(raw_range[1]))  # type: ignore[arg-type]

        elif kind == IntentKind.INVARIANT:
            raw_cond = data.get("condition")
            spec.condition = str(raw_cond) if raw_cond is not None else None

        return spec

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)

    def validate(self) -> list[str]:
        """Validate this intent spec for completeness and consistency.

        Returns a list of warning strings. An empty list means no issues.
        """
        warnings: list[str] = []

        if self.kind == IntentKind.BEHAVIOR:
            if self.trigger is None:
                warnings.append(f"[{self.name}] Behavior intent has no trigger defined")
            else:
                warnings.extend(self._validate_trigger(self.trigger))
            if self.expected is None:
                warnings.append(f"[{self.name}] Behavior intent has no expected outcome defined")
        elif self.kind == IntentKind.METRIC:
            if self.metric_range is None:
                warnings.append(f"[{self.name}] Metric intent has no range defined")
            elif self.metric_range[0] > self.metric_range[1]:
                warnings.append(
                    f"[{self.name}] Metric range is inverted: "
                    f"min ({self.metric_range[0]}) > max ({self.metric_range[1]})"
                )
        elif self.kind == IntentKind.ENTITY:
            if not self.entity_role:
                warnings.append(f"[{self.name}] Entity intent has no role defined")
        elif self.kind == IntentKind.INVARIANT:
            if not self.condition:
                warnings.append(f"[{self.name}] Invariant intent has no condition defined")

        return warnings

    def _validate_trigger(self, trigger: Trigger) -> list[str]:
        """Recursively validate a trigger tree."""
        warnings: list[str] = []
        if trigger.type == TriggerType.AFTER:
            delay = trigger.params.get("delay_ticks", 0)
            if isinstance(delay, (int, float)) and delay <= 0:
                warnings.append(
                    f"[{self.name}] After trigger has delay_ticks <= 0"
                )
        if trigger.type in (TriggerType.AND, TriggerType.OR):
            if not trigger.children:
                warnings.append(
                    f"[{self.name}] {trigger.type.value.upper()} trigger has no children"
                )
        for child in trigger.children:
            warnings.extend(self._validate_trigger(child))
        return warnings


# ---------------------------------------------------------------------------
# VerificationSuite
# ---------------------------------------------------------------------------

@dataclass
class VerificationSuite:
    """A collection of intent specs forming a complete verification suite.

    A suite groups related intents (e.g., all intents for breakout) and
    can be serialized/deserialized as a unit for regression test storage.
    """
    name: str
    description: str
    intents: list[IntentSpec] = field(default_factory=list)

    def to_dict(self) -> dict[str, object]:
        """Serialize to a plain dict for JSON storage."""
        return {
            "name": self.name,
            "description": self.description,
            "intents": [i.to_dict() for i in self.intents],
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        """Deserialize from a plain dict."""
        raw_intents = data.get("intents", [])
        intents: list[IntentSpec] = []
        if isinstance(raw_intents, list):
            intents = [IntentSpec.from_dict(i) for i in raw_intents]  # type: ignore[arg-type]
        return cls(
            name=str(data["name"]),
            description=str(data["description"]),
            intents=intents,
        )

    def to_json(self, indent: int | None = 2) -> str:
        """Serialize to a JSON string."""
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_json(cls, json_str: str) -> Self:
        """Deserialize from a JSON string."""
        data: dict[str, object] = json.loads(json_str)
        return cls.from_dict(data)

    def validate(self) -> list[str]:
        """Validate all intents in this suite.

        Returns a list of warning strings aggregated from all intents.
        """
        warnings: list[str] = []
        for intent in self.intents:
            warnings.extend(intent.validate())
        return warnings

    def save(self, path: str | Path) -> None:
        """Save this suite to a JSON file.

        Creates parent directories if they don't exist.
        """
        p = Path(path)
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(self.to_json(), encoding="utf-8")

    @classmethod
    def load(cls, path: str | Path) -> Self:
        """Load a suite from a JSON file.

        Raises FileNotFoundError if the file does not exist.
        """
        p = Path(path)
        if not p.exists():
            msg = f"Suite file not found: {p}"
            raise FileNotFoundError(msg)
        text = p.read_text(encoding="utf-8")
        return cls.from_json(text)
