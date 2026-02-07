# GDD Analyzer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Game Design Document analyzer that takes free-form GDD text, extracts a structured `GameDesignSpec`, identifies gaps via completeness checking, and auto-generates `IntentSpec`s — so the verification loop catches bugs like "paddle flies off-screen" and "ball stuck bouncing horizontally."

**Architecture:** Three-layer pipeline: (1) `GameDesignSpec` frozen dataclass IR with JSON serialization, (2) `CompletenessChecker` that validates specs against a domain-specific checklist and emits `ClarificationQuestion`s for gaps, (3) `IntentGenerator` that compiles specs into `IntentSpec`/`VerificationSuite`. Also adds `component_range:` invariant condition format to the verifier so generated bounds intents actually work instead of falling through to the free-form placeholder.

**Tech Stack:** Python 3.12, dataclasses, pytest, existing `nomai.intents` DSL and `nomai.verify` engine.

---

### Task 1: GameDesignSpec Data Model

**Files:**
- Create: `python/nomai-sdk/nomai/gdd.py`
- Test: `python/nomai-sdk/tests/test_gdd.py`

This task creates the structured intermediate representation. All types are frozen dataclasses with `to_dict`/`from_dict`/`to_json`/`from_json` (matching the pattern in `intents.py`).

**Step 1: Write failing tests for data model serialization**

In `python/nomai-sdk/tests/test_gdd.py`:

