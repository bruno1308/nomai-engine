"""Breakout game verification intent suite.

Canonical intent specification for a breakout clone, covering entity
existence, ball/paddle/brick behaviors, speed metrics, and bounds
invariants. Built using the ``nomai.intents`` DSL.

Usage::

    from nomai.breakout_intents import build_breakout_suite
    suite = build_breakout_suite()

v8 Spec Deviations (DSL adaptation)
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
The v8 spec (Section 11) uses conceptual pseudo-code that differs from
the implemented DSL in several ways:

- **Bounce semantics:** v8 models ``sign_flipped=True``; the DSL's
  ``value_relation()`` now supports ``"sign_flipped"`` and
  ``"magnitude_preserved"`` relations for proper bounce verification.
- **Metric magnitude:** v8 uses ``measurement="magnitude"``; the DSL
  works at the field level, so speed is checked per-axis (dx, dy).
- **Entity types:** v8 uses ``"controller"`` for paddle; this codebase
  consistently uses ``"character"``.
- **Wildcard/count:** v8 uses ``"brick_*"`` and ``min_count=20``; the
  DSL does not support wildcards or count constraints on entity intents.
- **Bounds invariants:** v8 uses free-form position expressions; the
  verification engine supports ``component_range:`` conditions for
  checking that component fields stay within numeric bounds.
"""

from __future__ import annotations

from nomai.intents import (
    IntentKind,
    IntentSpec,
    VerificationSuite,
    aggregate_changed,
    aggregate_condition,
    all_,
    any_,
    collision,
    component_changed,
    component_condition,
    entity_despawned,
    in_state,
    value_relation,
)


def build_breakout_suite() -> VerificationSuite:
    """Build the complete breakout verification suite.

    Returns a :class:`VerificationSuite` containing:

    - 3 entity intents (paddle, ball, bricks)
    - 6 behavior intents (wall bounce, paddle bounce, brick destroy,
      ball-brick reflection, ball-wall reflection, game won)
    - 2 metric intents (ball speed x, ball speed y)
    - 2 invariant intents (ball in bounds, paddle in bounds)

    All invariant conditions use evaluable formats (``aggregate:``,
    ``entity_count``, or ``component_range:``) where possible.  Bounds
    invariants use ``component_range:`` for real position-range checks.
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
            # -- Behavior intents (6) ----------------------------------
            _ball_bounces_off_walls(),
            _ball_bounces_off_paddle(),
            _brick_destroyed_on_hit(),
            _ball_reflects_on_brick_collision(),
            _ball_reflects_on_wall_collision(),
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


def _ball_reflects_on_brick_collision() -> IntentSpec:
    return IntentSpec(
        name="ball_reflects_on_brick_collision",
        kind=IntentKind.BEHAVIOR,
        description=(
            "When the ball collides with a brick, the ball's velocity.dy "
            "must flip sign (bounce). This is the physics-response check "
            "that validates correct collision resolution."
        ),
        trigger=collision("ball", "brick"),
        expected=value_relation("ball", "velocity", "dy", "sign_flipped"),
        timeout_ticks=600,
    )


def _ball_reflects_on_wall_collision() -> IntentSpec:
    return IntentSpec(
        name="ball_reflects_on_wall_collision",
        kind=IntentKind.BEHAVIOR,
        description=(
            "When the ball collides with a wall, at least one velocity "
            "component must flip sign (bounce). Side walls flip dx, "
            "top/bottom walls flip dy."
        ),
        trigger=collision("ball", "wall"),
        expected=any_(
            value_relation("ball", "velocity", "dx", "sign_flipped"),
            value_relation("ball", "velocity", "dy", "sign_flipped"),
        ),
        timeout_ticks=600,
    )


# -- Metric intents --------------------------------------------------------


def _ball_speed_x_bounded() -> IntentSpec:
    return IntentSpec(
        name="ball_speed_x_bounded",
        kind=IntentKind.METRIC,
        description="Ball horizontal speed (dx) must stay within [-500, 500].",
        metric_entity="ball",
        metric_component="velocity",
        metric_field="dx",
        metric_range=(-500.0, 500.0),
    )


def _ball_speed_y_bounded() -> IntentSpec:
    return IntentSpec(
        name="ball_speed_y_bounded",
        kind=IntentKind.METRIC,
        description="Ball vertical speed (dy) must stay within [-500, 500].",
        metric_entity="ball",
        metric_component="velocity",
        metric_field="dy",
        metric_range=(-500.0, 500.0),
    )


# -- Invariant intents -----------------------------------------------------


def _ball_in_bounds() -> IntentSpec:
    return IntentSpec(
        name="ball_in_bounds",
        kind=IntentKind.INVARIANT,
        description=(
            "Ball position must stay within game bounds (0-800 x) "
            "every tick."
        ),
        condition="component_range:ball.position.x in [0, 800]",
    )


def _paddle_in_bounds() -> IntentSpec:
    return IntentSpec(
        name="paddle_in_bounds",
        kind=IntentKind.INVARIANT,
        description=(
            "Paddle position must stay within game bounds "
            "(0-800 x) every tick."
        ),
        condition="component_range:paddle.position.x in [0, 800]",
    )
