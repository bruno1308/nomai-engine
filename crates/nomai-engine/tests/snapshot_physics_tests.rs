//! Snapshot + physics reconstruction tests.
//!
//! These tests validate that after snapshot restore, the rapier2d physics
//! world is correctly rebuilt from ECS component data, producing
//! deterministic behavior identical to the original simulation run.

use nomai_ecs::prelude::*;
use nomai_engine::prelude::*;
use nomai_engine::tick::{TickConfig, TickLoop};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a world with physics component types registered.
fn setup_world() -> World {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<PhysicsBody>("physics_body");
    world
}

/// Build a tick loop with a physics world and a ball + wall scenario.
///
/// Returns `(tick_loop, ball_entity, wall_entity)`.
fn build_physics_tick_loop() -> (TickLoop, EntityId, EntityId) {
    let mut world = setup_world();

    // Ball (dynamic circle) at origin, moving right.
    let mut ball_bundle = ComponentBundle::new();
    ball_bundle.add(world.registry(), Position { x: 0.0, y: 0.0 });
    ball_bundle.add(world.registry(), Velocity { dx: 50.0, dy: 0.0 });
    ball_bundle.add(
        world.registry(),
        PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        },
    );
    let ball = world.spawn_bundle(ball_bundle);

    // Wall (static box) at x=10.
    let mut wall_bundle = ComponentBundle::new();
    wall_bundle.add(world.registry(), Position { x: 10.0, y: 0.0 });
    wall_bundle.add(world.registry(), Velocity { dx: 0.0, dy: 0.0 });
    wall_bundle.add(
        world.registry(),
        PhysicsBody {
            body_type: PhysicsBodyType::Static,
            collider: ColliderShape::Box {
                half_width: 0.5,
                half_height: 5.0,
            },
            restitution: 1.0,
            is_sensor: false,
        },
    );
    let wall = world.spawn_bundle(wall_bundle);

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Create and attach physics world.
    let mut physics = PhysicsWorld::new_zero_gravity();
    physics.register_entity(
        ball,
        &Position { x: 0.0, y: 0.0 },
        &Velocity { dx: 50.0, dy: 0.0 },
        &PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        },
    );
    physics.register_entity(
        wall,
        &Position { x: 10.0, y: 0.0 },
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

    (tick_loop, ball, wall)
}

// ---------------------------------------------------------------------------
// Test 1: Snapshot restore preserves physics behavior (determinism)
// ---------------------------------------------------------------------------

