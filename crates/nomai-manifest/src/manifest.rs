//! Manifest generation pipeline for the Nomai Engine.
//!
//! The [`ManifestPipeline`] is the central orchestrator that turns raw ECS
//! commands into a structured, queryable, causal representation of game state.
//! It maintains a rolling entity index across ticks and produces a
//! [`TickManifest`] at the end of each tick.
//!
//! # Architecture
//!
//! The pipeline sits between the tick loop and downstream consumers (AI
//! verification, Python bindings, debug tools). Each tick follows this flow:
//!
//! 1. [`ManifestPipeline::begin_tick`] -- clear per-tick state.
//! 2. Systems run, commands are applied to the world by the tick loop.
//! 3. [`ManifestPipeline::process_commands`] -- populate journal, detect
//!    spawns/despawns, update entity index.
//! 4. Optionally record game events via [`ManifestPipeline::record_event`].
//! 5. [`ManifestPipeline::end_tick`] -- compute aggregates, assemble the
//!    [`TickManifest`], push to rolling history.
//!
//! # Causal Chains
//!
//! Every component change carries causality metadata (system ID, reason,
//! command index). The [`ManifestPipeline::build_causal_chain`] method walks
//! backward through manifest history to assemble a chain of [`CausalStep`]s
//! tracing a change back to its root cause.
//!
//! # JSON Serialization
//!
//! All manifest types derive `Serialize` and `Deserialize` for JSON output via
//! `serde_json`. This enables the Python verification engine to consume
//! manifests directly.
//!
//! # Example
//!
//! ```
//! use nomai_manifest::manifest::ManifestPipeline;
//! use nomai_ecs::prelude::*;
//!
//! let mut pipeline = ManifestPipeline::new();
//!
//! // Each tick:
//! pipeline.begin_tick();
//! // ... tick loop runs, produces commands ...
//! // pipeline.process_commands(&commands, tick, &world);
//! // let manifest = pipeline.end_tick(tick, sim_time, system_names, &world);
//! ```

use std::collections::{HashMap, VecDeque};

use nomai_ecs::command::{CausalReason, Command, CommandKind};
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::SystemId;
use nomai_ecs::world::World;
use serde::{Deserialize, Serialize};

use crate::journal::{ChangeJournal, ComponentChange};

// ---------------------------------------------------------------------------
// GameEvent
// ---------------------------------------------------------------------------

/// A game event with involved entities and causality.
///
/// Events represent higher-level occurrences than individual component changes.
/// They can be recorded by systems or derived from command patterns (e.g., a
/// "collision" event involving two entities).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameEvent {
    /// The type of event (e.g., `"collision"`, `"score_change"`, `"level_up"`).
    pub event_type: String,
    /// Human-readable description of the event.
    pub description: String,
    /// Entities involved in this event.
    pub involved_entities: Vec<EntityId>,
    /// The system that caused or detected this event.
    pub caused_by: SystemId,
    /// Why this event occurred.
    pub reason: CausalReason,
    /// The tick during which this event occurred.
    pub tick: u64,
}

// ---------------------------------------------------------------------------
// CausalStep
// ---------------------------------------------------------------------------

/// A single step in a causal chain.
///
/// Represents one link in the chain tracing a state change back to its root
/// cause. Each step corresponds to a command that was applied to the world.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalStep {
    /// The tick in which this step occurred.
    pub tick: u64,
    /// The command index within the tick's command buffer.
    pub command_index: u64,
    /// The system that issued the command.
    pub system_id: SystemId,
    /// The causal reason attached to the command.
    pub reason: CausalReason,
    /// Human-readable description of what happened in this step.
    pub description: String,
}

// ---------------------------------------------------------------------------
// CausalChain
// ---------------------------------------------------------------------------

/// A causal chain tracing back from an observed component change.
///
/// The chain is ordered from most recent (the observed change) to oldest
/// (the root cause, as far back as history allows). Each step provides
/// the system, reason, and tick that produced the change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalChain {
    /// The entity whose component was changed.
    pub entity_id: EntityId,
    /// The component type that was changed.
    pub component: String,
    /// Steps in the chain, from most recent to oldest.
    pub steps: Vec<CausalStep>,
}

// ---------------------------------------------------------------------------
// Aggregates
// ---------------------------------------------------------------------------

/// Aggregate data computed over entity state at the end of a tick.
///
/// Provides summary statistics about the world state, useful for quick
/// verification checks (e.g., "are there still 3 enemies alive?").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aggregates {
    /// Count of entities by identity tier (`"Semantic"` or `"Pooled"`).
    pub entity_count_by_tier: HashMap<String, usize>,
    /// Count of entities by entity_type (semantic entities) or pool_type (pooled entities).
    pub entity_count_by_type: HashMap<String, usize>,
    /// Total number of alive entities in the world.
    pub total_entity_count: usize,
}

// ---------------------------------------------------------------------------
// EntityEntry
// ---------------------------------------------------------------------------

/// A single entity entry in the entity index.
///
/// The entity index is maintained across ticks and tracks every entity that
/// has ever existed, including despawned ones. This enables historical queries
/// and causal chain assembly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityEntry {
    /// The entity's ID.
    pub entity_id: EntityId,
    /// The identity tier: `"Semantic"` or `"Pooled"`.
    pub tier: String,
    /// The entity type (from [`EntityIdentity::entity_type`] or [`PoolIdentity::pool_type`]).
    pub entity_type: String,
    /// The entity role (from [`EntityIdentity::role`], empty for pooled entities).
    pub role: String,
    /// Whether the entity is currently alive.
    pub alive: bool,
    /// The tick at which the entity was spawned.
    pub spawned_at_tick: u64,
    /// The tick at which the entity was despawned, or `None` if still alive.
    pub despawned_at_tick: Option<u64>,
}

