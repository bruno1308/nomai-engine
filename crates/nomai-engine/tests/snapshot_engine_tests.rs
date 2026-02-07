//! Engine-level snapshot tests: capture, restore, hash, branching, determinism.
//!
//! These tests validate the [`EngineSnapshot`] round-trip (capture -> restore),
//! BLAKE3 hash correctness, and snapshot-based branching for deterministic
//! simulation replay.

use nomai_ecs::prelude::*;
use nomai_engine::prelude::*;
use nomai_engine::tick::{InputFrame, TickConfig, TickLoop};

// ---------------------------------------------------------------------------
// Test component types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Position {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Velocity {
    dx: f64,
    dy: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Counter(u64);

// ---------------------------------------------------------------------------
// Test systems
// ---------------------------------------------------------------------------

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

fn setup_world() -> World {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<Counter>("counter");
    world
}

fn build_tick_loop_with_entities() -> TickLoop {
    let mut world = setup_world();

    // Entity 1: moving entity with counter.
    let mut b1 = ComponentBundle::new();
    b1.add(world.registry(), Position { x: 0.0, y: 0.0 });
    b1.add(world.registry(), Velocity { dx: 1.5, dy: -0.5 });
    b1.add(world.registry(), Counter(0));
    world.spawn_bundle(b1);

    // Entity 2: stationary entity with counter.
    let mut b2 = ComponentBundle::new();
    b2.add(world.registry(), Position { x: 100.0, y: 200.0 });
    b2.add(world.registry(), Counter(0));
    world.spawn_bundle(b2);

    // Entity 3: moving entity without counter.
    let mut b3 = ComponentBundle::new();
    b3.add(world.registry(), Position { x: 50.0, y: 50.0 });
    b3.add(world.registry(), Velocity { dx: -1.0, dy: 2.0 });
    world.spawn_bundle(b3);

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.add_system("movement", movement_system);
    tick_loop.add_system("counter", counter_system);

    tick_loop
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Run 50 ticks, capture snapshot, run 50 more, capture hash A.
/// Restore snapshot, run 50 more, capture hash B. hash_a == hash_b.
#[test]
fn snapshot_restore_determinism() {
    let mut tick_loop = build_tick_loop_with_entities();

    // Run 50 ticks.
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 50);

    // Capture snapshot at tick 50.
    let snapshot = tick_loop.capture_snapshot();
    assert_eq!(snapshot.tick_counter, 50);

    // Run 50 more ticks (tick 50..99), capture final hash A.
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 100);
    let hash_a = tick_loop.state_hash();

    // Restore to tick 50.
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    assert_eq!(tick_loop.tick_count(), 50);

    // Run 50 more ticks again (tick 50..99), capture final hash B.
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 100);
    let hash_b = tick_loop.state_hash();

    // Determinism: identical path from same snapshot must produce same hash.
    assert_eq!(
        hash_a, hash_b,
        "restored simulation diverged from original run"
    );
}

/// Fork at tick 50, branch A runs normally, branch B modifies a component
/// before running. The branches must produce different hashes.
#[test]
fn snapshot_branching_produces_different_states() {
    let mut tick_loop = build_tick_loop_with_entities();

    // Run to fork point.
    tick_loop.run_ticks(50);
    let fork = tick_loop.fork_snapshot();
    assert_eq!(fork.tick_counter, 50);

    // Branch A: run 50 more ticks normally.
    tick_loop.run_ticks(50);
    let hash_a = tick_loop.state_hash();
    let tick_a = tick_loop.tick_count();

    // Branch B: restore fork, modify a component, then run 50 ticks.
    tick_loop
        .restore_from_snapshot(&fork)
        .expect("restore should succeed");
    assert_eq!(tick_loop.tick_count(), 50);

    // Modify entity 1's position to create divergence.
    let entities: Vec<EntityId> = tick_loop
        .world()
        .query::<(&Position, &Velocity, &Counter)>()
        .map(|(e, _)| e)
        .collect();
    assert!(
        !entities.is_empty(),
        "should have entities with pos+vel+counter"
    );

    // Directly modify via world_mut (allowed for setup/testing).
    if let Some(pos) = tick_loop
        .world_mut()
        .get_component_mut::<Position>(entities[0])
    {
        pos.x += 9999.0;
    }

    tick_loop.run_ticks(50);
    let hash_b = tick_loop.state_hash();
    let tick_b = tick_loop.tick_count();

    // Both branches ran 50 ticks from the fork point.
    assert_eq!(tick_a, 100);
    assert_eq!(tick_b, 100);

    // Branches must produce different hashes because state diverged.
    assert_ne!(
        hash_a, hash_b,
        "branches with different state should produce different hashes"
    );
}

