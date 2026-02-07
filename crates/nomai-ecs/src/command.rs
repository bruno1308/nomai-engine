//! Command buffer with causality metadata for deferred ECS mutations.
//!
//! The [`CommandBuffer`] collects deferred mutations to the ECS world during a
//! tick. Each command carries causality metadata ([`SystemId`], [`CausalReason`])
//! that feeds the manifest's causal chains. Commands are applied in deterministic
//! FIFO order after all systems have run.
//!
//! # Design
//!
//! Every ECS state change in the Nomai Engine flows through the command buffer.
//! This is non-negotiable -- it is what makes the manifest useful. Direct
//! `world.set_component()` is only allowed in tests and initial setup.
//!
//! Component values in commands are stored as [`serde_json::Value`] for
//! flexibility and manifest compatibility. The [`CommandBuffer::apply`] method
//! uses the [`World`]'s deserializer registry to convert JSON values back to
//! typed components.
//!
//! # Example
//!
//! ```
//! use nomai_ecs::prelude::*;
//! use nomai_ecs::command::{CausalReason, CommandBuffer};
//!
//! #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
//! struct Health(u32);
//!
//! let mut world = World::new();
//! world.register_component::<Health>("health");
//! let entity = world.spawn_with(Health(100));
//!
//! let mut cmds = CommandBuffer::new();
//! cmds.set_component(
//!     entity,
//!     "health",
//!     serde_json::json!(50),
//!     SystemId(0),
//!     CausalReason::GameRule("damage_applied".to_owned()),
//! );
//!
//! let applied = cmds.apply(&mut world);
//! assert_eq!(applied.len(), 1);
//! assert_eq!(world.get_component::<Health>(entity), Some(&Health(50)));
//! ```

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::entity::EntityId;
use crate::identity::{EntityIdentity, PoolIdentity, SystemId};
use crate::world::World;

// ---------------------------------------------------------------------------
// CausalReason
// ---------------------------------------------------------------------------

/// Why a command was issued. This feeds the manifest's causal chains.
///
/// Prefer the most specific variant possible. `SystemInternal` is a last
/// resort -- using it weakens the manifest's diagnostic value
/// (see anti-pattern "The Broken Chain" in CLAUDE.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CausalReason {
    /// Caused by player input.
    PlayerInput(String),
    /// Caused by a collision between two entities.
    CollisionResponse(EntityId, EntityId),
    /// Caused by a game rule (e.g. "brick_destroyed_on_hit").
    GameRule(String),
    /// Caused by a state transition.
    StateTransition {
        /// The state being transitioned from.
        from: String,
        /// The state being transitioned to.
        to: String,
    },
    /// Caused by a timer firing.
    Timer(String),
    /// Internal system logic -- last resort, prefer more specific reasons.
    SystemInternal(String),
}

// ---------------------------------------------------------------------------
// CommandKind
// ---------------------------------------------------------------------------

/// The data payload for a command -- what mutation to perform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandKind {
    /// Set a component value on an entity (insert or overwrite).
    SetComponent {
        /// Registered name of the component type.
        component_name: String,
        /// Serialized component value (JSON for flexibility and manifest compat).
        value: serde_json::Value,
    },
    /// Remove a component from an entity.
    RemoveComponent {
        /// Registered name of the component type.
        component_name: String,
    },
    /// Despawn an entity entirely.
    Despawn,
    /// Spawn a semantic-tier entity with full identity and optional components.
    SpawnSemantic {
        /// The entity's semantic identity.
        identity: EntityIdentity,
        /// Serialized component values (name -> JSON) to attach on spawn.
        components: Vec<(String, serde_json::Value)>,
    },
    /// Spawn a pooled-tier entity with type-level identity and optional components.
    SpawnPooled {
        /// The entity's pool identity.
        identity: PoolIdentity,
        /// Serialized component values (name -> JSON) to attach on spawn.
        components: Vec<(String, serde_json::Value)>,
    },
}

// ---------------------------------------------------------------------------
// Command
// ---------------------------------------------------------------------------

