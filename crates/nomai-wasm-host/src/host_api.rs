//! Host API for WASM gameplay modules.
//!
//! Defines the [`HostState`] that lives inside the Wasmtime [`Store`] and the
//! [`register_host_api`] function that registers all host functions under the
//! `"nomai"` WASM import namespace.
//!
//! # Design
//!
//! - **Reads are immediate:** WASM modules can read entity count, tick number,
//!   sim time, and component values from a pre-populated world snapshot.
//! - **Writes are deferred:** Mutations (set_component, spawn, despawn) are
//!   accumulated in a [`CommandBuffer`] and applied after all scripts finish.
//! - **Every mutation carries causality:** All write commands use
//!   [`SystemId::WASM_GAMEPLAY`] and [`CausalReason::GameRule`] with a reason
//!   string provided by the WASM module.
//!
//! # Host Functions (registered under `"nomai"` module)
//!
//! ## Read
//! - `get_entity_count() -> i32`
//! - `sim_time() -> f64`
//! - `tick_number() -> i64`
//! - `get_component(entity_id: i64, name_ptr: i32, name_len: i32) -> i64`
//!
//! ## Write
//! - `set_component(entity_id: i64, name_ptr, name_len, value_ptr, value_len, reason_ptr, reason_len)`
//! - `spawn_semantic(identity_ptr, identity_len, components_ptr, components_len, reason_ptr, reason_len) -> i64`
//! - `spawn_pooled(identity_ptr, identity_len, components_ptr, components_len, reason_ptr, reason_len) -> i64`
//! - `despawn(entity_id: i64, reason_ptr: i32, reason_len: i32)`
//! - `emit_event(event_ptr: i32, event_len: i32)`
//!
//! ## Utility
//! - `log(level: i32, msg_ptr: i32, msg_len: i32)`

use std::collections::HashMap;

use nomai_ecs::command::{CausalReason, CommandBuffer};
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::{EntityIdentity, PoolIdentity, SystemId};
use nomai_manifest::manifest::GameEvent;
use wasmtime::{Caller, Linker};

// ---------------------------------------------------------------------------
// HostState
// ---------------------------------------------------------------------------

/// State held inside the Wasmtime [`Store`] for host function dispatch.
///
/// Contains the command buffer for deferred mutations, a read-only world
/// snapshot for immediate reads, and bookkeeping for tick metadata and
/// performance measurement.
pub struct HostState {
    /// Accumulated commands from WASM write operations. Drained after all
    /// scripts finish via [`drain_commands`](Self::drain_commands).
    pub commands: CommandBuffer,

    /// Accumulated game events from WASM. Drained after all scripts finish
    /// via [`drain_events`](Self::drain_events).
    pub events: Vec<GameEvent>,

    /// Current tick number, set before each tick via [`begin_tick`](Self::begin_tick).
    pub tick: u64,

    /// Current simulation time in seconds, set before each tick.
    pub sim_time: f64,

    /// Read-only world snapshot: maps `entity_id.to_raw()` to a map of
    /// component name -> JSON value. Populated before each tick via
    /// [`snapshot_world`](Self::snapshot_world).
    pub entity_components: HashMap<u64, HashMap<String, serde_json::Value>>,

    /// Number of alive entities in the world snapshot. Used by
    /// `get_entity_count()`.
    pub entity_count: usize,

    /// Number of host function calls made during the current tick.
    /// Reset on [`begin_tick`](Self::begin_tick). Used for performance
    /// benchmarking.
    pub host_call_count: u32,

    /// Deterministic RNG counter. Incremented on each `random_f32()` call.
    /// Can be seeded for reproducible behavior.
    pub rng_counter: u64,
}

impl HostState {
    /// Create a new `HostState` with all fields at their default/empty values.
    pub fn new() -> Self {
        Self {
            commands: CommandBuffer::new(),
            events: Vec::new(),
            tick: 0,
            sim_time: 0.0,
            entity_components: HashMap::new(),
            entity_count: 0,
            host_call_count: 0,
            rng_counter: 0,
        }
    }

    /// Prepare for a new tick. Resets per-tick state (host call counter)
    /// and sets tick metadata.
    ///
    /// Call this before executing the WASM module's `tick()` function each
    /// frame.
    pub fn begin_tick(&mut self, tick: u64, sim_time: f64) {
        self.tick = tick;
        self.sim_time = sim_time;
        self.host_call_count = 0;
        // Commands and events are NOT cleared here -- they accumulate across
        // multiple WASM modules if there are several. Use drain_commands()
        // and drain_events() after all modules have run.
    }

