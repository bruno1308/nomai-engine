//! Integration tests for the deterministic replay system.
//!
//! These tests validate recording, replaying, checkpoint verification,
//! divergence detection, and JSON serialization of [`ReplayLog`].

use nomai_ecs::prelude::*;
use nomai_engine::replay::{replay, ReplayEntry, ReplayLog, ReplayRecorder};
use nomai_engine::tick::{InputFrame, TickConfig, TickLoop};

// ---------------------------------------------------------------------------
// Test component types (matching snapshot_engine_tests.rs convention)
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

/// An alternative counter system that increments by 2 instead of 1.
/// Used to test divergence detection.
fn counter_system_double(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (counter,)) in world.query::<(&Counter,)>() {
        cmds.set_component(
            entity,
            "counter",
            serde_json::json!(counter.0 + 2),
            SystemId(2),
            CausalReason::SystemInternal("increment_double".to_owned()),
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

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.add_system("movement", movement_system);
    tick_loop.add_system("counter", counter_system);

    tick_loop
}

/// Record a simulation run and return the replay log.
fn record_simulation(
    tick_loop: &mut TickLoop,
    num_ticks: u64,
    checkpoint_interval: u64,
) -> ReplayLog {
    let snapshot = tick_loop.capture_snapshot();
    let mut recorder = ReplayRecorder::new(snapshot, checkpoint_interval);

    for _ in 0..num_ticks {
        let tick = tick_loop.tick_count();
        let input = tick_loop.current_input().clone();
        let hash = tick_loop.state_hash();
        recorder.record_tick(tick, &input, Some(hash));
        tick_loop.tick();
    }

    recorder.finish()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test 1: Record and replay an empty simulation (no entities, no inputs).
/// Verify replay succeeds with zero divergences.
#[test]
fn record_and_replay_empty_simulation() {
    let world = setup_world();
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Record 20 ticks of empty simulation.
    let log = record_simulation(&mut tick_loop, 20, 5);

    // Replay on a fresh tick loop (same config, no entities).
    let world2 = setup_world();
    let config2 = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut replay_loop = TickLoop::new(world2, config2);

    let result = replay(&mut replay_loop, &log).expect("replay should succeed");

    assert!(
        result.completed,
        "replay of empty simulation should complete"
    );
    assert_eq!(result.ticks_replayed, 20);
    assert!(
        result.first_divergence.is_none(),
        "no divergence expected in empty simulation replay"
    );
}

/// Test 2: Record 100 ticks with varying inputs, checkpoints every 10 ticks,
/// then replay and verify all checkpoints match.
#[test]
fn record_and_replay_with_inputs() {
    let mut tick_loop = build_tick_loop_with_entities();

    let snapshot = tick_loop.capture_snapshot();
    let mut recorder = ReplayRecorder::new(snapshot, 10);

    // Simulate 100 ticks with varying inputs.
    for i in 0..100u64 {
        let tick = tick_loop.tick_count();

        // Set varying inputs every 5 ticks.
        let mut input = InputFrame::default();
        if i % 5 == 0 {
            input
                .inputs
                .insert("move_x".to_string(), serde_json::json!(i as f64 * 0.1));
            input
                .inputs
                .insert("action".to_string(), serde_json::json!(format!("act_{i}")));
        }
        tick_loop.set_input(input.clone());

        let hash = tick_loop.state_hash();
        recorder.record_tick(tick, &input, Some(hash));
        tick_loop.tick();
    }

    let log = recorder.finish();

    // Replay on a fresh tick loop with the same systems.
    let mut replay_loop = build_tick_loop_with_entities();
    let result = replay(&mut replay_loop, &log).expect("replay should succeed");

    assert!(result.completed, "replay should complete successfully");
    assert_eq!(result.ticks_replayed, 100);
    assert!(
        result.first_divergence.is_none(),
        "no divergence expected when replaying identical simulation"
    );
}

/// Test 3: Record with system A, replay with system B, detect divergence.
#[test]
fn replay_detects_divergence_when_systems_differ() {
    // Record with normal counter system.
    let mut tick_loop = build_tick_loop_with_entities();
    let log = record_simulation(&mut tick_loop, 50, 5);

    // Replay with a different counter system (increments by 2).
    let mut world2 = setup_world();

    let mut b1 = ComponentBundle::new();
    b1.add(world2.registry(), Position { x: 0.0, y: 0.0 });
    b1.add(world2.registry(), Velocity { dx: 1.5, dy: -0.5 });
    b1.add(world2.registry(), Counter(0));
    world2.spawn_bundle(b1);

    let mut b2 = ComponentBundle::new();
    b2.add(world2.registry(), Position { x: 100.0, y: 200.0 });
    b2.add(world2.registry(), Counter(0));
    world2.spawn_bundle(b2);

    let config2 = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut replay_loop = TickLoop::new(world2, config2);
    replay_loop.add_system("movement", movement_system);
    // Use the double-increment counter system instead.
    replay_loop.add_system("counter", counter_system_double);

    let result =
        replay(&mut replay_loop, &log).expect("replay should succeed (even with divergence)");

    assert!(
        !result.completed,
        "replay should NOT complete when systems differ"
    );
    assert!(
        result.first_divergence.is_some(),
        "should detect divergence when systems differ"
    );

    let divergence = result.first_divergence.unwrap();
    // The first checkpoint after tick 0 where divergence is detected should be
    // at tick 5 (checkpoint_interval=5, first non-zero checkpoint).
    // Tick 0 checkpoint passes (same initial state), but by tick 5 the
    // counter system difference should be visible.
    assert!(
        divergence.tick > 0,
        "divergence should be detected after at least 1 tick of execution"
    );
    assert_ne!(
        divergence.expected_hash, divergence.actual_hash,
        "divergence hashes must differ"
    );
}

/// Test 4: ReplayLog round-trips through serde_json.
#[test]
fn replay_log_serializable_to_json() {
    let mut tick_loop = build_tick_loop_with_entities();

    // Set some input so we have non-empty entries.
    let mut input = InputFrame::default();
    input
        .inputs
        .insert("fire".to_string(), serde_json::json!(true));
    tick_loop.set_input(input);

    let log = record_simulation(&mut tick_loop, 30, 10);

    // Serialize to JSON.
    let json_str = serde_json::to_string_pretty(&log).expect("ReplayLog should serialize to JSON");
    assert!(!json_str.is_empty(), "JSON should not be empty");

    // Deserialize back.
    let restored: ReplayLog =
        serde_json::from_str(&json_str).expect("ReplayLog should deserialize from JSON");

    // Verify key fields match.
    assert_eq!(
        restored.initial_snapshot.tick_counter,
        log.initial_snapshot.tick_counter
    );
    assert_eq!(restored.initial_snapshot.hash, log.initial_snapshot.hash);
    assert_eq!(restored.entries.len(), log.entries.len());
    assert_eq!(restored.total_ticks, log.total_ticks);
    assert_eq!(restored.gameplay_module_hash, log.gameplay_module_hash);

    // Verify entries match structurally.
    for (orig, rest) in log.entries.iter().zip(restored.entries.iter()) {
        match (orig, rest) {
            (ReplayEntry::Input { tick: t1, .. }, ReplayEntry::Input { tick: t2, .. }) => {
                assert_eq!(t1, t2, "input entry tick mismatch");
            }
            (
                ReplayEntry::Checkpoint {
                    tick: t1,
                    state_hash: h1,
                },
                ReplayEntry::Checkpoint {
                    tick: t2,
                    state_hash: h2,
                },
            ) => {
                assert_eq!(t1, t2, "checkpoint entry tick mismatch");
                assert_eq!(h1, h2, "checkpoint hash mismatch");
            }
            _ => panic!("entry type mismatch between original and deserialized"),
        }
    }
}

/// Test 5: Verify checkpoints appear at the correct interval.
#[test]
fn checkpoint_interval_respected() {
    let mut tick_loop = build_tick_loop_with_entities();
    let checkpoint_interval = 7;

    let log = record_simulation(&mut tick_loop, 50, checkpoint_interval);

    // Collect checkpoint ticks.
    let checkpoint_ticks: Vec<u64> = log
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ReplayEntry::Checkpoint { tick, .. } => Some(*tick),
            _ => None,
        })
        .collect();

    // All checkpoint ticks should be multiples of the interval.
    for tick in &checkpoint_ticks {
        assert_eq!(
            tick % checkpoint_interval,
            0,
            "checkpoint at tick {tick} is not a multiple of interval {checkpoint_interval}"
        );
    }

    // We should have checkpoints at ticks 0, 7, 14, 21, 28, 35, 42, 49.
    let expected_ticks: Vec<u64> = (0..50).filter(|t| t % checkpoint_interval == 0).collect();
    assert_eq!(
        checkpoint_ticks, expected_ticks,
        "checkpoint ticks do not match expected interval pattern"
    );
}