/// A single deferred mutation with full causality metadata.
///
/// Commands are the atomic unit of ECS mutation in the Nomai Engine.
/// They carry enough context to reconstruct the full causal chain
/// in the manifest.
///
/// For spawn commands (`SpawnSemantic`, `SpawnPooled`), the `target` field is
/// `None` because the entity does not exist yet. After application, the
/// `spawned_entity` field is set to the newly created entity ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    /// Which entity this command targets. `None` for spawn commands.
    pub target: Option<EntityId>,
    /// What mutation to perform.
    pub kind: CommandKind,
    /// Which system issued this command.
    pub issued_by: SystemId,
    /// Why this command was issued (feeds manifest causality).
    pub reason: CausalReason,
    /// Sequential index within the buffer (set on insertion).
    pub command_index: u32,
    /// For spawn commands: the entity ID that was created after application.
    /// `None` before `apply()` is called.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub spawned_entity: Option<EntityId>,
    /// Whether the command was applied successfully.
    ///
    /// `false` before `apply()` is called. Set to `true` if the command
    /// mutated the world, `false` if it was skipped (e.g. stale entity).
    /// The change journal should only record entries for successful commands.
    #[serde(default)]
    pub applied_successfully: bool,
}

// ---------------------------------------------------------------------------
// ApplyReport
// ---------------------------------------------------------------------------

/// Summary of the last [`CommandBuffer::apply`] call.
///
/// This report provides visibility into command-buffer health for each tick.
/// `conflict_count` tracks how many (entity, component) pairs were targeted
/// by multiple commands in a single tick (last-write-wins semantics apply --
/// conflicts are warnings, not errors). `success_count` and `failed_count`
/// track how many commands applied successfully vs. failed (e.g. due to stale
/// entity references).
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    /// Number of (entity, component) pairs targeted by multiple commands.
    pub conflict_count: usize,
    /// Number of commands that failed to apply.
    pub failed_count: usize,
    /// Number of commands that applied successfully.
    pub success_count: usize,
}

// ---------------------------------------------------------------------------
// CommandBuffer
// ---------------------------------------------------------------------------

/// Collects commands during a tick and applies them deterministically.
///
/// Commands are applied in strict insertion order (FIFO). This is the
/// deterministic ordering guarantee: given the same set of systems running
/// in the same declared order, the same commands will be emitted and applied
/// in the same sequence.
///
/// After [`apply`](Self::apply), the buffer is cleared and the applied
/// commands are returned so that downstream consumers (change journal,
/// manifest) can record them.
pub struct CommandBuffer {
    commands: Vec<Command>,
    next_index: u32,
    last_apply_report: ApplyReport,
}

impl CommandBuffer {
    /// Create a new, empty command buffer.
    pub fn new() -> Self {
        Self {
            commands: Vec::new(),
            next_index: 0,
            last_apply_report: ApplyReport::default(),
        }
    }

    /// Queue a `SetComponent` command.
    ///
    /// The `value` is a JSON representation of the component. It will be
    /// deserialized into the correct type when [`apply`](Self::apply) is called.
    pub fn set_component(
        &mut self,
        target: EntityId,
        component_name: &str,
        value: serde_json::Value,
        issued_by: SystemId,
        reason: CausalReason,
    ) {
        self.push(
            Some(target),
            CommandKind::SetComponent {
                component_name: component_name.to_owned(),
                value,
            },
            issued_by,
            reason,
        );
    }

    /// Queue a `RemoveComponent` command.
    pub fn remove_component(
        &mut self,
        target: EntityId,
        component_name: &str,
        issued_by: SystemId,
        reason: CausalReason,
    ) {
        self.push(
            Some(target),
            CommandKind::RemoveComponent {
                component_name: component_name.to_owned(),
            },
            issued_by,
            reason,
        );
    }

    /// Queue a `Despawn` command.
    pub fn despawn(&mut self, target: EntityId, issued_by: SystemId, reason: CausalReason) {
        self.push(Some(target), CommandKind::Despawn, issued_by, reason);
    }

    /// Queue a `SpawnSemantic` command.
    ///
    /// Spawns a new semantic-tier entity when the buffer is applied.
    /// Components are provided as `(name, JSON value)` pairs and will be
    /// deserialized using the world's registered deserializers.
    ///
    /// The spawned entity's ID is available on the returned `Command` after
    /// [`apply`](Self::apply) via the `spawned_entity` field.
    pub fn spawn_semantic(
        &mut self,
        identity: EntityIdentity,
        components: Vec<(String, serde_json::Value)>,
        issued_by: SystemId,
        reason: CausalReason,
    ) {
        self.push(
            None,
            CommandKind::SpawnSemantic {
                identity,
                components,
            },
            issued_by,
            reason,
        );
    }