// ---------------------------------------------------------------------------
// TickManifest
// ---------------------------------------------------------------------------

/// The complete manifest for a single simulation tick.
///
/// Contains all state changes, events, and aggregate data for one tick.
/// This is the primary output of the manifest pipeline and the input to
/// the AI verification engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickManifest {
    /// The tick number.
    pub tick: u64,
    /// The simulation time in seconds at the end of this tick.
    pub sim_time: f64,
    /// Entity IDs that were spawned during this tick.
    pub entity_spawns: Vec<EntityId>,
    /// Entity IDs that were despawned during this tick.
    pub entity_despawns: Vec<EntityId>,
    /// All component changes that occurred during this tick.
    pub component_changes: Vec<ComponentChange>,
    /// Game events that occurred during this tick.
    pub events: Vec<GameEvent>,
    /// Aggregate data computed at the end of this tick.
    pub aggregates: Aggregates,
    /// Names of systems that were executed during this tick.
    pub systems_executed: Vec<String>,
    /// Total number of commands processed (successful + failed).
    pub commands_processed: usize,
    /// Number of commands that were applied successfully.
    pub commands_succeeded: usize,
}

// ---------------------------------------------------------------------------
// ManifestPipeline
// ---------------------------------------------------------------------------

/// The manifest pipeline. Maintains state across ticks and generates
/// per-tick manifests.
///
/// The pipeline owns a rolling entity index that tracks all entities that
/// have ever existed, a change journal for the current tick, and a bounded
/// history of recent tick manifests.
pub struct ManifestPipeline {
    /// Rolling entity index -- tracks all entities that have ever existed.
    entity_index: HashMap<EntityId, EntityEntry>,
    /// Change journal for the current tick.
    journal: ChangeJournal,
    /// Game events for the current tick.
    events: Vec<GameEvent>,
    /// Spawns detected during the current tick.
    current_spawns: Vec<EntityId>,
    /// Despawns detected during the current tick.
    current_despawns: Vec<EntityId>,
    /// Total commands processed in the current tick.
    current_commands_processed: usize,
    /// Successful commands in the current tick.
    current_commands_succeeded: usize,
    /// History of recent tick manifests (rolling window).
    history: VecDeque<TickManifest>,
    /// Maximum number of tick manifests to retain in history.
    max_history: usize,
}

impl ManifestPipeline {
    /// Create a new manifest pipeline with the default history size (60 ticks).
    pub fn new() -> Self {
        Self::with_max_history(60)
    }

    /// Create a new manifest pipeline with a custom history size.
    pub fn with_max_history(max_history: usize) -> Self {
        Self {
            entity_index: HashMap::new(),
            journal: ChangeJournal::new(),
            events: Vec::new(),
            current_spawns: Vec::new(),
            current_despawns: Vec::new(),
            current_commands_processed: 0,
            current_commands_succeeded: 0,
            history: VecDeque::new(),
            max_history,
        }
    }

    /// Begin a new tick: clear per-tick state.
    ///
    /// Must be called before [`process_commands`](Self::process_commands) and
    /// [`end_tick`](Self::end_tick) for each tick.
    pub fn begin_tick(&mut self) {
        self.journal.clear();
        self.events.clear();
        self.current_spawns.clear();
        self.current_despawns.clear();
        self.current_commands_processed = 0;
        self.current_commands_succeeded = 0;
    }