    /// Populate the read-only world snapshot from a flat map of entity
    /// components.
    ///
    /// The `snapshot` parameter maps raw entity IDs (`EntityId::to_raw()`)
    /// to their component maps. This is called by the engine before
    /// executing WASM modules so they can read state without direct ECS
    /// access.
    pub fn snapshot_world(
        &mut self,
        snapshot: HashMap<u64, HashMap<String, serde_json::Value>>,
        entity_count: usize,
    ) {
        self.entity_components = snapshot;
        self.entity_count = entity_count;
    }

    /// Drain all accumulated commands, returning them and leaving the
    /// buffer empty.
    ///
    /// Call this after all WASM modules have run for the tick.
    pub fn drain_commands(&mut self) -> CommandBuffer {
        std::mem::take(&mut self.commands)
    }

    /// Drain all accumulated events, returning them and leaving the
    /// vec empty.
    ///
    /// Call this after all WASM modules have run for the tick.
    pub fn drain_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Default for HostState {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HostState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostState")
            .field("tick", &self.tick)
            .field("sim_time", &self.sim_time)
            .field("entity_count", &self.entity_count)
            .field("host_call_count", &self.host_call_count)
            .field("rng_counter", &self.rng_counter)
            .field("pending_commands", &self.commands.len())
            .field("pending_events", &self.events.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Host function registration
// ---------------------------------------------------------------------------

/// Register all host functions under the `"nomai"` WASM import namespace.
///
/// After calling this, any WASM module that imports functions from `"nomai"`
/// will be able to call into these host functions.
///
/// # Errors
///
/// Returns an error if any function fails to register (should not happen
/// unless there is a Wasmtime API incompatibility).
pub fn register_host_api(linker: &mut Linker<HostState>) -> Result<(), anyhow::Error> {
    // -- READ functions -------------------------------------------------------

    linker.func_wrap("nomai", "get_entity_count", host_get_entity_count)?;
    linker.func_wrap("nomai", "sim_time", host_sim_time)?;
    linker.func_wrap("nomai", "tick_number", host_tick_number)?;
    linker.func_wrap("nomai", "get_component", host_get_component)?;

    // -- WRITE functions ------------------------------------------------------

    linker.func_wrap("nomai", "set_component", host_set_component)?;
    linker.func_wrap("nomai", "spawn_semantic", host_spawn_semantic)?;
    linker.func_wrap("nomai", "spawn_pooled", host_spawn_pooled)?;
    linker.func_wrap("nomai", "despawn", host_despawn)?;
    linker.func_wrap("nomai", "emit_event", host_emit_event)?;

    // -- UTILITY functions ----------------------------------------------------

    linker.func_wrap("nomai", "log", host_log)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helper: read a string from WASM linear memory
// ---------------------------------------------------------------------------

/// Read a UTF-8 string from WASM linear memory at the given (ptr, len).
///
/// # Errors
///
/// Returns an error if:
/// - The WASM module has no exported memory named `"memory"`.
/// - The (ptr, len) range is out of bounds.
/// - The bytes are not valid UTF-8.
fn read_wasm_string(
    caller: &mut Caller<'_, HostState>,
    ptr: i32,
    len: i32,
) -> Result<String, String> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| {
            "WASM module must export 'memory' to use string-based host functions -- \
             add `(memory (export \"memory\") 1)` to your module"
                .to_owned()
        })?;

    let data = memory.data(&caller);
    let start = ptr as usize;
    let end = start + len as usize;

    if end > data.len() {
        return Err(format!(
            "WASM string read out of bounds: ptr={ptr}, len={len}, memory_size={}",
            data.len()
        ));
    }

    String::from_utf8(data[start..end].to_vec())
        .map_err(|e| format!("WASM string at ptr={ptr} len={len} is not valid UTF-8: {e}"))
}

// ---------------------------------------------------------------------------
// READ host functions
// ---------------------------------------------------------------------------

/// `get_entity_count() -> i32`
///
/// Returns the number of alive entities in the world snapshot.
fn host_get_entity_count(mut caller: Caller<'_, HostState>) -> i32 {
    caller.data_mut().host_call_count += 1;
    caller.data().entity_count as i32
}

/// `sim_time() -> f64`
///
/// Returns the current simulation time in seconds.
fn host_sim_time(mut caller: Caller<'_, HostState>) -> f64 {
    caller.data_mut().host_call_count += 1;
    caller.data().sim_time
}

/// `tick_number() -> i64`
///
/// Returns the current tick number.
fn host_tick_number(mut caller: Caller<'_, HostState>) -> i64 {
    caller.data_mut().host_call_count += 1;
    caller.data().tick as i64
}

/// `get_component(entity_id: i64, name_ptr: i32, name_len: i32) -> i64`
///
/// Looks up a component value in the world snapshot. Returns 0 if found,
/// -1 if the entity or component does not exist. The actual component value
/// is written to a result buffer (future enhancement -- for now returns
/// existence check only).
fn host_get_component(
    mut caller: Caller<'_, HostState>,
    entity_id: i64,
    name_ptr: i32,
    name_len: i32,
) -> i64 {
    caller.data_mut().host_call_count += 1;

    let name = match read_wasm_string(&mut caller, name_ptr, name_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "get_component: failed to read component name from WASM memory");
            return -1;
        }
    };