/// Snapshots at different ticks should have different hashes.
#[test]
fn snapshot_hash_changes_with_state() {
    let mut tick_loop = build_tick_loop_with_entities();

    tick_loop.run_ticks(10);
    let snapshot_10 = tick_loop.capture_snapshot();

    tick_loop.run_ticks(10);
    let snapshot_20 = tick_loop.capture_snapshot();

    assert_eq!(snapshot_10.tick_counter, 10);
    assert_eq!(snapshot_20.tick_counter, 20);

    assert_ne!(
        snapshot_10.hash, snapshot_20.hash,
        "snapshots at different ticks should have different hashes"
    );
}

/// The hash field is a valid 64-character lowercase hex string (BLAKE3 = 32 bytes = 64 hex).
#[test]
fn snapshot_hash_is_blake3_hex() {
    let mut tick_loop = build_tick_loop_with_entities();
    tick_loop.run_ticks(5);

    let snapshot = tick_loop.capture_snapshot();
    let hash = &snapshot.hash;

    assert_eq!(
        hash.len(),
        64,
        "BLAKE3 hex digest should be 64 chars, got {}",
        hash.len()
    );

    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash should contain only hex characters, got: {hash}"
    );

    // BLAKE3 hex output is lowercase.
    assert_eq!(hash.to_lowercase(), *hash, "BLAKE3 hex should be lowercase");
}

/// EngineSnapshot must round-trip through JSON serialization.
#[test]
fn snapshot_serializable_to_json() {
    let mut tick_loop = build_tick_loop_with_entities();
    tick_loop.run_ticks(15);

    let snapshot = tick_loop.capture_snapshot();

    // Serialize to JSON.
    let json_str =
        serde_json::to_string_pretty(&snapshot).expect("snapshot should serialize to JSON");
    assert!(!json_str.is_empty());

    // Deserialize back.
    let restored: EngineSnapshot =
        serde_json::from_str(&json_str).expect("snapshot should deserialize from JSON");

    assert_eq!(restored.tick_counter, snapshot.tick_counter);
    assert_eq!(restored.fixed_dt, snapshot.fixed_dt);
    assert_eq!(restored.hash, snapshot.hash);
    assert_eq!(
        restored.current_input.inputs.len(),
        snapshot.current_input.inputs.len()
    );

    // The world snapshot should also match.
    assert_eq!(restored.world.entities.len(), snapshot.world.entities.len());
    assert_eq!(
        restored.world.allocator.generations,
        snapshot.world.allocator.generations
    );
}

/// Restore brings the tick counter back to the snapshot's value.
#[test]
fn restore_resets_tick_counter() {
    let mut tick_loop = build_tick_loop_with_entities();

    tick_loop.run_ticks(100);
    assert_eq!(tick_loop.tick_count(), 100);

    let snapshot = tick_loop.capture_snapshot();
    assert_eq!(snapshot.tick_counter, 100);

    // Run 50 more ticks.
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 150);

    // Restore to tick 100.
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    assert_eq!(tick_loop.tick_count(), 100);

    // Sim time should also reflect the restored tick counter.
    let expected_sim_time = 100.0 * tick_loop.fixed_dt();
    assert!(
        (tick_loop.sim_time() - expected_sim_time).abs() < 1e-12,
        "sim_time should match restored tick counter: expected {expected_sim_time}, got {}",
        tick_loop.sim_time()
    );
}