    /// Queue a `SpawnPooled` command.
    ///
    /// Spawns a new pooled-tier entity when the buffer is applied.
    /// Components are provided as `(name, JSON value)` pairs and will be
    /// deserialized using the world's registered deserializers.
    ///
    /// The spawned entity's ID is available on the returned `Command` after
    /// [`apply`](Self::apply) via the `spawned_entity` field.
    pub fn spawn_pooled(
        &mut self,
        identity: PoolIdentity,
        components: Vec<(String, serde_json::Value)>,
        issued_by: SystemId,
        reason: CausalReason,
    ) {
        self.push(
            None,
            CommandKind::SpawnPooled {
                identity,
                components,
            },
            issued_by,
            reason,
        );
    }

    /// Get all queued commands in insertion order.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// Number of queued commands.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Report from the last [`apply`](Self::apply) call.
    ///
    /// Returns a default (all-zero) report if `apply()` has never been called.
    pub fn last_apply_report(&self) -> &ApplyReport {
        &self.last_apply_report
    }

    /// Apply all commands to the world in deterministic insertion order.
    ///
    /// Returns the list of all commands (successful and failed) so the
    /// change journal (A6) and manifest can consume them. The buffer is
    /// cleared after application. Check each command's
    /// [`applied_successfully`](Command::applied_successfully) field to
    /// distinguish real mutations from failed attempts.
    ///
    /// Commands that target stale or non-existent entities are logged as
    /// warnings and skipped -- they are still included in the returned list
    /// so the manifest can record the attempted mutation.
    ///
    /// For spawn commands, the `spawned_entity` field on the returned
    /// `Command` is set to the newly created entity ID.
    pub fn apply(&mut self, world: &mut World) -> Vec<Command> {
        let mut commands = std::mem::take(&mut self.commands);
        self.next_index = 0;

        // --- Conflict detection ---
        use std::collections::HashMap;
        let mut seen: HashMap<(EntityId, String), Vec<u32>> = HashMap::new();
        for cmd in &commands {
            if let Some(target) = cmd.target {
                let component_name = match &cmd.kind {
                    CommandKind::SetComponent { component_name, .. } => {
                        Some(component_name.clone())
                    }
                    CommandKind::RemoveComponent { component_name } => Some(component_name.clone()),
                    _ => None,
                };
                if let Some(name) = component_name {
                    seen.entry((target, name))
                        .or_default()
                        .push(cmd.command_index);
                }
            }
        }
        let mut conflict_count = 0;
        for ((entity, component), indices) in &seen {
            if indices.len() > 1 {
                conflict_count += 1;
                tracing::warn!(
                    entity = ?entity,
                    component = %component,
                    command_indices = ?indices,
                    "conflict: {} commands target the same entity+component in this tick (last-write-wins)",
                    indices.len()
                );
            }
        }

        // --- Apply loop ---
        let mut success_count: usize = 0;
        let mut failed_count: usize = 0;

        for cmd in &mut commands {
            // Clone the kind to avoid borrow checker issues when spawn
            // commands need to set cmd.spawned_entity.
            let kind = cmd.kind.clone();
            let result = match &kind {
                CommandKind::SetComponent {
                    component_name,
                    value,
                } => {
                    let target = cmd
                        .target
                        .expect("SetComponent command must have a target entity");
                    world.set_component_by_name(target, component_name, value)
                }
                CommandKind::RemoveComponent { component_name } => {
                    let target = cmd
                        .target
                        .expect("RemoveComponent command must have a target entity");
                    world.remove_component_by_name(target, component_name)
                }
                CommandKind::Despawn => {
                    let target = cmd
                        .target
                        .expect("Despawn command must have a target entity");
                    world.despawn(target)
                }
                CommandKind::SpawnSemantic {
                    identity,
                    components,
                } => Self::apply_spawn_semantic(world, cmd, identity, components),
                CommandKind::SpawnPooled {
                    identity,
                    components,
                } => Self::apply_spawn_pooled(world, cmd, identity, components),
            };

            match result {
                Ok(()) => {
                    cmd.applied_successfully = true;
                    success_count += 1;
                }
                Err(e) => {
                    failed_count += 1;
                    warn!(
                        command_index = cmd.command_index,
                        target = ?cmd.target,
                        system_id = cmd.issued_by.0,
                        error = %e,
                        "command application failed"
                    );
                }
            }
        }

        // --- Store report ---
        self.last_apply_report = ApplyReport {
            conflict_count,
            success_count,
            failed_count,
        };

        commands
    }