```python
"""Tests for the GDD analyzer module."""

from __future__ import annotations

import json

import pytest

from nomai.gdd import (
    BoundsSpec,
    ClarificationQuestion,
    DegenerateStateSpec,
    EntitySpec,
    GameDesignSpec,
    InteractionSpec,
    InvariantSpec,
    PlayAreaSpec,
)


class TestEntitySpec:
    def test_roundtrip(self) -> None:
        spec = EntitySpec(
            name="paddle",
            entity_type="character",
            role="paddle",
            body_type="kinematic",
            bounds=BoundsSpec(x_min=0.0, x_max=800.0),
            speed_max=400.0,
            required_components=["position", "size"],
        )
        d = spec.to_dict()
        restored = EntitySpec.from_dict(d)
        assert restored == spec

    def test_minimal(self) -> None:
        spec = EntitySpec(name="particle", entity_type="effect", role="particle")
        d = spec.to_dict()
        assert d["name"] == "particle"
        assert d["bounds"] is None


class TestPlayAreaSpec:
    def test_roundtrip(self) -> None:
        spec = PlayAreaSpec(width=800.0, height=600.0)
        assert PlayAreaSpec.from_dict(spec.to_dict()) == spec


class TestInteractionSpec:
    def test_roundtrip(self) -> None:
        spec = InteractionSpec(
            entity_a="ball",
            entity_b="brick",
            behavior="reflect_and_destroy",
            description="Ball bounces off brick, brick is destroyed",
        )
        assert InteractionSpec.from_dict(spec.to_dict()) == spec


class TestInvariantSpec:
    def test_roundtrip(self) -> None:
        spec = InvariantSpec(
            name="ball_always_moving",
            entity="ball",
            component="velocity",
            field="dy",
            condition="never_zero",
            description="Ball must always have vertical movement",
        )
        assert InvariantSpec.from_dict(spec.to_dict()) == spec


class TestDegenerateStateSpec:
    def test_roundtrip(self) -> None:
        spec = DegenerateStateSpec(
            name="ball_stuck_horizontal",
            entity="ball",
            component="velocity",
            field="dy",
            condition="equals_zero",
            description="Ball stuck bouncing horizontally forever",
        )
        assert DegenerateStateSpec.from_dict(spec.to_dict()) == spec


class TestGameDesignSpec:
    def test_full_roundtrip(self) -> None:
        spec = _make_breakout_spec()
        json_str = spec.to_json()
        restored = GameDesignSpec.from_json(json_str)
        assert restored.title == spec.title
        assert len(restored.entities) == len(spec.entities)
        assert len(restored.interactions) == len(spec.interactions)
        assert len(restored.invariants) == len(spec.invariants)
        assert len(restored.degenerate_states) == len(spec.degenerate_states)

    def test_json_is_valid(self) -> None:
        spec = _make_breakout_spec()
        data = json.loads(spec.to_json())
        assert data["title"] == "Breakout"
        assert len(data["entities"]) == 3


def _make_breakout_spec() -> GameDesignSpec:
    """Helper to build a minimal breakout GameDesignSpec."""
    return GameDesignSpec(
        title="Breakout",
        description="Classic breakout game with paddle, ball, and bricks.",
        play_area=PlayAreaSpec(width=800.0, height=600.0),
        entities=[
            EntitySpec(
                name="paddle",
                entity_type="character",
                role="paddle",
                body_type="kinematic",
                bounds=BoundsSpec(x_min=0.0, x_max=800.0),
                speed_max=400.0,
                required_components=["position", "size"],
            ),
            EntitySpec(
                name="ball",
                entity_type="projectile",
                role="ball",
                body_type="dynamic",
                bounds=BoundsSpec(x_min=0.0, x_max=800.0, y_min=0.0, y_max=600.0),
                speed_max=500.0,
                required_components=["position", "velocity"],
            ),
            EntitySpec(
                name="brick",
                entity_type="destructible",
                role="brick",
                body_type="static",
                required_components=["position", "size"],
            ),
        ],
        interactions=[
            InteractionSpec(
                entity_a="ball",
                entity_b="paddle",
                behavior="reflect",
                description="Ball bounces off paddle",
            ),
            InteractionSpec(
                entity_a="ball",
                entity_b="brick",
                behavior="reflect_and_destroy",
                description="Ball bounces off brick, brick is destroyed",
            ),
            InteractionSpec(
                entity_a="ball",
                entity_b="wall",
                behavior="reflect",
                description="Ball bounces off wall",
            ),
        ],
        invariants=[
            InvariantSpec(
                name="ball_always_moving_vertically",
                entity="ball",
                component="velocity",
                field="dy",
                condition="never_zero",
                description="Ball must always have vertical velocity",
            ),
        ],
        degenerate_states=[
            DegenerateStateSpec(
                name="ball_stuck_horizontal",
                entity="ball",
                component="velocity",
                field="dy",
                condition="equals_zero",
                description="Ball stuck bouncing horizontally",
            ),
        ],
        win_condition="All bricks destroyed",
        lose_condition="Ball falls below paddle",
    )
```

**Step 2: Run tests to verify they fail**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_gdd.py" -v`
Expected: FAIL with `ImportError: cannot import name 'BoundsSpec' from 'nomai.gdd'`

**Step 3: Implement data model**

In `python/nomai-sdk/nomai/gdd.py`, create:

- `BoundsSpec(x_min, x_max, y_min, y_max)` — all optional floats, frozen dataclass
- `PlayAreaSpec(width, height)` — frozen dataclass
- `EntitySpec(name, entity_type, role, body_type, bounds, speed_max, required_components)` — frozen dataclass
- `InteractionSpec(entity_a, entity_b, behavior, description)` — frozen dataclass
- `InvariantSpec(name, entity, component, field, condition, description)` — frozen dataclass
- `DegenerateStateSpec(name, entity, component, field, condition, description)` — frozen dataclass
- `GameDesignSpec(title, description, play_area, entities, interactions, invariants, degenerate_states, win_condition, lose_condition)` — frozen dataclass

All with `to_dict`/`from_dict`/`to_json`/`from_json`. Follow the exact serialization pattern from `intents.py`.

Also create a stub `ClarificationQuestion` dataclass:
```python
@dataclass(frozen=True)
class ClarificationQuestion:
    question: str
    category: str  # "bounds", "interaction", "invariant", "degenerate", "lifecycle"
    severity: str  # "blocking", "important", "nice_to_have"
    context: str   # what triggered this question
```

