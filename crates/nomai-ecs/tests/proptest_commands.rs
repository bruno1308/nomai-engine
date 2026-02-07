//! Property tests for command buffer operations.
//!
//! These tests use `proptest` to generate random sequences of command buffer
//! operations and verify that invariants hold after applying each sequence.

use nomai_ecs::prelude::*;
use proptest::prelude::*;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Hp(u32);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Score(i64);

#[derive(Debug, Clone)]
enum CmdOp {
    SetHp(usize, u32),
    SetScore(usize, i64),
    RemoveHp(usize),
    Despawn(usize),
    SpawnSemantic,
}

fn cmd_op_strategy() -> impl Strategy<Value = CmdOp> {
    prop_oneof![
        (0..20usize, any::<u32>()).prop_map(|(i, v)| CmdOp::SetHp(i, v)),
        (0..20usize, any::<i64>()).prop_map(|(i, v)| CmdOp::SetScore(i, v)),
        (0..20usize).prop_map(CmdOp::RemoveHp),
        (0..20usize).prop_map(CmdOp::Despawn),
        Just(CmdOp::SpawnSemantic),
    ]
}

/// Build a command buffer from a list of operations against a set of known entities.
fn build_commands(ops: &[CmdOp], entities: &[EntityId]) -> CommandBuffer {
    let mut buf = CommandBuffer::new();

    for op in ops {
        match op {
            CmdOp::SetHp(idx, val) => {
                if !entities.is_empty() {
                    let idx = idx % entities.len();
                    buf.set_component(
                        entities[idx],
                        "hp",
                        serde_json::json!(*val),
                        SystemId(0),
                        CausalReason::SystemInternal("test".to_owned()),
                    );
                }
            }
            CmdOp::SetScore(idx, val) => {
                if !entities.is_empty() {
                    let idx = idx % entities.len();
                    buf.set_component(
                        entities[idx],
                        "score",
                        serde_json::json!(*val),
                        SystemId(0),
                        CausalReason::SystemInternal("test".to_owned()),
                    );
                }
            }
            CmdOp::RemoveHp(idx) => {
                if !entities.is_empty() {
                    let idx = idx % entities.len();
                    buf.remove_component(
                        entities[idx],
                        "hp",
                        SystemId(0),
                        CausalReason::SystemInternal("test".to_owned()),
                    );
                }
            }
            CmdOp::Despawn(idx) => {
                if !entities.is_empty() {
                    let idx = idx % entities.len();
                    buf.despawn(
                        entities[idx],
                        SystemId(0),
                        CausalReason::SystemInternal("test".to_owned()),
                    );
                }
            }
            CmdOp::SpawnSemantic => {
                buf.spawn_semantic(
                    EntityIdentity {
                        entity_type: "test".to_owned(),
                        role: "unit".to_owned(),
                        spawned_by: SystemId(0),
                        requirement_id: None,
                    },
                    vec![("hp".to_owned(), serde_json::json!(50))],
                    SystemId(0),
                    CausalReason::GameRule("spawn".to_owned()),
                );
            }
        }
    }

    buf
}

