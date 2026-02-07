//! Property tests for ECS operations.
//!
//! These tests use `proptest` to generate random sequences of ECS operations
//! and verify that world invariants hold after each sequence.

use nomai_ecs::prelude::*;
use proptest::prelude::*;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Pos {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Vel {
    dx: f32,
    dy: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Tag(u32);

/// Operations we can perform on the world.
#[derive(Debug, Clone)]
enum EcsOp {
    SpawnPos(f32, f32),
    SpawnPosVel(f32, f32, f32, f32),
    Despawn(usize),
    InsertVel(usize, f32, f32),
    RemoveVel(usize),
    QueryPos,
    QueryPosVel,
}

/// Strategy that generates finite (non-NaN, non-Inf) f32 values.
fn finite_f32() -> impl Strategy<Value = f32> {
    // Use i32 range mapped to f32 to avoid NaN/Inf issues in comparisons
    (-1_000_000i32..1_000_000i32).prop_map(|v| v as f32 * 0.01)
}

fn ecs_op_strategy() -> impl Strategy<Value = EcsOp> {
    prop_oneof![
        (finite_f32(), finite_f32()).prop_map(|(x, y)| EcsOp::SpawnPos(x, y)),
        (finite_f32(), finite_f32(), finite_f32(), finite_f32())
            .prop_map(|(x, y, dx, dy)| EcsOp::SpawnPosVel(x, y, dx, dy)),
        (0..100usize).prop_map(EcsOp::Despawn),
        (0..100usize, finite_f32(), finite_f32())
            .prop_map(|(i, dx, dy)| EcsOp::InsertVel(i, dx, dy)),
        (0..100usize).prop_map(EcsOp::RemoveVel),
        Just(EcsOp::QueryPos),
        Just(EcsOp::QueryPosVel),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn ecs_random_ops_preserve_invariants(ops in prop::collection::vec(ecs_op_strategy(), 1..50)) {
        let mut world = World::new();
        world.register_component::<Pos>("pos");
        world.register_component::<Vel>("vel");
        world.register_component::<Tag>("tag");

        let mut alive: Vec<EntityId> = Vec::new();

        for op in ops {
            match op {
                EcsOp::SpawnPos(x, y) => {
                    let e = world.spawn_with(Pos { x, y });
                    alive.push(e);
                }
                EcsOp::SpawnPosVel(x, y, dx, dy) => {
                    let mut b = ComponentBundle::new();
                    b.add(world.registry(), Pos { x, y });
                    b.add(world.registry(), Vel { dx, dy });
                    let e = world.spawn_bundle(b);
                    alive.push(e);
                }
                EcsOp::Despawn(idx) => {
                    if !alive.is_empty() {
                        let idx = idx % alive.len();
                        let e = alive.remove(idx);
                        let _ = world.despawn(e);
                    }
                }
                EcsOp::InsertVel(idx, dx, dy) => {
                    if !alive.is_empty() {
                        let idx = idx % alive.len();
                        let _ = world.insert_component(alive[idx], Vel { dx, dy });
                    }
                }
                EcsOp::RemoveVel(idx) => {
                    if !alive.is_empty() {
                        let idx = idx % alive.len();
                        let _ = world.remove_component::<Vel>(alive[idx]);
                    }
                }
                EcsOp::QueryPos => {
                    let count = world.query::<(&Pos,)>().count();
                    prop_assert!(count <= alive.len());
                }
                EcsOp::QueryPosVel => {
                    let count = world.query::<(&Pos, &Vel)>().count();
                    prop_assert!(count <= alive.len());
                }
            }

            // Invariant: entity_count matches our tracking.
            prop_assert_eq!(world.entity_count(), alive.len());

            // Invariant: all alive entities are really alive.
            for &e in &alive {
                prop_assert!(world.is_alive(e));
            }
        }
    }

    /// Verify that generational IDs catch stale references immediately.
    ///
    /// After despawning an entity, any access using the old EntityId must
    /// return None/Err (even if the index has been recycled by a new spawn).
    #[test]
    fn stale_ids_detected_after_despawn_and_recycle(
        spawn_count in 1..20usize,
        despawn_indices in prop::collection::vec(0..20usize, 1..10),
    ) {
        let mut world = World::new();
        world.register_component::<Pos>("pos");

        let mut entities: Vec<EntityId> = Vec::new();
        for i in 0..spawn_count {
            entities.push(world.spawn_with(Pos { x: i as f32, y: 0.0 }));
        }

        let mut stale_ids: Vec<EntityId> = Vec::new();

        // Despawn some entities
        for &idx in &despawn_indices {
            if !entities.is_empty() {
                let idx = idx % entities.len();
                let e = entities.remove(idx);
                let _ = world.despawn(e);
                stale_ids.push(e);
            }
        }

        // Spawn new entities to recycle indices
        for _ in 0..stale_ids.len() {
            let new_e = world.spawn_with(Pos { x: 999.0, y: 999.0 });
            entities.push(new_e);
        }

        // Verify stale IDs are still detected as stale
        for &stale in &stale_ids {
            prop_assert!(!world.is_alive(stale));
            prop_assert_eq!(world.get_component::<Pos>(stale), None);
        }

        // Verify alive entities are all accessible
        for &e in &entities {
            prop_assert!(world.is_alive(e));
            prop_assert!(world.get_component::<Pos>(e).is_some());
        }
    }

    /// Verify that archetype migration preserves component data.
    ///
    /// When a component is inserted or removed, the entity migrates to a new
    /// archetype. All existing component data must be preserved exactly.
    #[test]
    fn archetype_migration_preserves_data(
        initial_x in finite_f32(),
        initial_y in finite_f32(),
        vel_dx in finite_f32(),
        vel_dy in finite_f32(),
        do_remove in proptest::bool::ANY,
    ) {
        let mut world = World::new();
        world.register_component::<Pos>("pos");
        world.register_component::<Vel>("vel");

        // Spawn with Pos only.
        let e = world.spawn_with(Pos { x: initial_x, y: initial_y });

        // Migrate to {Pos, Vel}.
        world.insert_component(e, Vel { dx: vel_dx, dy: vel_dy }).unwrap();

        // Pos must be preserved.
        let pos = world.get_component::<Pos>(e).unwrap();
        prop_assert_eq!(pos.x, initial_x);
        prop_assert_eq!(pos.y, initial_y);

        // Vel must be present.
        let vel = world.get_component::<Vel>(e).unwrap();
        prop_assert_eq!(vel.dx, vel_dx);
        prop_assert_eq!(vel.dy, vel_dy);

        if do_remove {
            // Migrate back to {Pos} by removing Vel.
            world.remove_component::<Vel>(e).unwrap();

            // Pos must still be preserved after reverse migration.
            let pos = world.get_component::<Pos>(e).unwrap();
            prop_assert_eq!(pos.x, initial_x);
            prop_assert_eq!(pos.y, initial_y);

            // Vel must be gone.
            prop_assert!(!world.has_component::<Vel>(e));
        }
    }

    /// Verify that multiple entities in the same archetype maintain independent data.
    #[test]
    fn multiple_entities_independent_data(
        count in 2..50usize,
    ) {
        let mut world = World::new();
        world.register_component::<Pos>("pos");

        let mut entities = Vec::new();
        for i in 0..count {
            let e = world.spawn_with(Pos { x: i as f32, y: (i * 2) as f32 });
            entities.push(e);
        }

        // Each entity has its own distinct data.
        for (i, &e) in entities.iter().enumerate() {
            let pos = world.get_component::<Pos>(e).unwrap();
            prop_assert_eq!(pos.x, i as f32);
            prop_assert_eq!(pos.y, (i * 2) as f32);
        }

        // Despawn a random middle entity and verify the rest is intact.
        if count > 2 {
            let mid = count / 2;
            let mid_e = entities.remove(mid);
            world.despawn(mid_e).unwrap();

            prop_assert_eq!(world.entity_count(), entities.len());

            for &e in &entities {
                prop_assert!(world.is_alive(e));
                prop_assert!(world.get_component::<Pos>(e).is_some());
            }
        }
    }
}