**Step 4: Run tests to verify they pass**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_gdd.py" -v`
Expected: All PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/gdd.py python/nomai-sdk/tests/test_gdd.py
git commit -m "feat: GameDesignSpec data model with JSON serialization"
```

---

### Task 2: CompletenessChecker

**Files:**
- Modify: `python/nomai-sdk/nomai/gdd.py`
- Modify: `python/nomai-sdk/tests/test_gdd.py`

The checker validates a `GameDesignSpec` against a domain-specific checklist and returns `ClarificationQuestion`s for every gap.

**Step 1: Write failing tests for completeness checking**

Append to `tests/test_gdd.py`:

```python
from nomai.gdd import CompletenessChecker


class TestCompletenessChecker:
    def test_complete_spec_has_no_questions(self) -> None:
        spec = _make_breakout_spec()
        checker = CompletenessChecker()
        questions = checker.check(spec)
        assert len(questions) == 0

    def test_missing_bounds_on_dynamic_entity(self) -> None:
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=PlayAreaSpec(width=800.0, height=600.0),
            entities=[
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic", required_components=["position"]),
            ],
            interactions=[],
            invariants=[],
            degenerate_states=[],
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
            entities=[
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic", required_components=["position"]),
                EntitySpec(name="paddle", entity_type="character", role="paddle",
                           body_type="kinematic", required_components=["position"]),
            ],
            interactions=[],
            invariants=[],
            degenerate_states=[],
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
            entities=[
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic",
                           bounds=BoundsSpec(x_min=0, x_max=800, y_min=0, y_max=600),
                           required_components=["position"]),
            ],
            interactions=[],
            invariants=[],
            degenerate_states=[],
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        speed_qs = [q for q in questions if q.category == "invariant"]
        assert any("speed" in q.question.lower() for q in speed_qs)

    def test_no_play_area_asks_question(self) -> None:
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=None,
            entities=[
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic", required_components=["position"]),
            ],
            interactions=[],
            invariants=[],
            degenerate_states=[],
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        area_qs = [q for q in questions if q.category == "bounds"]
        assert any("play area" in q.question.lower() for q in area_qs)

    def test_no_degenerate_states_asks_question(self) -> None:
        spec = GameDesignSpec(
            title="Test",
            description="Test",
            play_area=PlayAreaSpec(width=800.0, height=600.0),
            entities=[
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic",
                           bounds=BoundsSpec(x_min=0, x_max=800, y_min=0, y_max=600),
                           speed_max=500.0,
                           required_components=["position"]),
            ],
            interactions=[],
            invariants=[],
            degenerate_states=[],
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        degen_qs = [q for q in questions if q.category == "degenerate"]
        assert len(degen_qs) >= 1
```

**Step 2: Run tests to verify they fail**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_gdd.py::TestCompletenessChecker" -v`
Expected: FAIL with `ImportError`

**Step 3: Implement CompletenessChecker**

Add `CompletenessChecker` class to `gdd.py`:

```python
class CompletenessChecker:
    """Validates a GameDesignSpec for gaps and missing constraints.

    Checks:
    - Every dynamic/kinematic entity has bounds defined
    - Every movable entity pair has an interaction specified
    - Every dynamic entity has a speed limit
    - Play area is defined
    - At least one degenerate state is identified
    - Every invariant references a valid entity
    """

    def check(self, spec: GameDesignSpec) -> list[ClarificationQuestion]:
        questions: list[ClarificationQuestion] = []
        questions.extend(self._check_play_area(spec))
        questions.extend(self._check_entity_bounds(spec))
        questions.extend(self._check_interaction_matrix(spec))
        questions.extend(self._check_speed_limits(spec))
        questions.extend(self._check_degenerate_states(spec))
        return questions
```

Each `_check_*` method returns a `list[ClarificationQuestion]`. The logic:

