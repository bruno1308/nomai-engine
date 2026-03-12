use nomai_manifest::scene::{SceneBounds, SceneEntity, SceneSnapshot};
use std::collections::HashMap;

#[test]
fn scene_snapshot_serialization_roundtrip() {
    let snapshot = SceneSnapshot {
        schema_version: 1,
        tick: 42,
        sim_time: 0.7,
        entities: vec![
            SceneEntity {
                entity_id: 1,
                entity_type: "character".into(),
                role: "paddle".into(),
                tier: "Semantic".into(),
                position: Some([400.0, 560.0]),
                size: Some([100.0, 15.0]),
                velocity: None,
                visible: true,
                z_index: 0.0,
                components: HashMap::new(),
            },
        ],
        bounds: SceneBounds {
            min_x: 350.0, min_y: 552.5,
            max_x: 450.0, max_y: 567.5,
        },
        entity_count: 1,
    };

    let json = serde_json::to_string(&snapshot).unwrap();
    let deserialized: SceneSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.tick, 42);
    assert_eq!(deserialized.entities.len(), 1);
    assert_eq!(deserialized.entities[0].role, "paddle");
    assert_eq!(deserialized.schema_version, 1);
}

#[test]
fn scene_entity_with_all_fields() {
    let mut comps = HashMap::new();
    comps.insert("score".into(), serde_json::json!(10));

    let entity = SceneEntity {
        entity_id: 2,
        entity_type: "projectile".into(),
        role: "ball".into(),
        tier: "Semantic".into(),
        position: Some([200.0, 300.0]),
        size: Some([16.0, 16.0]),
        velocity: Some([100.0, -150.0]),
        visible: true,
        z_index: 1.0,
        components: comps,
    };

    let json = serde_json::to_string(&entity).unwrap();
    let de: SceneEntity = serde_json::from_str(&json).unwrap();
    assert_eq!(de.velocity, Some([100.0, -150.0]));
    assert_eq!(de.components["score"], 10);
}
