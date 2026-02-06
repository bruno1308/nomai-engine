//! Tiered entity identity for the Nomai ECS.
//!
//! Every entity spawned through the public API must declare an identity tier.
//! This enables the manifest pipeline to provide the right level of detail:
//!
//! - [`IdentityTier::Semantic`]: Full manifest presence, full causality tracking.
//!   Used for game-meaningful entities like player, enemies, items, UI elements.
//!
//! - [`IdentityTier::Pooled`]: Type-level aggregation in manifest, instance data
//!   aggregated. Used for repeated entities like bullets, coins, tiles.
//!
//! The [`Identity`] enum is stored as a built-in component on every entity
//! created via [`World::spawn_semantic`](crate::world::World::spawn_semantic) or
//! [`World::spawn_pooled`](crate::world::World::spawn_pooled).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// IdentityTier
// ---------------------------------------------------------------------------

/// The identity tier an entity declares at spawn time.
///
/// Determines how the entity is represented in the manifest and how much
/// causality tracking overhead it incurs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IdentityTier {
    /// Full manifest presence, full causality tracking.
    /// For game-meaningful entities: player, enemies, items, UI elements.
    Semantic,
    /// Type-level aggregation in manifest. Instance data aggregated.
    /// For repeated entities: bullets, coins, tiles.
    Pooled,
}

// ---------------------------------------------------------------------------
// SystemId
// ---------------------------------------------------------------------------

/// Unique numeric ID for a system, used in causality tracking.
///
/// Every command emitted by a system carries the [`SystemId`] of its origin,
/// enabling the manifest to trace state changes back to their source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SystemId(pub u32);

impl SystemId {
    /// The player-spawner system (e.g., initial scene setup).
    pub const PLAYER_SPAWNER: SystemId = SystemId(1);
    /// WASM gameplay logic systems.
    pub const WASM_GAMEPLAY: SystemId = SystemId(100);
    /// Physics system (rapier2d integration).
    pub const PHYSICS: SystemId = SystemId(200);
    /// Engine-internal operations (not tied to a user-visible system).
    pub const ENGINE_INTERNAL: SystemId = SystemId(0);
}

// ---------------------------------------------------------------------------
// EntityIdentity
// ---------------------------------------------------------------------------

/// Identity metadata for a [`IdentityTier::Semantic`] entity.
///
/// Semantic entities get full individual tracking in the manifest. Each one
/// carries enough metadata to trace back to the game design intent that
/// motivated its existence.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityIdentity {
    /// Broad category: `"character"`, `"projectile"`, `"ui"`, etc.
    pub entity_type: String,
    /// Specific role within the category: `"player"`, `"enemy.melee"`,
    /// `"health_bar"`, etc.
    pub role: String,
    /// Which system spawned this entity.
    pub spawned_by: SystemId,
    /// Optional link back to a game-design requirement or intent spec.
    pub requirement_id: Option<String>,
}

// ---------------------------------------------------------------------------
// PoolIdentity
// ---------------------------------------------------------------------------

/// Identity metadata for a [`IdentityTier::Pooled`] entity.
///
/// Pooled entities are aggregated at the type level in the manifest. Individual
/// instances share a pool type and variant but are not individually tracked.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PoolIdentity {
    /// The pool category: `"destructible"`, `"collectible"`, `"particle"`, etc.
    pub pool_type: String,
    /// The specific variant within the pool: `"brick"`, `"coin"`, etc.
    pub variant: String,
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Combined identity info stored as a component on every entity.
///
/// This is automatically attached by [`World::spawn_semantic`](crate::world::World::spawn_semantic)
/// and [`World::spawn_pooled`](crate::world::World::spawn_pooled). It is a
/// built-in component type that does not need manual registration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Identity {
    /// A semantic-tier entity with full individual tracking.
    Semantic(EntityIdentity),
    /// A pooled-tier entity with type-level aggregation.
    Pooled(PoolIdentity),
}

impl Identity {
    /// Returns the [`IdentityTier`] of this identity.
    pub fn tier(&self) -> IdentityTier {
        match self {
            Identity::Semantic(_) => IdentityTier::Semantic,
            Identity::Pooled(_) => IdentityTier::Pooled,
        }
    }

    /// Returns the role string if this is a [`Identity::Semantic`] identity,
    /// or `None` for pooled entities.
    pub fn role(&self) -> Option<&str> {
        match self {
            Identity::Semantic(eid) => Some(&eid.role),
            Identity::Pooled(_) => None,
        }
    }

    /// Returns the entity type string for semantic, or pool type for pooled.
    pub fn type_name(&self) -> &str {
        match self {
            Identity::Semantic(eid) => &eid.entity_type,
            Identity::Pooled(pid) => &pid.pool_type,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_tier_from_semantic() {
        let identity = Identity::Semantic(EntityIdentity {
            entity_type: "character".to_owned(),
            role: "player".to_owned(),
            spawned_by: SystemId::PLAYER_SPAWNER,
            requirement_id: Some("REQ-001".to_owned()),
        });
        assert_eq!(identity.tier(), IdentityTier::Semantic);
        assert_eq!(identity.role(), Some("player"));
        assert_eq!(identity.type_name(), "character");
    }

    #[test]
    fn identity_tier_from_pooled() {
        let identity = Identity::Pooled(PoolIdentity {
            pool_type: "destructible".to_owned(),
            variant: "brick".to_owned(),
        });
        assert_eq!(identity.tier(), IdentityTier::Pooled);
        assert_eq!(identity.role(), None);
        assert_eq!(identity.type_name(), "destructible");
    }

    #[test]
    fn system_id_constants() {
        assert_eq!(SystemId::ENGINE_INTERNAL.0, 0);
        assert_eq!(SystemId::PLAYER_SPAWNER.0, 1);
        assert_eq!(SystemId::WASM_GAMEPLAY.0, 100);
        assert_eq!(SystemId::PHYSICS.0, 200);
    }

    #[test]
    fn identity_serialization_roundtrip() {
        let semantic = Identity::Semantic(EntityIdentity {
            entity_type: "character".to_owned(),
            role: "player".to_owned(),
            spawned_by: SystemId::PLAYER_SPAWNER,
            requirement_id: None,
        });
        let json = serde_json::to_string(&semantic).unwrap();
        let deserialized: Identity = serde_json::from_str(&json).unwrap();
        assert_eq!(semantic, deserialized);

        let pooled = Identity::Pooled(PoolIdentity {
            pool_type: "collectible".to_owned(),
            variant: "coin".to_owned(),
        });
        let json = serde_json::to_string(&pooled).unwrap();
        let deserialized: Identity = serde_json::from_str(&json).unwrap();
        assert_eq!(pooled, deserialized);
    }

    #[test]
    fn identity_tier_equality() {
        assert_eq!(IdentityTier::Semantic, IdentityTier::Semantic);
        assert_eq!(IdentityTier::Pooled, IdentityTier::Pooled);
        assert_ne!(IdentityTier::Semantic, IdentityTier::Pooled);
    }
}
