//! Week 4-5 Milestone: Physics pipeline produces full causal chains.
//!
//! Validates:
//! 1. Ball-wall collision produces manifest events with CollisionResponse causality
//! 2. Physics position updates flow through command buffer to manifest
//! 3. Physics and user systems coexist in the 8-phase pipeline
//! 4. Physics simulation is deterministic (identical state across runs)
//!
//! WASM integration is tested separately in `nomai-wasm-host` crate tests
//! and the Python milestone test (`test_milestone_week4_5.py`).

use nomai_engine::prelude::*;

// ---------------------------------------------------------------------------
// Shared test component types
// ---------------------------------------------------------------------------

/// A simple counter component used by the user-system coexistence test.
/// Defined at module level so that the system function and the test setup
/// share the same Rust `TypeId`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Counter(u32);

// ---------------------------------------------------------------------------
// Test 1: Physics collision produces manifest events with correct causality
// ---------------------------------------------------------------------------

#[test]
fn physics_collision_appears_in_manifest() {
    // 1. Create world, register position/velocity components using the
    //    physics module's types (f64-based, matching the names used by
    //    `run_physics_step` when emitting commands).
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");

    // 2. Create tick loop at 60 Hz, headless.
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // 3. Create physics world with zero gravity (top-down style).
    let mut physics = PhysicsWorld::new_zero_gravity();

    // Spawn ball (dynamic circle) at origin, moving right at 100 units/sec.
    let ball = tick_loop
        .world_mut()
        .spawn_with(Position { x: 0.0, y: 0.0 });
    physics.register_entity(
        ball,
        &Position { x: 0.0, y: 0.0 },
        &Velocity {
            dx: 100.0,
            dy: 0.0,
        },
        &PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        },
    );

    // Spawn wall (static box) at x=5, tall enough to always catch the ball.
    let wall = tick_loop
        .world_mut()
        .spawn_with(Position { x: 5.0, y: 0.0 });
    physics.register_entity(
        wall,
        &Position { x: 5.0, y: 0.0 },
        &Velocity { dx: 0.0, dy: 0.0 },
        &PhysicsBody {
            body_type: PhysicsBodyType::Static,
            collider: ColliderShape::Box {
                half_width: 0.5,
                half_height: 5.0,
            },
            restitution: 1.0,
            is_sensor: false,
        },
    );

    // Attach physics to tick loop.
    tick_loop.set_physics(physics);

    // 4. Run 30 ticks. At 100 u/s and 60 Hz the ball covers ~1.67 u/tick,
    //    so it reaches the wall (gap of ~4 units after subtracting radii)
    //    within the first 3 ticks. 30 ticks is generous and stays inside
    //    the 60-tick manifest history window.
    for _ in 0..30 {
        tick_loop.tick();
    }

    // 5. Scan manifests for collision events.
    let mut found_collision = false;
    let mut collision_tick = 0u64;

    for tick in 0..30 {
        if let Some(manifest) = tick_loop.manifest_at_tick(tick) {
            for event in &manifest.events {
                if event.event_type == "collision" {
                    found_collision = true;
                    collision_tick = tick;

                    // Collision should involve both entities.
                    assert_eq!(
                        event.involved_entities.len(),
                        2,
                        "collision event should involve exactly 2 entities"
                    );

                    // Should have CollisionResponse causality.
                    assert!(
                        matches!(event.reason, CausalReason::CollisionResponse(_, _)),
                        "collision event should have CollisionResponse reason, got {:?}",
                        event.reason,
                    );

                    // Should be attributed to the physics system.
                    assert_eq!(
                        event.caused_by,
                        SystemId::PHYSICS,
                        "collision event should be caused by the physics system"
                    );

                    // Verify the involved entities are our ball and wall.
                    let ids: Vec<u64> = event
                        .involved_entities
                        .iter()
                        .map(|e| e.to_raw())
                        .collect();
                    assert!(
                        ids.contains(&ball.to_raw()) && ids.contains(&wall.to_raw()),
                        "collision event should involve ball ({:?}) and wall ({:?}), got {:?}",
                        ball,
                        wall,
                        event.involved_entities,
                    );

                    break;
                }
            }
        }
        if found_collision {
            break;
        }
    }

    assert!(
        found_collision,
        "should have at least one collision event in 30 ticks"
    );

    // Ball at 100 u/s, wall at x=5.0, ball radius 0.5, wall half_width 0.5
    // = contact at ~4.0 units of travel.
    // At 100 u/s and 60 Hz: 4.0 / (100/60) ~= 2.4 ticks.
    // Allow up to 5 ticks for rapier sub-stepping margin.
    assert!(
        collision_tick < 5,
        "collision should happen within first 5 ticks, happened at tick {collision_tick}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Physics position updates appear as component changes in manifest
// ---------------------------------------------------------------------------

#[test]
fn physics_updates_position_in_manifest() {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    let mut physics = PhysicsWorld::new_zero_gravity();

    // Single dynamic body moving right -- no obstacles, no collisions.
    let entity = tick_loop
        .world_mut()
        .spawn_with(Position { x: 0.0, y: 0.0 });
    physics.register_entity(
        entity,
        &Position { x: 0.0, y: 0.0 },
        &Velocity {
            dx: 10.0,
            dy: 0.0,
        },
        &PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 0.5,
            is_sensor: false,
        },
    );

    tick_loop.set_physics(physics);

    // Run 1 tick.
    tick_loop.tick();

    let manifest = tick_loop
        .last_manifest()
        .expect("should have manifest after 1 tick");

    // Manifest should record position component changes from physics.
    let position_changes: Vec<_> = manifest
        .component_changes
        .iter()
        .filter(|c| c.entity_id == entity && c.component_type_name == "position")
        .collect();

    assert!(
        !position_changes.is_empty(),
        "physics should produce position component changes in manifest"
    );

    // Position change should come from the physics system with correct causality.
    for change in &position_changes {
        assert_eq!(
            change.changed_by,
            SystemId::PHYSICS,
            "position change should be issued by physics system"
        );
        assert!(
            matches!(change.reason, CausalReason::SystemInternal(_)),
            "physics position update should have SystemInternal reason, got {:?}",
            change.reason,
        );
    }

    // Manifest should also record velocity component changes.
    let velocity_changes: Vec<_> = manifest
        .component_changes
        .iter()
        .filter(|c| c.entity_id == entity && c.component_type_name == "velocity")
        .collect();

    assert!(
        !velocity_changes.is_empty(),
        "physics should produce velocity component changes in manifest"
    );

    // Velocity changes should also come from the physics system.
    for change in &velocity_changes {
        assert_eq!(
            change.changed_by,
            SystemId::PHYSICS,
            "velocity change should be issued by physics system"
        );
        assert!(
            matches!(change.reason, CausalReason::SystemInternal(_)),
            "physics velocity update should have SystemInternal reason, got {:?}",
            change.reason,
        );
    }

    // Verify physics system is in the list of executed systems.
    assert!(
        manifest
            .systems_executed
            .contains(&PHYSICS_SYSTEM_NAME.to_owned()),
        "physics system should appear in systems_executed: {:?}",
        manifest.systems_executed,
    );

    // After 1 tick at 10 u/s and 60 Hz, the body should have moved ~0.167 units.
    let pos = tick_loop
        .world()
        .get_component::<Position>(entity)
        .expect("entity should still exist");
    assert!(
        pos.x > 0.0,
        "position x should have increased from physics step, got {}",
        pos.x
    );
}

