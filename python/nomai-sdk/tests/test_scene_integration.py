"""Integration test: scene snapshot from live engine."""

import pytest
from nomai.engine import NomaiEngine
from nomai.scene import SceneSnapshot


@pytest.fixture
def breakout_engine():
    """Create a minimal breakout engine with paddle and ball."""
    engine = NomaiEngine(headless=True, fixed_dt=1.0 / 60.0)
    engine.register_component("position")
    engine.register_component("velocity")
    engine.register_component("size")
    engine.init_physics()

    engine.spawn_entity("character", "paddle", {
        "position": {"x": 400.0, "y": 560.0},
        "size": {"w": 100.0, "h": 15.0},
    })
    engine.spawn_entity("projectile", "ball", {
        "position": {"x": 400.0, "y": 300.0},
        "velocity": {"dx": 200.0, "dy": -300.0},
    })
    engine.tick()  # apply spawns
    return engine


class TestSceneSnapshotIntegration:
    def test_snapshot_has_entities(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        assert isinstance(snap, SceneSnapshot)
        assert snap.entity_count >= 2
        assert snap.schema_version == 1

    def test_paddle_in_snapshot(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        paddle = snap.entity_by_role("paddle")
        assert paddle is not None
        assert paddle.entity_type == "character"
        assert paddle.position is not None
        assert abs(paddle.position[0] - 400.0) < 1.0

    def test_ball_has_velocity(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        ball = snap.entity_by_role("ball")
        assert ball is not None
        assert ball.velocity is not None

    def test_snapshot_advances_with_tick(self, breakout_engine):
        snap1 = breakout_engine.scene_snapshot()
        breakout_engine.tick()
        snap2 = breakout_engine.scene_snapshot()
        assert snap2.tick == snap1.tick + 1

    def test_snapshot_deterministic(self, breakout_engine):
        """Same state produces identical snapshots."""
        snap1 = breakout_engine.scene_snapshot()
        snap2 = breakout_engine.scene_snapshot()
        # Compare as dicts (order-independent) since HashMap iteration
        # order is non-deterministic across calls.
        assert snap1.to_dict() == snap2.to_dict()

    def test_summary_readable(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        summary = snap.summary()
        assert "paddle" in summary
        assert "ball" in summary