    /// Process applied commands from a tick.
    ///
    /// Populates the change journal with component changes, detects
    /// spawns/despawns, and updates the entity index. Call after the tick
    /// loop's `tick()` method returns the applied commands.
    ///
    /// Only commands with `applied_successfully == true` are recorded in the
    /// journal and entity index. Failed commands are counted but not recorded.
    pub fn process_commands(&mut self, commands: &[Command], tick: u64, _world: &World) {
        self.current_commands_processed = commands.len();

        for cmd in commands {
            if !cmd.applied_successfully {
                continue;
            }

            self.current_commands_succeeded += 1;

            match &cmd.kind {
                CommandKind::SpawnSemantic {
                    identity,
                    components,
                } => {
                    let entity_id = cmd
                        .spawned_entity
                        .expect("successful SpawnSemantic must have spawned_entity");

                    // Add to entity index.
                    self.entity_index.insert(
                        entity_id,
                        EntityEntry {
                            entity_id,
                            tier: "Semantic".to_owned(),
                            entity_type: identity.entity_type.clone(),
                            role: identity.role.clone(),
                            alive: true,
                            spawned_at_tick: tick,
                            despawned_at_tick: None,
                        },
                    );

                    self.current_spawns.push(entity_id);

                    // Record component changes for spawn.
                    for (comp_name, value) in components {
                        self.journal.record_change(ComponentChange {
                            entity_id,
                            component_type_name: comp_name.clone(),
                            old_value: None,
                            new_value: Some(value.clone()),
                            changed_by: cmd.issued_by,
                            reason: cmd.reason.clone(),
                            command_index: cmd.command_index as u64,
                            tick,
                        });
                    }
                }

                CommandKind::SpawnPooled {
                    identity,
                    components,
                } => {
                    let entity_id = cmd
                        .spawned_entity
                        .expect("successful SpawnPooled must have spawned_entity");

                    // Add to entity index.
                    self.entity_index.insert(
                        entity_id,
                        EntityEntry {
                            entity_id,
                            tier: "Pooled".to_owned(),
                            entity_type: identity.pool_type.clone(),
                            role: identity.variant.clone(),
                            alive: true,
                            spawned_at_tick: tick,
                            despawned_at_tick: None,
                        },
                    );

                    self.current_spawns.push(entity_id);

                    // Record component changes for spawn.
                    for (comp_name, value) in components {
                        self.journal.record_change(ComponentChange {
                            entity_id,
                            component_type_name: comp_name.clone(),
                            old_value: None,
                            new_value: Some(value.clone()),
                            changed_by: cmd.issued_by,
                            reason: cmd.reason.clone(),
                            command_index: cmd.command_index as u64,
                            tick,
                        });
                    }
                }

                CommandKind::SetComponent {
                    component_name,
                    value,
                } => {
                    let entity_id = cmd.target.expect("SetComponent must have a target entity");

                    self.journal.record_change(ComponentChange {
                        entity_id,
                        component_type_name: component_name.clone(),
                        old_value: None, // Old value not available post-apply in this spike.
                        new_value: Some(value.clone()),
                        changed_by: cmd.issued_by,
                        reason: cmd.reason.clone(),
                        command_index: cmd.command_index as u64,
                        tick,
                    });
                }

                CommandKind::RemoveComponent { component_name } => {
                    let entity_id = cmd
                        .target
                        .expect("RemoveComponent must have a target entity");

                    self.journal.record_change(ComponentChange {
                        entity_id,
                        component_type_name: component_name.clone(),
                        old_value: None, // Old value not available post-apply in this spike.
                        new_value: None,
                        changed_by: cmd.issued_by,
                        reason: cmd.reason.clone(),
                        command_index: cmd.command_index as u64,
                        tick,
                    });
                }

                CommandKind::Despawn => {
                    let entity_id = cmd.target.expect("Despawn must have a target entity");

                    // Mark as despawned in entity index.
                    if let Some(entry) = self.entity_index.get_mut(&entity_id) {
                        entry.alive = false;
                        entry.despawned_at_tick = Some(tick);
                    }

                    self.current_despawns.push(entity_id);

                    // Record the despawn as a component change (entity removed).
                    self.journal.record_change(ComponentChange {
                        entity_id,
                        component_type_name: "__entity".to_owned(),
                        old_value: Some(serde_json::json!("alive")),
                        new_value: None,
                        changed_by: cmd.issued_by,
                        reason: cmd.reason.clone(),
                        command_index: cmd.command_index as u64,
                        tick,
                    });
                }
            }
        }
    }

    /// Record a game event for the current tick.
    ///
    /// Events represent higher-level occurrences that may involve multiple
    /// entities and commands. They are included in the tick manifest's
    /// `events` field.
    pub fn record_event(&mut self, event: GameEvent) {
        self.events.push(event);
    }

    /// Finalize the tick and produce a [`TickManifest`].
    ///
    /// Computes aggregates from the current world state and entity index,
    /// assembles the manifest, and pushes it to the rolling history. Old
    /// manifests beyond `max_history` are discarded.
    pub fn end_tick(
        &mut self,
        tick: u64,
        sim_time: f64,
        systems_executed: Vec<String>,
        world: &World,
    ) -> TickManifest {
        let aggregates = self.compute_aggregates(world);

        let manifest = TickManifest {
            tick,
            sim_time,
            entity_spawns: self.current_spawns.clone(),
            entity_despawns: self.current_despawns.clone(),
            component_changes: self.journal.all_changes().to_vec(),
            events: self.events.clone(),
            aggregates,
            systems_executed,
            commands_processed: self.current_commands_processed,
            commands_succeeded: self.current_commands_succeeded,
        };

        // Push to history, trimming if necessary.
        self.history.push_back(manifest.clone());
        while self.history.len() > self.max_history {
            self.history.pop_front();
        }

        manifest
    }

    /// Build a causal chain for a given component change.
    ///
    /// Starts with the given change and walks backward through manifest
    /// history, looking for prior changes to the same entity and component.
    /// Each matching historical change becomes a step in the chain.
    pub fn build_causal_chain(&self, change: &ComponentChange) -> CausalChain {
        let mut steps = Vec::new();

        // First step: the change itself.
        steps.push(CausalStep {
            tick: change.tick,
            command_index: change.command_index,
            system_id: change.changed_by,
            reason: change.reason.clone(),
            description: format!(
                "System {:?} changed {} on {:?}: {:?}",
                change.changed_by, change.component_type_name, change.entity_id, change.reason,
            ),
        });

        // Walk backward through history for prior changes to the same
        // entity + component.
        for manifest in self.history.iter().rev() {
            // Skip manifests at or after the change's tick (we want prior).
            if manifest.tick >= change.tick {
                continue;
            }

            for prior_change in &manifest.component_changes {
                if prior_change.entity_id == change.entity_id
                    && prior_change.component_type_name == change.component_type_name
                {
                    steps.push(CausalStep {
                        tick: prior_change.tick,
                        command_index: prior_change.command_index,
                        system_id: prior_change.changed_by,
                        reason: prior_change.reason.clone(),
                        description: format!(
                            "System {:?} changed {} on {:?}: {:?}",
                            prior_change.changed_by,
                            prior_change.component_type_name,
                            prior_change.entity_id,
                            prior_change.reason,
                        ),
                    });
                }
            }
        }

        CausalChain {
            entity_id: change.entity_id,
            component: change.component_type_name.clone(),
            steps,
        }
    }

    /// Get a reference to the entity index.
    pub fn entity_index(&self) -> &HashMap<EntityId, EntityEntry> {
        &self.entity_index
    }

    /// Get the manifest history.
    pub fn history(&self) -> &VecDeque<TickManifest> {
        &self.history
    }

    /// Get a specific historical manifest by tick number.
    ///
    /// Returns `None` if the tick is not in the rolling history window.
    pub fn manifest_at_tick(&self, tick: u64) -> Option<&TickManifest> {
        self.history.iter().find(|m| m.tick == tick)
    }