// ---------------------------------------------------------------------------
// Test 3: Physics and user systems coexist in the same tick
// ---------------------------------------------------------------------------

/// The counter system function. Uses the module-level `Counter` type so that
/// the ECS query resolves to the same component type as the test setup.
fn counter_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (val,)) in world.query::<(&Counter,)>() {
        cmds.set_component(
            entity,
            "counter",
            serde_json::json!(val.0 + 1),
            SystemId(1),
            CausalReason::SystemInternal("counter_increment".to_owned()),
        );
    }
}

#[test]
fn physics_and_user_systems_coexist() {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<Counter>("counter");

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Spawn a counter entity.
    let counter_entity = tick_loop.world_mut().spawn_with(Counter(0));

    // Register the counter system.
    tick_loop.add_system("counter", counter_system);

    // Add physics with a moving ball (no obstacles).
    let mut physics = PhysicsWorld::new_zero_gravity();
    let ball = tick_loop
        .world_mut()
        .spawn_with(Position { x: 0.0, y: 0.0 });
    physics.register_entity(
        ball,
        &Position { x: 0.0, y: 0.0 },
        &Velocity { dx: 5.0, dy: 0.0 },
        &PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 0.5,
            is_sensor: false,
        },
    );
    tick_loop.set_physics(physics);

    // Run 10 ticks.
    tick_loop.run_ticks(10);

    // Verify the counter was incremented 10 times by the user system.
    let counter_val = tick_loop
        .world()
        .get_component::<Counter>(counter_entity)
        .expect("counter entity should exist");
    assert_eq!(
        counter_val.0, 10,
        "user system counter should be 10 after 10 ticks, got {}",
        counter_val.0
    );

    // Get last manifest and check both systems are listed.
    let manifest = tick_loop.last_manifest().expect("should have manifest");

    assert!(
        manifest
            .systems_executed
            .contains(&"counter".to_owned()),
        "user system 'counter' should be in manifest systems_executed: {:?}",
        manifest.systems_executed,
    );
    assert!(
        manifest
            .systems_executed
            .contains(&PHYSICS_SYSTEM_NAME.to_owned()),
        "physics should be in manifest systems_executed: {:?}",
        manifest.systems_executed,
    );

    // Verify user system produced counter component changes in last tick.
    let counter_changes: Vec<_> = manifest
        .component_changes
        .iter()
        .filter(|c| c.entity_id == counter_entity && c.component_type_name == "counter")
        .collect();
    assert!(
        !counter_changes.is_empty(),
        "counter system should produce counter component changes in manifest"
    );
    // Counter changes should have SystemInternal causality from user system.
    for change in &counter_changes {
        assert!(
            matches!(&change.reason, CausalReason::SystemInternal(s) if s == "counter_increment"),
            "counter change should have SystemInternal(\"counter_increment\") reason, got {:?}",
            change.reason,
        );
    }

    // Verify physics produced position changes in last tick.
    let position_changes: Vec<_> = manifest
        .component_changes
        .iter()
        .filter(|c| c.entity_id == ball && c.component_type_name == "position")
        .collect();
    assert!(
        !position_changes.is_empty(),
        "physics should produce position component changes in manifest"
    );

    // Verify the ball actually moved (physics is working alongside user systems).
    let ball_pos = tick_loop
        .world()
        .get_component::<Position>(ball)
        .expect("ball should exist");
    assert!(
        ball_pos.x > 0.0,
        "ball should have moved right via physics, got x={}",
        ball_pos.x,
    );
}

