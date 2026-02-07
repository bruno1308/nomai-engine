//! Integration tests for ECS World snapshot/restore.

use nomai_ecs::prelude::*;

// -- test component types ---------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Velocity {
    dx: f32,
    dy: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Health(u32);

fn setup_world() -> World {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<Health>("health");
    world
}

// -- helpers ----------------------------------------------------------------

fn player_identity() -> EntityIdentity {
    EntityIdentity {
        entity_type: "character".to_owned(),
        role: "player".to_owned(),
        spawned_by: SystemId::PLAYER_SPAWNER,
        requirement_id: Some("REQ-001".to_owned()),
    }
}

fn enemy_identity(variant: &str) -> EntityIdentity {
    EntityIdentity {
        entity_type: "character".to_owned(),
        role: format!("enemy.{variant}"),
        spawned_by: SystemId::WASM_GAMEPLAY,
        requirement_id: None,
    }
}

fn brick_pool_identity() -> PoolIdentity {
    PoolIdentity {
        pool_type: "destructible".to_owned(),
        variant: "brick".to_owned(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn snapshot_empty_world() {
    let world = setup_world();
    let snapshot = world.capture_snapshot();

    assert!(snapshot.entities.is_empty());
    assert!(snapshot.allocator.generations.is_empty());
    assert!(snapshot.allocator.alive.is_empty());
    assert!(snapshot.allocator.free_indices.is_empty());

    // Restore into a fresh world.
    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    assert_eq!(world2.entity_count(), 0);
}

#[test]
fn snapshot_single_entity_roundtrip() {
    let mut world = setup_world();

    let mut bundle = ComponentBundle::new();
    bundle.add(world.registry(), Position { x: 10.0, y: 20.0 });
    bundle.add(world.registry(), Velocity { dx: 1.0, dy: -1.0 });
    let entity = world.spawn_bundle(bundle);

    let snapshot = world.capture_snapshot();
    assert_eq!(snapshot.entities.len(), 1);
    assert_eq!(snapshot.entities[0].entity_id, entity);

    // Restore into a fresh world (same component registrations).
    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    assert_eq!(world2.entity_count(), 1);
    assert!(world2.is_alive(entity));
    assert_eq!(
        world2.get_component::<Position>(entity),
        Some(&Position { x: 10.0, y: 20.0 })
    );
    assert_eq!(
        world2.get_component::<Velocity>(entity),
        Some(&Velocity { dx: 1.0, dy: -1.0 })
    );
}

#[test]
fn snapshot_multiple_entities_multiple_archetypes() {
    let mut world = setup_world();

    // Archetype 1: Position + Velocity
    let mut b1 = ComponentBundle::new();
    b1.add(world.registry(), Position { x: 1.0, y: 2.0 });
    b1.add(world.registry(), Velocity { dx: 3.0, dy: 4.0 });
    let e1 = world.spawn_bundle(b1);

    // Archetype 2: Position + Health
    let mut b2 = ComponentBundle::new();
    b2.add(world.registry(), Position { x: 5.0, y: 6.0 });
    b2.add(world.registry(), Health(100));
    let e2 = world.spawn_bundle(b2);

    // Archetype 3: Position only
    let e3 = world.spawn_with(Position { x: 7.0, y: 8.0 });

    let snapshot = world.capture_snapshot();
    assert_eq!(snapshot.entities.len(), 3);

    // Restore.
    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    assert_eq!(world2.entity_count(), 3);

    // Verify e1: Position + Velocity
    assert_eq!(
        world2.get_component::<Position>(e1),
        Some(&Position { x: 1.0, y: 2.0 })
    );
    assert_eq!(
        world2.get_component::<Velocity>(e1),
        Some(&Velocity { dx: 3.0, dy: 4.0 })
    );
    assert!(!world2.has_component::<Health>(e1));

    // Verify e2: Position + Health
    assert_eq!(
        world2.get_component::<Position>(e2),
        Some(&Position { x: 5.0, y: 6.0 })
    );
    assert_eq!(world2.get_component::<Health>(e2), Some(&Health(100)));
    assert!(!world2.has_component::<Velocity>(e2));

    // Verify e3: Position only
    assert_eq!(
        world2.get_component::<Position>(e3),
        Some(&Position { x: 7.0, y: 8.0 })
    );
    assert!(!world2.has_component::<Velocity>(e3));
    assert!(!world2.has_component::<Health>(e3));
}

#[test]
fn snapshot_preserves_entity_ids() {
    let mut world = setup_world();

    let e1 = world.spawn_with(Position { x: 1.0, y: 1.0 });
    let e2 = world.spawn_with(Position { x: 2.0, y: 2.0 });
    let e3 = world.spawn_with(Position { x: 3.0, y: 3.0 });

    let snapshot = world.capture_snapshot();

    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    // The exact same EntityId values must be alive after restore.
    assert!(world2.is_alive(e1));
    assert!(world2.is_alive(e2));
    assert!(world2.is_alive(e3));

    // Verify index and generation are preserved.
    assert_eq!(e1.index(), 0);
    assert_eq!(e1.generation(), 0);
    assert_eq!(e2.index(), 1);
    assert_eq!(e2.generation(), 0);
    assert_eq!(e3.index(), 2);
    assert_eq!(e3.generation(), 0);

    // Components accessible via the same EntityId.
    assert_eq!(
        world2.get_component::<Position>(e1),
        Some(&Position { x: 1.0, y: 1.0 })
    );
    assert_eq!(
        world2.get_component::<Position>(e2),
        Some(&Position { x: 2.0, y: 2.0 })
    );
    assert_eq!(
        world2.get_component::<Position>(e3),
        Some(&Position { x: 3.0, y: 3.0 })
    );
}

#[test]
fn snapshot_preserves_identity_tiers() {
    let mut world = setup_world();

    // Spawn a semantic entity with components.
    let mut sem_bundle = ComponentBundle::new();
    sem_bundle.add(world.registry(), Position { x: 10.0, y: 20.0 });
    let semantic_e = world.spawn_semantic(player_identity(), sem_bundle).unwrap();

    // Spawn a pooled entity with components.
    let mut pool_bundle = ComponentBundle::new();
    pool_bundle.add(world.registry(), Position { x: 50.0, y: 60.0 });
    let pooled_e = world
        .spawn_pooled(brick_pool_identity(), pool_bundle)
        .unwrap();

    let snapshot = world.capture_snapshot();

    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    // Semantic entity preserved.
    assert!(world2.is_alive(semantic_e));
    let identity = world2.get_identity(semantic_e).unwrap();
    assert_eq!(identity.tier(), IdentityTier::Semantic);
    match identity {
        Identity::Semantic(eid) => {
            assert_eq!(eid.entity_type, "character");
            assert_eq!(eid.role, "player");
            assert_eq!(eid.spawned_by, SystemId::PLAYER_SPAWNER);
            assert_eq!(eid.requirement_id.as_deref(), Some("REQ-001"));
        }
        Identity::Pooled(_) => panic!("expected Semantic, got Pooled"),
    }
    assert_eq!(
        world2.get_component::<Position>(semantic_e),
        Some(&Position { x: 10.0, y: 20.0 })
    );

    // Pooled entity preserved.
    assert!(world2.is_alive(pooled_e));
    let identity = world2.get_identity(pooled_e).unwrap();
    assert_eq!(identity.tier(), IdentityTier::Pooled);
    match identity {
        Identity::Pooled(pid) => {
            assert_eq!(pid.pool_type, "destructible");
            assert_eq!(pid.variant, "brick");
        }
        Identity::Semantic(_) => panic!("expected Pooled, got Semantic"),
    }
    assert_eq!(
        world2.get_component::<Position>(pooled_e),
        Some(&Position { x: 50.0, y: 60.0 })
    );
}

#[test]
fn snapshot_with_despawned_entities_preserves_allocator() {
    let mut world = setup_world();

    let e1 = world.spawn_with(Position { x: 1.0, y: 1.0 });
    let e2 = world.spawn_with(Position { x: 2.0, y: 2.0 });
    let e3 = world.spawn_with(Position { x: 3.0, y: 3.0 });

    // Despawn e2 -- generation bumps, index goes to free list.
    world.despawn(e2).unwrap();

    let snapshot = world.capture_snapshot();

    // Verify allocator state in snapshot.
    assert_eq!(snapshot.allocator.generations.len(), 3);
    assert!(snapshot.allocator.alive[0]); // e1 alive
    assert!(!snapshot.allocator.alive[1]); // e2 dead
    assert!(snapshot.allocator.alive[2]); // e3 alive
    assert_eq!(snapshot.allocator.generations[1], 1); // e2's generation bumped
    assert_eq!(snapshot.allocator.free_indices, vec![1]); // e2's index in free list

    // Only 2 alive entities in snapshot.
    assert_eq!(snapshot.entities.len(), 2);

    // Restore.
    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    assert_eq!(world2.entity_count(), 2);
    assert!(world2.is_alive(e1));
    assert!(!world2.is_alive(e2)); // e2 should still be dead
    assert!(world2.is_alive(e3));

    // The stale e2 handle should not resolve.
    assert_eq!(world2.get_component::<Position>(e2), None);

    // Allocating a new entity should reuse the free index with bumped generation.
    let e_new = world2.spawn_with(Position { x: 99.0, y: 99.0 });
    assert_eq!(e_new.index(), 1); // reuses e2's index
    assert_eq!(e_new.generation(), 1); // generation was bumped on despawn
}

#[test]
fn snapshot_json_serializable() {
    let mut world = setup_world();

    let mut bundle = ComponentBundle::new();
    bundle.add(world.registry(), Position { x: 42.0, y: 99.0 });
    bundle.add(world.registry(), Health(75));
    world.spawn_bundle(bundle);

    let snapshot = world.capture_snapshot();

    // Serialize to JSON string.
    let json_str = serde_json::to_string_pretty(&snapshot).unwrap();
    assert!(!json_str.is_empty());

    // Deserialize back.
    let restored_snapshot: WorldSnapshot = serde_json::from_str(&json_str).unwrap();

    assert_eq!(restored_snapshot.entities.len(), snapshot.entities.len());
    assert_eq!(
        restored_snapshot.allocator.generations,
        snapshot.allocator.generations
    );
    assert_eq!(restored_snapshot.allocator.alive, snapshot.allocator.alive);
    assert_eq!(
        restored_snapshot.allocator.free_indices,
        snapshot.allocator.free_indices
    );

    // Restore from the deserialized snapshot.
    let mut world2 = setup_world();
    world2.restore_from_snapshot(&restored_snapshot).unwrap();
    assert_eq!(world2.entity_count(), 1);
}

#[test]
fn snapshot_restore_into_same_world() {
    // Restore into the same world (not a fresh one) to verify it clears state.
    let mut world = setup_world();

    let e1 = world.spawn_with(Position { x: 1.0, y: 1.0 });
    let snapshot = world.capture_snapshot();

    // Mutate the world after snapshot.
    let e2 = world.spawn_with(Position { x: 2.0, y: 2.0 });
    world.despawn(e1).unwrap();

    // Now restore -- should get back to the snapshot state.
    world.restore_from_snapshot(&snapshot).unwrap();

    assert_eq!(world.entity_count(), 1);
    assert!(world.is_alive(e1));
    // e2 was spawned after snapshot, so it should not exist.
    assert!(!world.is_alive(e2));
    assert_eq!(
        world.get_component::<Position>(e1),
        Some(&Position { x: 1.0, y: 1.0 })
    );
}

#[test]
fn snapshot_with_mixed_identity_and_regular_entities() {
    let mut world = setup_world();

    // Regular entity (no identity -- spawned via low-level API).
    let e_regular = world.spawn_with(Position { x: 0.0, y: 0.0 });

    // Semantic entity.
    let mut sem_bundle = ComponentBundle::new();
    sem_bundle.add(world.registry(), Position { x: 10.0, y: 10.0 });
    sem_bundle.add(world.registry(), Velocity { dx: 1.0, dy: 2.0 });
    let e_semantic = world
        .spawn_semantic(enemy_identity("ranged"), sem_bundle)
        .unwrap();

    // Pooled entity.
    let mut pool_bundle = ComponentBundle::new();
    pool_bundle.add(world.registry(), Health(50));
    let e_pooled = world
        .spawn_pooled(brick_pool_identity(), pool_bundle)
        .unwrap();

    let snapshot = world.capture_snapshot();
    assert_eq!(snapshot.entities.len(), 3);

    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    assert_eq!(world2.entity_count(), 3);

    // Regular entity has no identity.
    assert!(world2.is_alive(e_regular));
    assert_eq!(
        world2.get_component::<Position>(e_regular),
        Some(&Position { x: 0.0, y: 0.0 })
    );

    // Semantic entity.
    assert!(world2.is_alive(e_semantic));
    assert_eq!(world2.get_tier(e_semantic).unwrap(), IdentityTier::Semantic);

    // Pooled entity.
    assert!(world2.is_alive(e_pooled));
    assert_eq!(world2.get_tier(e_pooled).unwrap(), IdentityTier::Pooled);
    assert_eq!(world2.get_component::<Health>(e_pooled), Some(&Health(50)));
}

#[test]
fn snapshot_determinism_two_captures_identical() {
    let mut world = setup_world();

    for i in 0..10 {
        let mut bundle = ComponentBundle::new();
        bundle.add(
            world.registry(),
            Position {
                x: i as f32,
                y: i as f32 * 2.0,
            },
        );
        bundle.add(world.registry(), Velocity { dx: 1.0, dy: -1.0 });
        world.spawn_bundle(bundle);
    }

    let snap1 = world.capture_snapshot();
    let snap2 = world.capture_snapshot();

    // Both snapshots should be identical JSON.
    let json1 = serde_json::to_string(&snap1).unwrap();
    let json2 = serde_json::to_string(&snap2).unwrap();
    assert_eq!(
        json1, json2,
        "two captures of the same state should produce identical JSON"
    );
}

#[test]
fn snapshot_roundtrip_then_query_works() {
    let mut world = setup_world();

    for i in 0..5 {
        let mut bundle = ComponentBundle::new();
        bundle.add(
            world.registry(),
            Position {
                x: i as f32,
                y: 0.0,
            },
        );
        bundle.add(world.registry(), Velocity { dx: 1.0, dy: 0.0 });
        world.spawn_bundle(bundle);
    }

    let snapshot = world.capture_snapshot();

    let mut world2 = setup_world();
    world2.restore_from_snapshot(&snapshot).unwrap();

    // Queries should work after restore.
    let count = world2.query::<(&Position, &Velocity)>().count();
    assert_eq!(count, 5);

    // Mutable queries should work too.
    for (_entity, (pos, _vel)) in world2.query_mut::<(&mut Position, &Velocity)>() {
        pos.x += 100.0;
    }

    // Verify mutation took effect.
    let positions: Vec<f32> = world2
        .query::<(&Position,)>()
        .map(|(_, (pos,))| pos.x)
        .collect();
    for p in positions {
        assert!(p >= 100.0, "position should have been incremented by 100");
    }
}
