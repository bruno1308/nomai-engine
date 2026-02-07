//! Week 6-7 Milestone: Snapshot/Restore, Deterministic Replay, Headless Simulation.
//!
//! Validates the integration of all Week 6-7 features:
//! 1. Snapshot determinism -- run straight to tick 300 vs. snapshot at tick 100,
//!    restore, run 200 more ticks: state hashes match.
//! 2. Replay determinism -- record a session with varying inputs, replay it,
//!    and verify all checkpoint hashes match.
//! 3. Snapshot branching -- fork at tick 50, run two branches with identical
//!    systems and no diverging inputs, verify they produce the same hash.
//! 4. Headless simulation -- runs without GPU/window, produces manifests.
//!
//! **Note:** The windowed/renderer acceptance criterion (Issue #40: "human can
//! see and play breakout") is validated by the `headless_toggle_tests.rs`
//! feature-gated tests and manual verification -- it cannot be automated in CI
//! without a GPU and display.
//!
//! All tests run headless (`headless: true` in `TickConfig`).

use nomai_ecs::prelude::*;
use nomai_engine::prelude::*;
use nomai_engine::replay::{replay, ReplayRecorder};
use nomai_engine::tick::{InputFrame, TickConfig, TickLoop};

// ---------------------------------------------------------------------------
// Shared test component types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Counter(u64);

// ---------------------------------------------------------------------------
// Shared test systems
// ---------------------------------------------------------------------------

/// Movement system: advances Position by Velocity each tick.
fn movement_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (pos, vel)) in world.query::<(&Position, &Velocity)>() {
        let new_pos = Position {
            x: pos.x + vel.dx,
            y: pos.y + vel.dy,
        };
        cmds.set_component(
            entity,
            "position",
            serde_json::json!({"x": new_pos.x, "y": new_pos.y}),
            SystemId(1),
            CausalReason::SystemInternal("movement".to_owned()),
        );
    }
}

