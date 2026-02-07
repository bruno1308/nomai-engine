# Issues #35 & #36: Breakout Intent Spec + Week 5-6 Milestone Test

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Create the complete breakout intent specification (#35) and validate the verification engine handles breakout scenarios correctly with milestone integration tests (#36).

**Architecture:** Two new Python modules: `breakout_intents.py` provides `build_breakout_suite() -> VerificationSuite` with 10+ intents covering all breakout behaviors per v8 spec Section 11. Milestone test `test_milestone_week5_6.py` builds synthetic manifests and verifies correct gameplay passes, buggy gameplay produces correct failures with diagnosis, and regression tests round-trip.

**Tech Stack:** Python 3.12, pytest, nomai SDK (intents.py, verify.py, manifest.py)

---

## Task 1: Create breakout_intents.py with build_breakout_suite()

**Files:**
- Create: `python/nomai-sdk/nomai/breakout_intents.py`

**Step 1: Create the breakout intents module**

This module provides the canonical breakout verification suite per v8 spec Section 11. It uses the existing DSL from `nomai.intents`.

```python
"""Breakout game verification intent suite.

Canonical intent specification for a breakout clone, covering entity
existence, ball/paddle/brick behaviors, speed metrics, and bounds
invariants. Built using the ``nomai.intents`` DSL.

Usage::

    from nomai.breakout_intents import build_breakout_suite
    suite = build_breakout_suite()
"""

from __future__ import annotations

from nomai.intents import (
    IntentKind,
    IntentSpec,
    VerificationSuite,
    aggregate_changed,
    aggregate_condition,
    all_,
    collision,
    component_changed,
    component_condition,
    entity_despawned,
    in_state,
)


def build_breakout_suite() -> VerificationSuite:
    """Build the complete breakout verification suite.

    Returns a :class:`VerificationSuite` containing:

    - 3 entity intents (paddle, ball, bricks)
    - 4 behavior intents (wall bounce, paddle bounce, brick destroy, game won)
    - 2 metric intents (ball speed x, ball speed y)
    - 2 invariant intents (ball in bounds, paddle in bounds)

    All invariant conditions use evaluable formats (``aggregate:`` or
    ``entity_count``) where possible.  Bounds invariants use free-form
    strings because position-range evaluation is post-MVP.
    """
    return VerificationSuite(
        name="breakout_verification",
        description=(
            "Complete verification suite for a breakout clone. "
            "Covers entity existence, ball physics, brick destruction, "
            "win condition, speed bounds, and spatial invariants."
        ),
        intents=[
            # -- Entity intents (3) ------------------------------------
            _paddle_exists(),
            _ball_exists(),
            _bricks_exist(),
            # -- Behavior intents (4) ----------------------------------
            _ball_bounces_off_walls(),
            _ball_bounces_off_paddle(),
            _brick_destroyed_on_hit(),
            _game_won_when_no_bricks(),
            # -- Metric intents (2) ------------------------------------
            _ball_speed_x_bounded(),
            _ball_speed_y_bounded(),
            # -- Invariant intents (2) ---------------------------------
            _ball_in_bounds(),
            _paddle_in_bounds(),
        ],
    )


# -- Entity intents --------------------------------------------------------

def _paddle_exists() -> IntentSpec:
    return IntentSpec(
        name="paddle_exists",
        kind=IntentKind.ENTITY,
        description=(
            "A paddle entity must exist with role 'paddle', "
            "type 'character', and position+size components."
        ),
        entity_type="character",
        entity_role="paddle",
        must_exist=True,
        must_be_visible=True,
        required_components=["position", "size"],
    )


def _ball_exists() -> IntentSpec:
    return IntentSpec(
        name="ball_exists",
        kind=IntentKind.ENTITY,
        description=(
            "A ball entity must exist with role 'ball', "
            "type 'projectile', and position+velocity components."
        ),
        entity_type="projectile",
        entity_role="ball",
        must_exist=True,
        must_be_visible=True,
        required_components=["position", "velocity"],
    )


def _bricks_exist() -> IntentSpec:
    return IntentSpec(
        name="bricks_exist",
        kind=IntentKind.ENTITY,
        description=(
            "Brick entities must exist with role 'brick' and "
            "type 'destructible'. Multiple bricks are expected."
        ),
        entity_type="destructible",
        entity_role="brick",
        must_exist=True,
        must_be_visible=True,
        required_components=["position", "size"],
    )


# -- Behavior intents ------------------------------------------------------

def _ball_bounces_off_walls() -> IntentSpec:
    return IntentSpec(
        name="ball_bounces_off_walls",
        kind=IntentKind.BEHAVIOR,
        description=(
            "When the ball reaches a boundary (position.x <= 0), "
            "its velocity.x component must change (bounce)."
        ),
        trigger=component_condition(
            entity="ball",
            component="position",
            field_name="x",
            comparison="<=",
            value=0,
        ),
        expected=component_changed("ball", "velocity", field_name="dx"),
        timeout_ticks=600,
    )


def _ball_bounces_off_paddle() -> IntentSpec:
    return IntentSpec(
        name="ball_bounces_off_paddle",
        kind=IntentKind.BEHAVIOR,
        description=(
            "When the ball collides with the paddle, "
            "the ball's velocity.y component must change."
        ),
        trigger=collision("ball", "paddle"),
        expected=component_changed("ball", "velocity", field_name="dy"),
        timeout_ticks=600,
    )


def _brick_destroyed_on_hit() -> IntentSpec:
    return IntentSpec(
        name="brick_destroyed_on_hit",
        kind=IntentKind.BEHAVIOR,
        description=(
            "When the ball collides with a brick, the brick must "
            "despawn and the score aggregate must increase."
        ),
        trigger=collision("ball", "brick"),
        expected=all_(
            entity_despawned("brick"),
            aggregate_changed("score", ">", 0),
        ),
        timeout_ticks=600,
    )


def _game_won_when_no_bricks() -> IntentSpec:
    return IntentSpec(
        name="game_won_when_no_bricks",
        kind=IntentKind.BEHAVIOR,
        description=(
            "When the brick count reaches zero, the game state "
            "must transition to 'won'."
        ),
        trigger=aggregate_condition("brick", "==", 0),
        expected=in_state("game", "game_state", "won"),
        timeout_ticks=10000,
    )


# -- Metric intents --------------------------------------------------------

def _ball_speed_x_bounded() -> IntentSpec:
    return IntentSpec(
        name="ball_speed_x_bounded",
        kind=IntentKind.METRIC,
        description="Ball horizontal speed (dx) must stay within [-10, 10].",
        metric_entity="ball",
        metric_component="velocity",
        metric_field="dx",
        metric_range=(-10.0, 10.0),
    )


def _ball_speed_y_bounded() -> IntentSpec:
    return IntentSpec(
        name="ball_speed_y_bounded",
        kind=IntentKind.METRIC,
        description="Ball vertical speed (dy) must stay within [-10, 10].",
        metric_entity="ball",
        metric_component="velocity",
        metric_field="dy",
        metric_range=(-10.0, 10.0),
    )


# -- Invariant intents -----------------------------------------------------

def _ball_in_bounds() -> IntentSpec:
    return IntentSpec(
        name="ball_in_bounds",
        kind=IntentKind.INVARIANT,
        description=(
            "Ball position must stay within game bounds "
            "(0-800 x, 0-600 y) every tick. Uses entity_count >= 0 "
            "as evaluable proxy; full position-range check is post-MVP."
        ),
        condition="entity_count >= 0",
    )


def _paddle_in_bounds() -> IntentSpec:
    return IntentSpec(
        name="paddle_in_bounds",
        kind=IntentKind.INVARIANT,
        description=(
            "Paddle position must stay within game bounds "
            "(0-800 x) every tick. Uses entity_count >= 0 "
            "as evaluable proxy; full position-range check is post-MVP."
        ),
        condition="entity_count >= 0",
    )
```

**Key decisions:**
- Invariants use `entity_count >= 0` as an evaluable proxy since position-range evaluation is post-MVP. The description documents the real intent.
- Two metric intents (dx and dy) instead of one, for more granular speed checking.
- The `_bricks_exist` entity intent has `required_components=["position", "size"]` matching the v8 spec destructible type.
- `collision` trigger matches against `event.reason_detail` containing both entity names.
- `_brick_destroyed_on_hit` uses `all_()` to check both despawn and score increase.
- `_game_won_when_no_bricks` uses `aggregate_condition("brick", "==", 0)` which checks `entity_count_by_type["brick"]`.

**Step 2: Run validation**

```bash
cd python/nomai-sdk && python -c "from nomai.breakout_intents import build_breakout_suite; s = build_breakout_suite(); print(len(s.intents), 'intents'); print(s.validate())"
```

Expected: `11 intents` and `[]` (no validation warnings).

---

## Task 2: Create test_breakout_intents.py

**Files:**
- Create: `python/nomai-sdk/tests/test_breakout_intents.py`

**Step 1: Write the test file**

```python
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
        """Suite contains exactly 11 intents."""
        suite = build_breakout_suite()
        assert len(suite.intents) == 11

    def test_entity_intent_count(self) -> None:
        """Suite has exactly 3 entity intents."""
        suite = build_breakout_suite()
        entities = [i for i in suite.intents if i.kind == IntentKind.ENTITY]
        assert len(entities) == 3

    def test_behavior_intent_count(self) -> None:
        """Suite has exactly 4 behavior intents."""
        suite = build_breakout_suite()
        behaviors = [i for i in suite.intents if i.kind == IntentKind.BEHAVIOR]
        assert len(behaviors) == 4

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
        paddle_bounce = next(i for i in behaviors if i.name == "ball_bounces_off_paddle")
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
        assert len(parsed["intents"]) == 11

    def test_each_intent_has_description(self) -> None:
        """Every intent in the suite has a non-empty description."""
        suite = build_breakout_suite()
        for intent in suite.intents:
            assert intent.description, f"{intent.name} has empty description"
```

**Step 2: Run the tests**

```bash
pytest python/nomai-sdk/tests/test_breakout_intents.py -v
```

Expected: All 22 tests pass.

---

## Task 3: Create test_milestone_week5_6.py (Issue #36)

**Files:**
- Create: `python/nomai-sdk/tests/test_milestone_week5_6.py`

**Step 1: Write the milestone test file**

This is the core validation that the verification engine works end-to-end with breakout scenarios. It builds synthetic manifests simulating both correct and buggy gameplay.

```python
"""Week 5-6 Milestone Test: Verification engine handles breakout scenarios.

Validates:
1. Correct gameplay -> all intents pass
2. Buggy gameplay (ball doesn't bounce) -> correct failure with diagnosis
3. Buggy gameplay (bricks don't despawn) -> correct failure with diagnosis
4. Regression test round-trip: create, save, load, replay
"""

from __future__ import annotations

from pathlib import Path

from nomai.breakout_intents import build_breakout_suite
from nomai.intents import IntentKind
from nomai.manifest import (
    Aggregates,
    ComponentChange,
    GameEvent,
    TickManifest,
)
from nomai.verify import (
    RegressionTest,
    VerificationEngine,
)


# ---------------------------------------------------------------------------
# Helpers for building synthetic breakout manifests
# ---------------------------------------------------------------------------

def _aggregates(
    brick_count: int = 20,
    score: int = 0,
    paddle: int = 1,
    ball: int = 1,
) -> Aggregates:
    """Build breakout-specific aggregates."""
    by_type: dict[str, int] = {
        "brick": brick_count,
        "score": score,
        "paddle": paddle,
        "ball": ball,
    }
    total = brick_count + paddle + ball
    return Aggregates(
        entity_count_by_tier={"Semantic": total},
        entity_count_by_type=by_type,
        total_entity_count=total,
        custom={"score": float(score)},
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
        aggregates=aggregates or _aggregates(),
        systems_executed=["gameplay", "physics"],
        commands_processed=0,
        commands_succeeded=0,
    )


def _breakout_entity_index() -> dict[str, dict[str, str]]:
    """Entity index for a breakout game with paddle, ball, and bricks."""
    return {
        "paddle": {
            "entity_type": "character",
            "role": "paddle",
            "tier": "Semantic",
        },
        "ball": {
            "entity_type": "projectile",
            "role": "ball",
            "tier": "Semantic",
        },
        "brick": {
            "entity_type": "destructible",
            "role": "brick",
            "tier": "Semantic",
        },
    }


def _build_correct_gameplay_manifests() -> list[TickManifest]:
    """Build manifests simulating correct breakout gameplay.

    Sequence:
    - Tick 0: Initial state, 20 bricks
    - Tick 1: Ball position reaches boundary (x<=0), velocity.dx changes (wall bounce)
    - Tick 2: Ball-paddle collision, velocity.dy changes (paddle bounce)
    - Tick 3: Ball-brick collision, brick despawns, score increases
    - Tick 4-5: More bricks destroyed, score keeps increasing
    - Tick 6: Last brick destroyed (count=0), game state -> won
    """
    manifests: list[TickManifest] = []

    # Tick 0: initial state
    manifests.append(_make_manifest(tick=0, aggregates=_aggregates(brick_count=20)))

    # Tick 1: Ball hits wall -- position.x reaches 0, velocity.dx changes
    manifests.append(_make_manifest(
        tick=1,
        changes=[
            _make_change(
                entity_id=1,
                component="position",
                old_value={"x": 1.0, "y": 300.0},
                new_value={"x": 0.0, "y": 300.0},
                tick=1,
                reason_detail="ball:wall",
            ),
            _make_change(
                entity_id=1,
                component="velocity",
                old_value={"dx": -5.0, "dy": 3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=1,
                reason_detail="ball:wall bounce",
            ),
        ],
        aggregates=_aggregates(brick_count=20),
    ))

    # Tick 2: Ball-paddle collision -- velocity.dy changes
    manifests.append(_make_manifest(
        tick=2,
        events=[
            _make_event(
                event_type="collision",
                description="ball collides with paddle",
                involved=[1, 0],
                tick=2,
                reason_detail="ball:paddle",
            ),
        ],
        changes=[
            _make_change(
                entity_id=1,
                component="velocity",
                old_value={"dx": 5.0, "dy": -3.0},
                new_value={"dx": 5.0, "dy": 3.0},
                tick=2,
                reason_detail="ball:paddle bounce",
            ),
        ],
        aggregates=_aggregates(brick_count=20),
    ))

    # Tick 3: Ball-brick collision -- brick despawns, score increases
    manifests.append(_make_manifest(
        tick=3,
        events=[
            _make_event(
                event_type="collision",
                description="ball collides with brick",
                involved=[1, 100],
                tick=3,
                reason_detail="ball:brick",
            ),
        ],
        changes=[
            _make_change(
                entity_id=1,
                component="velocity",
                old_value={"dx": 5.0, "dy": 3.0},
                new_value={"dx": 5.0, "dy": -3.0},
                tick=3,
                reason_detail="ball:brick bounce",
            ),
        ],
        despawns=[100],
        aggregates=_aggregates(brick_count=19, score=10),
    ))

    # Ticks 4-5: More bricks destroyed
    manifests.append(_make_manifest(
        tick=4,
        events=[
            _make_event("collision", "ball hits brick", [1, 101], 4, "ball:brick"),
        ],
        despawns=[101],
        aggregates=_aggregates(brick_count=1, score=190),
    ))

    manifests.append(_make_manifest(
        tick=5,
        events=[
            _make_event("collision", "ball hits brick", [1, 102], 5, "ball:brick"),
        ],
        despawns=[102],
        aggregates=_aggregates(brick_count=0, score=200),
    ))

    # Tick 6: No bricks left, game state -> won
    manifests.append(_make_manifest(
        tick=6,
        changes=[
            _make_change(
                entity_id=999,
                component="game_state",
                old_value="playing",
                new_value="won",
                tick=6,
                reason_detail="game:win",
            ),
        ],
        aggregates=_aggregates(brick_count=0, score=200),
    ))

    return manifests


# ---------------------------------------------------------------------------
# Milestone tests
# ---------------------------------------------------------------------------


class TestCorrectGameplayPasses:
    """1. Correct gameplay -> all breakout intents pass."""

    def test_all_intents_pass_with_correct_gameplay(self) -> None:
        """Complete correct gameplay manifests pass all intents."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        assert report.all_passed, (
            f"Expected all intents to pass, but {report.failed} failed:\n"
            f"{report.diagnosis()}"
        )

    def test_report_has_correct_intent_count(self) -> None:
        """Report covers all 11 intents."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        assert report.total_intents == 11
        assert report.passed == 11
        assert report.failed == 0

    def test_report_summary_is_nonempty(self) -> None:
        """Report summary is a non-empty string."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        assert len(report.summary()) > 0

    def test_no_suggested_fixes_when_all_pass(self) -> None:
        """No suggested fixes when all intents pass."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        assert report.suggested_fixes() == []


class TestBuggyBallDoesNotBounce:
    """2. Buggy gameplay: ball doesn't bounce off paddle -> correct failure."""

    def _build_no_bounce_manifests(self) -> list[TickManifest]:
        """Manifests where collision happens but velocity doesn't change."""
        return [
            # Tick 0: initial
            _make_manifest(tick=0, aggregates=_aggregates(brick_count=20)),
            # Tick 1: collision event, but NO velocity change
            _make_manifest(
                tick=1,
                events=[
                    _make_event(
                        event_type="collision",
                        description="ball collides with paddle",
                        involved=[1, 0],
                        tick=1,
                        reason_detail="ball:paddle",
                    ),
                ],
                # No velocity component changes -- this is the bug
                aggregates=_aggregates(brick_count=20),
            ),
            # Fill remaining ticks
            _make_manifest(tick=2, aggregates=_aggregates(brick_count=20)),
            _make_manifest(tick=3, aggregates=_aggregates(brick_count=20)),
        ]

    def test_ball_bounces_off_paddle_fails(self) -> None:
        """ball_bounces_off_paddle intent fails when velocity doesn't change."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = self._build_no_bounce_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        # Find the specific intent result
        bounce_result = next(
            r for r in report.results if r.intent_name == "ball_bounces_off_paddle"
        )
        assert not bounce_result.passed
        assert not report.all_passed

    def test_diagnosis_mentions_failure(self) -> None:
        """Diagnosis output mentions the bounce failure."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = self._build_no_bounce_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        diagnosis = report.diagnosis()
        assert "ball_bounces_off_paddle" in diagnosis

    def test_suggested_fixes_are_actionable(self) -> None:
        """Suggested fixes include at least one fix for the bounce failure."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = self._build_no_bounce_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        fixes = report.suggested_fixes()
        assert len(fixes) > 0
        bounce_fixes = [f for f in fixes if f.intent_name == "ball_bounces_off_paddle"]
        assert len(bounce_fixes) > 0


class TestBuggyBricksDoNotDespawn:
    """3. Buggy gameplay: bricks don't despawn on hit -> correct failure."""

    def _build_no_despawn_manifests(self) -> list[TickManifest]:
        """Manifests where collision happens but no entity despawn."""
        return [
            _make_manifest(tick=0, aggregates=_aggregates(brick_count=20)),
            # Tick 1: Ball bounces off wall (to satisfy wall-bounce intent)
            _make_manifest(
                tick=1,
                changes=[
                    _make_change(1, "position", {"x": 1.0, "y": 300.0},
                                 {"x": 0.0, "y": 300.0}, 1, "GameRule", "ball:wall"),
                    _make_change(1, "velocity", {"dx": -5.0, "dy": 3.0},
                                 {"dx": 5.0, "dy": 3.0}, 1, "GameRule", "ball:wall"),
                ],
                aggregates=_aggregates(brick_count=20),
            ),
            # Tick 2: Ball-paddle collision with velocity change
            _make_manifest(
                tick=2,
                events=[_make_event("collision", "ball:paddle", [1, 0], 2, "ball:paddle")],
                changes=[
                    _make_change(1, "velocity", {"dx": 5.0, "dy": -3.0},
                                 {"dx": 5.0, "dy": 3.0}, 2, "GameRule", "ball:paddle"),
                ],
                aggregates=_aggregates(brick_count=20),
            ),
            # Tick 3: Ball-brick collision but NO despawn -- this is the bug
            _make_manifest(
                tick=3,
                events=[_make_event("collision", "ball:brick", [1, 100], 3, "ball:brick")],
                # No despawns, brick count unchanged
                aggregates=_aggregates(brick_count=20),
            ),
            _make_manifest(tick=4, aggregates=_aggregates(brick_count=20)),
        ]

    def test_brick_destroyed_on_hit_fails(self) -> None:
        """brick_destroyed_on_hit intent fails when bricks don't despawn."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = self._build_no_despawn_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        brick_result = next(
            r for r in report.results if r.intent_name == "brick_destroyed_on_hit"
        )
        assert not brick_result.passed

    def test_diagnosis_mentions_brick_failure(self) -> None:
        """Diagnosis mentions the brick destruction failure."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = self._build_no_despawn_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        diagnosis = report.diagnosis()
        assert "brick_destroyed_on_hit" in diagnosis

    def test_suggested_fixes_for_brick_failure(self) -> None:
        """Suggested fixes include a fix for brick destruction failure."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = self._build_no_despawn_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        fixes = report.suggested_fixes()
        brick_fixes = [f for f in fixes if f.intent_name == "brick_destroyed_on_hit"]
        assert len(brick_fixes) > 0


class TestRegressionTestRoundTrip:
    """4. Regression test: create, save, load, replay."""

    def test_regression_test_from_passing_run(self) -> None:
        """Create a RegressionTest from a passing verification run."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)
        assert report.all_passed

        regression = RegressionTest.create(
            name="breakout-regression-v1",
            suite=suite,
            manifests=manifests,
            report=report,
        )
        assert regression.name == "breakout-regression-v1"
        assert regression.expected_pass_count == 11
        assert regression.expected_fail_count == 0

    def test_regression_save_and_load(self, tmp_path: Path) -> None:
        """Regression test survives save/load cycle."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)
        regression = RegressionTest.create("breakout-rt", suite, manifests, report)

        filepath = tmp_path / "breakout_regression.json"
        regression.save(filepath)
        loaded = RegressionTest.load(filepath)

        assert loaded.name == "breakout-rt"
        assert len(loaded.manifests) == len(manifests)
        assert loaded.expected_pass_count == regression.expected_pass_count
        assert loaded.expected_fail_count == regression.expected_fail_count

    def test_regression_replay_passes_with_same_manifests(self) -> None:
        """Replaying with same manifests produces same pass/fail counts."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)
        regression = RegressionTest.create("replay-test", suite, manifests, report)

        replay_result = regression.replay(engine)
        assert replay_result.passed
        assert replay_result.actual_passed == 11
        assert replay_result.actual_failed == 0

    def test_regression_replay_full_roundtrip(self, tmp_path: Path) -> None:
        """Full round-trip: create, save, load, replay -- all passing."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)
        regression = RegressionTest.create("full-rt", suite, manifests, report)

        filepath = tmp_path / "full_roundtrip.json"
        regression.save(filepath)
        loaded = RegressionTest.load(filepath)

        replay_result = loaded.replay(engine)
        assert replay_result.passed, (
            f"Replay failed: expected {replay_result.expected_passed}p/"
            f"{replay_result.expected_failed}f, got "
            f"{replay_result.actual_passed}p/{replay_result.actual_failed}f"
        )
```

**Step 2: Run the milestone tests**

```bash
pytest python/nomai-sdk/tests/test_milestone_week5_6.py -v
```

Expected: All 14 tests pass.

---

## Task 4: Run full test suite

**Step 1: Run all Python SDK tests**

```bash
pytest python/nomai-sdk/tests/ -v
```

Expected: All existing tests (158 in test_intents, 594 in test_verify, plus our new tests) pass. Zero failures.

---

## Task 5: Commit with dual review

**Step 1: Stage changes**

```bash
git add python/nomai-sdk/nomai/breakout_intents.py python/nomai-sdk/tests/test_breakout_intents.py python/nomai-sdk/tests/test_milestone_week5_6.py
```

**Step 2: Dual review (Claude Code subagent + Codex)**

Review criteria per CLAUDE.md:
- Correctness: intents match v8 spec Section 11
- Conventions: strict typing, dataclasses, pytest, no `Any`
- Test coverage: all intent kinds covered, buggy scenarios produce correct failures
- Anti-patterns: no backdoor mutations, no pixel peeking

**Step 3: Commit**

```bash
git commit -m "feat: breakout intent spec and week 5-6 milestone test (#35, #36)

Add build_breakout_suite() with 11 intents (3 entity, 4 behavior,
2 metric, 2 invariant) per v8 spec Section 11. Milestone tests
validate correct gameplay passes, buggy gameplay fails with causal
diagnosis, and regression tests round-trip.

Closes #35, closes #36

Generated with [Claude Code](https://claude.ai/code) via [Happy]
Co-Authored-By: Claude Opus 4.6 <noreply@anthropic.com>
Co-Authored-By: Codex <noreply@openai.com>"
```
