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
from nomai.intents import IntentKind, VerificationSuite
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
        "wall": {
            "entity_type": "boundary",
            "role": "wall",
            "tier": "Semantic",
        },
    }


def _build_correct_gameplay_manifests() -> list[TickManifest]:
    """Build manifests simulating correct breakout gameplay.

    Sequence:
    - Tick 0: Initial state, 20 bricks
    - Tick 1: Ball position reaches boundary (x<=0), velocity.dx changes
    - Tick 2: Ball-paddle collision, velocity.dy changes (paddle bounce)
    - Tick 3: Ball-brick collision, brick despawns, score increases
    - Tick 4-5: More bricks destroyed, score keeps increasing
    - Tick 6: Last brick destroyed (count=0), game state -> won
    """
    manifests: list[TickManifest] = []

    # Tick 0: initial state
    manifests.append(_make_manifest(tick=0, aggregates=_aggregates(brick_count=20)))

    # Tick 1: Ball hits wall -- collision event, velocity.dx flips sign
    manifests.append(_make_manifest(
        tick=1,
        events=[
            _make_event(
                event_type="collision",
                description="ball collides with wall",
                involved=[1, 50],  # 50 = wall entity
                tick=1,
                reason_detail="ball:wall",
            ),
        ],
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
        """Report covers all 13 intents."""
        suite = build_breakout_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()
        entity_index = _breakout_entity_index()

        report = engine.verify(suite, manifests, entity_index)

        assert report.total_intents == 13
        assert report.passed == 13
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
            r for r in report.results
            if r.intent_name == "ball_bounces_off_paddle"
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
        bounce_fixes = [
            f for f in fixes if f.intent_name == "ball_bounces_off_paddle"
        ]
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
                    _make_change(
                        1, "position", {"x": 1.0, "y": 300.0},
                        {"x": 0.0, "y": 300.0}, 1, "GameRule", "ball:wall",
                    ),
                    _make_change(
                        1, "velocity", {"dx": -5.0, "dy": 3.0},
                        {"dx": 5.0, "dy": 3.0}, 1, "GameRule", "ball:wall",
                    ),
                ],
                aggregates=_aggregates(brick_count=20),
            ),
            # Tick 2: Ball-paddle collision with velocity change
            _make_manifest(
                tick=2,
                events=[
                    _make_event(
                        "collision", "ball:paddle", [1, 0], 2, "ball:paddle",
                    ),
                ],
                changes=[
                    _make_change(
                        1, "velocity", {"dx": 5.0, "dy": -3.0},
                        {"dx": 5.0, "dy": 3.0}, 2, "GameRule", "ball:paddle",
                    ),
                ],
                aggregates=_aggregates(brick_count=20),
            ),
            # Tick 3: Ball-brick collision but NO despawn -- this is the bug
            _make_manifest(
                tick=3,
                events=[
                    _make_event(
                        "collision", "ball:brick", [1, 100], 3, "ball:brick",
                    ),
                ],
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
            r for r in report.results
            if r.intent_name == "brick_destroyed_on_hit"
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
        brick_fixes = [
            f for f in fixes if f.intent_name == "brick_destroyed_on_hit"
        ]
        assert len(brick_fixes) > 0


class TestRegressionTestRoundTrip:
    """4. Regression test: create, save, load, replay.

    Note: RegressionTest.replay() does not accept entity_index, so
    regression round-trip tests use a behavior-only subset of the suite.
    Entity intents require external context (entity_index) that the
    RegressionTest dataclass does not store.
    """

    @staticmethod
    def _replay_suite() -> VerificationSuite:
        """Build a suite excluding entity intents for replay compatibility."""
        full = build_breakout_suite()
        return VerificationSuite(
            name=full.name,
            description=full.description,
            intents=[
                i for i in full.intents if i.kind != IntentKind.ENTITY
            ],
        )

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
        assert regression.expected_pass_count == 13
        assert regression.expected_fail_count == 0

    def test_regression_save_and_load(self, tmp_path: Path) -> None:
        """Regression test survives save/load cycle."""
        suite = self._replay_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()

        report = engine.verify(suite, manifests)
        regression = RegressionTest.create(
            "breakout-rt", suite, manifests, report,
        )

        filepath = tmp_path / "breakout_regression.json"
        regression.save(filepath)
        loaded = RegressionTest.load(filepath)

        assert loaded.name == "breakout-rt"
        assert len(loaded.manifests) == len(manifests)
        assert loaded.expected_pass_count == regression.expected_pass_count
        assert loaded.expected_fail_count == regression.expected_fail_count

    def test_regression_replay_passes_with_same_manifests(self) -> None:
        """Replaying with same manifests produces same pass/fail counts."""
        suite = self._replay_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()

        report = engine.verify(suite, manifests)
        assert report.all_passed

        regression = RegressionTest.create(
            "replay-test", suite, manifests, report,
        )

        replay_result = regression.replay(engine)
        assert replay_result.passed
        # 10 = 13 total intents - 3 entity intents (excluded from replay suite)
        assert replay_result.actual_passed == 10
        assert replay_result.actual_failed == 0

    def test_regression_replay_full_roundtrip(self, tmp_path: Path) -> None:
        """Full round-trip: create, save, load, replay -- all passing."""
        suite = self._replay_suite()
        engine = VerificationEngine()
        manifests = _build_correct_gameplay_manifests()

        report = engine.verify(suite, manifests)
        assert report.all_passed

        regression = RegressionTest.create(
            "full-rt", suite, manifests, report,
        )

        filepath = tmp_path / "full_roundtrip.json"
        regression.save(filepath)
        loaded = RegressionTest.load(filepath)

        replay_result = loaded.replay(engine)
        assert replay_result.passed, (
            f"Replay failed: expected {replay_result.expected_passed}p/"
            f"{replay_result.expected_failed}f, got "
            f"{replay_result.actual_passed}p/{replay_result.actual_failed}f"
        )