- `_check_play_area`: If `spec.play_area is None`, ask "What are the play area dimensions?"
- `_check_entity_bounds`: For each entity with `body_type in ("dynamic", "kinematic")`, if `bounds is None`, ask about bounds.
- `_check_interaction_matrix`: For each pair of entities where at least one is dynamic/kinematic, check if an interaction exists (in either direction). If not, ask.
- `_check_speed_limits`: For each dynamic entity without `speed_max`, ask.
- `_check_degenerate_states`: If `spec.degenerate_states` is empty, ask.

**Step 4: Run tests to verify they pass**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_gdd.py" -v`
Expected: All PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/gdd.py python/nomai-sdk/tests/test_gdd.py
git commit -m "feat: CompletenessChecker for GameDesignSpec gap detection"
```

---

### Task 3: Add `component_range:` Invariant Condition to Verifier

**Files:**
- Modify: `python/nomai-sdk/nomai/verify.py:789-832` (the `_verify_invariant` method)
- Modify: `python/nomai-sdk/tests/test_verify.py`
- Modify: `python/nomai-sdk/nomai/breakout_intents.py:277-300` (replace placeholder invariants)

Currently, bounds invariants use `entity_count >= 0` as a placeholder because the verifier can't evaluate position-range conditions. This task adds a new `component_range:` condition format.

**Step 1: Write failing tests for component_range invariant**

Append to `tests/test_verify.py` (in appropriate test class):

```python
def test_component_range_invariant_passes_when_in_range(self) -> None:
    engine = VerificationEngine()
    intent = IntentSpec(
        name="ball_x_in_bounds",
        kind=IntentKind.INVARIANT,
        description="Ball x position must stay in [0, 800]",
        condition="component_range:ball.position.x in [0, 800]",
    )
    suite = VerificationSuite(name="test", description="test", intents=[intent])
    manifests = [
        _make_manifest(tick=0, changes=[
            _make_change("position", new_value={"x": 400.0, "y": 300.0},
                        entity_id=1, reason_detail="ball"),
        ]),
        _make_manifest(tick=1, changes=[
            _make_change("position", new_value={"x": 100.0, "y": 200.0},
                        entity_id=1, reason_detail="ball"),
        ]),
    ]
    report = engine.verify(suite, manifests)
    assert report.all_passed

def test_component_range_invariant_fails_when_out_of_range(self) -> None:
    engine = VerificationEngine()
    intent = IntentSpec(
        name="paddle_x_in_bounds",
        kind=IntentKind.INVARIANT,
        description="Paddle x must stay in [0, 800]",
        condition="component_range:paddle.position.x in [0, 800]",
    )
    suite = VerificationSuite(name="test", description="test", intents=[intent])
    manifests = [
        _make_manifest(tick=0, changes=[
            _make_change("position", new_value={"x": 400.0, "y": 300.0},
                        entity_id=1, reason_detail="paddle"),
        ]),
        _make_manifest(tick=1, changes=[
            _make_change("position", new_value={"x": -50.0, "y": 300.0},
                        entity_id=1, reason_detail="paddle"),
        ]),
    ]
    report = engine.verify(suite, manifests)
    assert not report.all_passed
    assert "paddle_x_in_bounds" in report.results[0].intent_name
    assert "-50" in report.results[0].failure_reason
```

**Step 2: Run tests to verify they fail**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_verify.py" -k "component_range" -v`
Expected: FAIL (free-form condition falls through as pass)

**Step 3: Implement component_range condition parser**

In `verify.py`, in `_verify_invariant` method (line 789), add a new condition branch before the free-form fallback:

```python
# Parse component_range conditions
# Format: "component_range:<entity>.<component>.<field> in [<min>, <max>]"
if condition.startswith("component_range:"):
    return self._verify_component_range_invariant(
        intent.name, condition, manifests
    )
```

Add the implementation method:

