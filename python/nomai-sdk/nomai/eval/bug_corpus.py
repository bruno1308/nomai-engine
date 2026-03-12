"""Seeded bug corpus for testing verification accuracy.

Each ``SeededBug`` describes a deliberately broken game scenario with
known ground truth.  The corpus is used by the verification dimension
eval to measure bug detection precision and recall.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field

from nomai.manifest import (
    Aggregates,
    ComponentChange,
    GameEvent,
    TickManifest,
)

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class SeededBug:
    """A deliberately broken game scenario with known ground truth.

    Attributes:
        bug_id: Unique identifier (e.g. ``"collision_passthrough"``).
        name: Human-readable name.
        description: What is wrong in this scenario.
        category: Bug category (``"collision"``, ``"event"``,
            ``"spawn"``, ``"physics"``, ``"lifecycle"``).
        severity: ``"critical"``, ``"major"``, or ``"minor"``.
        manifests: Tick manifests exhibiting the bug.
        expected_detection: Should the verification engine flag this?
        ground_truth_root_cause: What the causal chain should trace to.
    """

    bug_id: str
    name: str
    description: str
    category: str
    severity: str
    manifests: list[TickManifest] = field(default_factory=list)
    expected_detection: bool = True
    ground_truth_root_cause: str = ""


# ---------------------------------------------------------------------------
# Manifest builders (minimal mock data for each bug scenario)
# ---------------------------------------------------------------------------

_EMPTY_AGGREGATES = Aggregates(
    entity_count_by_tier={"Semantic": 3},
    entity_count_by_type={"ball": 1, "paddle": 1, "brick": 1},
    total_entity_count=3,
    custom={},
)


def _make_manifest(
    tick: int,
    changes: list[ComponentChange] | None = None,
    events: list[GameEvent] | None = None,
    spawns: list[int] | None = None,
    despawns: list[int] | None = None,
    aggregates: Aggregates | None = None,
) -> TickManifest:
    return TickManifest(
        tick=tick,
        sim_time=tick * (1.0 / 60.0),
        entity_spawns=spawns or [],
        entity_despawns=despawns or [],
        component_changes=changes or [],
        events=events or [],
        aggregates=aggregates or _EMPTY_AGGREGATES,
        systems_executed=["physics", "gameplay"],
        commands_processed=len(changes or []),
        commands_succeeded=len(changes or []),
    )


def _pos_change(
    entity_id: int,
    tick: int,
    old_x: float,
    old_y: float,
    new_x: float,
    new_y: float,
) -> ComponentChange:
    return ComponentChange(
        entity_id=entity_id,
        component_type_name="position",
        old_value={"x": old_x, "y": old_y},
        new_value={"x": new_x, "y": new_y},
        changed_by_system=0,
        reason_type="SystemInternal",
        reason_detail="physics_step",
        command_index=0,
        tick=tick,
    )


# ---------------------------------------------------------------------------
# Seeded bugs
# ---------------------------------------------------------------------------

def ball_passes_through_paddle() -> SeededBug:
    """Ball position crosses paddle Y without collision event."""
    manifests = [
        _make_manifest(tick=1, changes=[
            _pos_change(entity_id=0, tick=1, old_x=400.0, old_y=100.0, new_x=400.0, new_y=50.0),
        ]),
        # Ball crosses paddle at y=30, no collision recorded.
        _make_manifest(tick=2, changes=[
            _pos_change(entity_id=0, tick=2, old_x=400.0, old_y=50.0, new_x=400.0, new_y=10.0),
        ]),
    ]
    return SeededBug(
        bug_id="ball_passes_through_paddle",
        name="Ball passes through paddle",
        description="Ball position crosses paddle y-coordinate without any collision event being recorded.",
        category="collision",
        severity="critical",
        manifests=manifests,
        expected_detection=True,
        ground_truth_root_cause="collision_detection",
    )


def score_not_incremented() -> SeededBug:
    """Brick despawned but score aggregate does not change."""
    manifests = [
        _make_manifest(
            tick=1,
            events=[GameEvent(
                event_type="collision",
                description="ball hit brick",
                involved_entities=[0, 2],
                caused_by_system=0,
                reason_type="CollisionResponse",
                reason_detail="[0,2]",
                tick=1,
            )],
            despawns=[2],
            aggregates=Aggregates(
                entity_count_by_tier={"Semantic": 2},
                entity_count_by_type={"ball": 1, "paddle": 1},
                total_entity_count=2,
                custom={},  # Score NOT incremented -- this is the bug.
            ),
        ),
    ]
    return SeededBug(
        bug_id="score_not_incremented",
        name="Score not incremented on brick destroy",
        description="Brick despawned after collision but score custom aggregate is missing.",
        category="event",
        severity="major",
        manifests=manifests,
        expected_detection=True,
        ground_truth_root_cause="score_update",
    )


def entity_wrong_position() -> SeededBug:
    """Entity spawned with (0,0) instead of specified position."""
    manifests = [
        _make_manifest(
            tick=1,
            spawns=[5],
            changes=[ComponentChange(
                entity_id=5,
                component_type_name="position",
                old_value=None,
                new_value={"x": 0.0, "y": 0.0},  # Should be (200, 300).
                changed_by_system=0,
                reason_type="GameRule",
                reason_detail="entity_spawn",
                command_index=0,
                tick=1,
            )],
        ),
    ]
    return SeededBug(
        bug_id="entity_wrong_position",
        name="Entity spawns at wrong position",
        description="Entity position component set to (0,0) instead of the intended spawn coordinates.",
        category="spawn",
        severity="major",
        manifests=manifests,
        expected_detection=True,
        ground_truth_root_cause="spawn_position",
    )


def physics_body_missing() -> SeededBug:
    """Entity alive but position never changes (no physics body)."""
    manifests = [
        _make_manifest(tick=1, spawns=[3]),
        _make_manifest(tick=2),  # No position change for entity 3.
        _make_manifest(tick=3),  # Still no movement.
    ]
    return SeededBug(
        bug_id="physics_body_missing",
        name="Physics body not registered",
        description="Entity is alive but has no position changes across ticks -- physics body was never registered.",
        category="physics",
        severity="critical",
        manifests=manifests,
        expected_detection=True,
        ground_truth_root_cause="physics_registration",
    )


def brick_not_despawned() -> SeededBug:
    """Collision recorded but brick stays alive."""
    manifests = [
        _make_manifest(
            tick=1,
            events=[GameEvent(
                event_type="collision",
                description="ball hit brick",
                involved_entities=[0, 2],
                caused_by_system=0,
                reason_type="CollisionResponse",
                reason_detail="[0,2]",
                tick=1,
            )],
            # despawns is empty -- brick 2 should have been despawned.
        ),
    ]
    return SeededBug(
        bug_id="brick_not_despawned",
        name="Brick not despawned after hit",
        description="Collision event between ball and brick recorded, but brick is not in the despawn list.",
        category="lifecycle",
        severity="major",
        manifests=manifests,
        expected_detection=True,
        ground_truth_root_cause="despawn_logic",
    )


def clean_scenario() -> SeededBug:
    """A correctly working scenario -- should NOT be flagged."""
    manifests = [
        _make_manifest(
            tick=1,
            changes=[
                _pos_change(entity_id=0, tick=1, old_x=400.0, old_y=100.0, new_x=400.0, new_y=50.0),
            ],
            events=[GameEvent(
                event_type="collision",
                description="ball hit paddle",
                involved_entities=[0, 1],
                caused_by_system=0,
                reason_type="CollisionResponse",
                reason_detail="[0,1]",
                tick=1,
            )],
        ),
        _make_manifest(
            tick=2,
            changes=[
                _pos_change(entity_id=0, tick=2, old_x=400.0, old_y=50.0, new_x=400.0, new_y=100.0),
            ],
        ),
    ]
    return SeededBug(
        bug_id="clean_scenario",
        name="Clean scenario (no bug)",
        description="Correctly working ball-paddle collision. Should NOT be flagged.",
        category="none",
        severity="minor",
        manifests=manifests,
        expected_detection=False,
        ground_truth_root_cause="",
    )


# ---------------------------------------------------------------------------
# Full corpus
# ---------------------------------------------------------------------------

def full_corpus() -> list[SeededBug]:
    """Return the complete seeded bug corpus."""
    return [
        ball_passes_through_paddle(),
        score_not_incremented(),
        entity_wrong_position(),
        physics_body_missing(),
        brick_not_despawned(),
        clean_scenario(),
    ]
