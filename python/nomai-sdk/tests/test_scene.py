"""Tests for SceneSnapshot types and serialization."""

from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


class TestSceneEntity:
    def test_creation(self):
        e = SceneEntity(
            entity_id=1, entity_type="character", role="paddle",
            tier="Semantic", position=(400.0, 500.0), size=(100.0, 15.0),
            velocity=None, visible=True, z_index=0.0,
        )
        assert e.role == "paddle"
        assert e.position == (400.0, 500.0)

    def test_serialization_round_trip(self):
        e = SceneEntity(
            entity_id=1, entity_type="projectile", role="ball",
            tier="Semantic", position=(200.0, 300.0), size=(16.0, 16.0),
            velocity=(100.0, -150.0), visible=True, z_index=1.0,
            components={"score": 10},
        )
        d = e.to_dict()
        e2 = SceneEntity.from_dict(d)
        assert e2.entity_id == e.entity_id
        assert e2.velocity == (100.0, -150.0)
        assert e2.components == {"score": 10}


class TestSceneBounds:
    def test_round_trip(self):
        b = SceneBounds(min_x=-10.0, min_y=0.0, max_x=810.0, max_y=600.0)
        b2 = SceneBounds.from_dict(b.to_dict())
        assert b2 == b


class TestSceneSnapshot:
    def test_creation_and_summary(self):
        snap = SceneSnapshot(
            schema_version=1, tick=42, sim_time=0.7,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0),
                    size=(100.0, 15.0), velocity=None,
                    visible=True, z_index=0.0,
                ),
                SceneEntity(
                    entity_id=2, entity_type="projectile", role="ball",
                    tier="Semantic", position=(200.0, 300.0),
                    size=(16.0, 16.0), velocity=(200.0, -300.0),
                    visible=True, z_index=1.0,
                ),
            ],
            bounds=SceneBounds(min_x=150.0, min_y=292.0, max_x=450.0, max_y=567.5),
            entity_count=2,
        )
        assert snap.entity_count == 2
        assert snap.entity_by_role("paddle") is not None
        assert snap.entity_by_role("ball").velocity == (200.0, -300.0)
        assert len(snap.entities_by_type("character")) == 1
        summary = snap.summary()
        assert "tick 42" in summary
        assert "paddle" in summary

    def test_json_round_trip(self):
        snap = SceneSnapshot(
            schema_version=1, tick=10, sim_time=0.166,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0),
                    size=(100.0, 15.0), velocity=None,
                    visible=True, z_index=0.0,
                ),
            ],
            bounds=SceneBounds(min_x=350.0, min_y=552.5, max_x=450.0, max_y=567.5),
            entity_count=1,
        )
        json_str = snap.to_json()
        import json
        data = json.loads(json_str)
        snap2 = SceneSnapshot.from_dict(data)
        assert snap2.tick == snap.tick
        assert snap2.entities[0].role == "paddle"
        assert snap2.schema_version == 1