    /// Get a reference to the current tick's change journal.
    pub fn journal(&self) -> &ChangeJournal {
        &self.journal
    }

    // -- internal helpers ---------------------------------------------------

    /// Compute aggregate data from the entity index and world state.
    fn compute_aggregates(&self, world: &World) -> Aggregates {
        let mut entity_count_by_tier: HashMap<String, usize> = HashMap::new();
        let mut entity_count_by_type: HashMap<String, usize> = HashMap::new();
        let mut total_entity_count: usize = 0;

        // Walk the entity index for alive entities.
        for entry in self.entity_index.values() {
            if !entry.alive {
                continue;
            }
            total_entity_count += 1;
            *entity_count_by_tier.entry(entry.tier.clone()).or_insert(0) += 1;
            *entity_count_by_type
                .entry(entry.entity_type.clone())
                .or_insert(0) += 1;
        }

        // Also count entities that exist in the world but are not in our index
        // (e.g., entities spawned directly without going through the command buffer).
        // Use the world's entity count as a sanity baseline.
        let world_count = world.entity_count();
        if world_count > total_entity_count {
            // There are entities in the world we don't track (non-tiered spawns).
            // Record the difference as "Untracked".
            let untracked = world_count - total_entity_count;
            if untracked > 0 {
                *entity_count_by_tier
                    .entry("Untracked".to_owned())
                    .or_insert(0) += untracked;
                total_entity_count = world_count;
            }
        }

        Aggregates {
            entity_count_by_tier,
            entity_count_by_type,
            total_entity_count,
        }
    }
}

impl Default for ManifestPipeline {
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
    use nomai_ecs::command::{CausalReason, CommandBuffer};
    use nomai_ecs::entity::EntityId;
    use nomai_ecs::identity::{EntityIdentity, PoolIdentity, SystemId};
    use nomai_ecs::world::World;

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

    fn player_identity() -> EntityIdentity {
        EntityIdentity {
            entity_type: "character".to_owned(),
            role: "player".to_owned(),
            spawned_by: SystemId::PLAYER_SPAWNER,
            requirement_id: Some("REQ-001".to_owned()),
        }
    }

    fn enemy_identity() -> EntityIdentity {
        EntityIdentity {
            entity_type: "character".to_owned(),
            role: "enemy".to_owned(),
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

    // -- 1. Empty tick produces valid manifest ------------------------------

    #[test]
    fn empty_tick_produces_valid_manifest() {
        let world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        pipeline.begin_tick();
        let commands: Vec<Command> = Vec::new();
        pipeline.process_commands(&commands, 1, &world);
        let manifest = pipeline.end_tick(1, 1.0 / 60.0, vec!["physics".to_owned()], &world);

        assert_eq!(manifest.tick, 1);
        assert!((manifest.sim_time - 1.0 / 60.0).abs() < f64::EPSILON);
        assert!(manifest.entity_spawns.is_empty());
        assert!(manifest.entity_despawns.is_empty());
        assert!(manifest.component_changes.is_empty());
        assert!(manifest.events.is_empty());
        assert_eq!(manifest.systems_executed, vec!["physics"]);
        assert_eq!(manifest.commands_processed, 0);
        assert_eq!(manifest.commands_succeeded, 0);
        assert_eq!(manifest.aggregates.total_entity_count, 0);
    }

    // -- 2. Spawn command produces entity_spawns + entity_index entry ------

    #[test]
    fn spawn_produces_entity_spawns_and_index_entry() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        let mut buf = CommandBuffer::new();
        buf.spawn_semantic(
            player_identity(),
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
        let spawned_id = applied[0].spawned_entity.unwrap();

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        let manifest = pipeline.end_tick(1, 0.0, vec![], &world);

        assert_eq!(manifest.entity_spawns.len(), 1);
        assert_eq!(manifest.entity_spawns[0], spawned_id);

        // Entity index should have the entry.
        let entry = pipeline.entity_index().get(&spawned_id).unwrap();
        assert_eq!(entry.tier, "Semantic");
        assert_eq!(entry.entity_type, "character");
        assert_eq!(entry.role, "player");
        assert!(entry.alive);
        assert_eq!(entry.spawned_at_tick, 1);
        assert!(entry.despawned_at_tick.is_none());

        // Component changes should include the spawned components.
        assert!(manifest.component_changes.len() >= 2);
    }

    // -- 3. Despawn command produces entity_despawns + index updated -------

    #[test]
    fn despawn_produces_entity_despawns_and_updates_index() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        // First: spawn an entity.
        let mut buf1 = CommandBuffer::new();
        buf1.spawn_semantic(
            player_identity(),
            vec![("health".to_owned(), serde_json::json!(100))],
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("player_spawn".to_owned()),
        );
        let applied1 = buf1.apply(&mut world);
        let entity_id = applied1[0].spawned_entity.unwrap();

        pipeline.begin_tick();
        pipeline.process_commands(&applied1, 1, &world);
        pipeline.end_tick(1, 0.0, vec![], &world);

        // Second: despawn the entity.
        let mut buf2 = CommandBuffer::new();
        buf2.despawn(
            entity_id,
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("entity_destroyed".to_owned()),
        );
        let applied2 = buf2.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied2, 2, &world);
        let manifest = pipeline.end_tick(2, 0.0, vec![], &world);

        assert_eq!(manifest.entity_despawns.len(), 1);
        assert_eq!(manifest.entity_despawns[0], entity_id);

        // Entity index should be updated.
        let entry = pipeline.entity_index().get(&entity_id).unwrap();
        assert!(!entry.alive);
        assert_eq!(entry.despawned_at_tick, Some(2));
    }