```python
def _verify_component_range_invariant(
    self,
    intent_name: str,
    condition: str,
    manifests: list[TickManifest],
) -> IntentResult:
    """Evaluate a component_range invariant.

    Format: ``"component_range:<entity>.<component>.<field> in [<min>, <max>]"``
    """
    try:
        rest = condition[len("component_range:"):]
        path_part, range_part = rest.split(" in ")
        path_parts = path_part.strip().split(".")
        if len(path_parts) != 3:
            return IntentResult(
                intent_name=intent_name, passed=False,
                failure_reason=f"Malformed component_range: need entity.component.field, got '{path_part}'",
            )
        entity_name, component, field_name = path_parts
        range_str = range_part.strip().strip("[]")
        range_min, range_max = [float(x.strip()) for x in range_str.split(",")]
    except Exception as exc:
        return IntentResult(
            intent_name=intent_name, passed=False,
            failure_reason=f"Failed to parse component_range '{condition}': {exc}",
        )

    for manifest in manifests:
        for change in manifest.component_changes:
            if change.component_type_name != component:
                continue
            if not self._matches_entity(change, entity_name):
                continue
            value = self._extract_field_value(change.new_value, field_name)
            if not isinstance(value, (int, float)):
                continue
            if value < range_min or value > range_max:
                return IntentResult(
                    intent_name=intent_name,
                    passed=False,
                    trigger_tick=manifest.tick,
                    failure_reason=(
                        f"Entity '{entity_name}' {component}.{field_name} = {value} "
                        f"out of range [{range_min}, {range_max}] at tick {manifest.tick}"
                    ),
                    evidence=[change],
                    suggestion=(
                        f"Clamp '{entity_name}' {component}.{field_name} "
                        f"to stay within [{range_min}, {range_max}]."
                    ),
                )

    return IntentResult(intent_name=intent_name, passed=True)
```

**Step 4: Update breakout_intents.py to use real bounds**

Replace the placeholder invariants in `breakout_intents.py`:

```python
def _ball_in_bounds() -> IntentSpec:
    return IntentSpec(
        name="ball_in_bounds",
        kind=IntentKind.INVARIANT,
        description="Ball position must stay within game bounds (0-800 x, 0-600 y).",
        condition="component_range:ball.position.x in [0, 800]",
    )

def _paddle_in_bounds() -> IntentSpec:
    return IntentSpec(
        name="paddle_in_bounds",
        kind=IntentKind.INVARIANT,
        description="Paddle position must stay within game bounds (0-800 x).",
        condition="component_range:paddle.position.x in [0, 800]",
    )
```

Also remove the "entity_count >= 0 as evaluable proxy" comments from the module docstring and `build_breakout_suite` docstring.

**Step 5: Run all tests**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests" -v`
Expected: All PASS (some breakout regression test baselines may need updating)

**Step 6: Commit**

```bash
git add python/nomai-sdk/nomai/verify.py python/nomai-sdk/nomai/breakout_intents.py python/nomai-sdk/tests/test_verify.py
git commit -m "feat: component_range invariant condition for real bounds checking"
```

---

### Task 4: IntentGenerator

**Files:**
- Modify: `python/nomai-sdk/nomai/gdd.py`
- Modify: `python/nomai-sdk/tests/test_gdd.py`

Compiles a `GameDesignSpec` into a `VerificationSuite` by generating one atomic `IntentSpec` per constraint.

**Step 1: Write failing tests**

Append to `tests/test_gdd.py`:

