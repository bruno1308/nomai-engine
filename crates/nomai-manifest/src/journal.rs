//! Change journal for tracking every component mutation with causality metadata.
//!
//! The [`ChangeJournal`] records every component mutation that occurs during a
//! tick. Each entry ([`ComponentChange`]) captures the entity, component type,
//! old and new values, and full causality metadata (which system issued the
//! change, why, and at what command index).
//!
//! The journal is populated by the engine during command buffer application via
//! [`ChangeJournal::record_change`]. It is cleared at the start of each tick
//! via [`ChangeJournal::clear`].
//!
//! # Query API
//!
//! The journal supports querying changes by:
//! - **Entity**: [`ChangeJournal::changes_for_entity`]
//! - **Component type**: [`ChangeJournal::changes_for_component`]
//! - **System**: [`ChangeJournal::changes_by_system`]
//!
//! # Example
//!
//! ```
//! use nomai_manifest::journal::{ChangeJournal, ComponentChange};
//! use nomai_ecs::entity::EntityId;
//! use nomai_ecs::identity::SystemId;
//! use nomai_ecs::command::CausalReason;
//!
//! let mut journal = ChangeJournal::new();
//!
//! let entity = EntityId::new(0, 0);
//! journal.record_change(ComponentChange {
//!     entity_id: entity,
//!     component_type_name: "health".to_owned(),
//!     old_value: Some(serde_json::json!(100)),
//!     new_value: Some(serde_json::json!(75)),
//!     changed_by: SystemId(1),
//!     reason: CausalReason::GameRule("damage_applied".to_owned()),
//!     command_index: 0,
//!     tick: 1,
//! });
//!
//! assert_eq!(journal.len(), 1);
//! assert_eq!(journal.changes_for_entity(entity).count(), 1);
//! ```

use nomai_ecs::command::CausalReason;
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::SystemId;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ComponentChange
// ---------------------------------------------------------------------------

/// A single recorded component mutation with full causality metadata.
///
/// Each change captures:
/// - **What** changed: entity, component type, old and new values.
/// - **Who** changed it: the [`SystemId`] that issued the command.
/// - **Why** it changed: the [`CausalReason`] attached to the command.
/// - **When** it changed: the tick number and command index within that tick.
///
/// # Value semantics
///
/// - **Spawn** (component added for the first time): `old_value` is `None`,
///   `new_value` is `Some(...)`.
/// - **Modification** (component overwritten): both `old_value` and `new_value`
///   are `Some(...)`.
/// - **Removal / Despawn** (component removed): `old_value` is `Some(...)`,
///   `new_value` is `None`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentChange {
    /// The entity whose component was mutated.
    pub entity_id: EntityId,
    /// The registered name of the component type (e.g. `"health"`, `"position"`).
    pub component_type_name: String,
    /// The component value before the mutation, or `None` if the component
    /// did not exist (spawn / insert).
    pub old_value: Option<serde_json::Value>,
    /// The component value after the mutation, or `None` if the component
    /// was removed (remove / despawn).
    pub new_value: Option<serde_json::Value>,
    /// The system that issued the command producing this change.
    pub changed_by: SystemId,
    /// The causal reason attached to the command that produced this change.
    pub reason: CausalReason,
    /// The sequential command index within the tick's command buffer.
    pub command_index: u64,
    /// The tick number during which this change occurred.
    pub tick: u64,
}

// ---------------------------------------------------------------------------
// ChangeJournal
// ---------------------------------------------------------------------------

/// Accumulates [`ComponentChange`] entries during a tick and provides query
/// methods for downstream consumers (manifest pipeline, debugging tools).
///
/// The journal is designed to be cleared at the start of each tick via
/// [`clear`](Self::clear) and then populated incrementally as commands are
/// applied via [`record_change`](Self::record_change).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChangeJournal {
    changes: Vec<ComponentChange>,
}

impl ChangeJournal {
    /// Create a new, empty change journal.
    pub fn new() -> Self {
        Self {
            changes: Vec::new(),
        }
    }

    /// Record a single component change.
    ///
    /// This is called by the engine during command buffer application. Each
    /// applied command that mutates component state should produce one or more
    /// `ComponentChange` entries.
    pub fn record_change(&mut self, change: ComponentChange) {
        self.changes.push(change);
    }

    /// Clear all recorded changes. Called at the start of each tick.
    pub fn clear(&mut self) {
        self.changes.clear();
    }

