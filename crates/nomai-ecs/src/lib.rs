//! Nomai ECS -- Archetype-based Entity Component System with tiered identity.
//!
//! This crate provides the core ECS for the Nomai Engine. Entities are stored
//! in archetypes (one per unique set of component types) using a Structure-of-
//! Arrays (SoA) layout for cache-friendly iteration. Generational entity IDs
//! enable immediate stale-reference detection.
//!
//! # Quick Start
//!
//! ```
//! use nomai_ecs::prelude::*;
//!
//! #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
//! struct Position { x: f32, y: f32 }
//!
//! #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
//! struct Velocity { dx: f32, dy: f32 }
//!
//! let mut world = World::new();
//! world.register_component::<Position>("position");
//! world.register_component::<Velocity>("velocity");
//!
//! let mut bundle = ComponentBundle::new();
//! bundle.add(world.registry(), Position { x: 0.0, y: 0.0 });
//! bundle.add(world.registry(), Velocity { dx: 1.0, dy: 0.0 });
//! let entity = world.spawn_bundle(bundle);
//!
//! assert_eq!(world.get_component::<Position>(entity), Some(&Position { x: 0.0, y: 0.0 }));
//! ```

#![deny(unsafe_code)]

#[allow(unsafe_code)]
pub mod archetype;
pub mod command;
pub mod component;
pub mod entity;
pub mod identity;
#[allow(unsafe_code)]
pub mod query;
#[allow(unsafe_code)]
pub mod world;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors produced by ECS operations.
#[derive(Debug, thiserror::Error)]
pub enum EcsError {
    /// The entity does not exist (stale generation or never allocated).
    #[error("entity {entity:?} does not exist (stale or never allocated)")]
    StaleEntity {
        entity: entity::EntityId,
    },

    /// A component type was referenced that has not been registered.
    #[error("component type '{name}' not registered. Registered components: [{registered}]")]
    UnknownComponent {
        name: String,
        registered: String,
    },

    /// Deserialization of a component value failed.
    #[error("failed to deserialize component '{component}': {details}")]
    ComponentDeserializationError {
        component: String,
        details: String,
    },
}

// ---------------------------------------------------------------------------
// Prelude
// ---------------------------------------------------------------------------

/// Convenience re-exports for common usage.
pub mod prelude {
    pub use crate::archetype::{Archetype, ArchetypeId};
    pub use crate::command::{ApplyReport, CausalReason, Command, CommandBuffer, CommandKind};
    pub use crate::component::{ComponentInfo, ComponentRegistry, ComponentTypeId};
    pub use crate::entity::EntityId;
    pub use crate::identity::{EntityIdentity, Identity, IdentityTier, PoolIdentity, SystemId};
    pub use crate::query::{Query, QueryItem, QueryIter, QueryIterMut};
    pub use crate::world::{ComponentBundle, World};
    pub use crate::EcsError;
}