```python
from nomai.gdd import IntentGenerator
from nomai.intents import IntentKind, VerificationSuite


class TestIntentGenerator:
    def test_generates_entity_intents(self) -> None:
        spec = _make_breakout_spec()
        gen = IntentGenerator()
        suite = gen.generate(spec)
        entity_intents = [i for i in suite.intents if i.kind == IntentKind.ENTITY]
        assert len(entity_intents) == 3  # paddle, ball, brick

    def test_generates_bounds_invariants(self) -> None:
        spec = _make_breakout_spec()
        gen = IntentGenerator()
        suite = gen.generate(spec)
        invariants = [i for i in suite.intents if i.kind == IntentKind.INVARIANT]
        bounds_invariants = [i for i in invariants if "bounds" in i.name]
        # paddle has x bounds, ball has x+y bounds = 3 total
        assert len(bounds_invariants) >= 2

    def test_generates_speed_metrics(self) -> None:
        spec = _make_breakout_spec()
        gen = IntentGenerator()
        suite = gen.generate(spec)
        metrics = [i for i in suite.intents if i.kind == IntentKind.METRIC]
        assert len(metrics) >= 1  # ball speed

    def test_generates_interaction_behaviors(self) -> None:
        spec = _make_breakout_spec()
        gen = IntentGenerator()
        suite = gen.generate(spec)
        behaviors = [i for i in suite.intents if i.kind == IntentKind.BEHAVIOR]
        assert len(behaviors) >= 3  # ball-paddle, ball-brick, ball-wall

    def test_generates_degenerate_state_invariants(self) -> None:
        spec = _make_breakout_spec()
        gen = IntentGenerator()
        suite = gen.generate(spec)
        invariants = [i for i in suite.intents if i.kind == IntentKind.INVARIANT]
        degen = [i for i in invariants if "degenerate" in i.name or "stuck" in i.name]
        assert len(degen) >= 1

    def test_suite_is_json_serializable(self) -> None:
        spec = _make_breakout_spec()
        gen = IntentGenerator()
        suite = gen.generate(spec)
        json_str = suite.to_json()
        restored = VerificationSuite.from_json(json_str)
        assert restored.name == suite.name
        assert len(restored.intents) == len(suite.intents)

    def test_empty_spec_generates_no_intents(self) -> None:
        spec = GameDesignSpec(
            title="Empty", description="Empty",
            play_area=None, entities=[], interactions=[],
            invariants=[], degenerate_states=[],
        )
        gen = IntentGenerator()
        suite = gen.generate(spec)
        assert len(suite.intents) == 0
```

**Step 2: Run tests to verify they fail**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_gdd.py::TestIntentGenerator" -v`
Expected: FAIL with `ImportError`

**Step 3: Implement IntentGenerator**

Add to `gdd.py`:

```python
class IntentGenerator:
    """Compiles a GameDesignSpec into a VerificationSuite.

    Generates one atomic IntentSpec per constraint for maximum
    failure localization in the verification report.
    """

    def generate(self, spec: GameDesignSpec) -> VerificationSuite:
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
```

Generation rules:
- **Entity intents**: One per entity, using `entity_type`, `role`, `required_components`.
- **Bounds invariants**: For each entity with bounds, one `component_range:` invariant per axis that has min/max defined.
- **Speed metrics**: For each entity with `speed_max`, generate metric intents for velocity dx/dy with range `(-speed_max, speed_max)`.
- **Interaction behaviors**: For each interaction, generate a behavior intent with appropriate trigger (collision) and expected outcome (component_changed for reflect, entity_despawned for destroy).
- **Degenerate state invariants**: For each degenerate state with `condition="equals_zero"`, generate a metric intent that checks the field stays outside zero (use `metric_range` excluding zero, e.g. `(-inf, -0.01)` or `(0.01, inf)` — or use a `component_range:` with a warning description).

**Step 4: Run tests**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_gdd.py" -v`
Expected: All PASS

**Step 5: Commit**

```bash
git add python/nomai-sdk/nomai/gdd.py python/nomai-sdk/tests/test_gdd.py
git commit -m "feat: IntentGenerator compiles GameDesignSpec to VerificationSuite"
```

---

### Task 5: Update Exports and Regression Baselines

**Files:**
- Modify: `python/nomai-sdk/nomai/__init__.py`
- Modify: `tests/regression/breakout_fixed_baseline.json` (if exists)

**Step 1: Update `__init__.py` exports**