    /// Returns the number of recorded changes.
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// Returns `true` if no changes have been recorded.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Returns a slice of all recorded changes in insertion order.
    pub fn all_changes(&self) -> &[ComponentChange] {
        &self.changes
    }

    /// Returns an iterator over changes that affected the given entity.
    pub fn changes_for_entity(&self, entity: EntityId) -> impl Iterator<Item = &ComponentChange> {
        self.changes.iter().filter(move |c| c.entity_id == entity)
    }

    /// Returns an iterator over changes for the given component type name.
    pub fn changes_for_component<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a ComponentChange> {
        self.changes
            .iter()
            .filter(move |c| c.component_type_name == name)
    }

    /// Returns an iterator over changes issued by the given system.
    pub fn changes_by_system(&self, system_id: SystemId) -> impl Iterator<Item = &ComponentChange> {
        self.changes
            .iter()
            .filter(move |c| c.changed_by == system_id)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
mod tests {
    use super::*;

    // -- helpers ------------------------------------------------------------

    fn entity(index: u32) -> EntityId {
        EntityId::new(index, 0)
    }

    /// Build a modification change (old + new present).
    fn modification(
        eid: EntityId,
        component: &str,
        old: serde_json::Value,
        new: serde_json::Value,
        system: SystemId,
        reason: CausalReason,
        cmd_index: u64,
        tick: u64,
    ) -> ComponentChange {
        ComponentChange {
            entity_id: eid,
            component_type_name: component.to_owned(),
            old_value: Some(old),
            new_value: Some(new),
            changed_by: system,
            reason,
            command_index: cmd_index,
            tick,
        }
    }

    /// Build a spawn/insert change (no old value).
    fn spawn_change(
        eid: EntityId,
        component: &str,
        new: serde_json::Value,
        system: SystemId,
        reason: CausalReason,
        cmd_index: u64,
        tick: u64,
    ) -> ComponentChange {
        ComponentChange {
            entity_id: eid,
            component_type_name: component.to_owned(),
            old_value: None,
            new_value: Some(new),
            changed_by: system,
            reason,
            command_index: cmd_index,
            tick,
        }
    }

    /// Build a removal/despawn change (no new value).
    fn removal_change(
        eid: EntityId,
        component: &str,
        old: serde_json::Value,
        system: SystemId,
        reason: CausalReason,
        cmd_index: u64,
        tick: u64,
    ) -> ComponentChange {
        ComponentChange {
            entity_id: eid,
            component_type_name: component.to_owned(),
            old_value: Some(old),
            new_value: None,
            changed_by: system,
            reason,
            command_index: cmd_index,
            tick,
        }
    }

    // -- 1. Empty journal ---------------------------------------------------

    #[test]
    fn empty_journal() {
        let journal = ChangeJournal::new();
        assert!(journal.is_empty());
        assert_eq!(journal.len(), 0);
        assert_eq!(journal.all_changes().len(), 0);
        assert_eq!(journal.changes_for_entity(entity(0)).count(), 0);
        assert_eq!(journal.changes_for_component("health").count(), 0);
        assert_eq!(journal.changes_by_system(SystemId(0)).count(), 0);
    }

    // -- 2. Record and retrieve a single change -----------------------------

    #[test]
    fn record_single_change() {
        let mut journal = ChangeJournal::new();
        let e = entity(0);

        journal.record_change(modification(
            e,
            "health",
            serde_json::json!(100),
            serde_json::json!(75),
            SystemId(1),
            CausalReason::GameRule("damage_applied".to_owned()),
            0,
            1,
        ));

        assert_eq!(journal.len(), 1);
        assert!(!journal.is_empty());

        let change = &journal.all_changes()[0];
        assert_eq!(change.entity_id, e);
        assert_eq!(change.component_type_name, "health");
        assert_eq!(change.old_value, Some(serde_json::json!(100)));
        assert_eq!(change.new_value, Some(serde_json::json!(75)));
        assert_eq!(change.changed_by, SystemId(1));
        assert_eq!(change.command_index, 0);
        assert_eq!(change.tick, 1);
    }

    // -- 3. Query by entity -------------------------------------------------

    #[test]
    fn query_by_entity() {
        let mut journal = ChangeJournal::new();
        let e0 = entity(0);
        let e1 = entity(1);
        let e2 = entity(2);

        // e0: 2 changes, e1: 1 change, e2: 1 change
        journal.record_change(modification(
            e0,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("move_right".to_owned()),
            0,
            1,
        ));
        journal.record_change(modification(
            e1,
            "health",
            serde_json::json!(50),
            serde_json::json!(25),
            SystemId(2),
            CausalReason::GameRule("damage".to_owned()),
            1,
            1,
        ));
        journal.record_change(modification(
            e0,
            "velocity",
            serde_json::json!({"dx": 0.0, "dy": 0.0}),
            serde_json::json!({"dx": 5.0, "dy": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("accelerate".to_owned()),
            2,
            1,
        ));
        journal.record_change(modification(
            e2,
            "position",
            serde_json::json!({"x": 10.0, "y": 10.0}),
            serde_json::json!({"x": 11.0, "y": 10.0}),
            SystemId(3),
            CausalReason::SystemInternal("ai_movement".to_owned()),
            3,
            1,
        ));

        assert_eq!(journal.len(), 4);

        let e0_changes: Vec<_> = journal.changes_for_entity(e0).collect();
        assert_eq!(e0_changes.len(), 2);
        assert_eq!(e0_changes[0].component_type_name, "position");
        assert_eq!(e0_changes[1].component_type_name, "velocity");

        let e1_changes: Vec<_> = journal.changes_for_entity(e1).collect();
        assert_eq!(e1_changes.len(), 1);
        assert_eq!(e1_changes[0].component_type_name, "health");

        let e2_changes: Vec<_> = journal.changes_for_entity(e2).collect();
        assert_eq!(e2_changes.len(), 1);

        // Non-existent entity returns empty.
        let e99_changes: Vec<_> = journal.changes_for_entity(entity(99)).collect();
        assert_eq!(e99_changes.len(), 0);
    }

    // -- 4. Query by component type -----------------------------------------

    #[test]
    fn query_by_component_type() {
        let mut journal = ChangeJournal::new();
        let e0 = entity(0);
        let e1 = entity(1);

        journal.record_change(modification(
            e0,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("move".to_owned()),
            0,
            1,
        ));
        journal.record_change(modification(
            e0,
            "health",
            serde_json::json!(100),
            serde_json::json!(80),
            SystemId(2),
            CausalReason::GameRule("damage".to_owned()),
            1,
            1,
        ));
        journal.record_change(modification(
            e1,
            "position",
            serde_json::json!({"x": 5.0, "y": 5.0}),
            serde_json::json!({"x": 6.0, "y": 5.0}),
            SystemId(3),
            CausalReason::SystemInternal("ai".to_owned()),
            2,
            1,
        ));

        let pos_changes: Vec<_> = journal.changes_for_component("position").collect();
        assert_eq!(pos_changes.len(), 2);
        assert_eq!(pos_changes[0].entity_id, e0);
        assert_eq!(pos_changes[1].entity_id, e1);

        let hp_changes: Vec<_> = journal.changes_for_component("health").collect();
        assert_eq!(hp_changes.len(), 1);
        assert_eq!(hp_changes[0].entity_id, e0);

        // Non-existent component returns empty.
        let none: Vec<_> = journal.changes_for_component("nonexistent").collect();
        assert_eq!(none.len(), 0);
    }

    // -- 5. Query by system -------------------------------------------------

    #[test]
    fn query_by_system() {
        let mut journal = ChangeJournal::new();
        let e0 = entity(0);

        let sys_physics = SystemId::PHYSICS;
        let sys_gameplay = SystemId::WASM_GAMEPLAY;

        journal.record_change(modification(
            e0,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            serde_json::json!({"x": 1.0, "y": 0.0}),
            sys_physics,
            CausalReason::SystemInternal("physics_step".to_owned()),
            0,
            1,
        ));
        journal.record_change(modification(
            e0,
            "health",
            serde_json::json!(100),
            serde_json::json!(50),
            sys_gameplay,
            CausalReason::GameRule("damage".to_owned()),
            1,
            1,
        ));
        journal.record_change(modification(
            e0,
            "velocity",
            serde_json::json!({"dx": 0.0, "dy": 0.0}),
            serde_json::json!({"dx": 1.0, "dy": 0.0}),
            sys_physics,
            CausalReason::SystemInternal("physics_step".to_owned()),
            2,
            1,
        ));

        let physics_changes: Vec<_> = journal.changes_by_system(sys_physics).collect();
        assert_eq!(physics_changes.len(), 2);
        assert_eq!(physics_changes[0].component_type_name, "position");
        assert_eq!(physics_changes[1].component_type_name, "velocity");

        let gameplay_changes: Vec<_> = journal.changes_by_system(sys_gameplay).collect();
        assert_eq!(gameplay_changes.len(), 1);
        assert_eq!(gameplay_changes[0].component_type_name, "health");

        // System with no changes returns empty.
        let engine: Vec<_> = journal
            .changes_by_system(SystemId::ENGINE_INTERNAL)
            .collect();
        assert_eq!(engine.len(), 0);
    }

    // -- 6. Causality metadata preserved ------------------------------------

    #[test]
    fn causality_metadata_preserved() {
        let mut journal = ChangeJournal::new();
        let e = entity(0);

        let reason_input = CausalReason::PlayerInput("jump".to_owned());
        let reason_collision = CausalReason::CollisionResponse(entity(0), entity(1));
        let reason_state = CausalReason::StateTransition {
            from: "grounded".to_owned(),
            to: "airborne".to_owned(),
        };
        let reason_timer = CausalReason::Timer("cooldown_expired".to_owned());

        journal.record_change(ComponentChange {
            entity_id: e,
            component_type_name: "velocity".to_owned(),
            old_value: Some(serde_json::json!({"dx": 0.0, "dy": 0.0})),
            new_value: Some(serde_json::json!({"dx": 0.0, "dy": 10.0})),
            changed_by: SystemId(1),
            reason: reason_input.clone(),
            command_index: 0,
            tick: 5,
        });
        journal.record_change(ComponentChange {
            entity_id: e,
            component_type_name: "health".to_owned(),
            old_value: Some(serde_json::json!(100)),
            new_value: Some(serde_json::json!(90)),
            changed_by: SystemId(2),
            reason: reason_collision.clone(),
            command_index: 1,
            tick: 5,
        });
        journal.record_change(ComponentChange {
            entity_id: e,
            component_type_name: "state".to_owned(),
            old_value: Some(serde_json::json!("grounded")),
            new_value: Some(serde_json::json!("airborne")),
            changed_by: SystemId(3),
            reason: reason_state.clone(),
            command_index: 2,
            tick: 5,
        });
        journal.record_change(ComponentChange {
            entity_id: e,
            component_type_name: "cooldown".to_owned(),
            old_value: Some(serde_json::json!(0)),
            new_value: Some(serde_json::json!(60)),
            changed_by: SystemId(4),
            reason: reason_timer.clone(),
            command_index: 3,
            tick: 5,
        });

        let changes = journal.all_changes();
        assert_eq!(changes.len(), 4);

        // Verify each entry preserves its causality metadata.
        assert_eq!(changes[0].changed_by, SystemId(1));
        assert_eq!(changes[0].reason, reason_input);
        assert_eq!(changes[0].command_index, 0);
        assert_eq!(changes[0].tick, 5);

        assert_eq!(changes[1].changed_by, SystemId(2));
        assert_eq!(changes[1].reason, reason_collision);
        assert_eq!(changes[1].command_index, 1);

        assert_eq!(changes[2].changed_by, SystemId(3));
        assert_eq!(changes[2].reason, reason_state);
        assert_eq!(changes[2].command_index, 2);

        assert_eq!(changes[3].changed_by, SystemId(4));
        assert_eq!(changes[3].reason, reason_timer);
        assert_eq!(changes[3].command_index, 3);
    }

    // -- 7. Clear at tick boundary ------------------------------------------

    #[test]
    fn clear_at_tick_boundary() {
        let mut journal = ChangeJournal::new();
        let e = entity(0);

        // Tick 1: record some changes.
        journal.record_change(modification(
            e,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("move".to_owned()),
            0,
            1,
        ));
        journal.record_change(modification(
            e,
            "health",
            serde_json::json!(100),
            serde_json::json!(90),
            SystemId(2),
            CausalReason::GameRule("damage".to_owned()),
            1,
            1,
        ));
        assert_eq!(journal.len(), 2);

        // Clear for tick 2.
        journal.clear();
        assert!(journal.is_empty());
        assert_eq!(journal.len(), 0);
        assert_eq!(journal.all_changes().len(), 0);
        assert_eq!(journal.changes_for_entity(e).count(), 0);
        assert_eq!(journal.changes_for_component("position").count(), 0);
        assert_eq!(journal.changes_by_system(SystemId(1)).count(), 0);

        // Tick 2: record new changes.
        journal.record_change(modification(
            e,
            "position",
            serde_json::json!({"x": 1.0, "y": 0.0}),
            serde_json::json!({"x": 2.0, "y": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("move".to_owned()),
            0,
            2,
        ));
        assert_eq!(journal.len(), 1);

        let change = &journal.all_changes()[0];
        assert_eq!(change.tick, 2);
        assert_eq!(
            change.old_value,
            Some(serde_json::json!({"x": 1.0, "y": 0.0}))
        );
    }

    // -- 8. Spawn (new_value only) ------------------------------------------

    #[test]
    fn spawn_change_new_value_only() {
        let mut journal = ChangeJournal::new();
        let e = entity(5);

        journal.record_change(spawn_change(
            e,
            "position",
            serde_json::json!({"x": 10.0, "y": 20.0}),
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("player_spawn".to_owned()),
            0,
            1,
        ));
        journal.record_change(spawn_change(
            e,
            "health",
            serde_json::json!(100),
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("player_spawn".to_owned()),
            1,
            1,
        ));

        assert_eq!(journal.len(), 2);

        let changes: Vec<_> = journal.changes_for_entity(e).collect();
        assert_eq!(changes.len(), 2);

        // Spawn: old_value is None, new_value is Some.
        assert!(changes[0].old_value.is_none());
        assert_eq!(
            changes[0].new_value,
            Some(serde_json::json!({"x": 10.0, "y": 20.0}))
        );

        assert!(changes[1].old_value.is_none());
        assert_eq!(changes[1].new_value, Some(serde_json::json!(100)));

        // Both issued by PLAYER_SPAWNER.
        assert_eq!(changes[0].changed_by, SystemId::PLAYER_SPAWNER);
        assert_eq!(changes[1].changed_by, SystemId::PLAYER_SPAWNER);
    }

    // -- 9. Despawn / removal (old_value only) ------------------------------

    #[test]
    fn despawn_change_old_value_only() {
        let mut journal = ChangeJournal::new();
        let e = entity(3);

        journal.record_change(removal_change(
            e,
            "position",
            serde_json::json!({"x": 5.0, "y": 5.0}),
            SystemId(10),
            CausalReason::GameRule("entity_destroyed".to_owned()),
            0,
            4,
        ));
        journal.record_change(removal_change(
            e,
            "health",
            serde_json::json!(0),
            SystemId(10),
            CausalReason::GameRule("entity_destroyed".to_owned()),
            1,
            4,
        ));

        let changes: Vec<_> = journal.changes_for_entity(e).collect();
        assert_eq!(changes.len(), 2);

        // Despawn: old_value is Some, new_value is None.
        assert_eq!(
            changes[0].old_value,
            Some(serde_json::json!({"x": 5.0, "y": 5.0}))
        );
        assert!(changes[0].new_value.is_none());

        assert_eq!(changes[1].old_value, Some(serde_json::json!(0)));
        assert!(changes[1].new_value.is_none());

        assert_eq!(changes[0].tick, 4);
        assert_eq!(changes[1].tick, 4);
    }

    // -- 10. Mixed: spawn, modify, despawn in one tick ----------------------

    #[test]
    fn mixed_spawn_modify_despawn_in_single_tick() {
        let mut journal = ChangeJournal::new();
        let e = entity(7);
        let tick = 10;

        // 1. Spawn: position inserted.
        journal.record_change(spawn_change(
            e,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("enemy_spawn".to_owned()),
            0,
            tick,
        ));

        // 2. Spawn: health inserted.
        journal.record_change(spawn_change(
            e,
            "health",
            serde_json::json!(50),
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("enemy_spawn".to_owned()),
            1,
            tick,
        ));

        // 3. Modification: health reduced by damage.
        journal.record_change(modification(
            e,
            "health",
            serde_json::json!(50),
            serde_json::json!(0),
            SystemId::PHYSICS,
            CausalReason::CollisionResponse(entity(0), e),
            2,
            tick,
        ));

        // 4. Despawn: both components removed.
        journal.record_change(removal_change(
            e,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("enemy_destroyed".to_owned()),
            3,
            tick,
        ));
        journal.record_change(removal_change(
            e,
            "health",
            serde_json::json!(0),
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("enemy_destroyed".to_owned()),
            4,
            tick,
        ));

        assert_eq!(journal.len(), 5);

        // All changes for this entity.
        let entity_changes: Vec<_> = journal.changes_for_entity(e).collect();
        assert_eq!(entity_changes.len(), 5);

        // Health changes: spawn, modification, despawn.
        let health_changes: Vec<_> = journal.changes_for_component("health").collect();
        assert_eq!(health_changes.len(), 3);

        // Spawn (no old value).
        assert!(health_changes[0].old_value.is_none());
        assert_eq!(health_changes[0].new_value, Some(serde_json::json!(50)));

        // Modification (both values).
        assert_eq!(health_changes[1].old_value, Some(serde_json::json!(50)));
        assert_eq!(health_changes[1].new_value, Some(serde_json::json!(0)));

        // Despawn (no new value).
        assert_eq!(health_changes[2].old_value, Some(serde_json::json!(0)));
        assert!(health_changes[2].new_value.is_none());

        // Changes by system: WASM_GAMEPLAY issued 4 changes, PHYSICS issued 1.
        let gameplay: Vec<_> = journal.changes_by_system(SystemId::WASM_GAMEPLAY).collect();
        assert_eq!(gameplay.len(), 4);

        let physics: Vec<_> = journal.changes_by_system(SystemId::PHYSICS).collect();
        assert_eq!(physics.len(), 1);
        assert_eq!(
            physics[0].reason,
            CausalReason::CollisionResponse(entity(0), e)
        );
    }

    // -- 11. Multiple entities interleaved ----------------------------------

    #[test]
    fn multiple_entities_interleaved() {
        let mut journal = ChangeJournal::new();
        let e0 = entity(0);
        let e1 = entity(1);
        let e2 = entity(2);

        // Interleave changes across entities.
        journal.record_change(spawn_change(
            e0,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            SystemId(1),
            CausalReason::GameRule("spawn".to_owned()),
            0,
            1,
        ));
        journal.record_change(spawn_change(
            e1,
            "position",
            serde_json::json!({"x": 10.0, "y": 10.0}),
            SystemId(1),
            CausalReason::GameRule("spawn".to_owned()),
            1,
            1,
        ));
        journal.record_change(spawn_change(
            e2,
            "position",
            serde_json::json!({"x": 20.0, "y": 20.0}),
            SystemId(1),
            CausalReason::GameRule("spawn".to_owned()),
            2,
            1,
        ));
        journal.record_change(modification(
            e0,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(2),
            CausalReason::PlayerInput("move".to_owned()),
            3,
            1,
        ));
        journal.record_change(modification(
            e1,
            "position",
            serde_json::json!({"x": 10.0, "y": 10.0}),
            serde_json::json!({"x": 11.0, "y": 10.0}),
            SystemId(3),
            CausalReason::SystemInternal("ai".to_owned()),
            4,
            1,
        ));

        assert_eq!(journal.len(), 5);

        // e0: spawn + modify = 2 changes.
        assert_eq!(journal.changes_for_entity(e0).count(), 2);
        // e1: spawn + modify = 2 changes.
        assert_eq!(journal.changes_for_entity(e1).count(), 2);
        // e2: spawn only = 1 change.
        assert_eq!(journal.changes_for_entity(e2).count(), 1);

        // All position changes = 5.
        assert_eq!(journal.changes_for_component("position").count(), 5);

        // System 1 spawned all 3 = 3 changes.
        assert_eq!(journal.changes_by_system(SystemId(1)).count(), 3);
        // System 2 moved e0 = 1 change.
        assert_eq!(journal.changes_by_system(SystemId(2)).count(), 1);
        // System 3 moved e1 = 1 change.
        assert_eq!(journal.changes_by_system(SystemId(3)).count(), 1);
    }

    // -- 12. Command index ordering preserved ------------------------------

    #[test]
    fn command_index_ordering_preserved() {
        let mut journal = ChangeJournal::new();
        let e = entity(0);

        for i in 0..10u64 {
            journal.record_change(modification(
                e,
                "counter",
                serde_json::json!(i),
                serde_json::json!(i + 1),
                SystemId(0),
                CausalReason::SystemInternal(format!("step_{i}")),
                i,
                1,
            ));
        }

        let changes = journal.all_changes();
        assert_eq!(changes.len(), 10);

        // Verify insertion order matches command_index order.
        for (i, change) in changes.iter().enumerate() {
            assert_eq!(change.command_index, i as u64);
            assert_eq!(change.old_value, Some(serde_json::json!(i as u64)));
            assert_eq!(change.new_value, Some(serde_json::json!(i as u64 + 1)));
        }
    }

    // -- 13. Serialization roundtrip ----------------------------------------

    #[test]
    fn serialization_roundtrip() {
        let mut journal = ChangeJournal::new();
        let e = entity(42);

        journal.record_change(modification(
            e,
            "health",
            serde_json::json!(100),
            serde_json::json!(50),
            SystemId(7),
            CausalReason::CollisionResponse(entity(0), entity(42)),
            5,
            3,
        ));
        journal.record_change(spawn_change(
            e,
            "shield",
            serde_json::json!(true),
            SystemId(8),
            CausalReason::Timer("shield_activate".to_owned()),
            6,
            3,
        ));

        let json = serde_json::to_string(&journal).unwrap();
        let deserialized: ChangeJournal = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.len(), 2);

        let changes = deserialized.all_changes();
        assert_eq!(changes[0].entity_id, e);
        assert_eq!(changes[0].component_type_name, "health");
        assert_eq!(changes[0].old_value, Some(serde_json::json!(100)));
        assert_eq!(changes[0].new_value, Some(serde_json::json!(50)));
        assert_eq!(changes[0].changed_by, SystemId(7));
        assert_eq!(changes[0].command_index, 5);
        assert_eq!(changes[0].tick, 3);

        assert_eq!(changes[1].component_type_name, "shield");
        assert!(changes[1].old_value.is_none());
        assert_eq!(changes[1].new_value, Some(serde_json::json!(true)));
        assert_eq!(
            changes[1].reason,
            CausalReason::Timer("shield_activate".to_owned())
        );
    }

    // -- 14. Clear then re-use across ticks ---------------------------------

    #[test]
    fn clear_and_reuse_across_ticks() {
        let mut journal = ChangeJournal::new();
        let e = entity(0);

        // Simulate 3 ticks, verifying clear works each time.
        for tick in 1..=3u64 {
            journal.clear();
            assert!(journal.is_empty());

            for cmd_idx in 0..5u64 {
                journal.record_change(modification(
                    e,
                    "position",
                    serde_json::json!({"x": cmd_idx as f64, "y": 0.0}),
                    serde_json::json!({"x": (cmd_idx + 1) as f64, "y": 0.0}),
                    SystemId(1),
                    CausalReason::PlayerInput("move".to_owned()),
                    cmd_idx,
                    tick,
                ));
            }

            assert_eq!(journal.len(), 5);

            // All changes belong to the current tick.
            for change in journal.all_changes() {
                assert_eq!(change.tick, tick);
            }
        }
    }

    // -- 15. Different component types on same entity -----------------------

    #[test]
    fn different_components_same_entity_queryable() {
        let mut journal = ChangeJournal::new();
        let e = entity(0);

        journal.record_change(modification(
            e,
            "position",
            serde_json::json!({"x": 0.0, "y": 0.0}),
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("move".to_owned()),
            0,
            1,
        ));
        journal.record_change(modification(
            e,
            "health",
            serde_json::json!(100),
            serde_json::json!(90),
            SystemId(2),
            CausalReason::GameRule("damage".to_owned()),
            1,
            1,
        ));
        journal.record_change(modification(
            e,
            "velocity",
            serde_json::json!({"dx": 0.0, "dy": 0.0}),
            serde_json::json!({"dx": 5.0, "dy": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("accelerate".to_owned()),
            2,
            1,
        ));

        // All 3 changes for entity.
        assert_eq!(journal.changes_for_entity(e).count(), 3);

        // 1 position, 1 health, 1 velocity.
        assert_eq!(journal.changes_for_component("position").count(), 1);
        assert_eq!(journal.changes_for_component("health").count(), 1);
        assert_eq!(journal.changes_for_component("velocity").count(), 1);

        // System 1: position + velocity = 2.
        assert_eq!(journal.changes_by_system(SystemId(1)).count(), 2);
        // System 2: health = 1.
        assert_eq!(journal.changes_by_system(SystemId(2)).count(), 1);
    }
}