/// Test 6: Replay with no input changes still produces and verifies checkpoints.
#[test]
fn replay_with_empty_inputs_still_produces_checkpoints() {
    let mut tick_loop = build_tick_loop_with_entities();

    // Record 40 ticks without setting any inputs (default empty InputFrame).
    let log = record_simulation(&mut tick_loop, 40, 10);

    // Verify we have checkpoints even though inputs were empty.
    let checkpoint_count = log
        .entries
        .iter()
        .filter(|e| matches!(e, ReplayEntry::Checkpoint { .. }))
        .count();
    assert!(
        checkpoint_count > 0,
        "should have checkpoints even with empty inputs"
    );

    // Verify no Input entries were recorded (all inputs were empty).
    let input_count = log
        .entries
        .iter()
        .filter(|e| matches!(e, ReplayEntry::Input { .. }))
        .count();
    assert_eq!(
        input_count, 0,
        "should have no input entries when all inputs are empty"
    );

    // Replay and verify all checkpoints pass.
    let mut replay_loop = build_tick_loop_with_entities();
    let result = replay(&mut replay_loop, &log).expect("replay should succeed");

    assert!(result.completed, "replay should complete successfully");
    assert_eq!(result.ticks_replayed, 40);
    assert!(
        result.first_divergence.is_none(),
        "no divergence expected in replay with empty inputs"
    );
}