/// Run 50 ticks, snapshot, run 50 more and record hash A.
/// Restore snapshot, run 50 more and record hash B.
/// Hash A must equal hash B, proving physics reconstruction is deterministic.
#[test]
fn snapshot_restore_preserves_physics_behavior() {
    let (mut tick_loop, _ball, _wall) = build_physics_tick_loop();

    // Run 50 ticks to let the ball travel and potentially bounce.
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 50);

    // Capture snapshot at tick 50.
    let snapshot = tick_loop.capture_snapshot();
    assert_eq!(snapshot.tick_counter, 50);

    // Run 50 more ticks (tick 50..99) and capture final state hash.
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 100);
    let hash_a = tick_loop.state_hash();

    // Restore to tick 50. This triggers physics reconstruction internally.
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    assert_eq!(tick_loop.tick_count(), 50);

    // Verify physics world was reconstructed.
    assert!(
        tick_loop.physics().is_some(),
        "physics should still be attached after restore"
    );
    assert!(
        tick_loop.physics().unwrap().body_count() > 0,
        "physics should have bodies after reconstruction"
    );

    // Run 50 more ticks again (tick 50..99) and capture final state hash.
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 100);
    let hash_b = tick_loop.state_hash();

    // Determinism: same starting point + same systems = same hash.
    assert_eq!(
        hash_a, hash_b,
        "physics simulation diverged after snapshot restore"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Physics reconstruction registers all entities
// ---------------------------------------------------------------------------

/// Setup 5 physics entities, snapshot + restore, verify physics world has
/// all 5 entities re-registered with correct body count.
#[test]
fn physics_reconstruction_registers_all_entities() {
    let mut world = setup_world();

    // Spawn 5 dynamic entities with Position + Velocity + PhysicsBody.
    let mut entities = Vec::new();
    for i in 0..5u32 {
        let mut bundle = ComponentBundle::new();
        bundle.add(
            world.registry(),
            Position {
                x: i as f64 * 10.0,
                y: 0.0,
            },
        );
        bundle.add(world.registry(), Velocity { dx: 1.0, dy: 0.0 });
        bundle.add(
            world.registry(),
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 0.8,
                is_sensor: false,
            },
        );
        entities.push(world.spawn_bundle(bundle));
    }

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Attach physics world and register all 5 entities.
    let mut physics = PhysicsWorld::new_zero_gravity();
    for &entity in &entities {
        let pos = tick_loop.world().get_component::<Position>(entity).unwrap();
        let vel = tick_loop.world().get_component::<Velocity>(entity).unwrap();
        let body = tick_loop
            .world()
            .get_component::<PhysicsBody>(entity)
            .unwrap();
        physics.register_entity(entity, pos, vel, body);
    }
    assert_eq!(physics.body_count(), 5);
    tick_loop.set_physics(physics);

    // Run a few ticks so state diverges from initial.
    tick_loop.run_ticks(10);

    // Capture snapshot.
    let snapshot = tick_loop.capture_snapshot();

    // Run more ticks to mutate state.
    tick_loop.run_ticks(20);

    // Restore snapshot. Physics reconstruction should re-register all 5 entities.
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");

    // Verify physics entity count.
    let physics = tick_loop
        .physics()
        .expect("physics should be attached after restore");
    assert_eq!(
        physics.body_count(),
        5,
        "all 5 entities should be registered in reconstructed physics world"
    );

    // Verify each entity is registered.
    for &entity in &entities {
        assert!(
            physics.has_entity(entity),
            "entity {entity:?} should be registered in physics after reconstruction"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: Physics reconstruction clears stale bodies
// ---------------------------------------------------------------------------

/// Register entities, add more, snapshot with fewer (restore), verify only
/// the snapshot count survives (stale bodies from pre-restore are cleared).
#[test]
fn physics_reconstruction_clears_stale_bodies() {
    let mut world = setup_world();

    // Spawn 3 initial entities with physics components.
    let mut initial_entities = Vec::new();
    for i in 0..3u32 {
        let mut bundle = ComponentBundle::new();
        bundle.add(
            world.registry(),
            Position {
                x: i as f64 * 5.0,
                y: 0.0,
            },
        );
        bundle.add(world.registry(), Velocity { dx: 1.0, dy: 0.0 });
        bundle.add(
            world.registry(),
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 1.0,
                is_sensor: false,
            },
        );
        initial_entities.push(world.spawn_bundle(bundle));
    }

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Attach physics world with the 3 initial entities.
    let mut physics = PhysicsWorld::new_zero_gravity();
    for &entity in &initial_entities {
        let pos = tick_loop.world().get_component::<Position>(entity).unwrap();
        let vel = tick_loop.world().get_component::<Velocity>(entity).unwrap();
        let body = tick_loop
            .world()
            .get_component::<PhysicsBody>(entity)
            .unwrap();
        physics.register_entity(entity, pos, vel, body);
    }
    tick_loop.set_physics(physics);

    // Run a few ticks.
    tick_loop.run_ticks(5);

    // Capture snapshot with 3 entities.
    let snapshot = tick_loop.capture_snapshot();

    // Now spawn 2 more entities (after the snapshot point) and register them.
    for i in 3..5u32 {
        let mut bundle = ComponentBundle::new();
        bundle.add(
            tick_loop.world().registry(),
            Position {
                x: i as f64 * 5.0,
                y: 0.0,
            },
        );
        bundle.add(tick_loop.world().registry(), Velocity { dx: 2.0, dy: 0.0 });
        bundle.add(
            tick_loop.world().registry(),
            PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.3 },
                restitution: 0.5,
                is_sensor: false,
            },
        );
        let new_entity = tick_loop.world_mut().spawn_bundle(bundle);
        let pos = tick_loop
            .world()
            .get_component::<Position>(new_entity)
            .unwrap()
            .clone();
        let vel = tick_loop
            .world()
            .get_component::<Velocity>(new_entity)
            .unwrap()
            .clone();
        let body = tick_loop
            .world()
            .get_component::<PhysicsBody>(new_entity)
            .unwrap()
            .clone();
        tick_loop
            .physics_mut()
            .unwrap()
            .register_entity(new_entity, &pos, &vel, &body);
    }

    // Physics world now has 5 bodies.
    assert_eq!(
        tick_loop.physics().unwrap().body_count(),
        5,
        "should have 5 bodies before restore"
    );

    // Restore to the snapshot (which only had 3 entities).
    // Physics reconstruction should clear the 5 stale bodies and only
    // re-register the 3 entities from the snapshot's ECS state.
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");

    assert_eq!(
        tick_loop.physics().unwrap().body_count(),
        3,
        "after restore, physics should only have the 3 entities from snapshot"
    );
}