/// Create a fresh world with Hp and Score registered, plus 5 initial entities.
fn setup_world_and_entities() -> (World, Vec<EntityId>) {
    let mut world = World::new();
    world.register_component::<Hp>("hp");
    world.register_component::<Score>("score");

    let mut entities: Vec<EntityId> = Vec::new();
    for i in 0..5u32 {
        let e = world.spawn_with(Hp(100 + i));
        entities.push(e);
    }

    (world, entities)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// Random command sequences preserve internal consistency.
    ///
    /// Invariants checked:
    /// - Command indices are sequential starting from 0.
    /// - Successful spawn commands have `spawned_entity` set to `Some`.
    /// - ApplyReport counts match the actual success/failure counts.
    #[test]
    fn command_sequences_preserve_consistency(ops in prop::collection::vec(cmd_op_strategy(), 1..30)) {
        let (mut world, entities) = setup_world_and_entities();
        let mut buf = build_commands(&ops, &entities);

        // Apply all commands.
        let applied = buf.apply(&mut world);

        // Invariant: every command has a sequential command_index.
        for (i, cmd) in applied.iter().enumerate() {
            prop_assert_eq!(cmd.command_index, i as u32);
        }

        // Invariant: applied_successfully is well-defined (not uninitialized).
        // This is trivially true in Rust, but documents the intent.
        for cmd in &applied {
            let _ = cmd.applied_successfully;
        }

        // Invariant: entity count is non-negative (always true for usize, but
        // documents the intent that we have no underflow).
        let _ = world.entity_count();

        // Invariant: all spawn commands have spawned_entity set if successful.
        for cmd in &applied {
            if matches!(
                cmd.kind,
                CommandKind::SpawnSemantic { .. } | CommandKind::SpawnPooled { .. }
            ) && cmd.applied_successfully
            {
                prop_assert!(cmd.spawned_entity.is_some());
            }
        }

        // Invariant: the apply report counts match the actual counts.
        let report = buf.last_apply_report();
        let actual_success = applied.iter().filter(|c| c.applied_successfully).count();
        let actual_failed = applied.iter().filter(|c| !c.applied_successfully).count();
        prop_assert_eq!(report.success_count, actual_success);
        prop_assert_eq!(report.failed_count, actual_failed);
    }

    /// The same command sequence applied to two identical worlds produces
    /// identical success/failure results. This is the determinism guarantee.
    #[test]
    fn command_buffer_deterministic(ops in prop::collection::vec(cmd_op_strategy(), 1..20)) {
        fn run_once(ops: &[CmdOp]) -> Vec<bool> {
            let (mut world, entities) = setup_world_and_entities();
            let mut buf = build_commands(ops, &entities);
            buf.apply(&mut world)
                .iter()
                .map(|c| c.applied_successfully)
                .collect()
        }

        let run1 = run_once(&ops);
        let run2 = run_once(&ops);
        prop_assert_eq!(run1, run2);
    }

    /// Commands that target entities which have already been despawned in the
    /// same buffer gracefully fail (applied_successfully = false) rather than
    /// panicking or corrupting state.
    #[test]
    fn despawn_then_modify_is_graceful(
        hp_val in any::<u32>(),
        score_val in any::<i64>(),
    ) {
        let (mut world, entities) = setup_world_and_entities();
        let target = entities[0];

        let mut buf = CommandBuffer::new();

        // First: despawn the entity.
        buf.despawn(
            target,
            SystemId(0),
            CausalReason::GameRule("destroy".to_owned()),
        );

        // Then: try to modify it (should fail gracefully).
        buf.set_component(
            target,
            "hp",
            serde_json::json!(hp_val),
            SystemId(0),
            CausalReason::SystemInternal("post-despawn".to_owned()),
        );

        buf.set_component(
            target,
            "score",
            serde_json::json!(score_val),
            SystemId(0),
            CausalReason::SystemInternal("post-despawn".to_owned()),
        );

        let applied = buf.apply(&mut world);

        // The despawn should succeed.
        prop_assert!(applied[0].applied_successfully);

        // The subsequent modifications should fail.
        prop_assert!(!applied[1].applied_successfully);
        prop_assert!(!applied[2].applied_successfully);

        // The entity should be gone.
        prop_assert!(!world.is_alive(target));
    }

    /// Spawning via command creates entities that are properly alive and have
    /// the correct identity tier.
    #[test]
    fn spawn_commands_create_valid_entities(
        spawn_count in 1..20usize,
    ) {
        let mut world = World::new();
        world.register_component::<Hp>("hp");
        world.register_component::<Score>("score");

        let mut buf = CommandBuffer::new();
        for _ in 0..spawn_count {
            buf.spawn_semantic(
                EntityIdentity {
                    entity_type: "test".to_owned(),
                    role: "unit".to_owned(),
                    spawned_by: SystemId(0),
                    requirement_id: None,
                },
                vec![("hp".to_owned(), serde_json::json!(100))],
                SystemId(0),
                CausalReason::GameRule("test_spawn".to_owned()),
            );
        }

        let applied = buf.apply(&mut world);
        prop_assert_eq!(applied.len(), spawn_count);

        for cmd in &applied {
            prop_assert!(cmd.applied_successfully);
            let spawned = cmd.spawned_entity.unwrap();
            prop_assert!(world.is_alive(spawned));

            // Verify identity tier.
            let tier = world.get_tier(spawned).unwrap();
            prop_assert_eq!(tier, IdentityTier::Semantic);

            // Verify the component was set.
            let hp = world.get_component::<Hp>(spawned);
            prop_assert!(hp.is_some());
            prop_assert_eq!(hp.unwrap(), &Hp(100));
        }

        prop_assert_eq!(world.entity_count(), spawn_count);
    }

    /// The buffer is empty after apply, and command indices reset for the next
    /// batch.
    #[test]
    fn buffer_resets_after_apply(
        batch1_size in 1..10usize,
        batch2_size in 1..10usize,
    ) {
        let (mut world, entities) = setup_world_and_entities();

        // First batch
        let mut buf = CommandBuffer::new();
        for i in 0..batch1_size {
            if !entities.is_empty() {
                buf.set_component(
                    entities[i % entities.len()],
                    "hp",
                    serde_json::json!(i as u32),
                    SystemId(0),
                    CausalReason::SystemInternal("batch1".to_owned()),
                );
            }
        }
        let applied1 = buf.apply(&mut world);
        prop_assert_eq!(applied1.len(), batch1_size);
        prop_assert!(buf.is_empty());

        // Second batch: command indices should restart from 0.
        for i in 0..batch2_size {
            if !entities.is_empty() {
                buf.set_component(
                    entities[i % entities.len()],
                    "hp",
                    serde_json::json!((i + 100) as u32),
                    SystemId(0),
                    CausalReason::SystemInternal("batch2".to_owned()),
                );
            }
        }
        let applied2 = buf.apply(&mut world);
        prop_assert_eq!(applied2.len(), batch2_size);

        // Verify second batch indices start from 0.
        for (i, cmd) in applied2.iter().enumerate() {
            prop_assert_eq!(cmd.command_index, i as u32);
        }
    }
}