/// Test 7: Replay rejects logs with duplicate Input entries at the same tick.
#[test]
fn replay_rejects_duplicate_input_entries() {
    let tick_loop = build_tick_loop_with_entities();
    let snapshot = tick_loop.capture_snapshot();

    let mut input = InputFrame::default();
    input.inputs.insert("key".to_string(), serde_json::json!(1));

    // Construct a log with duplicate Input entries at the same tick.
    let log = ReplayLog {
        initial_snapshot: snapshot,
        gameplay_module_hash: None,
        total_ticks: 10,
        entries: vec![
            ReplayEntry::Input {
                tick: 0,
                input: input.clone(),
            },
            ReplayEntry::Input {
                tick: 0,
                input: input.clone(),
            },
        ],
    };

    let mut replay_loop = build_tick_loop_with_entities();
    let err = replay(&mut replay_loop, &log).expect_err("should reject duplicate Input entries");
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate Input entry at tick 0"),
        "error should mention duplicate Input: {msg}"
    );
}

/// Test 8: Replay rejects logs with duplicate Checkpoint entries at the same tick.
#[test]
fn replay_rejects_duplicate_checkpoint_entries() {
    let tick_loop = build_tick_loop_with_entities();
    let snapshot = tick_loop.capture_snapshot();
    let hash = tick_loop.state_hash();

    // Construct a log with duplicate Checkpoint entries at the same tick.
    let log = ReplayLog {
        initial_snapshot: snapshot,
        gameplay_module_hash: None,
        total_ticks: 10,
        entries: vec![
            ReplayEntry::Checkpoint {
                tick: 0,
                state_hash: hash.clone(),
            },
            ReplayEntry::Checkpoint {
                tick: 0,
                state_hash: hash,
            },
        ],
    };

    let mut replay_loop = build_tick_loop_with_entities();
    let err =
        replay(&mut replay_loop, &log).expect_err("should reject duplicate Checkpoint entries");
    let msg = err.to_string();
    assert!(
        msg.contains("duplicate Checkpoint entry at tick 0"),
        "error should mention duplicate Checkpoint: {msg}"
    );
}

/// Test 9: ReplayRecorder panics on non-monotonic tick ordering.
#[test]
#[should_panic(expected = "not strictly greater than previous tick")]
fn recorder_panics_on_non_monotonic_ticks() {
    let world = setup_world();
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let tick_loop = TickLoop::new(world, config);
    let snapshot = tick_loop.capture_snapshot();

    let mut recorder = ReplayRecorder::new(snapshot, 10);
    let input = InputFrame::default();

    recorder.record_tick(5, &input, None);
    recorder.record_tick(3, &input, None); // Should panic: 3 < 5
}

/// Test 10: ReplayRecorder panics on duplicate tick.
#[test]
#[should_panic(expected = "not strictly greater than previous tick")]
fn recorder_panics_on_duplicate_tick() {
    let world = setup_world();
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let tick_loop = TickLoop::new(world, config);
    let snapshot = tick_loop.capture_snapshot();

    let mut recorder = ReplayRecorder::new(snapshot, 10);
    let input = InputFrame::default();

    recorder.record_tick(5, &input, None);
    recorder.record_tick(5, &input, None); // Should panic: 5 == 5
}

/// Test 11: Replay returns error on tick range overflow.
#[test]
fn replay_rejects_tick_range_overflow() {
    let world = setup_world();
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Run some ticks so start_tick > 0.
    tick_loop.run_ticks(10);
    let snapshot = tick_loop.capture_snapshot();

    // Construct a log where start_tick + total_ticks overflows u64.
    let log = ReplayLog {
        initial_snapshot: snapshot,
        gameplay_module_hash: None,
        total_ticks: u64::MAX, // start_tick=10 + u64::MAX overflows
        entries: vec![],
    };

    let world2 = setup_world();
    let config2 = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut replay_loop = TickLoop::new(world2, config2);
    let err = replay(&mut replay_loop, &log).expect_err("should reject overflowing tick range");
    let msg = err.to_string();
    assert!(
        msg.contains("tick range overflow"),
        "error should mention overflow: {msg}"
    );
}