    /// Apply a `SpawnSemantic` command. Sets `cmd.spawned_entity` on success.
    fn apply_spawn_semantic(
        world: &mut World,
        cmd: &mut Command,
        identity: &EntityIdentity,
        components: &[(String, serde_json::Value)],
    ) -> Result<(), crate::EcsError> {
        use crate::world::ComponentBundle;

        let bundle = ComponentBundle::new();
        let entity = world.spawn_semantic(identity.clone(), bundle)?;

        // Record the spawned entity immediately so downstream causality
        // bookkeeping is consistent even if a component set fails.
        cmd.spawned_entity = Some(entity);

        // The entity was created — mark the spawn itself as successful.
        // Component-set failures below are logged but don't un-create the entity.
        cmd.applied_successfully = true;

        // Set additional components via deserialization.
        for (name, value) in components {
            if let Err(e) = world.set_component_by_name(entity, name, value) {
                warn!(
                    command_index = cmd.command_index,
                    entity = ?entity,
                    component = %name,
                    error = %e,
                    "spawn component set failed (entity was still created)"
                );
            }
        }

        Ok(())
    }

    /// Apply a `SpawnPooled` command. Sets `cmd.spawned_entity` on success.
    fn apply_spawn_pooled(
        world: &mut World,
        cmd: &mut Command,
        identity: &PoolIdentity,
        components: &[(String, serde_json::Value)],
    ) -> Result<(), crate::EcsError> {
        use crate::world::ComponentBundle;

        let bundle = ComponentBundle::new();
        let entity = world.spawn_pooled(identity.clone(), bundle)?;

        // Record the spawned entity immediately so downstream causality
        // bookkeeping is consistent even if a component set fails.
        cmd.spawned_entity = Some(entity);

        // The entity was created — mark the spawn itself as successful.
        // Component-set failures below are logged but don't un-create the entity.
        cmd.applied_successfully = true;

        // Set additional components via deserialization.
        for (name, value) in components {
            if let Err(e) = world.set_component_by_name(entity, name, value) {
                warn!(
                    command_index = cmd.command_index,
                    entity = ?entity,
                    component = %name,
                    error = %e,
                    "spawn component set failed (entity was still created)"
                );
            }
        }

        Ok(())
    }

    /// Push a pre-built command into the buffer.
    ///
    /// Used for merging commands from external sources (e.g., WASM modules)
    /// into the main command buffer. The command index is reassigned to
    /// maintain ordering within this buffer.
    pub fn push_raw(&mut self, mut cmd: Command) {
        cmd.command_index = self.next_index;
        self.next_index += 1;
        self.commands.push(cmd);
    }

    /// Clear the buffer without applying any commands.
    pub fn clear(&mut self) {
        self.commands.clear();
        self.next_index = 0;
    }

    // -- internal helpers ---------------------------------------------------

    fn push(
        &mut self,
        target: Option<EntityId>,
        kind: CommandKind,
        issued_by: SystemId,
        reason: CausalReason,
    ) {
        let index = self.next_index;
        self.next_index += 1;
        self.commands.push(Command {
            target,
            kind,
            issued_by,
            reason,
            command_index: index,
            spawned_entity: None,
            applied_successfully: false,
        });
    }
}

impl Default for CommandBuffer {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::IdentityTier;

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

    // -- 1. Basic command creation ------------------------------------------