/// state_hash() convenience method matches capture_snapshot().hash.
#[test]
fn state_hash_matches_snapshot_hash() {
    let mut tick_loop = build_tick_loop_with_entities();
    tick_loop.run_ticks(25);

    let snapshot = tick_loop.capture_snapshot();
    let direct_hash = tick_loop.state_hash();

    assert_eq!(
        snapshot.hash, direct_hash,
        "state_hash() should match capture_snapshot().hash"
    );
}

/// fork_snapshot() returns the same snapshot as capture_snapshot().
#[test]
fn fork_snapshot_equals_capture_snapshot() {
    let mut tick_loop = build_tick_loop_with_entities();
    tick_loop.run_ticks(30);

    let capture = tick_loop.capture_snapshot();
    let fork = tick_loop.fork_snapshot();

    assert_eq!(capture.tick_counter, fork.tick_counter);
    assert_eq!(capture.fixed_dt, fork.fixed_dt);
    assert_eq!(capture.hash, fork.hash);
    assert_eq!(capture.world.entities.len(), fork.world.entities.len());
}

/// Snapshot captures the current input frame.
#[test]
fn snapshot_captures_input_frame() {
    let mut tick_loop = build_tick_loop_with_entities();

    // Set some input before snapshot.
    let mut input = InputFrame::default();
    input
        .inputs
        .insert("move_x".to_owned(), serde_json::json!(1.0));
    input
        .inputs
        .insert("fire".to_owned(), serde_json::json!(true));
    tick_loop.set_input(input);

    tick_loop.run_ticks(5);

    let snapshot = tick_loop.capture_snapshot();
    assert_eq!(
        snapshot.current_input.inputs.get("move_x"),
        Some(&serde_json::json!(1.0))
    );
    assert_eq!(
        snapshot.current_input.inputs.get("fire"),
        Some(&serde_json::json!(true))
    );
}

/// Restore replays the input frame from the snapshot.
#[test]
fn restore_restores_input_frame() {
    let mut tick_loop = build_tick_loop_with_entities();

    let mut input = InputFrame::default();
    input
        .inputs
        .insert("action".to_owned(), serde_json::json!("jump"));
    tick_loop.set_input(input);

    tick_loop.run_ticks(10);
    let snapshot = tick_loop.capture_snapshot();

    // Change the input.
    let mut new_input = InputFrame::default();
    new_input
        .inputs
        .insert("action".to_owned(), serde_json::json!("shoot"));
    tick_loop.set_input(new_input);

    // Verify current input is different.
    assert_eq!(
        tick_loop.current_input().inputs.get("action"),
        Some(&serde_json::json!("shoot"))
    );

    // Restore snapshot -- should bring back the original input.
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    assert_eq!(
        tick_loop.current_input().inputs.get("action"),
        Some(&serde_json::json!("jump"))
    );
}

/// Snapshot of an empty world works correctly.
#[test]
fn snapshot_empty_world() {
    let world = setup_world();
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    let snapshot = tick_loop.capture_snapshot();
    assert_eq!(snapshot.tick_counter, 0);
    assert_eq!(snapshot.world.entities.len(), 0);
    assert_eq!(snapshot.hash.len(), 64);

    // Run some ticks on an empty world, then restore.
    tick_loop.run_ticks(10);
    assert_eq!(tick_loop.tick_count(), 10);

    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore of empty world should succeed");
    assert_eq!(tick_loop.tick_count(), 0);
    assert_eq!(tick_loop.world().entity_count(), 0);
}

/// Multiple restore cycles produce identical state.
#[test]
fn multiple_restore_cycles_are_idempotent() {
    let mut tick_loop = build_tick_loop_with_entities();

    tick_loop.run_ticks(20);
    let snapshot = tick_loop.capture_snapshot();

    // Cycle 1: run ticks, restore, check hash.
    tick_loop.run_ticks(30);
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    let hash_1 = tick_loop.state_hash();

    // Cycle 2: run ticks, restore, check hash.
    tick_loop.run_ticks(50);
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    let hash_2 = tick_loop.state_hash();

    // Cycle 3: run ticks, restore, check hash.
    tick_loop.run_ticks(10);
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");
    let hash_3 = tick_loop.state_hash();

    assert_eq!(hash_1, snapshot.hash);
    assert_eq!(hash_2, snapshot.hash);
    assert_eq!(hash_3, snapshot.hash);
}