// ---------------------------------------------------------------------------
// Integration Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::prelude::*;

    // -- test component types -----------------------------------------------

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

    // -- spawn / despawn integration ----------------------------------------

    #[test]
    fn spawn_entities_with_components_and_query_back() {
        let mut world = setup_world();

        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 1.0, y: 2.0 });
        b.add(world.registry(), Velocity { dx: 3.0, dy: 4.0 });
        let e = world.spawn_bundle(b);

        assert_eq!(
            world.get_component::<Position>(e),
            Some(&Position { x: 1.0, y: 2.0 })
        );
        assert_eq!(
            world.get_component::<Velocity>(e),
            Some(&Velocity { dx: 3.0, dy: 4.0 })
        );
    }

    #[test]
    fn despawn_entity_verify_gone() {
        let mut world = setup_world();
        let e = world.spawn_with(Position { x: 0.0, y: 0.0 });
        world.despawn(e).unwrap();
        assert!(!world.is_alive(e));
        assert_eq!(world.get_component::<Position>(e), None);
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn insert_component_triggers_migration() {
        let mut world = setup_world();
        let e = world.spawn_with(Position { x: 1.0, y: 2.0 });
        let arch_count_before = world.archetype_count();

        world
            .insert_component(e, Velocity { dx: 5.0, dy: 6.0 })
            .unwrap();

        assert!(world.has_component::<Velocity>(e));
        assert_eq!(
            world.get_component::<Position>(e),
            Some(&Position { x: 1.0, y: 2.0 })
        );
        // A new archetype was created for {Position, Velocity}.
        assert!(world.archetype_count() > arch_count_before);
    }

    #[test]
    fn remove_component_triggers_migration() {
        let mut world = setup_world();
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 1.0, y: 2.0 });
        b.add(world.registry(), Velocity { dx: 3.0, dy: 4.0 });
        let e = world.spawn_bundle(b);

        world.remove_component::<Velocity>(e).unwrap();

        assert!(!world.has_component::<Velocity>(e));
        assert_eq!(
            world.get_component::<Position>(e),
            Some(&Position { x: 1.0, y: 2.0 })
        );
    }

    #[test]
    fn get_set_components() {
        let mut world = setup_world();
        let e = world.spawn_with(Position { x: 0.0, y: 0.0 });
        if let Some(pos) = world.get_component_mut::<Position>(e) {
            pos.x = 42.0;
            pos.y = 99.0;
        }
        assert_eq!(
            world.get_component::<Position>(e),
            Some(&Position { x: 42.0, y: 99.0 })
        );
    }

    // -- query integration --------------------------------------------------

    #[test]
    fn query_matching_entities_only() {
        let mut world = setup_world();

        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 1.0, y: 2.0 });
        b.add(world.registry(), Velocity { dx: 3.0, dy: 4.0 });
        let e1 = world.spawn_bundle(b);

        let _e2 = world.spawn_with(Position { x: 10.0, y: 20.0 });

        let results: Vec<_> = world.query::<(&Position, &Velocity)>().collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, e1);
    }

    #[test]
    fn query_skips_entities_missing_required() {
        let mut world = setup_world();
        for i in 0..5 {
            world.spawn_with(Position {
                x: i as f32,
                y: 0.0,
            });
        }
        let results: Vec<_> = world.query::<(&Position, &Velocity)>().collect();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn mutable_query_modifies_components() {
        let mut world = setup_world();
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 0.0, y: 0.0 });
        b.add(world.registry(), Velocity { dx: 1.0, dy: 2.0 });
        let e = world.spawn_bundle(b);

        for (_entity, (pos, vel)) in world.query_mut::<(&mut Position, &Velocity)>() {
            pos.x += vel.dx;
            pos.y += vel.dy;
        }

        assert_eq!(
            world.get_component::<Position>(e),
            Some(&Position { x: 1.0, y: 2.0 })
        );
    }

    // -- scale test ---------------------------------------------------------

    #[test]
    fn scale_10k_entities() {
        let mut world = setup_world();

        // Spawn 10K entities with Position + Velocity.
        let mut entities = Vec::with_capacity(10_000);
        for i in 0..10_000u32 {
            let mut b = ComponentBundle::new();
            b.add(
                world.registry(),
                Position {
                    x: i as f32,
                    y: i as f32 * 2.0,
                },
            );
            b.add(world.registry(), Velocity { dx: 1.0, dy: -1.0 });
            let e = world.spawn_bundle(b);
            entities.push(e);
        }

        // Query all, verify count.
        let count = world.query::<(&Position, &Velocity)>().count();
        assert_eq!(count, 10_000);

        // Modify all velocities via mutable query.
        for (_entity, (vel,)) in world.query_mut::<(&mut Velocity,)>() {
            vel.dx *= 2.0;
            vel.dy *= 2.0;
        }

        // Verify modification.
        let vel = world.get_component::<Velocity>(entities[0]).unwrap();
        assert_eq!(vel.dx, 2.0);
        assert_eq!(vel.dy, -2.0);

        // Despawn half.
        for e in entities.iter().take(5_000) {
            world.despawn(*e).unwrap();
        }

        // Query again, verify count.
        let count = world.query::<(&Position, &Velocity)>().count();
        assert_eq!(count, 5_000);
        assert_eq!(world.entity_count(), 5_000);
    }

    // -- stale entity tests -------------------------------------------------

    #[test]
    fn stale_entity_despawn_returns_error() {
        let mut world = setup_world();
        let e = world.spawn_with(Position { x: 0.0, y: 0.0 });
        world.despawn(e).unwrap();
        assert!(world.despawn(e).is_err());
    }

    #[test]
    fn insert_on_stale_entity_returns_error() {
        let mut world = setup_world();
        let e = world.spawn_with(Position { x: 0.0, y: 0.0 });
        world.despawn(e).unwrap();
        let result = world.insert_component(e, Velocity { dx: 1.0, dy: 1.0 });
        assert!(result.is_err());
    }

    // -- multiple entities in same archetype --------------------------------

    #[test]
    fn multiple_entities_same_archetype() {
        let mut world = setup_world();
        let e1 = world.spawn_with(Position { x: 1.0, y: 1.0 });
        let e2 = world.spawn_with(Position { x: 2.0, y: 2.0 });
        let e3 = world.spawn_with(Position { x: 3.0, y: 3.0 });

        assert_eq!(
            world.get_component::<Position>(e1),
            Some(&Position { x: 1.0, y: 1.0 })
        );
        assert_eq!(
            world.get_component::<Position>(e2),
            Some(&Position { x: 2.0, y: 2.0 })
        );
        assert_eq!(
            world.get_component::<Position>(e3),
            Some(&Position { x: 3.0, y: 3.0 })
        );

        // Despawn middle entity, check remaining are correct.
        world.despawn(e2).unwrap();
        assert_eq!(world.entity_count(), 2);
        assert_eq!(
            world.get_component::<Position>(e1),
            Some(&Position { x: 1.0, y: 1.0 })
        );
        assert_eq!(
            world.get_component::<Position>(e3),
            Some(&Position { x: 3.0, y: 3.0 })
        );
    }

    #[test]
    fn insert_component_overwrite() {
        let mut world = setup_world();
        let e = world.spawn_with(Position { x: 1.0, y: 2.0 });
        // Insert same component type again -- should overwrite.
        world
            .insert_component(e, Position { x: 99.0, y: 100.0 })
            .unwrap();
        assert_eq!(
            world.get_component::<Position>(e),
            Some(&Position { x: 99.0, y: 100.0 })
        );
    }

    // -- tiered identity tests ----------------------------------------------

    /// Helper to create a standard semantic identity for testing.
    fn player_identity() -> EntityIdentity {
        EntityIdentity {
            entity_type: "character".to_owned(),
            role: "player".to_owned(),
            spawned_by: SystemId::PLAYER_SPAWNER,
            requirement_id: Some("REQ-001".to_owned()),
        }
    }

    /// Helper to create an enemy identity for testing.
    fn enemy_identity(variant: &str) -> EntityIdentity {
        EntityIdentity {
            entity_type: "character".to_owned(),
            role: format!("enemy.{variant}"),
            spawned_by: SystemId::WASM_GAMEPLAY,
            requirement_id: None,
        }
    }

    /// Helper to create a pooled brick identity for testing.
    fn brick_pool_identity() -> PoolIdentity {
        PoolIdentity {
            pool_type: "destructible".to_owned(),
            variant: "brick".to_owned(),
        }
    }

    /// Helper to create a pooled coin identity for testing.
    fn coin_pool_identity() -> PoolIdentity {
        PoolIdentity {
            pool_type: "collectible".to_owned(),
            variant: "coin".to_owned(),
        }
    }

    #[test]
    fn spawn_semantic_entity_identity_retrievable() {
        let mut world = setup_world();

        let bundle = ComponentBundle::new();
        let e = world.spawn_semantic(player_identity(), bundle).unwrap();

        // Identity should be retrievable.
        let identity = world.get_identity(e).unwrap();
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
    }

    #[test]
    fn spawn_pooled_entity_identity_retrievable() {
        let mut world = setup_world();

        let bundle = ComponentBundle::new();
        let e = world.spawn_pooled(brick_pool_identity(), bundle).unwrap();

        let identity = world.get_identity(e).unwrap();
        assert_eq!(identity.tier(), IdentityTier::Pooled);

        match identity {
            Identity::Pooled(pid) => {
                assert_eq!(pid.pool_type, "destructible");
                assert_eq!(pid.variant, "brick");
            }
            Identity::Semantic(_) => panic!("expected Pooled, got Semantic"),
        }
    }

    #[test]
    fn tier_query_returns_correct_entities() {
        let mut world = setup_world();

        // Spawn 2 semantic entities.
        let s1 = world
            .spawn_semantic(player_identity(), ComponentBundle::new())
            .unwrap();
        let s2 = world
            .spawn_semantic(enemy_identity("melee"), ComponentBundle::new())
            .unwrap();

        // Spawn 3 pooled entities.
        let p1 = world
            .spawn_pooled(brick_pool_identity(), ComponentBundle::new())
            .unwrap();
        let p2 = world
            .spawn_pooled(brick_pool_identity(), ComponentBundle::new())
            .unwrap();
        let p3 = world
            .spawn_pooled(coin_pool_identity(), ComponentBundle::new())
            .unwrap();

        // Query by tier.
        let semantic = world.entities_by_tier(IdentityTier::Semantic);
        assert_eq!(semantic.len(), 2);
        assert!(semantic.contains(&s1));
        assert!(semantic.contains(&s2));

        let pooled = world.entities_by_tier(IdentityTier::Pooled);
        assert_eq!(pooled.len(), 3);
        assert!(pooled.contains(&p1));
        assert!(pooled.contains(&p2));
        assert!(pooled.contains(&p3));
    }

    #[test]
    fn role_query_returns_correct_entities() {
        let mut world = setup_world();

        let player = world
            .spawn_semantic(player_identity(), ComponentBundle::new())
            .unwrap();
        let melee = world
            .spawn_semantic(enemy_identity("melee"), ComponentBundle::new())
            .unwrap();
        let _ranged = world
            .spawn_semantic(enemy_identity("ranged"), ComponentBundle::new())
            .unwrap();

        // Also spawn some pooled entities -- should not appear in role queries.
        let _brick = world
            .spawn_pooled(brick_pool_identity(), ComponentBundle::new())
            .unwrap();

        // Query by role.
        let players = world.entities_by_role("player");
        assert_eq!(players.len(), 1);
        assert_eq!(players[0], player);

        let melee_enemies = world.entities_by_role("enemy.melee");
        assert_eq!(melee_enemies.len(), 1);
        assert_eq!(melee_enemies[0], melee);

        let ranged_enemies = world.entities_by_role("enemy.ranged");
        assert_eq!(ranged_enemies.len(), 1);

        // Non-existent role returns empty.
        let none = world.entities_by_role("nonexistent");
        assert!(none.is_empty());
    }

    #[test]
    fn identity_is_mandatory_on_tiered_spawn() {
        let mut world = setup_world();

        // spawn_semantic always attaches Identity.
        let e = world
            .spawn_semantic(player_identity(), ComponentBundle::new())
            .unwrap();
        assert!(world.has_component::<Identity>(e));

        // spawn_pooled always attaches Identity.
        let e2 = world
            .spawn_pooled(brick_pool_identity(), ComponentBundle::new())
            .unwrap();
        assert!(world.has_component::<Identity>(e2));
    }

    #[test]
    fn mixed_tier_with_components() {
        let mut world = setup_world();

        // Spawn a semantic entity with Position + Velocity.
        let mut sem_bundle = ComponentBundle::new();
        sem_bundle.add(world.registry(), Position { x: 10.0, y: 20.0 });
        sem_bundle.add(world.registry(), Velocity { dx: 1.0, dy: 2.0 });
        let semantic_e = world.spawn_semantic(player_identity(), sem_bundle).unwrap();

        // Spawn a pooled entity with only Position.
        let mut pool_bundle = ComponentBundle::new();
        pool_bundle.add(world.registry(), Position { x: 50.0, y: 60.0 });
        let pooled_e = world
            .spawn_pooled(brick_pool_identity(), pool_bundle)
            .unwrap();

        // Verify identity on both.
        assert_eq!(world.get_tier(semantic_e).unwrap(), IdentityTier::Semantic);
        assert_eq!(world.get_tier(pooled_e).unwrap(), IdentityTier::Pooled);

        // Verify regular components on semantic entity.
        assert_eq!(
            world.get_component::<Position>(semantic_e),
            Some(&Position { x: 10.0, y: 20.0 })
        );
        assert_eq!(
            world.get_component::<Velocity>(semantic_e),
            Some(&Velocity { dx: 1.0, dy: 2.0 })
        );

        // Verify regular components on pooled entity.
        assert_eq!(
            world.get_component::<Position>(pooled_e),
            Some(&Position { x: 50.0, y: 60.0 })
        );
        // Pooled entity should NOT have Velocity.
        assert!(!world.has_component::<Velocity>(pooled_e));

        // Both should show up in Position queries.
        let pos_count = world.query::<(&Position,)>().count();
        assert_eq!(pos_count, 2);

        // Only semantic entity should have Velocity.
        let vel_count = world.query::<(&Position, &Velocity)>().count();
        assert_eq!(vel_count, 1);
    }

    #[test]
    fn get_tier_returns_error_for_dead_entity() {
        let mut world = setup_world();
        let e = world
            .spawn_semantic(player_identity(), ComponentBundle::new())
            .unwrap();
        world.despawn(e).unwrap();
        assert!(world.get_tier(e).is_err());
    }

    #[test]
    fn entities_by_tier_empty_when_no_tiered_entities() {
        let world = setup_world();
        assert!(world.entities_by_tier(IdentityTier::Semantic).is_empty());
        assert!(world.entities_by_tier(IdentityTier::Pooled).is_empty());
    }

    #[test]
    fn despawn_tiered_entity_removes_from_queries() {
        let mut world = setup_world();

        let e1 = world
            .spawn_semantic(player_identity(), ComponentBundle::new())
            .unwrap();
        let _e2 = world
            .spawn_semantic(enemy_identity("melee"), ComponentBundle::new())
            .unwrap();

        assert_eq!(world.entities_by_tier(IdentityTier::Semantic).len(), 2);

        world.despawn(e1).unwrap();

        assert_eq!(world.entities_by_tier(IdentityTier::Semantic).len(), 1);
        assert!(world.entities_by_role("player").is_empty());
    }
}
