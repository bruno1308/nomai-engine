//! Tests for the game-agnostic 2D renderer.
//!
//! These tests focus on headless/GPU-less validation: draw command extraction
//! from ECS world state (both native physics AND dynamic JSON components),
//! color assignment, and camera math. No GPU context is required.

#[cfg(feature = "renderer")]
mod tests {
    use nomai_ecs::identity::{EntityIdentity, Identity, SystemId};
    use nomai_ecs::world::{ComponentBundle, World};
    use nomai_engine::physics::{ColliderShape, PhysicsBody, PhysicsBodyType, Position};
    use nomai_engine::render::{Camera2D, DebugRenderer};

    /// Set up a world with physics component types registered.
    fn setup_world() -> World {
        let mut world = World::new();
        world.register_component::<Position>("physics_position");
        world.register_component::<PhysicsBody>("physics_body");
        world
    }

    /// Set up a world with dynamic JSON component types for a match-3 game.
    fn setup_dynamic_world() -> World {
        let mut world = World::new();
        world.register_dynamic_component::<serde_json::Value>("position");
        world.register_dynamic_component::<serde_json::Value>("size");
        world.register_dynamic_component::<serde_json::Value>("tile_type");
        world
    }

    /// Spawn an entity with Position and PhysicsBody (and optional identity).
    fn spawn_physics_entity(
        world: &mut World,
        pos: Position,
        body: PhysicsBody,
        identity: Option<Identity>,
    ) -> nomai_ecs::entity::EntityId {
        let mut bundle = ComponentBundle::new();
        bundle.add(world.registry(), pos);
        bundle.add(world.registry(), body);
        if let Some(id) = identity {
            bundle.add(world.registry(), id);
        }
        world.spawn_bundle(bundle)
    }

    /// Spawn a dynamic entity with position + size JSON components.
    fn spawn_dynamic_entity(
        world: &mut World,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        identity: Option<Identity>,
        tile_type: Option<serde_json::Value>,
    ) -> nomai_ecs::entity::EntityId {
        let entity = if let Some(id) = identity {
            let bundle = ComponentBundle::new();
            match id {
                Identity::Semantic(eid) => world.spawn_semantic(eid, bundle).unwrap(),
                Identity::Pooled(pid) => world.spawn_pooled(pid, bundle).unwrap(),
            }
        } else {
            // Spawn a bare entity (empty bundle -- no components yet).
            let bundle = ComponentBundle::new();
            world.spawn_bundle(bundle)
        };

        // Set dynamic components by name.
        world
            .set_component_by_name(entity, "position", &serde_json::json!({"x": x, "y": y}))
            .unwrap();
        world
            .set_component_by_name(entity, "size", &serde_json::json!({"w": w, "h": h}))
            .unwrap();
        if let Some(tt) = tile_type {
            world
                .set_component_by_name(entity, "tile_type", &tt)
                .unwrap();
        }
        entity
    }