/// Restoring from a snapshot with a tampered hash should fail.
#[test]
fn restore_rejects_corrupted_hash() {
    let mut tick_loop = build_tick_loop_with_entities();
    tick_loop.run_ticks(10);

    let mut snapshot = tick_loop.capture_snapshot();

    // Tamper with the hash.
    snapshot.hash = "0".repeat(64);

    let result = tick_loop.restore_from_snapshot(&snapshot);
    assert!(result.is_err(), "restore should fail on corrupted hash");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("hash mismatch"),
        "error should mention hash mismatch, got: {err_msg}"
    );
}

/// Restoring from a snapshot restores the fixed_dt value.
#[test]
fn restore_restores_fixed_dt() {
    let mut world = setup_world();
    let mut b = ComponentBundle::new();
    b.add(world.registry(), Counter(0));
    world.spawn_bundle(b);

    // Create tick loop with dt = 1/60.
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.run_ticks(10);

    let snapshot = tick_loop.capture_snapshot();
    assert!((snapshot.fixed_dt - 1.0 / 60.0).abs() < 1e-15);

    // Create a NEW tick loop with a different dt.
    let mut world2 = setup_world();
    let mut b2 = ComponentBundle::new();
    b2.add(world2.registry(), Counter(0));
    world2.spawn_bundle(b2);

    let config2 = TickConfig {
        fixed_dt: 1.0 / 30.0,
        headless: true,
    };
    let mut tick_loop2 = TickLoop::new(world2, config2);
    assert!((tick_loop2.fixed_dt() - 1.0 / 30.0).abs() < 1e-15);

    // Restore snapshot from the 1/60 tick loop into the 1/30 tick loop.
    tick_loop2
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");

    // fixed_dt should now be 1/60, matching the snapshot.
    assert!(
        (tick_loop2.fixed_dt() - 1.0 / 60.0).abs() < 1e-15,
        "fixed_dt should be restored to snapshot value (1/60), got {}",
        tick_loop2.fixed_dt()
    );
}

/// Entity state is correctly preserved across snapshot/restore.
#[test]
fn snapshot_preserves_entity_component_data() {
    let mut tick_loop = build_tick_loop_with_entities();

    tick_loop.run_ticks(25);

    // Read entity state before snapshot.
    let positions_before: Vec<(u64, f64, f64)> = tick_loop
        .world()
        .query::<(&Position,)>()
        .map(|(e, (p,))| (e.to_raw(), p.x, p.y))
        .collect();
    let counters_before: Vec<(u64, u64)> = tick_loop
        .world()
        .query::<(&Counter,)>()
        .map(|(e, (c,))| (e.to_raw(), c.0))
        .collect();

    let snapshot = tick_loop.capture_snapshot();

    // Mutate the world by running more ticks.
    tick_loop.run_ticks(50);

    // Restore.
    tick_loop
        .restore_from_snapshot(&snapshot)
        .expect("restore should succeed");

    // Read entity state after restore.
    let positions_after: Vec<(u64, f64, f64)> = tick_loop
        .world()
        .query::<(&Position,)>()
        .map(|(e, (p,))| (e.to_raw(), p.x, p.y))
        .collect();
    let counters_after: Vec<(u64, u64)> = tick_loop
        .world()
        .query::<(&Counter,)>()
        .map(|(e, (c,))| (e.to_raw(), c.0))
        .collect();

    // Sort for deterministic comparison.
    let mut pos_before = positions_before;
    pos_before.sort_by_key(|(id, _, _)| *id);
    let mut pos_after = positions_after;
    pos_after.sort_by_key(|(id, _, _)| *id);

    let mut ctr_before = counters_before;
    ctr_before.sort_by_key(|(id, _)| *id);
    let mut ctr_after = counters_after;
    ctr_after.sort_by_key(|(id, _)| *id);

    assert_eq!(
        pos_before, pos_after,
        "entity positions should be identical after restore"
    );
    assert_eq!(
        ctr_before, ctr_after,
        "entity counters should be identical after restore"
    );
}