    #[test]
    fn basic_command_creation() {
        let entity = EntityId::new(0, 0);
        let mut buf = CommandBuffer::new();

        // SetComponent
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 2.0}),
            SystemId(1),
            CausalReason::PlayerInput("move_right".to_owned()),
        );

        // RemoveComponent
        buf.remove_component(
            entity,
            "velocity",
            SystemId(2),
            CausalReason::GameRule("stop_on_collision".to_owned()),
        );

        // Despawn
        buf.despawn(
            entity,
            SystemId(3),
            CausalReason::CollisionResponse(EntityId::new(0, 0), EntityId::new(1, 0)),
        );

        assert_eq!(buf.len(), 3);

        let cmds = buf.commands();
        assert_eq!(cmds[0].command_index, 0);
        assert_eq!(cmds[1].command_index, 1);
        assert_eq!(cmds[2].command_index, 2);

        // Verify each kind.
        assert!(matches!(cmds[0].kind, CommandKind::SetComponent { .. }));
        assert!(matches!(cmds[1].kind, CommandKind::RemoveComponent { .. }));
        assert!(matches!(cmds[2].kind, CommandKind::Despawn));
    }

    // -- 2. Command ordering -----------------------------------------------

    #[test]
    fn command_ordering_interleaved_systems() {
        let entity = EntityId::new(0, 0);
        let mut buf = CommandBuffer::new();

        // Interleave commands from 3 different systems.
        for i in 0..10u32 {
            let system_id = SystemId(i % 3);
            buf.set_component(
                entity,
                "health",
                serde_json::json!(i),
                system_id,
                CausalReason::SystemInternal(format!("step_{i}")),
            );
        }

        assert_eq!(buf.len(), 10);

        // Verify strict insertion order.
        for (i, cmd) in buf.commands().iter().enumerate() {
            assert_eq!(cmd.command_index, i as u32);
            assert_eq!(cmd.issued_by, SystemId((i as u32) % 3));
        }
    }

    // -- 3. Deterministic application --------------------------------------

    #[test]
    fn deterministic_application() {
        // Build the same command sequence twice and apply to two identical
        // worlds. Results must be identical.
        fn build_commands(entity: EntityId) -> CommandBuffer {
            let mut buf = CommandBuffer::new();
            buf.set_component(
                entity,
                "position",
                serde_json::json!({"x": 10.0, "y": 20.0}),
                SystemId(0),
                CausalReason::PlayerInput("spawn".to_owned()),
            );
            buf.set_component(
                entity,
                "health",
                serde_json::json!(42),
                SystemId(1),
                CausalReason::GameRule("set_health".to_owned()),
            );
            buf
        }

        let mut world1 = setup_world();
        let e1 = world1.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut world2 = setup_world();
        let e2 = world2.spawn_with(Position { x: 0.0, y: 0.0 });

        let applied1 = build_commands(e1).apply(&mut world1);
        let applied2 = build_commands(e2).apply(&mut world2);

        // Same number of applied commands.
        assert_eq!(applied1.len(), applied2.len());

        // Same component state after apply.
        assert_eq!(
            world1.get_component::<Position>(e1),
            world2.get_component::<Position>(e2),
        );
        assert_eq!(
            world1.get_component::<Health>(e1),
            world2.get_component::<Health>(e2),
        );
    }

    // -- 4. SetComponent via command ---------------------------------------

    #[test]
    fn set_component_via_command() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 42.0, "y": 99.0}),
            SystemId(0),
            CausalReason::PlayerInput("teleport".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 1);
        assert_eq!(
            world.get_component::<Position>(entity),
            Some(&Position { x: 42.0, y: 99.0 }),
        );
    }

    // -- 5. Despawn via command --------------------------------------------

    #[test]
    fn despawn_via_command() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });
        assert!(world.is_alive(entity));

        let mut buf = CommandBuffer::new();
        buf.despawn(
            entity,
            SystemId(0),
            CausalReason::GameRule("entity_destroyed".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 1);
        assert!(!world.is_alive(entity));
        assert_eq!(world.entity_count(), 0);
    }

    // -- 6. RemoveComponent via command ------------------------------------

    #[test]
    fn remove_component_via_command() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 1.0, y: 2.0 });
        world
            .insert_component(entity, Velocity { dx: 3.0, dy: 4.0 })
            .unwrap();
        assert!(world.has_component::<Velocity>(entity));

        let mut buf = CommandBuffer::new();
        buf.remove_component(
            entity,
            "velocity",
            SystemId(0),
            CausalReason::GameRule("stop_movement".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 1);
        assert!(!world.has_component::<Velocity>(entity));
        // Position should still be there.
        assert_eq!(
            world.get_component::<Position>(entity),
            Some(&Position { x: 1.0, y: 2.0 }),
        );
    }

    // -- 7. Causality metadata preserved -----------------------------------

    #[test]
    fn causality_metadata_preserved() {
        let mut world = setup_world();
        let e1 = world.spawn_with(Position { x: 0.0, y: 0.0 });
        let e2 = world.spawn_with(Position { x: 1.0, y: 1.0 });

        let mut buf = CommandBuffer::new();

        buf.set_component(
            e1,
            "position",
            serde_json::json!({"x": 5.0, "y": 5.0}),
            SystemId(10),
            CausalReason::PlayerInput("move".to_owned()),
        );

        buf.despawn(e2, SystemId(20), CausalReason::CollisionResponse(e1, e2));

        buf.set_component(
            e1,
            "health",
            serde_json::json!(75),
            SystemId(30),
            CausalReason::StateTransition {
                from: "alive".to_owned(),
                to: "damaged".to_owned(),
            },
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 3);

        // Verify causality metadata is fully preserved.
        assert_eq!(applied[0].issued_by, SystemId(10));
        assert_eq!(
            applied[0].reason,
            CausalReason::PlayerInput("move".to_owned()),
        );

        assert_eq!(applied[1].issued_by, SystemId(20));
        assert_eq!(applied[1].reason, CausalReason::CollisionResponse(e1, e2),);

        assert_eq!(applied[2].issued_by, SystemId(30));
        assert_eq!(
            applied[2].reason,
            CausalReason::StateTransition {
                from: "alive".to_owned(),
                to: "damaged".to_owned(),
            },
        );
    }

    // -- 8. Buffer clears after apply --------------------------------------

    #[test]
    fn buffer_clears_after_apply() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 1.0}),
            SystemId(0),
            CausalReason::SystemInternal("test".to_owned()),
        );

        assert_eq!(buf.len(), 1);
        let _applied = buf.apply(&mut world);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    // -- 9. Empty buffer apply is no-op ------------------------------------

    #[test]
    fn empty_buffer_apply_noop() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 5.0, y: 10.0 });

        let mut buf = CommandBuffer::new();
        let applied = buf.apply(&mut world);

        assert!(applied.is_empty());
        // World should be unchanged.
        assert_eq!(
            world.get_component::<Position>(entity),
            Some(&Position { x: 5.0, y: 10.0 }),
        );
        assert_eq!(world.entity_count(), 1);
    }

    // -- 10. Multiple commands same entity: SetComponent + Despawn ---------

    #[test]
    fn multiple_commands_same_entity_set_then_despawn() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();

        // First: set component (will succeed).
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 99.0, "y": 99.0}),
            SystemId(0),
            CausalReason::SystemInternal("update".to_owned()),
        );

        // Then: despawn (will succeed, entity is gone after this).
        buf.despawn(
            entity,
            SystemId(1),
            CausalReason::GameRule("destroy".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 2);

        // Despawn wins -- entity should be gone.
        assert!(!world.is_alive(entity));
        assert_eq!(world.entity_count(), 0);
    }

    // -- 11. SpawnSemantic via command --------------------------------------

    #[test]
    fn spawn_semantic_via_command() {
        let mut world = setup_world();

        let identity = EntityIdentity {
            entity_type: "character".to_owned(),
            role: "player".to_owned(),
            spawned_by: SystemId::PLAYER_SPAWNER,
            requirement_id: Some("REQ-001".to_owned()),
        };

        let mut buf = CommandBuffer::new();
        buf.spawn_semantic(
            identity.clone(),
            vec![
                (
                    "position".to_owned(),
                    serde_json::json!({"x": 10.0, "y": 20.0}),
                ),
                ("health".to_owned(), serde_json::json!(100)),
            ],
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("player_spawn".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 1);

        // The spawned entity should exist.
        let spawned = applied[0]
            .spawned_entity
            .expect("spawn should set spawned_entity");
        assert!(world.is_alive(spawned));

        // Identity should be semantic.
        let tier = world.get_tier(spawned).unwrap();
        assert_eq!(tier, IdentityTier::Semantic);

        // Components should be set.
        assert_eq!(
            world.get_component::<Position>(spawned),
            Some(&Position { x: 10.0, y: 20.0 }),
        );
        assert_eq!(world.get_component::<Health>(spawned), Some(&Health(100)),);

        // Target should be None for spawn commands.
        assert_eq!(applied[0].target, None);

        // Causality should be preserved.
        assert_eq!(applied[0].issued_by, SystemId::PLAYER_SPAWNER);
    }

    // -- 12. SpawnPooled via command ----------------------------------------

    #[test]
    fn spawn_pooled_via_command() {
        let mut world = setup_world();

        let identity = PoolIdentity {
            pool_type: "destructible".to_owned(),
            variant: "brick".to_owned(),
        };

        let mut buf = CommandBuffer::new();
        buf.spawn_pooled(
            identity.clone(),
            vec![(
                "position".to_owned(),
                serde_json::json!({"x": 5.0, "y": 5.0}),
            )],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("brick_spawn".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 1);

        let spawned = applied[0]
            .spawned_entity
            .expect("spawn should set spawned_entity");
        assert!(world.is_alive(spawned));

        let tier = world.get_tier(spawned).unwrap();
        assert_eq!(tier, IdentityTier::Pooled);

        assert_eq!(
            world.get_component::<Position>(spawned),
            Some(&Position { x: 5.0, y: 5.0 }),
        );
    }

    // -- 13. Spawn then modify in same buffer ------------------------------

    #[test]
    fn spawn_then_modify_via_command() {
        let mut world = setup_world();

        let identity = EntityIdentity {
            entity_type: "character".to_owned(),
            role: "enemy".to_owned(),
            spawned_by: SystemId::WASM_GAMEPLAY,
            requirement_id: None,
        };

        // First buffer: spawn the entity.
        let mut buf1 = CommandBuffer::new();
        buf1.spawn_semantic(
            identity,
            vec![("health".to_owned(), serde_json::json!(50))],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("enemy_spawn".to_owned()),
        );

        let applied1 = buf1.apply(&mut world);
        let spawned = applied1[0].spawned_entity.unwrap();

        // Second buffer: modify the spawned entity.
        let mut buf2 = CommandBuffer::new();
        buf2.set_component(
            spawned,
            "health",
            serde_json::json!(25),
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("damage".to_owned()),
        );

        let applied2 = buf2.apply(&mut world);
        assert_eq!(applied2.len(), 1);
        assert_eq!(world.get_component::<Health>(spawned), Some(&Health(25)),);
    }

    // -- 14. applied_successfully: success cases ------------------------------

    #[test]
    fn applied_successfully_true_on_success() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 2.0}),
            SystemId(0),
            CausalReason::PlayerInput("test".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 1);
        assert!(
            applied[0].applied_successfully,
            "successful SetComponent should have applied_successfully = true"
        );
    }

    // -- 15. applied_successfully: stale entity fails -------------------------

    #[test]
    fn applied_successfully_false_on_stale_entity() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });
        world.despawn(entity).unwrap();

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 2.0}),
            SystemId(0),
            CausalReason::SystemInternal("stale_test".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 1);
        assert!(
            !applied[0].applied_successfully,
            "command targeting stale entity should have applied_successfully = false"
        );
    }

    // -- 16. applied_successfully: despawn success ----------------------------

    #[test]
    fn applied_successfully_true_on_despawn() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();
        buf.despawn(
            entity,
            SystemId(0),
            CausalReason::GameRule("destroy".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert!(applied[0].applied_successfully);
        assert!(!world.is_alive(entity));
    }

    // -- 17. applied_successfully: spawn success ------------------------------

    #[test]
    fn applied_successfully_true_on_spawn() {
        let mut world = setup_world();

        let identity = EntityIdentity {
            entity_type: "test".to_owned(),
            role: "unit".to_owned(),
            spawned_by: SystemId(0),
            requirement_id: None,
        };

        let mut buf = CommandBuffer::new();
        buf.spawn_semantic(
            identity,
            vec![("health".to_owned(), serde_json::json!(100))],
            SystemId(0),
            CausalReason::GameRule("spawn".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert!(applied[0].applied_successfully);
        assert!(applied[0].spawned_entity.is_some());
    }

    // -- 18. applied_successfully: mixed success/failure ----------------------

    #[test]
    fn applied_successfully_mixed_batch() {
        let mut world = setup_world();
        let alive = world.spawn_with(Position { x: 0.0, y: 0.0 });
        let doomed = world.spawn_with(Position { x: 1.0, y: 1.0 });
        world.despawn(doomed).unwrap();

        let mut buf = CommandBuffer::new();

        // This should succeed.
        buf.set_component(
            alive,
            "position",
            serde_json::json!({"x": 5.0, "y": 5.0}),
            SystemId(0),
            CausalReason::PlayerInput("move".to_owned()),
        );

        // This should fail (stale entity).
        buf.set_component(
            doomed,
            "position",
            serde_json::json!({"x": 9.0, "y": 9.0}),
            SystemId(0),
            CausalReason::SystemInternal("stale".to_owned()),
        );

        let applied = buf.apply(&mut world);
        assert_eq!(applied.len(), 2);
        assert!(
            applied[0].applied_successfully,
            "command on alive entity should succeed"
        );
        assert!(
            !applied[1].applied_successfully,
            "command on stale entity should fail"
        );
    }

    // -- 19. Conflict detection -----------------------------------------------

    #[test]
    fn conflict_detection_warns_on_duplicate_target() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(0),
            CausalReason::PlayerInput("move1".to_owned()),
        );
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 2.0, "y": 0.0}),
            SystemId(1),
            CausalReason::PlayerInput("move2".to_owned()),
        );

        let _applied = buf.apply(&mut world);
        // Last-write-wins: position should be (2.0, 0.0).
        assert_eq!(
            world.get_component::<Position>(entity),
            Some(&Position { x: 2.0, y: 0.0 })
        );
        // Report should show 1 conflict.
        assert_eq!(buf.last_apply_report().conflict_count, 1);
    }

    // -- 20. No conflict for different components ----------------------------

    #[test]
    fn no_conflict_for_different_components() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(0),
            CausalReason::PlayerInput("move".to_owned()),
        );
        buf.set_component(
            entity,
            "health",
            serde_json::json!(100),
            SystemId(1),
            CausalReason::GameRule("heal".to_owned()),
        );

        let _applied = buf.apply(&mut world);
        assert_eq!(buf.last_apply_report().conflict_count, 0);
    }

    // -- 21. Apply report counts success and failure -------------------------

    #[test]
    fn apply_report_counts_success_and_failure() {
        let mut world = setup_world();
        let alive = world.spawn_with(Position { x: 0.0, y: 0.0 });
        let dead = world.spawn_with(Position { x: 1.0, y: 0.0 });
        world.despawn(dead).unwrap();

        let mut buf = CommandBuffer::new();
        buf.set_component(
            alive,
            "position",
            serde_json::json!({"x": 5.0, "y": 0.0}),
            SystemId(0),
            CausalReason::PlayerInput("move".to_owned()),
        );
        buf.set_component(
            dead,
            "position",
            serde_json::json!({"x": 9.0, "y": 0.0}),
            SystemId(0),
            CausalReason::SystemInternal("stale".to_owned()),
        );

        let _applied = buf.apply(&mut world);
        let report = buf.last_apply_report();
        assert_eq!(report.success_count, 1);
        assert_eq!(report.failed_count, 1);
    }

    // -- 22. No conflict for different entities ------------------------------

    #[test]
    fn no_conflict_for_different_entities() {
        let mut world = setup_world();
        let e1 = world.spawn_with(Position { x: 0.0, y: 0.0 });
        let e2 = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut buf = CommandBuffer::new();
        buf.set_component(
            e1,
            "position",
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId(0),
            CausalReason::PlayerInput("move1".to_owned()),
        );
        buf.set_component(
            e2,
            "position",
            serde_json::json!({"x": 2.0, "y": 0.0}),
            SystemId(0),
            CausalReason::PlayerInput("move2".to_owned()),
        );

        let _applied = buf.apply(&mut world);
        assert_eq!(buf.last_apply_report().conflict_count, 0);
    }
}