Add to `__init__.py`:
```python
from nomai.gdd import (
    BoundsSpec,
    ClarificationQuestion,
    CompletenessChecker,
    DegenerateStateSpec,
    EntitySpec,
    GameDesignSpec,
    IntentGenerator,
    InteractionSpec,
    InvariantSpec,
    PlayAreaSpec,
)
```

And add to `__all__`.

**Step 2: Update regression baseline if needed**

If the breakout invariant condition changes (from `entity_count >= 0` to `component_range:...`) break the regression baseline JSON, update it.

**Step 3: Run full test suite**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests" -v`
Expected: All PASS

**Step 4: Commit**

```bash
git add python/nomai-sdk/nomai/__init__.py tests/regression/
git commit -m "feat: export GDD analyzer types from nomai SDK"
```

---

### Task 6: Full Pipeline Integration Test

**Files:**
- Modify: `python/nomai-sdk/tests/test_gdd.py`

**Step 1: Write integration test**

```python
class TestFullPipeline:
    """End-to-end: GameDesignSpec -> CompletenessChecker -> IntentGenerator -> verify."""

    def test_breakout_pipeline(self) -> None:
        # 1. Build spec
        spec = _make_breakout_spec()

        # 2. Check completeness
        checker = CompletenessChecker()
        questions = checker.check(spec)
        assert len(questions) == 0, f"Unexpected gaps: {questions}"

        # 3. Generate intents
        gen = IntentGenerator()
        suite = gen.generate(spec)
        assert len(suite.intents) > 0

        # 4. Verify the suite is well-formed
        for intent in suite.intents:
            warnings = intent.validate()
            assert len(warnings) == 0, f"{intent.name}: {warnings}"

    def test_incomplete_spec_produces_questions(self) -> None:
        # Minimal spec with known gaps
        spec = GameDesignSpec(
            title="Minimal",
            description="Test",
            play_area=None,
            entities=[
                EntitySpec(name="ball", entity_type="projectile", role="ball",
                           body_type="dynamic", required_components=["position"]),
                EntitySpec(name="paddle", entity_type="character", role="paddle",
                           body_type="kinematic", required_components=["position"]),
            ],
            interactions=[],
            invariants=[],
            degenerate_states=[],
        )
        checker = CompletenessChecker()
        questions = checker.check(spec)
        # Should ask about: play area, ball bounds, paddle bounds,
        # ball speed, ball-paddle interaction, degenerate states
        assert len(questions) >= 4
        categories = {q.category for q in questions}
        assert "bounds" in categories
        assert "interaction" in categories
        assert "degenerate" in categories
```

**Step 2: Run tests**

Run: `python -m pytest "B:\Projects\Nomai\python\nomai-sdk\tests\test_gdd.py" -v`
Expected: All PASS

**Step 3: Commit**

```bash
git add python/nomai-sdk/tests/test_gdd.py
git commit -m "test: full GDD pipeline integration tests"
```

---

## Summary

| Task | What | Files |
|------|------|-------|
| 1 | GameDesignSpec data model | `gdd.py`, `test_gdd.py` |
| 2 | CompletenessChecker | `gdd.py`, `test_gdd.py` |
| 3 | `component_range:` invariant condition | `verify.py`, `breakout_intents.py`, `test_verify.py` |
| 4 | IntentGenerator | `gdd.py`, `test_gdd.py` |
| 5 | Exports + regression baselines | `__init__.py`, baselines |
| 6 | Full pipeline integration test | `test_gdd.py` |

After this, the AI agent's workflow becomes:
1. Game designer writes free-form GDD
2. AI extracts `GameDesignSpec` from the prose (using the schema as a target)
3. `CompletenessChecker.check()` finds gaps → AI asks designer clarifying questions
4. `IntentGenerator.generate()` compiles to `VerificationSuite`
5. `VerificationEngine.verify()` runs against manifests
6. Bugs like "paddle off-screen" and "ball stuck horizontal" are caught automatically
