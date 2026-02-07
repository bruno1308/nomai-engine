//! Tests for the debug 2D renderer.
//!
//! These tests focus on headless/GPU-less validation: draw command extraction
//! from ECS world state and camera matrix math. No GPU context is required.

#[cfg(feature = "renderer")]
mod tests {
    use nomai_ecs::identity::{EntityIdentity, Identity, PoolIdentity, SystemId};
    use nomai_ecs::world::{ComponentBundle, World};
    use nomai_engine::physics::{
        ColliderShape, PhysicsBody, PhysicsBodyType, Position, Velocity,
    };
    use nomai_engine::render::{Camera2D, DebugRenderer};

    /// Set up a world with physics component types registered.
    fn setup_world() -> World {
        let mut world = World::new();
        world.register_component::<Position>("position");
        world.register_component::<Velocity>("velocity");
        world.register_component::<PhysicsBody>("physics_body");
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

    // -----------------------------------------------------------------------
    // extract_draw_commands tests
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
    fn extract_draw_commands_from_world_with_entities() {
        let mut world = setup_world();

        // Spawn a paddle (semantic entity with role "paddle").
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

        // Spawn a ball (semantic entity with role "ball").
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

        // Spawn a wall (semantic entity with role "wall").
        spawn_physics_entity(
            &mut world,
            Position { x: 0.0, y: 300.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 5.0,
                    half_height: 300.0,
                },
                restitution: 1.0,
                is_sensor: false,
            },
            Some(Identity::Semantic(EntityIdentity {
                entity_type: "wall".to_owned(),
                role: "wall_left".to_owned(),
                spawned_by: SystemId::ENGINE_INTERNAL,
                requirement_id: None,
            })),
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(
            commands.len(),
            3,
            "should have 3 draw commands for 3 entities"
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
        // Box with half_width=40 -> full width=80.
        assert!(
            (cmd.width - 80.0).abs() < f32::EPSILON,
            "width should be 80.0, got {}",
            cmd.width
        );
        // Box with half_height=7.5 -> full height=15.
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
        // Circle with radius=5 -> diameter=10 for both width and height.
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
    fn extract_draw_commands_paddle_color_blue() {
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

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let cmd = &commands[0];
        // Paddle should be blue: approximately [0.267, 0.533, 1.0, 1.0].
        assert!(
            cmd.color[2] > 0.9,
            "paddle blue channel should be high, got {}",
            cmd.color[2]
        );
        assert!(
            (cmd.color[3] - 1.0).abs() < f32::EPSILON,
            "alpha should be 1.0"
        );
    }

    #[test]
    fn extract_draw_commands_ball_color_white() {
        let mut world = setup_world();

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
        assert_eq!(commands.len(), 1);
        let cmd = &commands[0];
        // Ball should be white: [1.0, 1.0, 1.0, 1.0].
        assert!(
            (cmd.color[0] - 1.0).abs() < f32::EPSILON,
            "ball R should be 1.0, got {}",
            cmd.color[0]
        );
        assert!(
            (cmd.color[1] - 1.0).abs() < f32::EPSILON,
            "ball G should be 1.0, got {}",
            cmd.color[1]
        );
        assert!(
            (cmd.color[2] - 1.0).abs() < f32::EPSILON,
            "ball B should be 1.0, got {}",
            cmd.color[2]
        );
    }

    #[test]
    fn extract_draw_commands_wall_color_gray() {
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 0.0, y: 300.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 5.0,
                    half_height: 300.0,
                },
                restitution: 1.0,
                is_sensor: false,
            },
            Some(Identity::Semantic(EntityIdentity {
                entity_type: "wall".to_owned(),
                role: "wall_left".to_owned(),
                spawned_by: SystemId::ENGINE_INTERNAL,
                requirement_id: None,
            })),
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        let cmd = &commands[0];
        // Wall should be gray: approximately [0.533, 0.533, 0.533, 1.0].
        assert!(
            (cmd.color[0] - cmd.color[1]).abs() < 0.01,
            "wall R and G should be equal (gray)"
        );
        assert!(
            (cmd.color[1] - cmd.color[2]).abs() < 0.01,
            "wall G and B should be equal (gray)"
        );
        assert!(
            cmd.color[0] > 0.4 && cmd.color[0] < 0.7,
            "wall gray value should be mid-range, got {}",
            cmd.color[0]
        );
    }

    #[test]
    fn extract_draw_commands_brick_color_varies() {
        let mut world = setup_world();

        // Spawn bricks at different Y positions using pooled identity.
        for i in 0..6 {
            let y = i as f64 * 25.0;
            spawn_physics_entity(
                &mut world,
                Position { x: 100.0, y },
                PhysicsBody {
                    body_type: PhysicsBodyType::Static,
                    collider: ColliderShape::Box {
                        half_width: 30.0,
                        half_height: 10.0,
                    },
                    restitution: 0.5,
                    is_sensor: false,
                },
                Some(Identity::Pooled(PoolIdentity {
                    pool_type: "destructible".to_owned(),
                    variant: "brick".to_owned(),
                })),
            );
        }

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 6, "should have 6 brick draw commands");

        // Verify that not all colors are the same (row variation).
        let unique_colors: std::collections::HashSet<String> = commands
            .iter()
            .map(|cmd| format!("{:.2},{:.2},{:.2}", cmd.color[0], cmd.color[1], cmd.color[2]))
            .collect();
        assert!(
            unique_colors.len() > 1,
            "brick colors should vary by row, but all are the same"
        );
    }