    // -- 4. SetComponent produces component_changes entry -----------------

    #[test]
    fn set_component_produces_component_change() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

        let mut pipeline = ManifestPipeline::new();

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 42.0, "y": 99.0}),
            SystemId::PHYSICS,
            CausalReason::PlayerInput("move".to_owned()),
        );
        let applied = buf.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        let manifest = pipeline.end_tick(1, 0.0, vec![], &world);

        assert_eq!(manifest.component_changes.len(), 1);
        let change = &manifest.component_changes[0];
        assert_eq!(change.entity_id, entity);
        assert_eq!(change.component_type_name, "position");
        assert_eq!(
            change.new_value,
            Some(serde_json::json!({"x": 42.0, "y": 99.0}))
        );
        assert_eq!(change.changed_by, SystemId::PHYSICS);
        assert_eq!(change.reason, CausalReason::PlayerInput("move".to_owned()));
    }

    // -- 5. Multiple commands in one tick all recorded ---------------------

    #[test]
    fn multiple_commands_all_recorded() {
        let mut world = setup_world();
        let e1 = world.spawn_with(Position { x: 0.0, y: 0.0 });
        let e2 = world.spawn_with(Position { x: 1.0, y: 1.0 });

        let mut pipeline = ManifestPipeline::new();

        let mut buf = CommandBuffer::new();
        buf.set_component(
            e1,
            "position",
            serde_json::json!({"x": 10.0, "y": 20.0}),
            SystemId(1),
            CausalReason::PlayerInput("move".to_owned()),
        );
        buf.set_component(
            e2,
            "position",
            serde_json::json!({"x": 30.0, "y": 40.0}),
            SystemId(2),
            CausalReason::SystemInternal("ai_move".to_owned()),
        );
        buf.spawn_semantic(
            enemy_identity(),
            vec![("health".to_owned(), serde_json::json!(50))],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("enemy_spawn".to_owned()),
        );
        let applied = buf.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        let manifest = pipeline.end_tick(1, 0.0, vec![], &world);

        // 2 SetComponent + 1 spawn with 1 component = 3 component changes.
        assert_eq!(manifest.component_changes.len(), 3);
        assert_eq!(manifest.commands_processed, 3);
        assert_eq!(manifest.commands_succeeded, 3);
        assert_eq!(manifest.entity_spawns.len(), 1);
    }

    // -- 6. Aggregates computed correctly ---------------------------------

    #[test]
    fn aggregates_computed_correctly() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        // Spawn 2 semantic entities and 3 pooled entities via command buffer.
        let mut buf = CommandBuffer::new();
        buf.spawn_semantic(
            player_identity(),
            vec![],
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("spawn".to_owned()),
        );
        buf.spawn_semantic(
            enemy_identity(),
            vec![],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("spawn".to_owned()),
        );
        buf.spawn_pooled(
            brick_pool_identity(),
            vec![],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("spawn".to_owned()),
        );
        buf.spawn_pooled(
            brick_pool_identity(),
            vec![],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("spawn".to_owned()),
        );
        buf.spawn_pooled(
            PoolIdentity {
                pool_type: "collectible".to_owned(),
                variant: "coin".to_owned(),
            },
            vec![],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("spawn".to_owned()),
        );
        let applied = buf.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        let manifest = pipeline.end_tick(1, 0.0, vec![], &world);

        assert_eq!(manifest.aggregates.total_entity_count, 5);
        assert_eq!(
            *manifest
                .aggregates
                .entity_count_by_tier
                .get("Semantic")
                .unwrap_or(&0),
            2
        );
        assert_eq!(
            *manifest
                .aggregates
                .entity_count_by_tier
                .get("Pooled")
                .unwrap_or(&0),
            3
        );
        assert_eq!(
            *manifest
                .aggregates
                .entity_count_by_type
                .get("character")
                .unwrap_or(&0),
            2
        );
        assert_eq!(
            *manifest
                .aggregates
                .entity_count_by_type
                .get("destructible")
                .unwrap_or(&0),
            2
        );
        assert_eq!(
            *manifest
                .aggregates
                .entity_count_by_type
                .get("collectible")
                .unwrap_or(&0),
            1
        );
    }

    // -- 7. Systems executed list correct ----------------------------------

    #[test]
    fn systems_executed_list_correct() {
        let world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        pipeline.begin_tick();
        pipeline.process_commands(&[], 1, &world);
        let manifest = pipeline.end_tick(
            1,
            0.0,
            vec![
                "physics".to_owned(),
                "movement".to_owned(),
                "gameplay".to_owned(),
            ],
            &world,
        );

        assert_eq!(manifest.systems_executed.len(), 3);
        assert_eq!(manifest.systems_executed[0], "physics");
        assert_eq!(manifest.systems_executed[1], "movement");
        assert_eq!(manifest.systems_executed[2], "gameplay");
    }

    // -- 8. Entity index maintained across ticks --------------------------

    #[test]
    fn entity_index_maintained_across_ticks() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        // Tick 1: spawn entity.
        let mut buf1 = CommandBuffer::new();
        buf1.spawn_semantic(
            player_identity(),
            vec![("health".to_owned(), serde_json::json!(100))],
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("spawn".to_owned()),
        );
        let applied1 = buf1.apply(&mut world);
        let entity_id = applied1[0].spawned_entity.unwrap();

        pipeline.begin_tick();
        pipeline.process_commands(&applied1, 1, &world);
        pipeline.end_tick(1, 0.0, vec![], &world);

        // Tick 2: modify entity.
        let mut buf2 = CommandBuffer::new();
        buf2.set_component(
            entity_id,
            "health",
            serde_json::json!(75),
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("damage".to_owned()),
        );
        let applied2 = buf2.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied2, 2, &world);
        pipeline.end_tick(2, 0.0, vec![], &world);

        // Entity should still be in the index from tick 1.
        let entry = pipeline.entity_index().get(&entity_id).unwrap();
        assert!(entry.alive);
        assert_eq!(entry.spawned_at_tick, 1);
        assert_eq!(entry.entity_type, "character");
        assert_eq!(entry.role, "player");

        // History should have 2 manifests.
        assert_eq!(pipeline.history().len(), 2);
    }

    // -- 9. Manifest serialization roundtrip ------------------------------

    #[test]
    fn manifest_serialization_roundtrip() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        let mut buf = CommandBuffer::new();
        buf.spawn_semantic(
            player_identity(),
            vec![
                (
                    "position".to_owned(),
                    serde_json::json!({"x": 5.0, "y": 10.0}),
                ),
                ("health".to_owned(), serde_json::json!(100)),
            ],
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("player_spawn".to_owned()),
        );
        let applied = buf.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        pipeline.record_event(GameEvent {
            event_type: "spawn".to_owned(),
            description: "Player spawned".to_owned(),
            involved_entities: vec![applied[0].spawned_entity.unwrap()],
            caused_by: SystemId::PLAYER_SPAWNER,
            reason: CausalReason::GameRule("player_spawn".to_owned()),
            tick: 1,
        });
        let manifest = pipeline.end_tick(1, 1.0 / 60.0, vec!["spawner".to_owned()], &world);

        // Serialize to JSON.
        let json = serde_json::to_string_pretty(&manifest).unwrap();

        // Deserialize back.
        let deserialized: TickManifest = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.tick, manifest.tick);
        assert_eq!(deserialized.sim_time, manifest.sim_time);
        assert_eq!(
            deserialized.entity_spawns.len(),
            manifest.entity_spawns.len()
        );
        assert_eq!(
            deserialized.component_changes.len(),
            manifest.component_changes.len()
        );
        assert_eq!(deserialized.events.len(), manifest.events.len());
        assert_eq!(deserialized.systems_executed, manifest.systems_executed);
        assert_eq!(deserialized.commands_processed, manifest.commands_processed);
        assert_eq!(deserialized.commands_succeeded, manifest.commands_succeeded);
    }

    // -- 10. CausalChain assembly from a component change -----------------

    #[test]
    fn causal_chain_assembly() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });
        let mut pipeline = ManifestPipeline::new();

        // Tick 1: initial modification.
        let mut buf1 = CommandBuffer::new();
        buf1.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 0.0}),
            SystemId::PHYSICS,
            CausalReason::PlayerInput("move_right".to_owned()),
        );
        let applied1 = buf1.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied1, 1, &world);
        pipeline.end_tick(1, 0.0, vec![], &world);

        // Tick 2: further modification.
        let mut buf2 = CommandBuffer::new();
        buf2.set_component(
            entity,
            "position",
            serde_json::json!({"x": 2.0, "y": 0.0}),
            SystemId::PHYSICS,
            CausalReason::PlayerInput("move_right".to_owned()),
        );
        let applied2 = buf2.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied2, 2, &world);
        let manifest2 = pipeline.end_tick(2, 0.0, vec![], &world);

        // Build causal chain from tick 2's change.
        let change = &manifest2.component_changes[0];
        let chain = pipeline.build_causal_chain(change);

        assert_eq!(chain.entity_id, entity);
        assert_eq!(chain.component, "position");
        assert_eq!(chain.steps.len(), 2); // tick 2 + tick 1

        // Most recent step first.
        assert_eq!(chain.steps[0].tick, 2);
        assert_eq!(chain.steps[0].system_id, SystemId::PHYSICS);

        // Prior step.
        assert_eq!(chain.steps[1].tick, 1);
        assert_eq!(chain.steps[1].system_id, SystemId::PHYSICS);
    }

    // -- 11. History rolling window works ---------------------------------

    #[test]
    fn history_rolling_window() {
        let world = setup_world();
        let mut pipeline = ManifestPipeline::with_max_history(3);

        for tick in 1..=5u64 {
            pipeline.begin_tick();
            pipeline.process_commands(&[], tick, &world);
            pipeline.end_tick(tick, tick as f64 * 0.016, vec![], &world);
        }

        // Only the last 3 ticks should be in history.
        assert_eq!(pipeline.history().len(), 3);
        assert_eq!(pipeline.history()[0].tick, 3);
        assert_eq!(pipeline.history()[1].tick, 4);
        assert_eq!(pipeline.history()[2].tick, 5);

        // manifest_at_tick should find recent ones.
        assert!(pipeline.manifest_at_tick(5).is_some());
        assert!(pipeline.manifest_at_tick(3).is_some());

        // But not old ones.
        assert!(pipeline.manifest_at_tick(1).is_none());
        assert!(pipeline.manifest_at_tick(2).is_none());
    }

    // -- 12. Integration: 10-tick simulation with spawns/mods/despawns ----

    #[test]
    fn integration_10_tick_simulation() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        let mut spawned_entities: Vec<EntityId> = Vec::new();

        // Tick 1: Spawn 3 entities.
        {
            let mut buf = CommandBuffer::new();
            buf.spawn_semantic(
                player_identity(),
                vec![
                    (
                        "position".to_owned(),
                        serde_json::json!({"x": 0.0, "y": 0.0}),
                    ),
                    ("health".to_owned(), serde_json::json!(100)),
                ],
                SystemId::PLAYER_SPAWNER,
                CausalReason::GameRule("player_spawn".to_owned()),
            );
            buf.spawn_semantic(
                enemy_identity(),
                vec![
                    (
                        "position".to_owned(),
                        serde_json::json!({"x": 50.0, "y": 0.0}),
                    ),
                    ("health".to_owned(), serde_json::json!(50)),
                ],
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule("enemy_spawn".to_owned()),
            );
            buf.spawn_pooled(
                brick_pool_identity(),
                vec![(
                    "position".to_owned(),
                    serde_json::json!({"x": 25.0, "y": 25.0}),
                )],
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule("brick_spawn".to_owned()),
            );
            let applied = buf.apply(&mut world);
            for cmd in &applied {
                if let Some(eid) = cmd.spawned_entity {
                    spawned_entities.push(eid);
                }
            }

            pipeline.begin_tick();
            pipeline.process_commands(&applied, 1, &world);
            let m = pipeline.end_tick(1, 1.0 / 60.0, vec!["spawner".to_owned()], &world);

            assert_eq!(m.entity_spawns.len(), 3);
            assert_eq!(m.aggregates.total_entity_count, 3);
        }

        let player = spawned_entities[0];
        let enemy = spawned_entities[1];
        let brick = spawned_entities[2];

        // Ticks 2-5: Move player, damage enemy.
        for tick in 2..=5u64 {
            let mut buf = CommandBuffer::new();
            buf.set_component(
                player,
                "position",
                serde_json::json!({"x": tick as f64 * 5.0, "y": 0.0}),
                SystemId::PHYSICS,
                CausalReason::PlayerInput("move_right".to_owned()),
            );
            buf.set_component(
                enemy,
                "health",
                serde_json::json!(50u32.saturating_sub(tick as u32 * 10)),
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule("take_damage".to_owned()),
            );
            let applied = buf.apply(&mut world);

            pipeline.begin_tick();
            pipeline.process_commands(&applied, tick, &world);
            let m = pipeline.end_tick(
                tick,
                tick as f64 / 60.0,
                vec!["physics".to_owned(), "gameplay".to_owned()],
                &world,
            );

            assert_eq!(m.component_changes.len(), 2);
            assert_eq!(m.commands_succeeded, 2);
        }

        // Tick 6: Despawn the enemy (health reached 0).
        {
            let mut buf = CommandBuffer::new();
            buf.despawn(
                enemy,
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule("enemy_destroyed".to_owned()),
            );
            let applied = buf.apply(&mut world);

            pipeline.begin_tick();
            pipeline.process_commands(&applied, 6, &world);
            let m = pipeline.end_tick(6, 6.0 / 60.0, vec!["reaper".to_owned()], &world);

            assert_eq!(m.entity_despawns.len(), 1);
            assert_eq!(m.entity_despawns[0], enemy);
            assert_eq!(m.aggregates.total_entity_count, 2);
        }

        // Ticks 7-9: Continue moving player, despawn brick on tick 8.
        for tick in 7..=9u64 {
            let mut buf = CommandBuffer::new();
            buf.set_component(
                player,
                "position",
                serde_json::json!({"x": tick as f64 * 5.0, "y": 0.0}),
                SystemId::PHYSICS,
                CausalReason::PlayerInput("move_right".to_owned()),
            );
            if tick == 8 {
                buf.despawn(
                    brick,
                    SystemId::WASM_GAMEPLAY,
                    CausalReason::GameRule("brick_destroyed".to_owned()),
                );
            }
            let applied = buf.apply(&mut world);

            pipeline.begin_tick();
            pipeline.process_commands(&applied, tick, &world);
            let m = pipeline.end_tick(
                tick,
                tick as f64 / 60.0,
                vec!["physics".to_owned(), "gameplay".to_owned()],
                &world,
            );

            if tick == 8 {
                assert_eq!(m.entity_despawns.len(), 1);
                assert_eq!(m.aggregates.total_entity_count, 1);
            }
        }

        // Tick 10: Spawn a new pooled entity.
        {
            let mut buf = CommandBuffer::new();
            buf.spawn_pooled(
                PoolIdentity {
                    pool_type: "collectible".to_owned(),
                    variant: "coin".to_owned(),
                },
                vec![(
                    "position".to_owned(),
                    serde_json::json!({"x": 100.0, "y": 0.0}),
                )],
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule("coin_spawn".to_owned()),
            );
            buf.set_component(
                player,
                "position",
                serde_json::json!({"x": 50.0, "y": 0.0}),
                SystemId::PHYSICS,
                CausalReason::PlayerInput("move_right".to_owned()),
            );
            let applied = buf.apply(&mut world);

            pipeline.begin_tick();
            pipeline.process_commands(&applied, 10, &world);
            let m = pipeline.end_tick(
                10,
                10.0 / 60.0,
                vec!["physics".to_owned(), "spawner".to_owned()],
                &world,
            );

            assert_eq!(m.entity_spawns.len(), 1);
            assert_eq!(m.aggregates.total_entity_count, 2); // player + new coin
        }

        // Final verification: entity index has all 4 entities ever created.
        assert_eq!(pipeline.entity_index().len(), 4);

        // Player is alive.
        let player_entry = pipeline.entity_index().get(&player).unwrap();
        assert!(player_entry.alive);

        // Enemy is dead.
        let enemy_entry = pipeline.entity_index().get(&enemy).unwrap();
        assert!(!enemy_entry.alive);
        assert_eq!(enemy_entry.despawned_at_tick, Some(6));

        // Brick is dead.
        let brick_entry = pipeline.entity_index().get(&brick).unwrap();
        assert!(!brick_entry.alive);
        assert_eq!(brick_entry.despawned_at_tick, Some(8));

        // History should have all 10 ticks (default max_history is 60).
        assert_eq!(pipeline.history().len(), 10);

        // Verify causal chain on player position from tick 10.
        let tick10 = pipeline.manifest_at_tick(10).unwrap();
        let player_pos_change = tick10
            .component_changes
            .iter()
            .find(|c| c.entity_id == player && c.component_type_name == "position")
            .expect("player position change should exist in tick 10");

        let chain = pipeline.build_causal_chain(player_pos_change);
        assert_eq!(chain.entity_id, player);
        assert_eq!(chain.component, "position");
        // Should have multiple steps: tick 10 + ticks 2-5 + ticks 7-9 = 8 steps.
        assert!(
            chain.steps.len() >= 2,
            "causal chain should have multiple steps, got {}",
            chain.steps.len()
        );

        // Verify JSON roundtrip on the final manifest.
        let json = serde_json::to_string_pretty(&tick10).unwrap();
        let roundtrip: TickManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtrip.tick, 10);
        assert_eq!(
            roundtrip.component_changes.len(),
            tick10.component_changes.len()
        );
    }

    // -- 13. Game events recorded in manifest -----------------------------

    #[test]
    fn game_events_recorded_in_manifest() {
        let world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        pipeline.begin_tick();
        pipeline.record_event(GameEvent {
            event_type: "collision".to_owned(),
            description: "Player collided with enemy".to_owned(),
            involved_entities: vec![EntityId::new(0, 0), EntityId::new(1, 0)],
            caused_by: SystemId::PHYSICS,
            reason: CausalReason::CollisionResponse(EntityId::new(0, 0), EntityId::new(1, 0)),
            tick: 1,
        });
        pipeline.record_event(GameEvent {
            event_type: "score_change".to_owned(),
            description: "Score increased by 100".to_owned(),
            involved_entities: vec![EntityId::new(0, 0)],
            caused_by: SystemId::WASM_GAMEPLAY,
            reason: CausalReason::GameRule("score_on_hit".to_owned()),
            tick: 1,
        });
        pipeline.process_commands(&[], 1, &world);
        let manifest = pipeline.end_tick(1, 0.0, vec![], &world);

        assert_eq!(manifest.events.len(), 2);
        assert_eq!(manifest.events[0].event_type, "collision");
        assert_eq!(manifest.events[1].event_type, "score_change");
    }

    // -- 14. Failed commands are counted but not recorded -----------------

    #[test]
    fn failed_commands_counted_not_recorded() {
        let mut world = setup_world();
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });
        // Despawn the entity so commands targeting it will fail.
        world.despawn(entity).unwrap();

        let mut pipeline = ManifestPipeline::new();

        let mut buf = CommandBuffer::new();
        buf.set_component(
            entity,
            "position",
            serde_json::json!({"x": 1.0, "y": 1.0}),
            SystemId(0),
            CausalReason::SystemInternal("stale".to_owned()),
        );
        let applied = buf.apply(&mut world);
        assert!(!applied[0].applied_successfully);

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        let manifest = pipeline.end_tick(1, 0.0, vec![], &world);

        assert_eq!(manifest.commands_processed, 1);
        assert_eq!(manifest.commands_succeeded, 0);
        assert!(manifest.component_changes.is_empty());
    }

    // -- 15. Pooled entity spawn recorded correctly -----------------------

    #[test]
    fn pooled_entity_spawn_recorded_correctly() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        let mut buf = CommandBuffer::new();
        buf.spawn_pooled(
            brick_pool_identity(),
            vec![(
                "position".to_owned(),
                serde_json::json!({"x": 10.0, "y": 10.0}),
            )],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("brick_spawn".to_owned()),
        );
        let applied = buf.apply(&mut world);
        let brick_id = applied[0].spawned_entity.unwrap();

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        let manifest = pipeline.end_tick(1, 0.0, vec![], &world);

        assert_eq!(manifest.entity_spawns.len(), 1);

        let entry = pipeline.entity_index().get(&brick_id).unwrap();
        assert_eq!(entry.tier, "Pooled");
        assert_eq!(entry.entity_type, "destructible");
        assert_eq!(entry.role, "brick");
    }

    // -- 16. Aggregates update after despawn -------------------------------

    #[test]
    fn aggregates_update_after_despawn() {
        let mut world = setup_world();
        let mut pipeline = ManifestPipeline::new();

        // Spawn 2 entities.
        let mut buf = CommandBuffer::new();
        buf.spawn_semantic(
            player_identity(),
            vec![],
            SystemId::PLAYER_SPAWNER,
            CausalReason::GameRule("spawn".to_owned()),
        );
        buf.spawn_semantic(
            enemy_identity(),
            vec![],
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("spawn".to_owned()),
        );
        let applied = buf.apply(&mut world);
        let enemy_id = applied[1].spawned_entity.unwrap();

        pipeline.begin_tick();
        pipeline.process_commands(&applied, 1, &world);
        let m1 = pipeline.end_tick(1, 0.0, vec![], &world);
        assert_eq!(m1.aggregates.total_entity_count, 2);

        // Despawn enemy.
        let mut buf2 = CommandBuffer::new();
        buf2.despawn(
            enemy_id,
            SystemId::WASM_GAMEPLAY,
            CausalReason::GameRule("destroy".to_owned()),
        );
        let applied2 = buf2.apply(&mut world);

        pipeline.begin_tick();
        pipeline.process_commands(&applied2, 2, &world);
        let m2 = pipeline.end_tick(2, 0.0, vec![], &world);
        assert_eq!(m2.aggregates.total_entity_count, 1);
        assert_eq!(
            *m2.aggregates
                .entity_count_by_tier
                .get("Semantic")
                .unwrap_or(&0),
            1
        );
    }
}