// ---------------------------------------------------------------------------
// Test 4: Collision events carry correct CausalReason::CollisionResponse
// ---------------------------------------------------------------------------

#[test]
fn collision_event_carries_collision_response_causality() {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);
    let mut physics = PhysicsWorld::new_zero_gravity();

    // Ball moving toward wall at high speed for fast collision.
    let ball = tick_loop
        .world_mut()
        .spawn_with(Position { x: 0.0, y: 0.0 });
    physics.register_entity(
        ball,
        &Position { x: 0.0, y: 0.0 },
        &Velocity {
            dx: 200.0,
            dy: 0.0,
        },
        &PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        },
    );

    let wall = tick_loop
        .world_mut()
        .spawn_with(Position { x: 3.0, y: 0.0 });
    physics.register_entity(
        wall,
        &Position { x: 3.0, y: 0.0 },
        &Velocity { dx: 0.0, dy: 0.0 },
        &PhysicsBody {
            body_type: PhysicsBodyType::Static,
            collider: ColliderShape::Box {
                half_width: 0.5,
                half_height: 5.0,
            },
            restitution: 1.0,
            is_sensor: false,
        },
    );

    tick_loop.set_physics(physics);

    // Run 10 ticks.
    for _ in 0..10 {
        tick_loop.tick();
    }

    // Collect all collision events across the history.
    let mut collision_events: Vec<GameEvent> = Vec::new();
    for tick in 0..10 {
        if let Some(manifest) = tick_loop.manifest_at_tick(tick) {
            for event in &manifest.events {
                if event.event_type == "collision" {
                    collision_events.push(event.clone());
                }
            }
        }
    }

    assert!(
        !collision_events.is_empty(),
        "should have at least one collision event"
    );

    // Verify all collision events have correct structure.
    for event in &collision_events {
        // CausalReason should be CollisionResponse with the correct entity pair.
        match &event.reason {
            CausalReason::CollisionResponse(a, b) => {
                let pair = [a.to_raw(), b.to_raw()];
                assert!(
                    pair.contains(&ball.to_raw()) && pair.contains(&wall.to_raw()),
                    "CollisionResponse should contain ball and wall entity IDs"
                );
            }
            other => {
                panic!(
                    "collision event should have CollisionResponse reason, got {:?}",
                    other
                );
            }
        }

        // caused_by should be physics system.
        assert_eq!(event.caused_by, SystemId::PHYSICS);

        // event_type should be "collision".
        assert_eq!(event.event_type, "collision");
    }
}