    let raw_id = entity_id as u64;
    let exists = caller
        .data()
        .entity_components
        .get(&raw_id)
        .and_then(|components| components.get(&name))
        .is_some();

    if exists {
        0
    } else {
        -1
    }
}

// ---------------------------------------------------------------------------
// WRITE host functions
// ---------------------------------------------------------------------------

/// `set_component(entity_id: i64, name_ptr, name_len, value_ptr, value_len, reason_ptr, reason_len)`
///
/// Queues a `SetComponent` command with `SystemId::WASM_GAMEPLAY` and
/// `CausalReason::GameRule(reason)`.
#[allow(clippy::too_many_arguments)]
fn host_set_component(
    mut caller: Caller<'_, HostState>,
    entity_id: i64,
    name_ptr: i32,
    name_len: i32,
    value_ptr: i32,
    value_len: i32,
    reason_ptr: i32,
    reason_len: i32,
) {
    caller.data_mut().host_call_count += 1;

    let name = match read_wasm_string(&mut caller, name_ptr, name_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "set_component: failed to read component name");
            return;
        }
    };

    let value_str = match read_wasm_string(&mut caller, value_ptr, value_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "set_component: failed to read value");
            return;
        }
    };

    let reason = match read_wasm_string(&mut caller, reason_ptr, reason_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "set_component: failed to read reason");
            return;
        }
    };

    let value: serde_json::Value = match serde_json::from_str(&value_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                value = %value_str,
                "set_component: value is not valid JSON -- pass a JSON-encoded string"
            );
            return;
        }
    };

    let target = EntityId::from_raw(entity_id as u64);

    caller.data_mut().commands.set_component(
        target,
        &name,
        value,
        SystemId::WASM_GAMEPLAY,
        CausalReason::GameRule(reason),
    );
}

/// `spawn_semantic(identity_ptr, identity_len, components_ptr, components_len, reason_ptr, reason_len) -> i64`
///
/// Queues a `SpawnSemantic` command. Identity and components are passed as
/// JSON strings. Returns a placeholder entity ID (0) -- the real ID is
/// assigned when the command buffer is applied.
fn host_spawn_semantic(
    mut caller: Caller<'_, HostState>,
    identity_ptr: i32,
    identity_len: i32,
    components_ptr: i32,
    components_len: i32,
    reason_ptr: i32,
    reason_len: i32,
) -> i64 {
    caller.data_mut().host_call_count += 1;

    let identity_str = match read_wasm_string(&mut caller, identity_ptr, identity_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "spawn_semantic: failed to read identity");
            return -1;
        }
    };

    let components_str = match read_wasm_string(&mut caller, components_ptr, components_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "spawn_semantic: failed to read components");
            return -1;
        }
    };

    let reason = match read_wasm_string(&mut caller, reason_ptr, reason_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "spawn_semantic: failed to read reason");
            return -1;
        }
    };

    let identity: EntityIdentity = match serde_json::from_str(&identity_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                identity = %identity_str,
                "spawn_semantic: identity is not valid JSON EntityIdentity"
            );
            return -1;
        }
    };

    let components: Vec<(String, serde_json::Value)> = match serde_json::from_str(&components_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                components = %components_str,
                "spawn_semantic: components is not valid JSON array of (name, value) pairs"
            );
            return -1;
        }
    };

    caller.data_mut().commands.spawn_semantic(
        identity,
        components,
        SystemId::WASM_GAMEPLAY,
        CausalReason::GameRule(reason),
    );

    0 // placeholder -- real ID assigned on apply
}