/// Counter system: increments a Counter component by 1 each tick.
fn counter_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (counter,)) in world.query::<(&Counter,)>() {
        cmds.set_component(
            entity,
            "counter",
            serde_json::json!(counter.0 + 1),
            SystemId(2),
            CausalReason::SystemInternal("increment".to_owned()),
        );
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Register all component types used across the milestone tests.
fn setup_world() -> World {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<Counter>("counter");
    world.register_component::<PhysicsBody>("physics_body");
    world
}

/// Build a tick loop with entities and user systems (no physics).
/// Contains:
///   Entity 1: Position + Velocity + Counter (moving entity)
///   Entity 2: Position + Counter (stationary entity)
fn build_tick_loop_with_entities() -> TickLoop {
    let mut world = setup_world();

    let mut b1 = ComponentBundle::new();
    b1.add(world.registry(), Position { x: 0.0, y: 0.0 });
    b1.add(world.registry(), Velocity { dx: 1.5, dy: -0.5 });
    b1.add(world.registry(), Counter(0));
    world.spawn_bundle(b1);

    let mut b2 = ComponentBundle::new();
    b2.add(world.registry(), Position { x: 100.0, y: 200.0 });
    b2.add(world.registry(), Counter(0));
    world.spawn_bundle(b2);

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.add_system("movement", movement_system);
    tick_loop.add_system("counter", counter_system);

    tick_loop
}

/// Build a tick loop with physics entities (ball + wall) and user systems.
/// Used for physics-aware snapshot/replay tests.
///
/// Note: `movement_system` is intentionally omitted because rapier2d handles
/// position/velocity updates for physics entities via the physics step. Adding
/// the user-level movement system would double-apply velocity changes.
fn build_physics_tick_loop() -> TickLoop {
    let mut world = setup_world();

    // Ball: dynamic circle at origin, moving right.
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
    ball_bundle.add(world.registry(), Counter(0));
    let ball = world.spawn_bundle(ball_bundle);

    // Wall: static box at x=20 (far enough for a long run).
    let mut wall_bundle = ComponentBundle::new();
    wall_bundle.add(world.registry(), Position { x: 20.0, y: 0.0 });
    wall_bundle.add(world.registry(), Velocity { dx: 0.0, dy: 0.0 });
    wall_bundle.add(
        world.registry(),
        PhysicsBody {
            body_type: PhysicsBodyType::Static,
            collider: ColliderShape::Box {
                half_width: 0.5,
                half_height: 10.0,
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
    tick_loop.add_system("counter", counter_system);

    // Set up rapier physics world.
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
        &Position { x: 20.0, y: 0.0 },
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

    tick_loop
}

// ---------------------------------------------------------------------------
// Test 1: Snapshot determinism
//
// Run straight to tick 300 and record the hash.
// Then: fresh run to tick 100, snapshot, restore, run 200 more ticks.
// The hash at tick 300 must match.
// ---------------------------------------------------------------------------

#[test]
fn milestone_snapshot_determinism() {
    // --- Path A: straight run to tick 300 ---
    let mut straight = build_tick_loop_with_entities();
    straight.run_ticks(300);
    let hash_straight = straight.state_hash();
    assert_eq!(
        straight.tick_count(),
        300,
        "straight run should be at tick 300"
    );

    // --- Path B: run to tick 100, snapshot, restore, run 200 more ---
    let mut snapped = build_tick_loop_with_entities();
    snapped.run_ticks(100);
    assert_eq!(
        snapped.tick_count(),
        100,
        "snapped run should be at tick 100 before snapshot"
    );

    let snapshot = snapped.capture_snapshot();
    assert_eq!(
        snapshot.tick_counter, 100,
        "snapshot should record tick 100"
    );
    assert_eq!(
        snapshot.hash.len(),
        64,
        "BLAKE3 hex digest should be 64 chars"
    );

    // Restore the snapshot (round-trip through serialize/deserialize is implicit
    // in capture -> restore since it re-verifies the hash).
    snapped
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    assert_eq!(
        snapped.tick_count(),
        100,
        "tick count should be 100 after restore"
    );

    snapped.run_ticks(200);
    let hash_restored = snapped.state_hash();
    assert_eq!(
        snapped.tick_count(),
        300,
        "restored run should be at tick 300"
    );

    // Core assertion: both paths produce the same state hash at tick 300.
    assert_eq!(
        hash_straight, hash_restored,
        "snapshot+restore path must produce same hash as straight-through run. \
         Straight: {hash_straight}, Restored: {hash_restored}"
    );
}

// ---------------------------------------------------------------------------
// Test 2: Replay determinism
//
// Record a session of 200 ticks with varying inputs and checkpoints every
// 10 ticks. Replay the session and verify all checkpoints match (no divergence).
// ---------------------------------------------------------------------------

#[test]
fn milestone_replay_determinism() {
    // --- Recording phase ---
    let mut record_loop = build_tick_loop_with_entities();

    let initial_snapshot = record_loop.capture_snapshot();
    let mut recorder = ReplayRecorder::new(initial_snapshot, 10);

    for i in 0..200u64 {
        let tick = record_loop.tick_count();

        // Inject varying inputs at regular intervals to exercise the input path.
        let mut input = InputFrame::default();
        if i % 7 == 0 {
            input
                .inputs
                .insert("move_x".to_string(), serde_json::json!(i as f64 * 0.3));
        }
        if i % 13 == 0 {
            input.inputs.insert(
                "action".to_string(),
                serde_json::json!(format!("jump_{i}")),
            );
        }
        record_loop.set_input(input.clone());

        let hash = record_loop.state_hash();
        recorder.record_tick(tick, &input, Some(hash));
        record_loop.tick();
    }

    let log = recorder.finish();

    // Verify the log has the expected structure.
    assert_eq!(log.total_ticks, 200, "log should contain 200 ticks");

    // Count checkpoints: every 10 ticks out of 200 => ticks 0, 10, 20, ..., 190 = 20.
    let checkpoint_count = log
        .entries
        .iter()
        .filter(|e| matches!(e, nomai_engine::replay::ReplayEntry::Checkpoint { .. }))
        .count();
    assert_eq!(
        checkpoint_count, 20,
        "should have 20 checkpoints (every 10 ticks over 200)"
    );

    // --- Replay phase ---
    let mut replay_loop = build_tick_loop_with_entities();
    let result = replay(&mut replay_loop, &log).expect("replay should succeed");

    assert!(result.completed, "replay should complete successfully");
    assert_eq!(result.ticks_replayed, 200, "should replay all 200 ticks");
    assert_eq!(
        replay_loop.tick_count(),
        200,
        "replay tick count should match recording"
    );
    assert!(
        result.first_divergence.is_none(),
        "no divergence expected: same systems + same inputs. Got: {:?}",
        result.first_divergence
    );

    // Additionally verify the final state hash matches the recording.
    let final_hash_record = record_loop.state_hash();
    let final_hash_replay = replay_loop.state_hash();
    assert_eq!(
        final_hash_record, final_hash_replay,
        "final state hash after replay must match recording"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Snapshot branching
//
// Run to tick 50, fork. Run both branches 100 more ticks with the same
// (empty) inputs and the same systems. They must produce the same hash.
// ---------------------------------------------------------------------------

#[test]
fn milestone_snapshot_branching() {
    let mut tick_loop = build_tick_loop_with_entities();
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 50);

    // Fork at tick 50.
    let fork = tick_loop.fork_snapshot();
    assert_eq!(fork.tick_counter, 50);

    // --- Branch A: continue from current state ---
    tick_loop.run_ticks(100);
    let hash_a = tick_loop.state_hash();
    assert_eq!(tick_loop.tick_count(), 150);

    // --- Branch B: restore from fork, run same 100 ticks ---
    tick_loop
        .restore_from_snapshot(&fork)
        .expect("restore from fork should succeed");
    assert_eq!(
        tick_loop.tick_count(),
        50,
        "tick count should be 50 after restoring fork"
    );

    tick_loop.run_ticks(100);
    let hash_b = tick_loop.state_hash();
    assert_eq!(tick_loop.tick_count(), 150);

    // Core assertion: same initial state + same systems + same (empty) inputs
    // = identical state.
    assert_eq!(
        hash_a, hash_b,
        "two branches from the same fork with identical systems should produce \
         the same state hash. Branch A: {hash_a}, Branch B: {hash_b}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: Headless simulation
//
// Verify that headless mode works without GPU/window, produces manifests,
// and runs the full tick pipeline.
// ---------------------------------------------------------------------------

#[test]
fn milestone_headless_runs_without_gpu() {
    let mut tick_loop = build_tick_loop_with_entities();

    // Confirm headless mode is enabled.
    assert!(
        tick_loop.is_headless(),
        "tick loop should be in headless mode"
    );

    // Run 100 ticks headless.
    tick_loop.run_ticks(100);
    assert_eq!(tick_loop.tick_count(), 100);

    // Verify manifests were produced.
    // The manifest history window is 60 ticks, so ticks 40..99 should all be present.
    for tick in 40..100 {
        let manifest = tick_loop
            .manifest_at_tick(tick)
            .unwrap_or_else(|| panic!("manifest at tick {tick} should exist (history window is 60)"));

        // Each manifest should record the systems that executed.
        assert!(
            manifest
                .systems_executed
                .contains(&"movement".to_owned()),
            "manifest at tick {} should list movement system, got {:?}",
            tick,
            manifest.systems_executed
        );
        assert!(
            manifest
                .systems_executed
                .contains(&"counter".to_owned()),
            "manifest at tick {} should list counter system, got {:?}",
            tick,
            manifest.systems_executed
        );
        // Commands should have been processed (movement + counter for 2 entities).
        assert!(
            manifest.commands_processed > 0,
            "manifest at tick {} should have processed commands, got {}",
            tick,
            manifest.commands_processed
        );
    }

    // Verify simulation state is correct.
    // Entity 1 starts at (0, 0) with velocity (1.5, -0.5), so after 100 ticks:
    // position should be (150, -50).
    let moving_entities: Vec<_> = tick_loop
        .world()
        .query::<(&Position, &Velocity)>()
        .collect();
    assert_eq!(moving_entities.len(), 1, "should have 1 moving entity");
    let (_, (pos, _)) = &moving_entities[0];
    assert!(
        (pos.x - 150.0).abs() < 1e-10,
        "moving entity x should be 150.0 after 100 ticks, got {}",
        pos.x
    );
    assert!(
        (pos.y - (-50.0)).abs() < 1e-10,
        "moving entity y should be -50.0 after 100 ticks, got {}",
        pos.y
    );

    // Verify counter values.
    let counter_entities: Vec<_> = tick_loop.world().query::<(&Counter,)>().collect();
    assert_eq!(
        counter_entities.len(),
        2,
        "should have 2 entities with Counter component"
    );
    for (entity, (counter,)) in &counter_entities {
        assert_eq!(
            counter.0, 100,
            "counter for entity {:?} should be 100 after 100 ticks, got {}",
            entity, counter.0
        );
    }

    // Verify state hash is non-empty and deterministic.
    let hash = tick_loop.state_hash();
    assert_eq!(hash.len(), 64, "BLAKE3 hex digest should be 64 chars");
    // Hash should be consistent on a second call.
    let hash2 = tick_loop.state_hash();
    assert_eq!(hash, hash2, "state hash should be deterministic");
}

// ---------------------------------------------------------------------------
// Test 5: Snapshot determinism with physics
//
// Ensures that snapshot + restore works correctly when physics is involved.
// Physics state is reconstructed from ECS components during restore.
// ---------------------------------------------------------------------------

#[test]
fn milestone_snapshot_determinism_with_physics() {
    // --- Path A: straight run to tick 100 ---
    let mut straight = build_physics_tick_loop();
    straight.run_ticks(100);
    let hash_straight = straight.state_hash();

    // --- Path B: run to tick 50, snapshot, run 30 more to dirty state,
    //     then restore, run 50 more from the snapshot ---
    let mut snapped = build_physics_tick_loop();
    snapped.run_ticks(50);

    let snapshot = snapped.capture_snapshot();

    // Dirty the state: run 30 extra ticks so physics state diverges
    // from the snapshot. This forces `restore_from_snapshot` to actually
    // reconstruct the rapier world from ECS components rather than
    // getting lucky with an already-matching state.
    snapped.run_ticks(30);
    assert_eq!(snapped.tick_count(), 80, "should be at tick 80 after dirtying");

    snapped
        .restore_from_snapshot(&snapshot)
        .expect("physics snapshot restore should succeed");
    assert_eq!(snapped.tick_count(), 50, "should be back at tick 50 after restore");

    snapped.run_ticks(50);
    let hash_restored = snapped.state_hash();

    assert_eq!(
        snapped.tick_count(),
        100,
        "restored physics run should be at tick 100"
    );
    assert_eq!(
        hash_straight, hash_restored,
        "physics snapshot+restore must produce same hash as straight run. \
         Straight: {hash_straight}, Restored: {hash_restored}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: Replay determinism with physics
//
// Record a physics session and replay it, verifying checkpoint hashes match.
// ---------------------------------------------------------------------------

#[test]
fn milestone_replay_determinism_with_physics() {
    // --- Recording phase ---
    let mut record_loop = build_physics_tick_loop();

    let initial_snapshot = record_loop.capture_snapshot();
    let mut recorder = ReplayRecorder::new(initial_snapshot, 10);

    for _ in 0..100u64 {
        let tick = record_loop.tick_count();
        let input = record_loop.current_input().clone();
        let hash = record_loop.state_hash();
        recorder.record_tick(tick, &input, Some(hash));
        record_loop.tick();
    }

    let log = recorder.finish();
    assert_eq!(log.total_ticks, 100);

    // --- Replay phase ---
    let mut replay_loop = build_physics_tick_loop();
    let result = replay(&mut replay_loop, &log).expect("physics replay should succeed");

    assert!(
        result.completed,
        "physics replay should complete successfully"
    );
    assert_eq!(result.ticks_replayed, 100);
    assert!(
        result.first_divergence.is_none(),
        "no divergence expected in physics replay. Got: {:?}",
        result.first_divergence
    );

    // Verify final state hash matches the recording.
    let final_hash_record = record_loop.state_hash();
    let final_hash_replay = replay_loop.state_hash();
    assert_eq!(
        final_hash_record, final_hash_replay,
        "final state hash after physics replay must match recording"
    );
}