    // -----------------------------------------------------------------------
    // Phase 1: Native physics entity draw command extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_draw_commands_from_empty_world() {
        let world = setup_world();
        let commands = DebugRenderer::extract_draw_commands(&world);
        assert!(
            commands.is_empty(),
            "empty world should produce no draw commands"
        );
    }

    #[test]
    fn extract_draw_commands_from_physics_entities() {
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 400.0, y: 50.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Kinematic,
                collider: ColliderShape::Box {
                    half_width: 40.0,
                    half_height: 7.5,
                },
                restitution: 1.0,
                is_sensor: false,
            },
            Some(Identity::Semantic(EntityIdentity {
                entity_type: "paddle".to_owned(),
                role: "paddle".to_owned(),
                spawned_by: SystemId::PLAYER_SPAWNER,
                requirement_id: None,
            })),
        );

        spawn_physics_entity(
            &mut world,
            Position { x: 400.0, y: 300.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 5.0 },
                restitution: 1.0,
                is_sensor: false,
            },
            Some(Identity::Semantic(EntityIdentity {
                entity_type: "ball".to_owned(),
                role: "ball".to_owned(),
                spawned_by: SystemId::ENGINE_INTERNAL,
                requirement_id: None,
            })),
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(
            commands.len(),
            2,
            "should have 2 draw commands for 2 physics entities"
        );
    }

    #[test]
    fn extract_draw_commands_correct_positions() {
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 100.0, y: 200.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 5.0 },
                restitution: 1.0,
                is_sensor: false,
            },
            None,
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let cmd = &commands[0];
        assert!(
            (cmd.x - 100.0).abs() < f32::EPSILON,
            "x should be 100.0, got {}",
            cmd.x
        );
        assert!(
            (cmd.y - 200.0).abs() < f32::EPSILON,
            "y should be 200.0, got {}",
            cmd.y
        );
    }

    #[test]
    fn extract_draw_commands_box_collider_size() {
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 0.0, y: 0.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Kinematic,
                collider: ColliderShape::Box {
                    half_width: 40.0,
                    half_height: 7.5,
                },
                restitution: 1.0,
                is_sensor: false,
            },
            None,
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let cmd = &commands[0];
        assert!(
            (cmd.width - 80.0).abs() < f32::EPSILON,
            "width should be 80.0, got {}",
            cmd.width
        );
        assert!(
            (cmd.height - 15.0).abs() < f32::EPSILON,
            "height should be 15.0, got {}",
            cmd.height
        );
    }

    #[test]
    fn extract_draw_commands_circle_collider_size() {
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 0.0, y: 0.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 5.0 },
                restitution: 1.0,
                is_sensor: false,
            },
            None,
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let cmd = &commands[0];
        assert!(
            (cmd.width - 10.0).abs() < f32::EPSILON,
            "width should be 10.0, got {}",
            cmd.width
        );
        assert!(
            (cmd.height - 10.0).abs() < f32::EPSILON,
            "height should be 10.0, got {}",
            cmd.height
        );
    }

    #[test]
    fn physics_entity_without_physics_body_not_rendered() {
        let mut world = setup_world();

        // Entity with only Position, no PhysicsBody.
        let mut bundle = ComponentBundle::new();
        bundle.add(world.registry(), Position { x: 100.0, y: 200.0 });
        world.spawn_bundle(bundle);

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert!(
            commands.is_empty(),
            "entity without PhysicsBody should not produce a draw command"
        );
    }

    // -----------------------------------------------------------------------
    // Phase 2: Dynamic JSON component entity rendering
    // -----------------------------------------------------------------------

    #[test]
    fn extract_draw_commands_from_dynamic_entities() {
        let mut world = setup_dynamic_world();

        // Spawn 4 tile entities with position + size.
        for i in 0..4 {
            let eid = EntityIdentity {
                entity_type: "tile".to_owned(),
                role: "tile".to_owned(),
                spawned_by: SystemId(0),
                requirement_id: None,
            };
            spawn_dynamic_entity(
                &mut world,
                i as f64,
                0.0,
                1.0,
                1.0,
                Some(Identity::Semantic(eid)),
                Some(serde_json::json!({"type_id": i, "name": format!("type_{i}")})),
            );
        }

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(
            commands.len(),
            4,
            "should render 4 dynamic entities with position+size"
        );
    }

    #[test]
    fn dynamic_entity_correct_position_and_size() {
        let mut world = setup_dynamic_world();

        let eid = EntityIdentity {
            entity_type: "block".to_owned(),
            role: "block".to_owned(),
            spawned_by: SystemId(0),
            requirement_id: None,
        };
        spawn_dynamic_entity(
            &mut world,
            3.5,
            7.2,
            2.0,
            1.5,
            Some(Identity::Semantic(eid)),
            None,
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let cmd = &commands[0];
        assert!((cmd.x - 3.5).abs() < 1e-5, "x should be 3.5, got {}", cmd.x);
        assert!((cmd.y - 7.2).abs() < 1e-5, "y should be 7.2, got {}", cmd.y);
        assert!(
            (cmd.width - 2.0).abs() < 1e-5,
            "width should be 2.0, got {}",
            cmd.width
        );
        assert!(
            (cmd.height - 1.5).abs() < 1e-5,
            "height should be 1.5, got {}",
            cmd.height
        );
    }

    #[test]
    fn dynamic_entity_without_size_not_rendered() {
        let mut world = World::new();
        world.register_dynamic_component::<serde_json::Value>("position");

        // Entity with position but no size -- should NOT render.
        let entity = world.spawn_bundle(ComponentBundle::new());
        world
            .set_component_by_name(entity, "position", &serde_json::json!({"x": 1.0, "y": 2.0}))
            .unwrap();

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert!(
            commands.is_empty(),
            "entity with position but no size should not render"
        );
    }

    #[test]
    fn dynamic_entity_without_position_not_rendered() {
        let mut world = World::new();
        world.register_dynamic_component::<serde_json::Value>("size");

        // Entity with size but no position -- should NOT render.
        let entity = world.spawn_bundle(ComponentBundle::new());
        world
            .set_component_by_name(entity, "size", &serde_json::json!({"w": 1.0, "h": 1.0}))
            .unwrap();

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert!(
            commands.is_empty(),
            "entity with size but no position should not render"
        );
    }

    // -----------------------------------------------------------------------
    // Color mapping tests
    // -----------------------------------------------------------------------

    #[test]
    fn entity_with_identity_gets_palette_color() {
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 0.0, y: 0.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 5.0 },
                restitution: 1.0,
                is_sensor: false,
            },
            Some(Identity::Semantic(EntityIdentity {
                entity_type: "player".to_owned(),
                role: "hero".to_owned(),
                spawned_by: SystemId(0),
                requirement_id: None,
            })),
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let c = &commands[0].color;
        // Should NOT be the default magenta [0.8, 0.2, 0.8, 1.0].
        let is_default =
            (c[0] - 0.8).abs() < 0.01 && (c[1] - 0.2).abs() < 0.01 && (c[2] - 0.8).abs() < 0.01;
        assert!(
            !is_default,
            "entity with identity should get a palette color, not default magenta"
        );
        assert!((c[3] - 1.0).abs() < f32::EPSILON, "alpha should be 1.0");
    }

    #[test]
    fn entity_without_identity_gets_default_color() {
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 0.0, y: 0.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 5.0 },
                restitution: 1.0,
                is_sensor: false,
            },
            None,
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let c = &commands[0].color;
        // Should be the default magenta [0.8, 0.2, 0.8, 1.0].
        assert!(
            (c[0] - 0.8).abs() < 0.01 && (c[1] - 0.2).abs() < 0.01 && (c[2] - 0.8).abs() < 0.01,
            "entity without identity should get default magenta, got {:?}",
            c
        );
    }

    #[test]
    fn same_identity_same_color() {
        let mut world = setup_world();

        for _ in 0..2 {
            spawn_physics_entity(
                &mut world,
                Position { x: 0.0, y: 0.0 },
                PhysicsBody {
                    body_type: PhysicsBodyType::Dynamic,
                    collider: ColliderShape::Circle { radius: 5.0 },
                    restitution: 1.0,
                    is_sensor: false,
                },
                Some(Identity::Semantic(EntityIdentity {
                    entity_type: "enemy".to_owned(),
                    role: "goblin".to_owned(),
                    spawned_by: SystemId(0),
                    requirement_id: None,
                })),
            );
        }

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0].color, commands[1].color,
            "entities with same identity should get same color"
        );
    }

    #[test]
    fn different_identity_likely_different_color() {
        let mut world = setup_world();

        let roles = ["warrior", "mage", "archer", "healer"];
        for role in &roles {
            spawn_physics_entity(
                &mut world,
                Position { x: 0.0, y: 0.0 },
                PhysicsBody {
                    body_type: PhysicsBodyType::Dynamic,
                    collider: ColliderShape::Circle { radius: 5.0 },
                    restitution: 1.0,
                    is_sensor: false,
                },
                Some(Identity::Semantic(EntityIdentity {
                    entity_type: role.to_string(),
                    role: role.to_string(),
                    spawned_by: SystemId(0),
                    requirement_id: None,
                })),
            );
        }

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 4);
        let unique_colors: std::collections::HashSet<String> = commands
            .iter()
            .map(|cmd| {
                format!(
                    "{:.3},{:.3},{:.3}",
                    cmd.color[0], cmd.color[1], cmd.color[2]
                )
            })
            .collect();
        assert!(
            unique_colors.len() >= 2,
            "different identities should produce some color variation, got {} unique colors",
            unique_colors.len()
        );
    }

    #[test]
    fn dynamic_entities_with_type_component_get_varied_colors() {
        let mut world = setup_dynamic_world();

        // Spawn tiles with different tile_type values.
        for i in 0..6 {
            let eid = EntityIdentity {
                entity_type: "tile".to_owned(),
                role: "tile".to_owned(),
                spawned_by: SystemId(0),
                requirement_id: None,
            };
            spawn_dynamic_entity(
                &mut world,
                i as f64,
                0.0,
                1.0,
                1.0,
                Some(Identity::Semantic(eid)),
                Some(serde_json::json!({"type_id": i, "name": format!("gem_{i}")})),
            );
        }

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 6);
        let unique_colors: std::collections::HashSet<String> = commands
            .iter()
            .map(|cmd| {
                format!(
                    "{:.3},{:.3},{:.3}",
                    cmd.color[0], cmd.color[1], cmd.color[2]
                )
            })
            .collect();
        assert!(
            unique_colors.len() >= 3,
            "tiles with different type_ids should get varied colors, got {} unique",
            unique_colors.len()
        );
    }

    // -----------------------------------------------------------------------
    // Mixed: physics + dynamic entities in same world
    // -----------------------------------------------------------------------

    #[test]
    fn mixed_physics_and_dynamic_entities() {
        let mut world = World::new();
        world.register_component::<Position>("physics_position");
        world.register_component::<PhysicsBody>("physics_body");
        world.register_dynamic_component::<serde_json::Value>("position");
        world.register_dynamic_component::<serde_json::Value>("size");

        // Spawn a physics entity.
        spawn_physics_entity(
            &mut world,
            Position { x: 400.0, y: 300.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 5.0 },
                restitution: 1.0,
                is_sensor: false,
            },
            Some(Identity::Semantic(EntityIdentity {
                entity_type: "ball".to_owned(),
                role: "ball".to_owned(),
                spawned_by: SystemId(0),
                requirement_id: None,
            })),
        );

        // Spawn a dynamic entity.
        let eid = EntityIdentity {
            entity_type: "tile".to_owned(),
            role: "tile".to_owned(),
            spawned_by: SystemId(0),
            requirement_id: None,
        };
        spawn_dynamic_entity(
            &mut world,
            3.0,
            4.0,
            1.0,
            1.0,
            Some(Identity::Semantic(eid)),
            None,
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(
            commands.len(),
            2,
            "should render both physics and dynamic entities"
        );
    }

    #[test]
    fn physics_entity_not_double_counted() {
        let mut world = World::new();
        world.register_component::<Position>("physics_position");
        world.register_component::<PhysicsBody>("physics_body");
        // Also register dynamic "position" and "size".
        world.register_dynamic_component::<serde_json::Value>("position");
        world.register_dynamic_component::<serde_json::Value>("size");

        // Spawn a physics entity that also gets dynamic position+size.
        let entity = spawn_physics_entity(
            &mut world,
            Position { x: 100.0, y: 200.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 5.0 },
                restitution: 1.0,
                is_sensor: false,
            },
            None,
        );
        world
            .set_component_by_name(
                entity,
                "position",
                &serde_json::json!({"x": 100.0, "y": 200.0}),
            )
            .unwrap();
        world
            .set_component_by_name(entity, "size", &serde_json::json!({"w": 10.0, "h": 10.0}))
            .unwrap();

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(
            commands.len(),
            1,
            "physics entity should not be double-counted even with dynamic position+size"
        );
    }

    // -----------------------------------------------------------------------
    // Camera2D tests
    // -----------------------------------------------------------------------

    #[test]
    fn camera_2d_default() {
        let cam = Camera2D::default();
        assert!((cam.width - 800.0).abs() < f32::EPSILON);
        assert!((cam.height - 600.0).abs() < f32::EPSILON);
        assert!((cam.x - 400.0).abs() < f32::EPSILON);
        assert!((cam.y - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn camera_2d_orthographic_projection_center() {
        let cam = Camera2D {
            width: 800.0,
            height: 600.0,
            x: 400.0,
            y: 300.0,
        };
        let mat = cam.orthographic_matrix();

        let clip_x = mat[0] * 400.0 + mat[12];
        let clip_y = mat[5] * 300.0 + mat[13];
        assert!(
            clip_x.abs() < 1e-5,
            "center X should map to clip 0, got {clip_x}"
        );
        assert!(
            clip_y.abs() < 1e-5,
            "center Y should map to clip 0, got {clip_y}"
        );
    }

    #[test]
    fn camera_2d_orthographic_projection_corners() {
        let cam = Camera2D {
            width: 800.0,
            height: 600.0,
            x: 400.0,
            y: 300.0,
        };
        let mat = cam.orthographic_matrix();

        let clip_x_left = mat[0] * 0.0 + mat[12];
        assert!(
            (clip_x_left - (-1.0)).abs() < 1e-5,
            "left edge should map to clip -1, got {clip_x_left}"
        );

        let clip_x_right = mat[0] * 800.0 + mat[12];
        assert!(
            (clip_x_right - 1.0).abs() < 1e-5,
            "right edge should map to clip 1, got {clip_x_right}"
        );

        let clip_y_bottom = mat[5] * 0.0 + mat[13];
        assert!(
            (clip_y_bottom - (-1.0)).abs() < 1e-5,
            "bottom edge should map to clip -1, got {clip_y_bottom}"
        );

        let clip_y_top = mat[5] * 600.0 + mat[13];
        assert!(
            (clip_y_top - 1.0).abs() < 1e-5,
            "top edge should map to clip 1, got {clip_y_top}"
        );
    }

    #[test]
    fn camera_2d_orthographic_projection_off_center() {
        let cam = Camera2D {
            width: 100.0,
            height: 100.0,
            x: 0.0,
            y: 0.0,
        };
        let mat = cam.orthographic_matrix();

        let clip_x = mat[0] * 0.0 + mat[12];
        let clip_y = mat[5] * 0.0 + mat[13];
        assert!(
            clip_x.abs() < 1e-5,
            "origin X should be clip 0, got {clip_x}"
        );
        assert!(
            clip_y.abs() < 1e-5,
            "origin Y should be clip 0, got {clip_y}"
        );

        let clip_x_edge = mat[0] * 50.0 + mat[12];
        let clip_y_edge = mat[5] * 50.0 + mat[13];
        assert!(
            (clip_x_edge - 1.0).abs() < 1e-5,
            "right edge should be clip 1, got {clip_x_edge}"
        );
        assert!(
            (clip_y_edge - 1.0).abs() < 1e-5,
            "top edge should be clip 1, got {clip_y_edge}"
        );
    }

    #[test]
    fn camera_2d_matrix_shape() {
        let cam = Camera2D::default();
        let mat = cam.orthographic_matrix();

        assert_eq!(mat.len(), 16);
        assert!(
            (mat[15] - 1.0).abs() < 1e-5,
            "mat[15] should be 1.0, got {}",
            mat[15]
        );
        assert!(
            (mat[10] - 1.0).abs() < 1e-5,
            "mat[10] should be 1.0 for z passthrough"
        );
    }

    // -----------------------------------------------------------------------
    // World::get_component_by_name tests
    // -----------------------------------------------------------------------

    #[test]
    fn get_component_by_name_returns_json() {
        let mut world = World::new();
        world.register_dynamic_component::<serde_json::Value>("position");

        let entity = world.spawn_bundle(ComponentBundle::new());
        world
            .set_component_by_name(
                entity,
                "position",
                &serde_json::json!({"x": 42.0, "y": 99.0}),
            )
            .unwrap();

        let val = world.get_component_by_name(entity, "position");
        assert!(val.is_some(), "should find the position component");
        let v = val.unwrap();
        assert_eq!(v["x"], 42.0);
        assert_eq!(v["y"], 99.0);
    }

    #[test]
    fn get_component_by_name_returns_none_for_missing() {
        let mut world = World::new();
        world.register_dynamic_component::<serde_json::Value>("position");
        let entity = world.spawn_bundle(ComponentBundle::new());

        // Entity exists but doesn't have "position".
        assert!(world.get_component_by_name(entity, "position").is_none());
        // Component doesn't exist at all.
        assert!(world.get_component_by_name(entity, "nonexistent").is_none());
    }
}