/// `spawn_pooled(identity_ptr, identity_len, components_ptr, components_len, reason_ptr, reason_len) -> i64`
///
/// Queues a `SpawnPooled` command. Identity and components are passed as
/// JSON strings. Returns a placeholder entity ID (0) -- the real ID is
/// assigned when the command buffer is applied.
fn host_spawn_pooled(
    mut caller: Caller<'_, HostState>,
    identity_ptr: i32,
    identity_len: i32,
    components_ptr: i32,
    components_len: i32,
    reason_ptr: i32,
    reason_len: i32,
) -> i64 {
    caller.data_mut().host_call_count += 1;

    let identity_str = match read_wasm_string(&mut caller, identity_ptr, identity_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "spawn_pooled: failed to read identity");
            return -1;
        }
    };

    let components_str = match read_wasm_string(&mut caller, components_ptr, components_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "spawn_pooled: failed to read components");
            return -1;
        }
    };

    let reason = match read_wasm_string(&mut caller, reason_ptr, reason_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "spawn_pooled: failed to read reason");
            return -1;
        }
    };

    let identity: PoolIdentity = match serde_json::from_str(&identity_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                identity = %identity_str,
                "spawn_pooled: identity is not valid JSON PoolIdentity"
            );
            return -1;
        }
    };

    let components: Vec<(String, serde_json::Value)> = match serde_json::from_str(&components_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                components = %components_str,
                "spawn_pooled: components is not valid JSON array of (name, value) pairs"
            );
            return -1;
        }
    };

    caller.data_mut().commands.spawn_pooled(
        identity,
        components,
        SystemId::WASM_GAMEPLAY,
        CausalReason::GameRule(reason),
    );

    0 // placeholder -- real ID assigned on apply
}

/// `despawn(entity_id: i64, reason_ptr: i32, reason_len: i32)`
///
/// Queues a `Despawn` command with `SystemId::WASM_GAMEPLAY` and
/// `CausalReason::GameRule(reason)`.
fn host_despawn(
    mut caller: Caller<'_, HostState>,
    entity_id: i64,
    reason_ptr: i32,
    reason_len: i32,
) {
    caller.data_mut().host_call_count += 1;

    let reason = match read_wasm_string(&mut caller, reason_ptr, reason_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "despawn: failed to read reason");
            return;
        }
    };

    let target = EntityId::from_raw(entity_id as u64);

    caller.data_mut().commands.despawn(
        target,
        SystemId::WASM_GAMEPLAY,
        CausalReason::GameRule(reason),
    );
}

/// `emit_event(event_ptr: i32, event_len: i32)`
///
/// Emits a game event. The event is passed as a JSON string that must
/// deserialize to a [`GameEvent`].
fn host_emit_event(mut caller: Caller<'_, HostState>, event_ptr: i32, event_len: i32) {
    caller.data_mut().host_call_count += 1;

    let event_str = match read_wasm_string(&mut caller, event_ptr, event_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "emit_event: failed to read event data");
            return;
        }
    };

    let event: GameEvent = match serde_json::from_str(&event_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                event = %event_str,
                "emit_event: event is not valid JSON GameEvent"
            );
            return;
        }
    };

    caller.data_mut().events.push(event);
}

// ---------------------------------------------------------------------------
// UTILITY host functions
// ---------------------------------------------------------------------------

/// `log(level: i32, msg_ptr: i32, msg_len: i32)`
///
/// Log a message from WASM. Level mapping:
/// - 0 = trace
/// - 1 = debug
/// - 2 = info
/// - 3 = warn
/// - 4 = error
fn host_log(mut caller: Caller<'_, HostState>, level: i32, msg_ptr: i32, msg_len: i32) {
    caller.data_mut().host_call_count += 1;

    let msg = match read_wasm_string(&mut caller, msg_ptr, msg_len) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "log: failed to read message from WASM memory");
            return;
        }
    };

    match level {
        0 => tracing::trace!(source = "wasm", "{msg}"),
        1 => tracing::debug!(source = "wasm", "{msg}"),
        2 => tracing::info!(source = "wasm", "{msg}"),
        3 => tracing::warn!(source = "wasm", "{msg}"),
        4 => tracing::error!(source = "wasm", "{msg}"),
        _ => tracing::info!(source = "wasm", level = level, "{msg}"),
    }
}