// ---------------------------------------------------------------------------
// Test 5: Determinism -- two identical physics runs produce identical manifests
// ---------------------------------------------------------------------------

#[test]
fn physics_determinism_identical_runs() {
    /// Result from a deterministic simulation run: final positions/velocities
    /// plus per-tick manifest summaries.
    #[derive(Debug, PartialEq)]
    struct SimResult {
        ball_pos: (f64, f64),
        ball_vel: (f64, f64),
        tick_count: u64,
        manifest_summaries: Vec<(u64, usize, usize)>,
    }

    fn run_physics_simulation() -> SimResult {
        let mut world = World::new();
        world.register_component::<Position>("position");
        world.register_component::<Velocity>("velocity");

        let config = TickConfig {
            fixed_dt: 1.0 / 60.0,
            headless: true,
        };
        let mut tick_loop = TickLoop::new(world, config);
        let mut physics = PhysicsWorld::new_zero_gravity();

        let ball = tick_loop
            .world_mut()
            .spawn_with(Position { x: 0.0, y: 0.0 });
        physics.register_entity(
            ball,
            &Position { x: 0.0, y: 0.0 },
            &Velocity {
                dx: 50.0,
                dy: 10.0,
            },
            &PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        let wall = tick_loop
            .world_mut()
            .spawn_with(Position { x: 5.0, y: 0.0 });
        physics.register_entity(
            wall,
            &Position { x: 5.0, y: 0.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 0.5,
                    half_height: 10.0,
                },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        tick_loop.set_physics(physics);

        // Run 50 ticks.
        for _ in 0..50 {
            tick_loop.tick();
        }

        // Read final ball state from the world.
        let final_pos = tick_loop
            .world()
            .get_component::<Position>(ball)
            .expect("ball should exist")
            .clone();
        let final_vel = tick_loop
            .world()
            .get_component::<Velocity>(ball)
            .expect("ball should exist")
            .clone();

        // Collect per-tick summary: (tick, num_events, num_component_changes).
        let mut summaries = Vec::new();
        for tick in 0..50 {
            if let Some(manifest) = tick_loop.manifest_at_tick(tick) {
                summaries.push((tick, manifest.events.len(), manifest.component_changes.len()));
            }
        }

        SimResult {
            ball_pos: (final_pos.x, final_pos.y),
            ball_vel: (final_vel.dx, final_vel.dy),
            tick_count: tick_loop.tick_count(),
            manifest_summaries: summaries,
        }
    }

    let run1 = run_physics_simulation();
    let run2 = run_physics_simulation();

    // Compare actual world state -- not just counts.
    assert_eq!(
        run1.ball_pos, run2.ball_pos,
        "ball final position diverged: run1={:?}, run2={:?}",
        run1.ball_pos, run2.ball_pos,
    );
    assert_eq!(
        run1.ball_vel, run2.ball_vel,
        "ball final velocity diverged: run1={:?}, run2={:?}",
        run1.ball_vel, run2.ball_vel,
    );
    assert_eq!(
        run1.tick_count, run2.tick_count,
        "tick count diverged"
    );

    // Also compare manifest summaries.
    assert_eq!(
        run1.manifest_summaries.len(),
        run2.manifest_summaries.len(),
        "both runs should produce the same number of manifests"
    );
    for (i, (s1, s2)) in run1.manifest_summaries.iter().zip(run2.manifest_summaries.iter()).enumerate() {
        assert_eq!(
            s1, s2,
            "manifest summary diverged at index {i}: run1={s1:?}, run2={s2:?}"
        );
    }
}