    #[test]
    fn extract_draw_commands_entity_without_identity_gets_default() {
        let mut world = setup_world();

        // Spawn entity with physics but no identity.
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
        // Default color is magenta-ish: [0.8, 0.2, 0.8, 1.0].
        assert!(
            (cmd.color[3] - 1.0).abs() < f32::EPSILON,
            "alpha should be 1.0"
        );
    }

    #[test]
    fn extract_draw_commands_skips_entities_without_physics() {
        let mut world = setup_world();

        // Spawn entity with only Position, no PhysicsBody.
        let mut bundle = ComponentBundle::new();
        bundle.add(world.registry(), Position { x: 100.0, y: 200.0 });
        world.spawn_bundle(bundle);

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert!(
            commands.is_empty(),
            "entity without PhysicsBody should not produce a draw command"
        );
    }

    #[test]
    fn extract_draw_commands_multiple_entity_types() {
        let mut world = setup_world();

        // Paddle.
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

        // Ball.
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

        // Bricks.
        for i in 0..5 {
            spawn_physics_entity(
                &mut world,
                Position {
                    x: 100.0 + i as f64 * 70.0,
                    y: 500.0,
                },
                PhysicsBody {
                    body_type: PhysicsBodyType::Static,
                    collider: ColliderShape::Box {
                        half_width: 30.0,
                        half_height: 10.0,
                    },
                    restitution: 0.5,
                    is_sensor: false,
                },
                Some(Identity::Pooled(PoolIdentity {
                    pool_type: "destructible".to_owned(),
                    variant: "brick".to_owned(),
                })),
            );
        }

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(
            commands.len(),
            7,
            "should have 1 paddle + 1 ball + 5 bricks = 7"
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
        // Camera centered at (400, 300) with 800x600 viewport.
        let cam = Camera2D {
            width: 800.0,
            height: 600.0,
            x: 400.0,
            y: 300.0,
        };
        let mat = cam.orthographic_matrix();

        // The center of the camera (400, 300) should map to clip (0, 0).
        // clip_x = mat[0]*x + mat[12] (column-major)
        // clip_y = mat[5]*y + mat[13]
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

        // Left edge (x=0): should map to clip_x = -1.
        let clip_x_left = mat[0] * 0.0 + mat[12];
        assert!(
            (clip_x_left - (-1.0)).abs() < 1e-5,
            "left edge should map to clip -1, got {clip_x_left}"
        );

        // Right edge (x=800): should map to clip_x = 1.
        let clip_x_right = mat[0] * 800.0 + mat[12];
        assert!(
            (clip_x_right - 1.0).abs() < 1e-5,
            "right edge should map to clip 1, got {clip_x_right}"
        );

        // Bottom edge (y=0): should map to clip_y = -1.
        let clip_y_bottom = mat[5] * 0.0 + mat[13];
        assert!(
            (clip_y_bottom - (-1.0)).abs() < 1e-5,
            "bottom edge should map to clip -1, got {clip_y_bottom}"
        );

        // Top edge (y=600): should map to clip_y = 1.
        let clip_y_top = mat[5] * 600.0 + mat[13];
        assert!(
            (clip_y_top - 1.0).abs() < 1e-5,
            "top edge should map to clip 1, got {clip_y_top}"
        );
    }

    #[test]
    fn camera_2d_orthographic_projection_off_center() {
        // Camera centered at (0, 0) -- left/bottom will be negative world coords.
        let cam = Camera2D {
            width: 100.0,
            height: 100.0,
            x: 0.0,
            y: 0.0,
        };
        let mat = cam.orthographic_matrix();

        // World origin (0, 0) is at camera center -> clip (0, 0).
        let clip_x = mat[0] * 0.0 + mat[12];
        let clip_y = mat[5] * 0.0 + mat[13];
        assert!(clip_x.abs() < 1e-5, "origin X should be clip 0, got {clip_x}");
        assert!(clip_y.abs() < 1e-5, "origin Y should be clip 0, got {clip_y}");

        // World (50, 50) is at right-top edge -> clip (1, 1).
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

        // Should be 16 elements (4x4 matrix).
        assert_eq!(mat.len(), 16);

        // Bottom-right element (column 3, row 3) should be 1.0 for orthographic.
        assert!(
            (mat[15] - 1.0).abs() < 1e-5,
            "mat[15] should be 1.0, got {}",
            mat[15]
        );

        // z-column (column 2) should be identity-like for 2D.
        assert!(
            (mat[10] - 1.0).abs() < 1e-5,
            "mat[10] should be 1.0 for z passthrough"
        );
    }

    // -----------------------------------------------------------------------
    // Edge-case color mapping tests (entity_type fallback + variant)
    // -----------------------------------------------------------------------

    #[test]
    fn extract_draw_commands_semantic_entity_type_fallback() {
        // Semantic entity where the role doesn't match but entity_type does.
        let mut world = setup_world();

        // Entity with entity_type "paddle" but role "main_player_paddle_unit".
        // The role check uses contains("paddle") so this still matches.
        // But test the entity_type fallback by using a role that doesn't match.
        spawn_physics_entity(
            &mut world,
            Position { x: 10.0, y: 20.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 5.0,
                    half_height: 5.0,
                },
                restitution: 0.5,
                is_sensor: false,
            },
            Some(Identity::Semantic(EntityIdentity {
                entity_type: "paddle".to_owned(),
                role: "player_input_receiver".to_owned(), // role doesn't contain "paddle"
                spawned_by: SystemId::ENGINE_INTERNAL,
                requirement_id: None,
            })),
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        // Should get paddle color via entity_type fallback.
        let c = &commands[0];
        assert!(
            c.color[2] > 0.8,
            "entity_type 'paddle' should get blue (paddle color) via fallback, got {:?}",
            c.color
        );
    }

    #[test]
    fn extract_draw_commands_pooled_variant_brick() {
        // Pooled entity where pool_type doesn't match but variant is "brick".
        let mut world = setup_world();

        spawn_physics_entity(
            &mut world,
            Position { x: 50.0, y: 100.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 30.0,
                    half_height: 10.0,
                },
                restitution: 0.5,
                is_sensor: false,
            },
            Some(Identity::Pooled(PoolIdentity {
                pool_type: "obstacle".to_owned(), // doesn't match brick/destructible
                variant: "brick".to_owned(),       // but variant says brick
            })),
        );

        let commands = DebugRenderer::extract_draw_commands(&world);
        assert_eq!(commands.len(), 1);
        // Should get a brick color (one of the BRICK_COLORS).
        let c = &commands[0];
        // Brick colors all have alpha 1.0 and are not the default magenta.
        assert!(
            c.color[3] > 0.99,
            "brick should have alpha 1.0, got {:?}",
            c.color
        );
        // Should not be the default magenta color.
        assert!(
            !(c.color[0] > 0.9 && c.color[1] < 0.1 && c.color[2] > 0.9),
            "pooled variant 'brick' should not get default magenta, got {:?}",
            c.color
        );
    }
}
