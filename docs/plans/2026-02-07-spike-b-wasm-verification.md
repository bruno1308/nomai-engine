# Spike B: WASM Gameplay + Verification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Prove that a WASM gameplay module can emit causally-tagged commands, and a Python verification engine can detect behavioral correctness from the manifest alone.

**Architecture:** Two parallel tracks converge at integration. Track A builds the Rust WASM sandbox (wasmtime host, gameplay API, hot-swap, AS pipeline) in `crates/nomai-wasm-host/`. Track B builds the Python verification engine (intent specs, verification loop) in `python/nomai-sdk/`. Both consume the existing `nomai-ecs` and `nomai-manifest` crates from Spike A. Integration test (B8) and benchmark (B9) validate the combined pipeline.

**Tech Stack:** Rust 1.83.0, wasmtime 27.0.0 (fuel metering), AssemblyScript 0.28.2, Python 3.12+, pytest, serde_json, criterion

---

## Dependency Graph

```
B1 (Wasmtime Integration)         B6 (Intent Spec Python)
  │                                 │
  ├── B2 (Gameplay Host API)        └── B7 (Verification Engine)
  │     │                                  │
  │     ├── B3 (Hot-Swap)                  │
  │     ├── B5 (Causality WASM)            │
  │     │                                  │
  │     └───────────┐    ┌─────────────────┘
  │                 │    │
  └── B4 (AS)      B8 (Integration Test)
        │
        └── B9 (WASM Overhead Benchmark)

Parallel Tracks:
  Track A (Rust):   B1 → B2 → B3, B5 (parallel after B2)
  Track B (Python): B6 → B7
  Track C (AS):     B4 (after B1)
  Track D:          B8 (after B5 + B7), B9 (after B2 + B4)
  Gate:             After B8 + B9
```

## Agents

| Task | Agent | Domain |
|------|-------|--------|
| B1, B2, B3, B5 | `wasm-sandbox` | `crates/nomai-wasm-host/` |
| B4 | `wasm-sandbox` | `gameplay/` |
| B6, B7 | `python-verification` | `python/nomai-sdk/` |
| B8 | `wasm-sandbox` + `python-verification` (orchestrator coordinates) | `tests/` |
| B9 | `spike-validator` | `benchmarks/` |

---

## Task B1: Wasmtime Integration

**GitHub Issue:** #10

**Files:**
- Create: `crates/nomai-wasm-host/Cargo.toml`
- Create: `crates/nomai-wasm-host/src/lib.rs`
- Create: `crates/nomai-wasm-host/src/module.rs`
- Create: `crates/nomai-wasm-host/src/error.rs`
- Modify: `Cargo.toml` (workspace root — add member + wasmtime dep)
- Modify: `justfile` (no changes needed, `--workspace` already covers new crate)
- Create: `crates/nomai-wasm-host/tests/fixtures/noop.wat` (test fixture)
- Create: `crates/nomai-wasm-host/tests/fixtures/counter.wat` (test fixture)

### Step 1: Add crate to workspace

Modify `Cargo.toml` (workspace root):
```toml
[workspace]
resolver = "2"
members = [
    "crates/nomai-ecs",
    "crates/nomai-manifest",
    "crates/nomai-engine",
    "crates/nomai-wasm-host",
]

[workspace.dependencies]
# ... existing deps ...
wasmtime = "27.0.0"
```

Create `crates/nomai-wasm-host/Cargo.toml`:
```toml
[package]
name = "nomai-wasm-host"
version = "0.1.0"
edition.workspace = true

[dependencies]
nomai-ecs = { path = "../nomai-ecs" }
nomai-manifest = { path = "../nomai-manifest" }
wasmtime = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
anyhow = { workspace = true }

[dev-dependencies]
tracing-subscriber = { workspace = true }
```

### Step 2: Create error types

Create `crates/nomai-wasm-host/src/error.rs`:
```rust
//! Error types for the WASM host.

use thiserror::Error;

/// Errors from the WASM sandbox.
#[derive(Debug, Error)]
pub enum WasmError {
    /// Failed to load or compile a WASM module.
    #[error("failed to compile WASM module: {0}")]
    CompileError(String),

    /// The module is missing a required export.
    #[error("module missing required export: {0}")]
    MissingExport(String),

    /// WASM execution ran out of fuel (instruction budget exceeded).
    #[error("WASM execution ran out of fuel (budget: {budget})")]
    OutOfFuel { budget: u64 },

    /// WASM execution trapped.
    #[error("WASM execution trapped: {0}")]
    Trap(String),

    /// WASM memory limit exceeded.
    #[error("WASM memory limit exceeded (limit: {limit_bytes} bytes)")]
    MemoryLimitExceeded { limit_bytes: usize },

    /// Generic wasmtime error.
    #[error("wasmtime error: {0}")]
    Runtime(#[from] anyhow::Error),
}
```

### Step 3: Create WasmModule

Create `crates/nomai-wasm-host/src/module.rs`:
```rust
//! WASM module loading, instantiation, and execution.
//!
//! The [`WasmModule`] struct wraps a wasmtime instance with:
//! - Fuel-based metering (configurable instruction budget per tick)
//! - Memory limit (16MB default)
//! - No WASI / no filesystem / no network / no wall-clock access
//! - Exported `tick()` function call

use tracing::info;
use wasmtime::*;

use crate::error::WasmError;

/// Configuration for a WASM sandbox.
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Maximum fuel (instructions) per tick. 0 = unlimited (not recommended).
    pub fuel_per_tick: u64,
    /// Maximum memory in bytes. Default: 16MB.
    pub memory_limit_bytes: usize,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            fuel_per_tick: 1_000_000, // 1M instructions
            memory_limit_bytes: 16 * 1024 * 1024, // 16MB
        }
    }
}

/// A loaded and instantiated WASM module, ready to execute `tick()`.
pub struct WasmModule {
    store: Store<()>,
    instance: Instance,
    tick_fn: TypedFunc<(), ()>,
    config: WasmConfig,
}

impl WasmModule {
    /// Load a WASM module from bytes (binary .wasm or text .wat).
    pub fn from_bytes(wasm_bytes: &[u8], config: WasmConfig) -> Result<Self, WasmError> {
        let mut engine_config = wasmtime::Config::new();
        engine_config.consume_fuel(true);

        let engine = Engine::new(&engine_config)
            .map_err(|e| WasmError::CompileError(e.to_string()))?;

        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| WasmError::CompileError(e.to_string()))?;

        let mut store = Store::new(&engine, ());

        // Set fuel budget
        store.set_fuel(config.fuel_per_tick)
            .map_err(|e| WasmError::Runtime(e.into()))?;

        // Instantiate with an empty linker (no WASI, no imports for now)
        let linker = Linker::new(&engine);
        let instance = linker.instantiate(&mut store, &module)
            .map_err(|e| WasmError::Runtime(e.into()))?;

        // Find the tick export
        let tick_fn = instance
            .get_typed_func::<(), ()>(&mut store, "tick")
            .map_err(|_| WasmError::MissingExport("tick".to_string()))?;

        info!(
            fuel = config.fuel_per_tick,
            memory_limit = config.memory_limit_bytes,
            "WASM module loaded"
        );

        Ok(Self {
            store,
            instance,
            tick_fn,
            config,
        })
    }

    /// Execute the module's `tick()` function.
    ///
    /// Returns the fuel consumed during execution.
    /// Resets fuel to full budget before each call.
    pub fn call_tick(&mut self) -> Result<u64, WasmError> {
        // Reset fuel budget for this tick
        let fuel_before = self.config.fuel_per_tick;
        self.store.set_fuel(fuel_before)
            .map_err(|e| WasmError::Runtime(e.into()))?;

        // Call tick
        match self.tick_fn.call(&mut self.store, ()) {
            Ok(()) => {}
            Err(trap) => {
                // Check if it's a fuel exhaustion
                if trap.to_string().contains("fuel") {
                    return Err(WasmError::OutOfFuel {
                        budget: fuel_before,
                    });
                }
                return Err(WasmError::Trap(trap.to_string()));
            }
        }

        // Calculate fuel consumed
        let fuel_remaining = self.store.get_fuel()
            .map_err(|e| WasmError::Runtime(e.into()))?;
        let fuel_consumed = fuel_before - fuel_remaining;

        Ok(fuel_consumed)
    }

    /// Get the current fuel remaining.
    pub fn fuel_remaining(&self) -> Result<u64, WasmError> {
        self.store.get_fuel().map_err(|e| WasmError::Runtime(e.into()))
    }

    /// Get the configuration.
    pub fn config(&self) -> &WasmConfig {
        &self.config
    }

    /// Get access to the wasmtime instance (for host API extension in B2).
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    /// Get mutable access to the store (for host API extension in B2).
    pub fn store_mut(&mut self) -> &mut Store<()> {
        &mut self.store
    }
}
```

### Step 4: Create lib.rs

Create `crates/nomai-wasm-host/src/lib.rs`:
```rust
//! WASM gameplay sandbox for the Nomai Engine.
//!
//! This crate provides the Wasmtime-based sandbox that loads and executes
//! AI-generated gameplay modules (typically compiled from AssemblyScript).
//! WASM modules interact with the engine through a restricted host API
//! that routes all mutations through the ECS command buffer with causality.
//!
//! # Security
//!
//! - No WASI: no filesystem, no network, no wall-clock access
//! - Fuel metering: configurable instruction budget per tick
//! - Memory limit: 16MB default per module
//! - All state lives in the ECS, not in WASM memory

#![deny(unsafe_code)]

pub mod error;
pub mod module;

pub use error::WasmError;
pub use module::{WasmConfig, WasmModule};
```

### Step 5: Create WAT test fixtures

Create `crates/nomai-wasm-host/tests/fixtures/noop.wat`:
```wat
(module
  (func (export "tick"))
)
```

Create `crates/nomai-wasm-host/tests/fixtures/counter.wat`:
```wat
(module
  (global $count (mut i32) (i32.const 0))
  (func (export "tick")
    global.get $count
    i32.const 1
    i32.add
    global.set $count
  )
  (func (export "get_count") (result i32)
    global.get $count
  )
)
```

Create `crates/nomai-wasm-host/tests/fixtures/fuel_hog.wat`:
```wat
(module
  ;; Loops forever until fuel runs out
  (func (export "tick")
    (local $i i32)
    (loop $loop
      local.get $i
      i32.const 1
      i32.add
      local.set $i
      br $loop
    )
  )
)
```

### Step 6: Write tests

Tests go in `crates/nomai-wasm-host/src/module.rs` (inline `#[cfg(test)]` module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_path(name: &str) -> String {
        format!(
            "{}/../tests/fixtures/{}",
            env!("CARGO_MANIFEST_DIR"),
            name
        )
    }

    fn load_fixture(name: &str) -> Vec<u8> {
        std::fs::read(fixture_path(name))
            .unwrap_or_else(|e| panic!("failed to read fixture {name}: {e}"))
    }

    #[test]
    fn load_noop_module() {
        let wasm = load_fixture("noop.wat");
        let module = WasmModule::from_bytes(&wasm, WasmConfig::default());
        assert!(module.is_ok(), "should load noop module");
    }

    #[test]
    fn call_noop_tick() {
        let wasm = load_fixture("noop.wat");
        let mut module = WasmModule::from_bytes(&wasm, WasmConfig::default()).unwrap();
        let fuel = module.call_tick().unwrap();
        assert!(fuel > 0, "even noop should consume some fuel");
    }

    #[test]
    fn call_counter_tick() {
        let wasm = load_fixture("counter.wat");
        let mut module = WasmModule::from_bytes(&wasm, WasmConfig::default()).unwrap();

        // Call tick multiple times
        for _ in 0..5 {
            module.call_tick().unwrap();
        }

        // Verify counter incremented via exported get_count
        let get_count = module.instance()
            .get_typed_func::<(), i32>(module.store_mut(), "get_count")
            .unwrap();
        let count = get_count.call(module.store_mut(), ()).unwrap();
        assert_eq!(count, 5, "counter should be 5 after 5 ticks");
    }

    #[test]
    fn fuel_exhaustion_traps() {
        let wasm = load_fixture("fuel_hog.wat");
        let config = WasmConfig {
            fuel_per_tick: 10_000, // Small budget
            ..Default::default()
        };
        let mut module = WasmModule::from_bytes(&wasm, config).unwrap();

        let result = module.call_tick();
        assert!(result.is_err(), "fuel hog should run out of fuel");
        match result.unwrap_err() {
            WasmError::OutOfFuel { budget } => {
                assert_eq!(budget, 10_000);
            }
            other => panic!("expected OutOfFuel, got: {other}"),
        }
    }

    #[test]
    fn fuel_resets_between_ticks() {
        let wasm = load_fixture("noop.wat");
        let config = WasmConfig {
            fuel_per_tick: 100_000,
            ..Default::default()
        };
        let mut module = WasmModule::from_bytes(&wasm, config).unwrap();

        let fuel1 = module.call_tick().unwrap();
        let fuel2 = module.call_tick().unwrap();

        // Both should consume the same amount (deterministic)
        assert_eq!(fuel1, fuel2, "fuel consumed should be identical between ticks");
    }

    #[test]
    fn missing_tick_export_errors() {
        // A module with no tick export
        let wat = b"(module (func (export \"not_tick\")))";
        let result = WasmModule::from_bytes(wat, WasmConfig::default());
        assert!(result.is_err());
        match result.unwrap_err() {
            WasmError::MissingExport(name) => assert_eq!(name, "tick"),
            other => panic!("expected MissingExport, got: {other}"),
        }
    }

    #[test]
    fn no_wasi_no_imports() {
        // A module that tries to import WASI should fail
        let wat = br#"(module
            (import "wasi_snapshot_preview1" "fd_write"
                (func (param i32 i32 i32 i32) (result i32)))
            (func (export "tick"))
        )"#;
        let result = WasmModule::from_bytes(wat, WasmConfig::default());
        assert!(result.is_err(), "WASI imports should not be satisfied");
    }
}
```

### Step 7: Build and verify

```bash
just build     # Should compile including new crate
just test      # Should pass all existing + new tests
just clippy    # Zero warnings
```

### Step 8: Commit

```bash
git add crates/nomai-wasm-host/ Cargo.toml Cargo.lock
git commit -m "feat(wasm): B1 wasmtime integration with fuel metering and sandbox"
```

**Acceptance:** WASM module loads, executes with fuel metering, traps on budget exceeded, no WASI access.

---

## Task B2: Gameplay Host API

**GitHub Issue:** #11

**Depends on:** B1

**Files:**
- Create: `crates/nomai-wasm-host/src/host_api.rs`
- Modify: `crates/nomai-wasm-host/src/lib.rs` (add module export)
- Modify: `crates/nomai-wasm-host/src/module.rs` (refactor to accept linker with host fns)
- Create: `crates/nomai-wasm-host/tests/fixtures/host_api_test.wat` (test fixture)

### Design

The host API uses a shared `HostState` struct stored in wasmtime's `Store<HostState>`. WASM calls host functions → they read/write `HostState` → `HostState` holds a reference to the `World` (read-only) and a `CommandBuffer` (write). After `tick()`, the orchestrator drains commands from the `HostState`.

**Host Functions (WASM → Rust):**

Read functions (query ECS state):
- `host_get_component(entity_id_raw: u64, name_ptr: i32, name_len: i32) -> i32` — returns JSON ptr in WASM linear memory
- `host_query_entities_by_type(type_ptr: i32, type_len: i32) -> i32` — returns entity ID array ptr
- `host_get_entity_count() -> i32`
- `host_sim_time() -> f64`
- `host_tick_number() -> u64`

Write functions (emit commands):
- `host_spawn_semantic(identity_ptr: i32, identity_len: i32, components_ptr: i32, components_len: i32, reason_ptr: i32, reason_len: i32)`
- `host_spawn_pooled(identity_ptr: i32, identity_len: i32, components_ptr: i32, components_len: i32, reason_ptr: i32, reason_len: i32)`
- `host_set_component(entity_id_raw: u64, name_ptr: i32, name_len: i32, value_ptr: i32, value_len: i32, reason_ptr: i32, reason_len: i32)`
- `host_despawn(entity_id_raw: u64, reason_ptr: i32, reason_len: i32)`
- `host_emit_event(event_json_ptr: i32, event_json_len: i32)`

Utility:
- `host_log(level: i32, msg_ptr: i32, msg_len: i32)`
- `host_random_f32() -> f32`

**Key design decisions:**
- All string/JSON data passes through WASM linear memory via ptr+len pairs
- The WASM module must allocate memory for outgoing data and expose an `alloc(size: i32) -> i32` export
- All write functions route through `CommandBuffer` with `SystemId::WASM_GAMEPLAY`
- Reason strings from WASM → `CausalReason::GameRule(reason)`

### Step 1: Create HostState and host function implementations

Create `crates/nomai-wasm-host/src/host_api.rs`:
```rust
//! Host API functions exposed to WASM gameplay modules.
//!
//! All state mutations flow through the ECS command buffer with
//! `SystemId::WASM_GAMEPLAY` and `CausalReason::GameRule(reason)`.
//! Read functions access a shared world reference.
//!
//! String data passes through WASM linear memory via (ptr, len) pairs.
//! The WASM module must export an `alloc(size: i32) -> i32` function
//! for the host to write return data.

use nomai_ecs::command::{CausalReason, CommandBuffer};
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::{EntityIdentity, PoolIdentity, SystemId};
use nomai_ecs::world::World;
use nomai_manifest::manifest::GameEvent;
use serde_json;
use tracing::{debug, warn};
use wasmtime::*;

/// Shared state accessible by host functions during WASM execution.
///
/// The orchestrator sets up `world` (immutable ref) and `commands`
/// before calling `tick()`, then drains `commands` and `events` afterward.
pub struct HostState {
    /// Commands accumulated during this tick's WASM execution.
    pub commands: CommandBuffer,
    /// Events accumulated during this tick's WASM execution.
    pub events: Vec<GameEvent>,
    /// Current tick number (set by orchestrator before tick).
    pub tick: u64,
    /// Current simulation time (set by orchestrator before tick).
    pub sim_time: f64,
    /// Serialized world snapshot for read queries.
    /// Maps entity_id_raw -> component_name -> serde_json::Value.
    /// Pre-populated by orchestrator before tick.
    pub entity_components: std::collections::HashMap<u64, std::collections::HashMap<String, serde_json::Value>>,
    /// Entity count.
    pub entity_count: usize,
    /// Host call counter for benchmarking.
    pub host_call_count: u32,
    /// Deterministic RNG seed (incremented per call).
    pub rng_counter: u64,
}

impl HostState {
    /// Create a new empty host state.
    pub fn new() -> Self {
        Self {
            commands: CommandBuffer::new(),
            events: Vec::new(),
            tick: 0,
            sim_time: 0.0,
            entity_components: std::collections::HashMap::new(),
            entity_count: 0,
            host_call_count: 0,
            rng_counter: 0,
        }
    }

    /// Reset for a new tick. Clears commands and events but keeps world data.
    pub fn begin_tick(&mut self, tick: u64, sim_time: f64) {
        self.commands.clear();
        self.events.clear();
        self.tick = tick;
        self.sim_time = sim_time;
        self.host_call_count = 0;
    }

    /// Populate the read-only world snapshot from the ECS world.
    pub fn snapshot_world(&mut self, world: &World) {
        self.entity_components.clear();
        self.entity_count = world.entity_count();
        // World snapshot population will be implemented when query API is ready.
        // For now, entity_components is populated manually by the orchestrator
        // using world.get_component_json() or similar.
    }

    /// Drain all accumulated commands.
    pub fn drain_commands(&mut self) -> CommandBuffer {
        std::mem::replace(&mut self.commands, CommandBuffer::new())
    }

    /// Drain all accumulated events.
    pub fn drain_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }
}

impl Default for HostState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helper: read string from WASM memory
// ---------------------------------------------------------------------------

fn read_wasm_string(caller: &Caller<'_, HostState>, ptr: i32, len: i32) -> Result<String> {
    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| anyhow::anyhow!("WASM module has no 'memory' export"))?;

    let data = memory.data(caller);
    let start = ptr as usize;
    let end = start + len as usize;

    if end > data.len() {
        anyhow::bail!("WASM memory read out of bounds: {start}..{end} (memory size: {})", data.len());
    }

    let bytes = &data[start..end];
    String::from_utf8(bytes.to_vec())
        .map_err(|e| anyhow::anyhow!("invalid UTF-8 from WASM: {e}"))
}

// ---------------------------------------------------------------------------
// Helper: write bytes to WASM memory via alloc export
// ---------------------------------------------------------------------------

fn write_to_wasm(caller: &mut Caller<'_, HostState>, data: &[u8]) -> Result<(i32, i32)> {
    let alloc_fn = caller
        .get_export("alloc")
        .and_then(|e| e.into_func())
        .ok_or_else(|| anyhow::anyhow!("WASM module has no 'alloc' export"))?;

    let alloc = alloc_fn.typed::<i32, i32>(caller.as_context())?;
    let len = data.len() as i32;
    let ptr = alloc.call(caller.as_context_mut(), len)?;

    let memory = caller
        .get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or_else(|| anyhow::anyhow!("WASM module has no 'memory' export"))?;

    let mem_data = memory.data_mut(caller);
    let start = ptr as usize;
    let end = start + len as usize;
    if end > mem_data.len() {
        anyhow::bail!("WASM alloc returned out-of-bounds pointer");
    }
    mem_data[start..end].copy_from_slice(data);

    Ok((ptr, len))
}

// ---------------------------------------------------------------------------
// Register all host functions on a Linker
// ---------------------------------------------------------------------------

/// Register all gameplay host functions on the given linker.
///
/// The linker is used when instantiating WASM modules. All functions
/// are registered under the `"nomai"` module namespace.
pub fn register_host_api(linker: &mut Linker<HostState>) -> Result<()> {
    // --- Read functions ---

    linker.func_wrap("nomai", "get_entity_count", |caller: Caller<'_, HostState>| -> i32 {
        caller.data().entity_count as i32
    })?;

    linker.func_wrap("nomai", "sim_time", |caller: Caller<'_, HostState>| -> f64 {
        caller.data().sim_time
    })?;

    linker.func_wrap("nomai", "tick_number", |caller: Caller<'_, HostState>| -> i64 {
        caller.data().tick as i64
    })?;

    linker.func_wrap("nomai", "get_component",
        |mut caller: Caller<'_, HostState>, entity_id_raw: i64, name_ptr: i32, name_len: i32| -> i64 {
            caller.data_mut().host_call_count += 1;

            let name = match read_wasm_string(&caller, name_ptr, name_len) {
                Ok(n) => n,
                Err(e) => {
                    warn!("get_component: failed to read component name: {e}");
                    return 0; // null pointer = not found
                }
            };

            let value = caller.data()
                .entity_components
                .get(&(entity_id_raw as u64))
                .and_then(|components| components.get(&name))
                .and_then(|v| serde_json::to_string(v).ok());

            match value {
                Some(json_str) => {
                    let bytes = json_str.as_bytes();
                    match write_to_wasm(&mut caller, bytes) {
                        Ok((ptr, len)) => {
                            // Pack ptr and len into a single i64
                            ((ptr as i64) << 32) | (len as i64 & 0xFFFFFFFF)
                        }
                        Err(e) => {
                            warn!("get_component: failed to write to WASM: {e}");
                            0
                        }
                    }
                }
                None => 0, // not found
            }
        }
    )?;

    // --- Write functions ---

    linker.func_wrap("nomai", "set_component",
        |mut caller: Caller<'_, HostState>,
         entity_id_raw: i64,
         name_ptr: i32, name_len: i32,
         value_ptr: i32, value_len: i32,
         reason_ptr: i32, reason_len: i32| {

            caller.data_mut().host_call_count += 1;

            let name = match read_wasm_string(&caller, name_ptr, name_len) {
                Ok(n) => n,
                Err(e) => { warn!("set_component: bad name: {e}"); return; }
            };
            let value_str = match read_wasm_string(&caller, value_ptr, value_len) {
                Ok(v) => v,
                Err(e) => { warn!("set_component: bad value: {e}"); return; }
            };
            let reason = match read_wasm_string(&caller, reason_ptr, reason_len) {
                Ok(r) => r,
                Err(e) => { warn!("set_component: bad reason: {e}"); return; }
            };

            let value: serde_json::Value = match serde_json::from_str(&value_str) {
                Ok(v) => v,
                Err(e) => { warn!("set_component: invalid JSON value: {e}"); return; }
            };

            let entity_id = EntityId::from_raw(entity_id_raw as u64);
            debug!(entity = ?entity_id, component = %name, reason = %reason, "WASM set_component");

            caller.data_mut().commands.set_component(
                entity_id,
                &name,
                value,
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule(reason),
            );
        }
    )?;

    linker.func_wrap("nomai", "spawn_semantic",
        |mut caller: Caller<'_, HostState>,
         identity_ptr: i32, identity_len: i32,
         components_ptr: i32, components_len: i32,
         reason_ptr: i32, reason_len: i32| {

            caller.data_mut().host_call_count += 1;

            let identity_str = match read_wasm_string(&caller, identity_ptr, identity_len) {
                Ok(s) => s,
                Err(e) => { warn!("spawn_semantic: bad identity: {e}"); return; }
            };
            let components_str = match read_wasm_string(&caller, components_ptr, components_len) {
                Ok(s) => s,
                Err(e) => { warn!("spawn_semantic: bad components: {e}"); return; }
            };
            let reason = match read_wasm_string(&caller, reason_ptr, reason_len) {
                Ok(r) => r,
                Err(e) => { warn!("spawn_semantic: bad reason: {e}"); return; }
            };

            let identity: EntityIdentity = match serde_json::from_str(&identity_str) {
                Ok(id) => id,
                Err(e) => { warn!("spawn_semantic: invalid identity JSON: {e}"); return; }
            };
            let components: Vec<(String, serde_json::Value)> = match serde_json::from_str(&components_str) {
                Ok(c) => c,
                Err(e) => { warn!("spawn_semantic: invalid components JSON: {e}"); return; }
            };

            debug!(role = %identity.role, reason = %reason, "WASM spawn_semantic");

            caller.data_mut().commands.spawn_semantic(
                identity,
                components,
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule(reason),
            );
        }
    )?;

    linker.func_wrap("nomai", "spawn_pooled",
        |mut caller: Caller<'_, HostState>,
         identity_ptr: i32, identity_len: i32,
         components_ptr: i32, components_len: i32,
         reason_ptr: i32, reason_len: i32| {

            caller.data_mut().host_call_count += 1;

            let identity_str = match read_wasm_string(&caller, identity_ptr, identity_len) {
                Ok(s) => s,
                Err(e) => { warn!("spawn_pooled: bad identity: {e}"); return; }
            };
            let components_str = match read_wasm_string(&caller, components_ptr, components_len) {
                Ok(s) => s,
                Err(e) => { warn!("spawn_pooled: bad components: {e}"); return; }
            };
            let reason = match read_wasm_string(&caller, reason_ptr, reason_len) {
                Ok(r) => r,
                Err(e) => { warn!("spawn_pooled: bad reason: {e}"); return; }
            };

            let identity: PoolIdentity = match serde_json::from_str(&identity_str) {
                Ok(id) => id,
                Err(e) => { warn!("spawn_pooled: invalid identity JSON: {e}"); return; }
            };
            let components: Vec<(String, serde_json::Value)> = match serde_json::from_str(&components_str) {
                Ok(c) => c,
                Err(e) => { warn!("spawn_pooled: invalid components JSON: {e}"); return; }
            };

            debug!(pool_type = %identity.pool_type, reason = %reason, "WASM spawn_pooled");

            caller.data_mut().commands.spawn_pooled(
                identity,
                components,
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule(reason),
            );
        }
    )?;

    linker.func_wrap("nomai", "despawn",
        |mut caller: Caller<'_, HostState>,
         entity_id_raw: i64,
         reason_ptr: i32, reason_len: i32| {

            caller.data_mut().host_call_count += 1;

            let reason = match read_wasm_string(&caller, reason_ptr, reason_len) {
                Ok(r) => r,
                Err(e) => { warn!("despawn: bad reason: {e}"); return; }
            };

            let entity_id = EntityId::from_raw(entity_id_raw as u64);
            debug!(entity = ?entity_id, reason = %reason, "WASM despawn");

            caller.data_mut().commands.despawn(
                entity_id,
                SystemId::WASM_GAMEPLAY,
                CausalReason::GameRule(reason),
            );
        }
    )?;

    linker.func_wrap("nomai", "emit_event",
        |mut caller: Caller<'_, HostState>,
         event_ptr: i32, event_len: i32| {

            caller.data_mut().host_call_count += 1;

            let event_str = match read_wasm_string(&caller, event_ptr, event_len) {
                Ok(s) => s,
                Err(e) => { warn!("emit_event: bad event JSON: {e}"); return; }
            };

            let event: GameEvent = match serde_json::from_str(&event_str) {
                Ok(e) => e,
                Err(e) => { warn!("emit_event: invalid event JSON: {e}"); return; }
            };

            debug!(event_type = %event.event_type, "WASM emit_event");
            caller.data_mut().events.push(event);
        }
    )?;

    // --- Utility functions ---

    linker.func_wrap("nomai", "log",
        |caller: Caller<'_, HostState>, level: i32, msg_ptr: i32, msg_len: i32| {
            let msg = match read_wasm_string(&caller, msg_ptr, msg_len) {
                Ok(m) => m,
                Err(_) => return,
            };
            match level {
                0 => tracing::debug!(source = "wasm", "{msg}"),
                1 => tracing::info!(source = "wasm", "{msg}"),
                2 => tracing::warn!(source = "wasm", "{msg}"),
                _ => tracing::error!(source = "wasm", "{msg}"),
            }
        }
    )?;

    Ok(())
}
```

### Step 2: Refactor WasmModule to use HostState store

Modify `crates/nomai-wasm-host/src/module.rs` to use `Store<HostState>` instead of `Store<()>`:

The `WasmModule::from_bytes` constructor now:
1. Creates `Store<HostState>` with `HostState::new()`
2. Creates a `Linker<HostState>` and calls `register_host_api(&mut linker)`
3. Instantiates the module with the linker

Key method additions:
```rust
/// Access the host state (to set up world snapshot, read commands).
pub fn host_state(&self) -> &HostState { self.store.data() }
pub fn host_state_mut(&mut self) -> &mut HostState { self.store.data_mut() }
```

### Step 3: Write WAT fixture for host API testing

Create `crates/nomai-wasm-host/tests/fixtures/host_api_test.wat`:
```wat
(module
  ;; Import host functions
  (import "nomai" "get_entity_count" (func $get_entity_count (result i32)))
  (import "nomai" "sim_time" (func $sim_time (result f64)))
  (import "nomai" "tick_number" (func $tick_number (result i64)))
  (import "nomai" "set_component"
    (func $set_component (param i64 i32 i32 i32 i32 i32 i32)))
  (import "nomai" "log" (func $log (param i32 i32 i32)))

  ;; Memory export (required by host for string passing)
  (memory (export "memory") 1)

  ;; String data stored in memory
  ;; Offset 0: component name "position"
  (data (i32.const 0) "position")
  ;; Offset 8: JSON value '{"x":1.0,"y":2.0}'
  (data (i32.const 8) "{\"x\":1.0,\"y\":2.0}")
  ;; Offset 26: reason "wasm_test_move"
  (data (i32.const 26) "wasm_test_move")
  ;; Offset 40: log message "tick called"
  (data (i32.const 40) "tick called")

  ;; Simple alloc: just returns the given offset (for testing only)
  (global $alloc_ptr (mut i32) (i32.const 1024))
  (func (export "alloc") (param $size i32) (result i32)
    (local $ptr i32)
    global.get $alloc_ptr
    local.set $ptr
    global.get $alloc_ptr
    local.get $size
    i32.add
    global.set $alloc_ptr
    local.get $ptr
  )

  ;; Exported results for test verification
  (global $last_entity_count (mut i32) (i32.const -1))
  (global $last_tick (mut i64) (i64.const -1))

  (func (export "tick")
    ;; Read entity count
    call $get_entity_count
    global.set $last_entity_count

    ;; Read tick number
    call $tick_number
    global.set $last_tick

    ;; Log
    i32.const 1       ;; info level
    i32.const 40      ;; msg ptr
    i32.const 11      ;; msg len "tick called"
    call $log

    ;; Emit a set_component command
    ;; entity_id_raw = 0 (first entity)
    i64.const 0
    ;; name_ptr=0, name_len=8 ("position")
    i32.const 0
    i32.const 8
    ;; value_ptr=8, value_len=18 (JSON)
    i32.const 8
    i32.const 18
    ;; reason_ptr=26, reason_len=14 ("wasm_test_move")
    i32.const 26
    i32.const 14
    call $set_component
  )

  (func (export "get_last_entity_count") (result i32)
    global.get $last_entity_count
  )

  (func (export "get_last_tick") (result i64)
    global.get $last_tick
  )
)
```

### Step 4: Write tests

```rust
#[test]
fn host_api_set_component_emits_command() {
    let wasm = load_fixture("host_api_test.wat");
    let mut module = WasmModuleWithHost::from_bytes(&wasm, WasmConfig::default()).unwrap();

    module.host_state_mut().entity_count = 5;
    module.host_state_mut().tick = 42;
    module.host_state_mut().sim_time = 0.7;

    module.call_tick().unwrap();

    // Verify commands were emitted
    let cmds = module.host_state().commands.commands();
    assert_eq!(cmds.len(), 1, "should have 1 set_component command");
    assert_eq!(cmds[0].issued_by, SystemId::WASM_GAMEPLAY);
    assert!(matches!(&cmds[0].reason, CausalReason::GameRule(r) if r == "wasm_test_move"));
}

#[test]
fn host_api_reads_entity_count() {
    let wasm = load_fixture("host_api_test.wat");
    let mut module = WasmModuleWithHost::from_bytes(&wasm, WasmConfig::default()).unwrap();

    module.host_state_mut().entity_count = 42;
    module.call_tick().unwrap();

    let get_count = module.instance()
        .get_typed_func::<(), i32>(module.store_mut(), "get_last_entity_count")
        .unwrap();
    let count = get_count.call(module.store_mut(), ()).unwrap();
    assert_eq!(count, 42);
}

#[test]
fn host_api_reads_tick_number() {
    let wasm = load_fixture("host_api_test.wat");
    let mut module = WasmModuleWithHost::from_bytes(&wasm, WasmConfig::default()).unwrap();

    module.host_state_mut().tick = 99;
    module.call_tick().unwrap();

    let get_tick = module.instance()
        .get_typed_func::<(), i64>(module.store_mut(), "get_last_tick")
        .unwrap();
    let tick = get_tick.call(module.store_mut(), ()).unwrap();
    assert_eq!(tick, 99);
}

#[test]
fn host_api_commands_carry_wasm_system_id() {
    let wasm = load_fixture("host_api_test.wat");
    let mut module = WasmModuleWithHost::from_bytes(&wasm, WasmConfig::default()).unwrap();

    module.call_tick().unwrap();

    for cmd in module.host_state().commands.commands() {
        assert_eq!(cmd.issued_by, SystemId::WASM_GAMEPLAY,
            "all WASM commands must carry WASM_GAMEPLAY system ID");
    }
}

#[test]
fn host_call_counter_increments() {
    let wasm = load_fixture("host_api_test.wat");
    let mut module = WasmModuleWithHost::from_bytes(&wasm, WasmConfig::default()).unwrap();

    module.call_tick().unwrap();

    assert!(module.host_state().host_call_count > 0,
        "host call counter should increment");
}
```

### Step 5: Build, test, commit

```bash
just ci
git add crates/nomai-wasm-host/
git commit -m "feat(wasm): B2 gameplay host API with read/write/utility functions"
```

**Acceptance:** WASM module reads state and emits causally-tagged commands. Commands flow through the same pipeline as native commands.

---

## Task B3: Hot-Swap

**GitHub Issue:** #12

**Depends on:** B2

**Files:**
- Modify: `crates/nomai-wasm-host/src/module.rs` (add `swap_module` method)
- Create: `crates/nomai-wasm-host/tests/fixtures/counter_v2.wat`

### Design

Hot-swap replaces the WASM module at a tick boundary. Since all state lives in the ECS (not in WASM memory), no state migration is needed. The process:

1. Current tick completes normally
2. `WasmModule::swap(new_bytes)` is called
3. New module is compiled + validated (must export `tick`)
4. Old instance is dropped
5. Next `call_tick()` uses the new module

### Step 1: Add swap method to WasmModule

```rust
/// Replace the current WASM module with new bytes.
/// The new module must export `tick()`. No state migration needed —
/// all game state lives in the ECS, not in WASM memory.
///
/// Returns Ok on success, Err if the new module is invalid.
pub fn swap(&mut self, new_wasm_bytes: &[u8]) -> Result<(), WasmError> {
    let start = std::time::Instant::now();

    // Compile new module using existing engine
    let engine = self.store.engine().clone();
    let module = Module::new(&engine, new_wasm_bytes)
        .map_err(|e| WasmError::CompileError(e.to_string()))?;

    // Create new linker and register host API
    let mut linker = Linker::new(&engine);
    register_host_api(&mut linker)?;

    // Instantiate new module
    let instance = linker.instantiate(&mut self.store, &module)
        .map_err(|e| WasmError::Runtime(e.into()))?;

    // Validate tick export
    let tick_fn = instance
        .get_typed_func::<(), ()>(&mut self.store, "tick")
        .map_err(|_| WasmError::MissingExport("tick".to_string()))?;

    // Replace
    self.instance = instance;
    self.tick_fn = tick_fn;

    let elapsed = start.elapsed();
    info!(elapsed_ms = elapsed.as_millis(), "WASM module hot-swapped");

    Ok(())
}
```

### Step 2: Create test fixtures

`crates/nomai-wasm-host/tests/fixtures/counter_v2.wat`:
```wat
(module
  ;; V2: counts by 10 instead of 1
  (global $count (mut i32) (i32.const 0))
  (func (export "tick")
    global.get $count
    i32.const 10
    i32.add
    global.set $count
  )
  (func (export "get_count") (result i32)
    global.get $count
  )
)
```

### Step 3: Write tests

```rust
#[test]
fn hot_swap_changes_behavior() {
    let v1 = load_fixture("counter.wat");
    let v2 = load_fixture("counter_v2.wat");

    let mut module = WasmModule::from_bytes(&v1, WasmConfig::default()).unwrap();

    // Run v1: increments by 1
    module.call_tick().unwrap();

    // Swap to v2
    module.swap(&v2).unwrap();

    // Run v2: increments by 10 (from 0, since state is in WASM which resets)
    module.call_tick().unwrap();

    // State resets because WASM memory is replaced — this is expected.
    // All persistent state should be in the ECS.
}

#[test]
fn hot_swap_validates_exports() {
    let v1 = load_fixture("noop.wat");
    let bad = b"(module (func (export \"not_tick\")))";

    let mut module = WasmModule::from_bytes(&v1, WasmConfig::default()).unwrap();

    let result = module.swap(bad);
    assert!(matches!(result, Err(WasmError::MissingExport(_))));
}

#[test]
fn hot_swap_performance() {
    let v1 = load_fixture("noop.wat");
    let v2 = load_fixture("counter.wat");

    let mut module = WasmModule::from_bytes(&v1, WasmConfig::default()).unwrap();

    let start = std::time::Instant::now();
    module.swap(&v2).unwrap();
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 100,
        "hot-swap should take <100ms, took {}ms", elapsed.as_millis());
}
```

### Step 4: Build, test, commit

```bash
just ci
git commit -m "feat(wasm): B3 hot-swap module replacement at tick boundary"
```

**Acceptance:** Module swap takes <100ms. Simulation continues without state loss.

---

## Task B4: AssemblyScript Compilation Pipeline

**GitHub Issue:** #13

**Depends on:** B1

**Files:**
- Create: `gameplay/assembly/package.json`
- Create: `gameplay/assembly/asconfig.json`
- Create: `gameplay/assembly/tsconfig.json`
- Create: `gameplay/assembly/host.ts` (host bindings)
- Create: `gameplay/assembly/index.ts` (example gameplay)
- Create: `gameplay/build/` (output dir)
- Modify: `justfile` (add `build-gameplay` target)

### Step 1: Set up AssemblyScript project

`gameplay/assembly/package.json`:
```json
{
  "name": "nomai-gameplay",
  "version": "0.1.0",
  "private": true,
  "scripts": {
    "build": "asc assembly/index.ts --outFile build/gameplay.wasm --optimize --exportRuntime"
  },
  "devDependencies": {
    "assemblyscript": "0.28.2"
  }
}
```

`gameplay/assembly/asconfig.json`:
```json
{
  "targets": {
    "release": {
      "outFile": "build/gameplay.wasm",
      "optimize": true,
      "sourceMap": false
    },
    "debug": {
      "outFile": "build/gameplay.debug.wasm",
      "debug": true,
      "sourceMap": true
    }
  },
  "options": {
    "exportRuntime": true
  }
}
```

### Step 2: Create host bindings

`gameplay/assembly/host.ts`:
```typescript
// Host function bindings for the Nomai engine.
// These are provided by the Rust WASM host at runtime.

// Read functions
@external("nomai", "get_entity_count")
export declare function get_entity_count(): i32;

@external("nomai", "sim_time")
export declare function sim_time(): f64;

@external("nomai", "tick_number")
export declare function tick_number(): i64;

@external("nomai", "get_component")
export declare function get_component(entity_id: i64, name_ptr: i32, name_len: i32): i64;

// Write functions — each takes a reason string for causality
@external("nomai", "set_component")
export declare function set_component(
  entity_id: i64,
  name_ptr: i32, name_len: i32,
  value_ptr: i32, value_len: i32,
  reason_ptr: i32, reason_len: i32
): void;

@external("nomai", "spawn_semantic")
export declare function spawn_semantic(
  identity_ptr: i32, identity_len: i32,
  components_ptr: i32, components_len: i32,
  reason_ptr: i32, reason_len: i32
): void;

@external("nomai", "spawn_pooled")
export declare function spawn_pooled(
  identity_ptr: i32, identity_len: i32,
  components_ptr: i32, components_len: i32,
  reason_ptr: i32, reason_len: i32
): void;

@external("nomai", "despawn")
export declare function despawn(
  entity_id: i64,
  reason_ptr: i32, reason_len: i32
): void;

@external("nomai", "emit_event")
export declare function emit_event(
  event_ptr: i32, event_len: i32
): void;

// Utility
@external("nomai", "log")
export declare function log(level: i32, msg_ptr: i32, msg_len: i32): void;

// Helper to convert a string to ptr+len for host calls
export function stringToPtr(s: string): i32 {
  return changetype<i32>(String.UTF8.encode(s));
}

export function stringLen(s: string): i32 {
  return String.UTF8.byteLength(s);
}
```

### Step 3: Create example gameplay module

`gameplay/assembly/index.ts`:
```typescript
import {
  get_entity_count,
  tick_number,
  set_component,
  log
} from "./host";

// Example: simple gameplay that moves an entity each tick
export function tick(): void {
  const tick = tick_number();
  const entityCount = get_entity_count();

  // Log tick info
  const msg = `tick ${tick}, entities: ${entityCount}`;
  const msgBuf = String.UTF8.encode(msg);
  log(1, changetype<i32>(msgBuf), msgBuf.byteLength);

  // Move entity 0 to the right each tick
  const entityId: i64 = 0;
  const name = "position";
  const nameBuf = String.UTF8.encode(name);
  const value = `{"x":${f64(tick)},"y":0.0}`;
  const valueBuf = String.UTF8.encode(value);
  const reason = "move_right_each_tick";
  const reasonBuf = String.UTF8.encode(reason);

  set_component(
    entityId,
    changetype<i32>(nameBuf), nameBuf.byteLength,
    changetype<i32>(valueBuf), valueBuf.byteLength,
    changetype<i32>(reasonBuf), reasonBuf.byteLength
  );
}
```

### Step 4: Add justfile target

Add to `justfile`:
```
# Compile AssemblyScript gameplay to WASM
build-gameplay:
    cd gameplay && npm run build
```

### Step 5: Build and test

```bash
cd gameplay && npm install && npm run build
# Verify gameplay/build/gameplay.wasm exists

# Load in Rust test:
just test  # Should pass including new test that loads the AS-compiled wasm
```

### Step 6: Write Rust integration test

In `crates/nomai-wasm-host/src/module.rs` or separate integration test:

```rust
#[test]
#[ignore] // Requires npm + AS build; run with `just test -- --ignored`
fn load_assemblyscript_module() {
    let wasm_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../gameplay/build/gameplay.wasm");
    let wasm = std::fs::read(wasm_path)
        .expect("AS gameplay.wasm not found — run `just build-gameplay` first");

    let mut module = WasmModuleWithHost::from_bytes(&wasm, WasmConfig::default()).unwrap();
    module.host_state_mut().entity_count = 3;
    module.host_state_mut().tick = 1;

    module.call_tick().unwrap();

    let cmds = module.host_state().commands.commands();
    assert!(!cmds.is_empty(), "AS module should emit at least one command");
}
```

### Step 7: Commit

```bash
git add gameplay/ justfile crates/nomai-wasm-host/
git commit -m "feat(wasm): B4 AssemblyScript compilation pipeline with host bindings"
```

**Acceptance:** AS gameplay module compiles, loads, and executes correctly.

---

## Task B5: Causality Across WASM Boundary

**GitHub Issue:** #14

**Depends on:** B2

**Files:**
- Create: `crates/nomai-wasm-host/src/integration.rs` (orchestrator that ties WASM → ECS → manifest)
- Create: `crates/nomai-wasm-host/tests/fixtures/causality_test.wat`
- Modify: `crates/nomai-wasm-host/src/lib.rs` (add module export)

### Design

This task validates the full causal chain: WASM module emits command with reason string → command carries `CausalReason::GameRule(reason)` and `SystemId::WASM_GAMEPLAY` → command applied to world → manifest records `ComponentChange` with causality → `build_causal_chain()` traces back to WASM reason.

### Step 1: Create integration orchestrator

`crates/nomai-wasm-host/src/integration.rs`:
```rust
//! Integration helper: connects WASM module execution to the full
//! ECS tick loop and manifest pipeline.

use nomai_ecs::command::CommandBuffer;
use nomai_ecs::world::World;
use nomai_manifest::manifest::{ManifestPipeline, TickManifest};

use crate::module::{WasmConfig, WasmModule};
use crate::error::WasmError;

/// Runs a full tick with WASM gameplay: execute WASM → drain commands →
/// apply to world → process manifest.
///
/// Returns the tick manifest and the number of commands applied.
pub fn run_wasm_tick(
    module: &mut WasmModule,
    world: &mut World,
    manifest: &mut ManifestPipeline,
    tick: u64,
    sim_time: f64,
) -> Result<(TickManifest, usize), WasmError> {
    // 1. Prepare host state
    module.host_state_mut().begin_tick(tick, sim_time);
    module.host_state_mut().snapshot_world(world);

    // 2. Begin manifest tick
    manifest.begin_tick();

    // 3. Execute WASM
    module.call_tick()?;

    // 4. Drain commands and events from WASM
    let mut cmd_buf = module.host_state_mut().drain_commands();
    let events = module.host_state_mut().drain_events();

    // 5. Apply commands to world
    let applied = cmd_buf.apply(world);
    let cmd_count = applied.len();

    // 6. Process commands into manifest
    manifest.process_commands(&applied, tick, world);

    // 7. Record events
    for event in events {
        manifest.record_event(event);
    }

    // 8. Finalize manifest
    let tick_manifest = manifest.end_tick(
        tick,
        sim_time,
        vec!["wasm_gameplay".to_string()],
        world,
    );

    Ok((tick_manifest, cmd_count))
}
```

### Step 2: Create WAT fixture that reacts to state

`crates/nomai-wasm-host/tests/fixtures/causality_test.wat`:
A WASM module that reads entity count and emits a command with a specific reason string that traces through the causal chain.

### Step 3: Write integration test

```rust
#[test]
fn causal_chain_traces_through_wasm_boundary() {
    // Setup world with a registered component and entity
    let mut world = World::new();
    world.register_component::<Position>("position");
    let entity = world.spawn_semantic(
        EntityIdentity {
            entity_type: "character".to_owned(),
            role: "player".to_owned(),
            spawned_by: SystemId::ENGINE_INTERNAL,
            requirement_id: None,
        },
        ComponentBundle::new(),
    ).unwrap();

    // Setup manifest pipeline
    let mut manifest = ManifestPipeline::new();

    // Load WASM module that emits set_component
    let wasm = load_fixture("host_api_test.wat");
    let mut module = WasmModule::from_bytes(&wasm, WasmConfig::default()).unwrap();

    // Run a tick
    let (tick_manifest, cmd_count) = run_wasm_tick(
        &mut module, &mut world, &mut manifest, 1, 1.0/60.0
    ).unwrap();

    assert!(cmd_count > 0, "WASM should have emitted commands");

    // Verify manifest has component changes
    assert!(!tick_manifest.component_changes.is_empty(),
        "manifest should record component changes");

    // Verify causality chain
    let change = &tick_manifest.component_changes[0];
    assert_eq!(change.changed_by, SystemId::WASM_GAMEPLAY);
    assert!(matches!(&change.reason,
        CausalReason::GameRule(r) if r == "wasm_test_move"),
        "causal reason should trace back to WASM reason string");

    // Build causal chain
    let chain = manifest.build_causal_chain(change);
    assert!(!chain.steps.is_empty(), "causal chain should have steps");
    assert_eq!(chain.steps[0].system_id, SystemId::WASM_GAMEPLAY);
}
```

### Step 4: Commit

```bash
just ci
git commit -m "feat(wasm): B5 unbroken causality across WASM boundary"
```

**Acceptance:** Causal chains are unbroken from manifest observation through WASM boundary to root cause.

---

## Task B6: Intent Spec Data Structures (Python)

**GitHub Issue:** #15

**Depends on:** Nothing (parallel track)

**Files:**
- Create: `python/nomai-sdk/pyproject.toml`
- Create: `python/nomai-sdk/nomai/__init__.py`
- Create: `python/nomai-sdk/nomai/intents.py`
- Create: `python/nomai-sdk/nomai/manifest.py`
- Create: `python/nomai-sdk/tests/__init__.py`
- Create: `python/nomai-sdk/tests/test_intents.py`

### Step 1: Set up Python project

`python/nomai-sdk/pyproject.toml`:
```toml
[build-system]
requires = ["setuptools>=68.0"]
build-backend = "setuptools.backends._legacy:_Backend"

[project]
name = "nomai-sdk"
version = "0.1.0"
requires-python = ">=3.12"
dependencies = []

[project.optional-dependencies]
dev = ["pytest>=7.0", "pyright>=1.1"]

[tool.pyright]
pythonVersion = "3.12"
typeCheckingMode = "strict"
```

### Step 2: Create manifest data types

`python/nomai-sdk/nomai/manifest.py`:
```python
"""Manifest data types mirroring Rust TickManifest for Python consumption."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True)
class ComponentChange:
    """A single component mutation with causality."""
    entity_id: int
    component_type_name: str
    old_value: Any | None
    new_value: Any | None
    changed_by_system: int
    reason_type: str  # "PlayerInput", "GameRule", etc.
    reason_detail: str  # The reason string
    command_index: int
    tick: int


@dataclass(frozen=True)
class GameEvent:
    """A game event with involved entities and causality."""
    event_type: str
    description: str
    involved_entities: list[int]
    caused_by_system: int
    reason_type: str
    reason_detail: str
    tick: int


@dataclass(frozen=True)
class Aggregates:
    """Aggregate statistics for a tick."""
    entity_count_by_tier: dict[str, int] = field(default_factory=dict)
    entity_count_by_type: dict[str, int] = field(default_factory=dict)
    total_entity_count: int = 0


@dataclass(frozen=True)
class CausalStep:
    """One step in a causal chain."""
    tick: int
    command_index: int
    system_id: int
    reason_type: str
    reason_detail: str
    description: str


@dataclass(frozen=True)
class CausalChain:
    """A chain of causal steps tracing a change back to its root cause."""
    entity_id: int
    component: str
    steps: list[CausalStep] = field(default_factory=list)


@dataclass(frozen=True)
class EntityEntry:
    """An entity in the manifest's entity index."""
    entity_id: int
    tier: str  # "Semantic" or "Pooled"
    entity_type: str
    role: str
    alive: bool
    spawned_at_tick: int
    despawned_at_tick: int | None = None


@dataclass
class TickManifest:
    """The manifest for a single tick — the primary verification surface."""
    tick: int
    sim_time: float
    entity_spawns: list[int] = field(default_factory=list)
    entity_despawns: list[int] = field(default_factory=list)
    component_changes: list[ComponentChange] = field(default_factory=list)
    events: list[GameEvent] = field(default_factory=list)
    aggregates: Aggregates = field(default_factory=Aggregates)
    systems_executed: list[str] = field(default_factory=list)
    commands_processed: int = 0
    commands_succeeded: int = 0

    @staticmethod
    def from_json(data: dict[str, Any]) -> TickManifest:
        """Parse a TickManifest from JSON dict (as produced by Rust serde)."""
        changes = [
            ComponentChange(
                entity_id=c["entity_id"],
                component_type_name=c["component_type_name"],
                old_value=c.get("old_value"),
                new_value=c.get("new_value"),
                changed_by_system=c["changed_by"]["0"] if isinstance(c["changed_by"], dict) else c["changed_by"],
                reason_type=_parse_reason_type(c["reason"]),
                reason_detail=_parse_reason_detail(c["reason"]),
                command_index=c["command_index"],
                tick=c["tick"],
            )
            for c in data.get("component_changes", [])
        ]

        events = [
            GameEvent(
                event_type=e["event_type"],
                description=e["description"],
                involved_entities=e.get("involved_entities", []),
                caused_by_system=e["caused_by"]["0"] if isinstance(e["caused_by"], dict) else e["caused_by"],
                reason_type=_parse_reason_type(e["reason"]),
                reason_detail=_parse_reason_detail(e["reason"]),
                tick=e["tick"],
            )
            for e in data.get("events", [])
        ]

        agg_data = data.get("aggregates", {})
        aggregates = Aggregates(
            entity_count_by_tier=agg_data.get("entity_count_by_tier", {}),
            entity_count_by_type=agg_data.get("entity_count_by_type", {}),
            total_entity_count=agg_data.get("total_entity_count", 0),
        )

        return TickManifest(
            tick=data["tick"],
            sim_time=data["sim_time"],
            entity_spawns=data.get("entity_spawns", []),
            entity_despawns=data.get("entity_despawns", []),
            component_changes=changes,
            events=events,
            aggregates=aggregates,
            systems_executed=data.get("systems_executed", []),
            commands_processed=data.get("commands_processed", 0),
            commands_succeeded=data.get("commands_succeeded", 0),
        )


def _parse_reason_type(reason: Any) -> str:
    """Extract the reason type from a serde-serialized CausalReason."""
    if isinstance(reason, dict):
        # Serde serializes enums as {"VariantName": data}
        return next(iter(reason.keys()))
    if isinstance(reason, str):
        return reason
    return "Unknown"


def _parse_reason_detail(reason: Any) -> str:
    """Extract the reason detail string from a serde-serialized CausalReason."""
    if isinstance(reason, dict):
        value = next(iter(reason.values()))
        if isinstance(value, str):
            return value
        if isinstance(value, dict):
            return str(value)
    if isinstance(reason, str):
        return reason
    return ""
```

### Step 3: Create intent spec data structures

`python/nomai-sdk/nomai/intents.py`:
```python
"""Intent specification DSL for the Nomai verification engine.

Intent specs describe what SHOULD happen in a simulation. The verification
engine checks these against the manifest to determine correctness.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from enum import Enum, auto
from typing import Any


# ---------------------------------------------------------------------------
# Trigger Expressions — WHEN to check
# ---------------------------------------------------------------------------

class TriggerType(Enum):
    """Types of trigger conditions."""
    COLLISION = auto()
    STATE_TRANSITION = auto()
    AGGREGATE_CONDITION = auto()
    COMPONENT_CONDITION = auto()
    EVENT_OCCURRED = auto()
    TICK_REACHED = auto()
    AND = auto()
    OR = auto()


@dataclass(frozen=True)
class Trigger:
    """A condition that activates a behavioral check."""
    type: TriggerType
    params: dict[str, Any] = field(default_factory=dict)
    children: list[Trigger] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        result: dict[str, Any] = {
            "type": self.type.name,
            "params": self.params,
        }
        if self.children:
            result["children"] = [c.to_dict() for c in self.children]
        return result

    @staticmethod
    def from_dict(data: dict[str, Any]) -> Trigger:
        children = [Trigger.from_dict(c) for c in data.get("children", [])]
        return Trigger(
            type=TriggerType[data["type"]],
            params=data.get("params", {}),
            children=children,
        )


# Trigger constructors
def collision(entity_a_role: str, entity_b_role: str) -> Trigger:
    return Trigger(TriggerType.COLLISION, {"entity_a_role": entity_a_role, "entity_b_role": entity_b_role})

def state_transition(entity_role: str, from_state: str, to_state: str) -> Trigger:
    return Trigger(TriggerType.STATE_TRANSITION, {"entity_role": entity_role, "from": from_state, "to": to_state})

def aggregate_condition(aggregate_name: str, operator: str, value: float) -> Trigger:
    return Trigger(TriggerType.AGGREGATE_CONDITION, {"aggregate": aggregate_name, "op": operator, "value": value})

def component_condition(entity_role: str, component: str, field_name: str, operator: str, value: Any) -> Trigger:
    return Trigger(TriggerType.COMPONENT_CONDITION, {"entity_role": entity_role, "component": component, "field": field_name, "op": operator, "value": value})

def event_occurred(event_type: str) -> Trigger:
    return Trigger(TriggerType.EVENT_OCCURRED, {"event_type": event_type})

def tick_reached(tick: int) -> Trigger:
    return Trigger(TriggerType.TICK_REACHED, {"tick": tick})

def and_(*triggers: Trigger) -> Trigger:
    return Trigger(TriggerType.AND, children=list(triggers))

def or_(*triggers: Trigger) -> Trigger:
    return Trigger(TriggerType.OR, children=list(triggers))


# ---------------------------------------------------------------------------
# Expected Outcomes — WHAT should happen
# ---------------------------------------------------------------------------

class ExpectedType(Enum):
    """Types of expected outcomes."""
    COMPONENT_CHANGED = auto()
    ENTITY_DESPAWNED = auto()
    AGGREGATE_CHANGED = auto()
    IN_STATE = auto()
    EVENT_EMITTED = auto()
    ALL = auto()
    ANY = auto()


@dataclass(frozen=True)
class Expected:
    """An expected outcome after a trigger fires."""
    type: ExpectedType
    params: dict[str, Any] = field(default_factory=dict)
    children: list[Expected] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        result: dict[str, Any] = {
            "type": self.type.name,
            "params": self.params,
        }
        if self.children:
            result["children"] = [c.to_dict() for c in self.children]
        return result

    @staticmethod
    def from_dict(data: dict[str, Any]) -> Expected:
        children = [Expected.from_dict(c) for c in data.get("children", [])]
        return Expected(
            type=ExpectedType[data["type"]],
            params=data.get("params", {}),
            children=children,
        )


# Expected constructors
def component_changed(entity_role: str, component: str, field_name: str, expected_value: Any) -> Expected:
    return Expected(ExpectedType.COMPONENT_CHANGED, {"entity_role": entity_role, "component": component, "field": field_name, "value": expected_value})

def entity_despawned(entity_role: str) -> Expected:
    return Expected(ExpectedType.ENTITY_DESPAWNED, {"entity_role": entity_role})

def aggregate_changed(aggregate_name: str, operator: str, value: float) -> Expected:
    return Expected(ExpectedType.AGGREGATE_CHANGED, {"aggregate": aggregate_name, "op": operator, "value": value})

def in_state(entity_role: str, state: str) -> Expected:
    return Expected(ExpectedType.IN_STATE, {"entity_role": entity_role, "state": state})

def event_emitted(event_type: str) -> Expected:
    return Expected(ExpectedType.EVENT_EMITTED, {"event_type": event_type})

def all_(*expected: Expected) -> Expected:
    return Expected(ExpectedType.ALL, children=list(expected))

def any_(*expected: Expected) -> Expected:
    return Expected(ExpectedType.ANY, children=list(expected))


# ---------------------------------------------------------------------------
# Intent Specs
# ---------------------------------------------------------------------------

class IntentKind(Enum):
    """The kind of verification intent."""
    ENTITY = auto()       # Entity existence + required components
    BEHAVIOR = auto()     # Trigger → expected outcome
    METRIC = auto()       # Component value within range over time
    INVARIANT = auto()    # Condition must hold every tick


@dataclass
class IntentSpec:
    """A single verification intent."""
    name: str
    kind: IntentKind
    description: str = ""
    # Entity intent fields
    entity_role: str | None = None
    entity_type: str | None = None
    required_components: list[str] = field(default_factory=list)
    # Behavior intent fields
    trigger: Trigger | None = None
    expected: Expected | None = None
    timeout_ticks: int = 600  # 10 seconds at 60Hz
    # Metric intent fields
    component: str | None = None
    field_name: str | None = None
    min_value: float | None = None
    max_value: float | None = None
    # Invariant intent fields
    condition: Trigger | None = None  # Must hold every tick

    def to_dict(self) -> dict[str, Any]:
        result: dict[str, Any] = {
            "name": self.name,
            "kind": self.kind.name,
            "description": self.description,
        }
        if self.entity_role is not None:
            result["entity_role"] = self.entity_role
        if self.entity_type is not None:
            result["entity_type"] = self.entity_type
        if self.required_components:
            result["required_components"] = self.required_components
        if self.trigger is not None:
            result["trigger"] = self.trigger.to_dict()
        if self.expected is not None:
            result["expected"] = self.expected.to_dict()
        if self.kind == IntentKind.BEHAVIOR:
            result["timeout_ticks"] = self.timeout_ticks
        if self.component is not None:
            result["component"] = self.component
        if self.field_name is not None:
            result["field"] = self.field_name
        if self.min_value is not None:
            result["min_value"] = self.min_value
        if self.max_value is not None:
            result["max_value"] = self.max_value
        if self.condition is not None:
            result["condition"] = self.condition.to_dict()
        return result

    @staticmethod
    def from_dict(data: dict[str, Any]) -> IntentSpec:
        trigger = Trigger.from_dict(data["trigger"]) if "trigger" in data else None
        expected = Expected.from_dict(data["expected"]) if "expected" in data else None
        condition = Trigger.from_dict(data["condition"]) if "condition" in data else None
        return IntentSpec(
            name=data["name"],
            kind=IntentKind[data["kind"]],
            description=data.get("description", ""),
            entity_role=data.get("entity_role"),
            entity_type=data.get("entity_type"),
            required_components=data.get("required_components", []),
            trigger=trigger,
            expected=expected,
            timeout_ticks=data.get("timeout_ticks", 600),
            component=data.get("component"),
            field_name=data.get("field"),
            min_value=data.get("min_value"),
            max_value=data.get("max_value"),
            condition=condition,
        )

    def to_json(self) -> str:
        return json.dumps(self.to_dict(), indent=2)

    @staticmethod
    def from_json(json_str: str) -> IntentSpec:
        return IntentSpec.from_dict(json.loads(json_str))


@dataclass
class VerificationSuite:
    """A collection of intent specs for a scenario."""
    name: str
    description: str = ""
    intents: list[IntentSpec] = field(default_factory=list)

    def to_json(self) -> str:
        return json.dumps({
            "name": self.name,
            "description": self.description,
            "intents": [i.to_dict() for i in self.intents],
        }, indent=2)

    @staticmethod
    def from_json(json_str: str) -> VerificationSuite:
        data = json.loads(json_str)
        return VerificationSuite(
            name=data["name"],
            description=data.get("description", ""),
            intents=[IntentSpec.from_dict(i) for i in data.get("intents", [])],
        )
```

### Step 4: Write tests

`python/nomai-sdk/tests/test_intents.py`: Test construction, JSON serialization roundtrip, and expressing breakout verification assertions (paddle exists, ball bounces, bricks destroyed).

### Step 5: Commit

```bash
cd python/nomai-sdk && pip install -e ".[dev]" && pytest tests/ -v
git add python/nomai-sdk/
git commit -m "feat(python): B6 intent spec data structures with JSON serialization"
```

**Acceptance:** Intent specs can express all breakout verification assertions. Round-trip serialization works.

---

## Task B7: Verification Engine

**GitHub Issue:** #16

**Depends on:** B6

**Files:**
- Create: `python/nomai-sdk/nomai/verify.py`
- Create: `python/nomai-sdk/tests/test_verify.py`

### Design

The verification engine takes a `VerificationSuite` and a list of `TickManifest` objects, and checks each intent against the manifests. It produces a `VerificationReport` with per-intent results.

Four verification modes:
1. **Entity**: Check entity exists in entity index with required components
2. **Behavior**: Wait for trigger in manifests, then check expected outcome
3. **Metric**: Check component value within [min, max] across all ticks
4. **Invariant**: Check condition holds every tick

### Step 1: Create verification engine

`python/nomai-sdk/nomai/verify.py`:
```python
"""Verification engine — checks intent specs against manifest data.

The verification engine operates purely on manifest data. It never
looks at rendered output (no Pixel Peeking).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from nomai.intents import (
    IntentSpec, IntentKind, VerificationSuite,
    Trigger, TriggerType, Expected, ExpectedType,
)
from nomai.manifest import TickManifest, ComponentChange, CausalChain, CausalStep


@dataclass
class IntentResult:
    """Result of checking a single intent."""
    intent_name: str
    passed: bool
    failure_reason: str = ""
    trigger_tick: int | None = None
    evidence: list[ComponentChange] = field(default_factory=list)
    causal_chain: CausalChain | None = None
    suggestion: str = ""

    def to_dict(self) -> dict[str, Any]:
        result: dict[str, Any] = {
            "intent_name": self.intent_name,
            "passed": self.passed,
        }
        if not self.passed:
            result["failure_reason"] = self.failure_reason
            if self.suggestion:
                result["suggestion"] = self.suggestion
        if self.trigger_tick is not None:
            result["trigger_tick"] = self.trigger_tick
        return result


@dataclass
class VerificationReport:
    """Complete verification report for a suite."""
    suite_name: str
    total_intents: int
    passed: int
    failed: int
    results: list[IntentResult] = field(default_factory=list)

    @property
    def all_passed(self) -> bool:
        return self.failed == 0

    def summary(self) -> str:
        status = "PASS" if self.all_passed else "FAIL"
        lines = [f"[{status}] {self.suite_name}: {self.passed}/{self.total_intents} intents passed"]
        for r in self.results:
            mark = "PASS" if r.passed else "FAIL"
            lines.append(f"  [{mark}] {r.intent_name}")
            if not r.passed:
                lines.append(f"         Reason: {r.failure_reason}")
                if r.suggestion:
                    lines.append(f"         Suggestion: {r.suggestion}")
        return "\n".join(lines)

    def to_dict(self) -> dict[str, Any]:
        return {
            "suite_name": self.suite_name,
            "total_intents": self.total_intents,
            "passed": self.passed,
            "failed": self.failed,
            "all_passed": self.all_passed,
            "results": [r.to_dict() for r in self.results],
        }


class VerificationEngine:
    """Checks intent specs against manifest history."""

    def verify(
        self,
        suite: VerificationSuite,
        manifests: list[TickManifest],
        entity_index: dict[int, dict[str, Any]] | None = None,
    ) -> VerificationReport:
        results: list[IntentResult] = []

        for intent in suite.intents:
            match intent.kind:
                case IntentKind.ENTITY:
                    result = self._verify_entity(intent, manifests, entity_index or {})
                case IntentKind.BEHAVIOR:
                    result = self._verify_behavior(intent, manifests)
                case IntentKind.METRIC:
                    result = self._verify_metric(intent, manifests)
                case IntentKind.INVARIANT:
                    result = self._verify_invariant(intent, manifests)
            results.append(result)

        passed = sum(1 for r in results if r.passed)
        failed = len(results) - passed

        return VerificationReport(
            suite_name=suite.name,
            total_intents=len(results),
            passed=passed,
            failed=failed,
            results=results,
        )

    def _verify_entity(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
        entity_index: dict[int, dict[str, Any]],
    ) -> IntentResult:
        """Check that an entity with the given role exists."""
        # Search for entity in spawns across all manifests
        for m in manifests:
            for change in m.component_changes:
                if (change.component_type_name == "__identity" and
                    change.new_value is not None):
                    # Check if this matches our entity role/type
                    identity = change.new_value
                    if isinstance(identity, dict):
                        sem = identity.get("Semantic", {})
                        if sem.get("role") == intent.entity_role:
                            return IntentResult(
                                intent_name=intent.name,
                                passed=True,
                                trigger_tick=m.tick,
                            )

        # Also check entity_index
        for eid, entry in entity_index.items():
            if entry.get("role") == intent.entity_role:
                return IntentResult(
                    intent_name=intent.name,
                    passed=True,
                )

        return IntentResult(
            intent_name=intent.name,
            passed=False,
            failure_reason=f"Entity with role '{intent.entity_role}' not found in manifest",
            suggestion=f"Add a spawn command for entity with role '{intent.entity_role}'",
        )

    def _verify_behavior(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Wait for trigger, then check expected outcome."""
        if intent.trigger is None or intent.expected is None:
            return IntentResult(
                intent_name=intent.name,
                passed=False,
                failure_reason="Behavior intent missing trigger or expected",
            )

        # Find the tick where trigger fires
        trigger_tick_idx = None
        for i, m in enumerate(manifests):
            if self._check_trigger(intent.trigger, m):
                trigger_tick_idx = i
                break

        if trigger_tick_idx is None:
            return IntentResult(
                intent_name=intent.name,
                passed=False,
                failure_reason=f"Trigger never fired within {len(manifests)} ticks",
                suggestion="Check that the triggering condition can occur (e.g., entities exist, collisions happen)",
            )

        # Check expected outcome in remaining manifests
        for m in manifests[trigger_tick_idx:]:
            if self._check_expected(intent.expected, m):
                return IntentResult(
                    intent_name=intent.name,
                    passed=True,
                    trigger_tick=manifests[trigger_tick_idx].tick,
                )

        return IntentResult(
            intent_name=intent.name,
            passed=False,
            trigger_tick=manifests[trigger_tick_idx].tick,
            failure_reason=f"Trigger fired at tick {manifests[trigger_tick_idx].tick} but expected outcome never observed",
            suggestion="Check that the gameplay logic responds to the trigger correctly",
        )

    def _verify_metric(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Check component value within range across all ticks."""
        for m in manifests:
            for change in m.component_changes:
                if (intent.entity_role and intent.component and
                    change.component_type_name == intent.component):
                    value = change.new_value
                    if isinstance(value, dict) and intent.field_name:
                        value = value.get(intent.field_name)
                    if isinstance(value, (int, float)):
                        if intent.min_value is not None and value < intent.min_value:
                            return IntentResult(
                                intent_name=intent.name,
                                passed=False,
                                trigger_tick=m.tick,
                                failure_reason=f"Metric violation at tick {m.tick}: {intent.component}.{intent.field_name} = {value} < {intent.min_value}",
                                evidence=[change],
                            )
                        if intent.max_value is not None and value > intent.max_value:
                            return IntentResult(
                                intent_name=intent.name,
                                passed=False,
                                trigger_tick=m.tick,
                                failure_reason=f"Metric violation at tick {m.tick}: {intent.component}.{intent.field_name} = {value} > {intent.max_value}",
                                evidence=[change],
                            )

        return IntentResult(intent_name=intent.name, passed=True)

    def _verify_invariant(
        self,
        intent: IntentSpec,
        manifests: list[TickManifest],
    ) -> IntentResult:
        """Check condition holds every tick."""
        if intent.condition is None:
            return IntentResult(
                intent_name=intent.name,
                passed=False,
                failure_reason="Invariant intent missing condition",
            )

        for m in manifests:
            if not self._check_trigger(intent.condition, m):
                return IntentResult(
                    intent_name=intent.name,
                    passed=False,
                    trigger_tick=m.tick,
                    failure_reason=f"Invariant violated at tick {m.tick}",
                    suggestion="Check that the invariant condition is maintained by gameplay logic",
                )

        return IntentResult(intent_name=intent.name, passed=True)

    def _check_trigger(self, trigger: Trigger, manifest: TickManifest) -> bool:
        """Check if a trigger condition is met in a manifest."""
        match trigger.type:
            case TriggerType.TICK_REACHED:
                return manifest.tick >= trigger.params.get("tick", 0)
            case TriggerType.EVENT_OCCURRED:
                event_type = trigger.params.get("event_type", "")
                return any(e.event_type == event_type for e in manifest.events)
            case TriggerType.COMPONENT_CONDITION:
                return self._check_component_condition(trigger, manifest)
            case TriggerType.AGGREGATE_CONDITION:
                return self._check_aggregate_condition(trigger, manifest)
            case TriggerType.AND:
                return all(self._check_trigger(c, manifest) for c in trigger.children)
            case TriggerType.OR:
                return any(self._check_trigger(c, manifest) for c in trigger.children)
            case _:
                return False

    def _check_expected(self, expected: Expected, manifest: TickManifest) -> bool:
        """Check if an expected outcome is met in a manifest."""
        match expected.type:
            case ExpectedType.COMPONENT_CHANGED:
                return self._check_component_changed(expected, manifest)
            case ExpectedType.ENTITY_DESPAWNED:
                role = expected.params.get("entity_role", "")
                return len(manifest.entity_despawns) > 0
            case ExpectedType.EVENT_EMITTED:
                event_type = expected.params.get("event_type", "")
                return any(e.event_type == event_type for e in manifest.events)
            case ExpectedType.ALL:
                return all(self._check_expected(c, manifest) for c in expected.children)
            case ExpectedType.ANY:
                return any(self._check_expected(c, manifest) for c in expected.children)
            case _:
                return False

    def _check_component_condition(self, trigger: Trigger, manifest: TickManifest) -> bool:
        component = trigger.params.get("component", "")
        field_name = trigger.params.get("field", "")
        op = trigger.params.get("op", "==")
        value = trigger.params.get("value")

        for change in manifest.component_changes:
            if change.component_type_name == component and change.new_value is not None:
                actual = change.new_value
                if isinstance(actual, dict) and field_name:
                    actual = actual.get(field_name)
                if self._compare(actual, op, value):
                    return True
        return False

    def _check_aggregate_condition(self, trigger: Trigger, manifest: TickManifest) -> bool:
        aggregate = trigger.params.get("aggregate", "")
        op = trigger.params.get("op", "==")
        value = trigger.params.get("value", 0)

        if aggregate == "total_entity_count":
            return self._compare(manifest.aggregates.total_entity_count, op, value)
        actual = manifest.aggregates.entity_count_by_type.get(aggregate, 0)
        return self._compare(actual, op, value)

    def _check_component_changed(self, expected: Expected, manifest: TickManifest) -> bool:
        component = expected.params.get("component", "")
        field_name = expected.params.get("field", "")
        value = expected.params.get("value")

        for change in manifest.component_changes:
            if change.component_type_name == component and change.new_value is not None:
                actual = change.new_value
                if isinstance(actual, dict) and field_name:
                    actual = actual.get(field_name)
                if actual == value:
                    return True
        return False

    @staticmethod
    def _compare(actual: Any, op: str, expected: Any) -> bool:
        match op:
            case "==" | "eq":
                return actual == expected
            case "!=" | "ne":
                return actual != expected
            case ">" | "gt":
                return actual > expected
            case ">=" | "gte":
                return actual >= expected
            case "<" | "lt":
                return actual < expected
            case "<=" | "lte":
                return actual <= expected
            case _:
                return False
```

### Step 2: Write tests

`python/nomai-sdk/tests/test_verify.py`: Mock manifests for pass/fail cases across all four verification modes.

### Step 3: Commit

```bash
cd python/nomai-sdk && pytest tests/ -v
git add python/nomai-sdk/
git commit -m "feat(python): B7 verification engine with entity/behavior/metric/invariant checks"
```

**Acceptance:** Verification engine correctly distinguishes correct from incorrect behavior using only manifest data.

---

## Task B8: Integration Test — Intentional Failures

**GitHub Issue:** #17

**Depends on:** B5, B7

**Files:**
- Create: `tests/integration/spike_b_integration.rs` (Rust side)
- Create: `python/nomai-sdk/tests/test_integration_b8.py` (Python side)
- Create: `crates/nomai-wasm-host/tests/fixtures/correct_movement.wat`
- Create: `crates/nomai-wasm-host/tests/fixtures/buggy_movement.wat`
- Create: `tests/fixtures/b8_manifests/` (exported manifest JSON)

### Design

Create a simple scenario: 3 entities (A, B, C). Entity A should move toward entity B each tick. The correct WASM moves A toward B. The buggy WASM moves A away from B.

**Flow:**
1. Rust test sets up world with entities, runs correct WASM → exports manifest JSON
2. Rust test runs buggy WASM → exports manifest JSON
3. Python test loads both manifests + intent spec → verifies correct passes, buggy fails
4. Python test verifies the failure report includes causal chain explaining WHY

### Step 1: Create WAT fixtures

`correct_movement.wat`: Moves entity A's position.x += 1 each tick (toward B at x=10)
`buggy_movement.wat`: Moves entity A's position.x -= 1 each tick (away from B)

### Step 2: Rust integration test

Runs both WASM modules through the full pipeline, serializes manifests to JSON files.

### Step 3: Python integration test

```python
def test_correct_gameplay_passes_verification():
    manifests = load_manifests("tests/fixtures/b8_manifests/correct/")
    suite = build_movement_intent_suite()
    engine = VerificationEngine()
    report = engine.verify(suite, manifests)
    assert report.all_passed

def test_buggy_gameplay_fails_verification():
    manifests = load_manifests("tests/fixtures/b8_manifests/buggy/")
    suite = build_movement_intent_suite()
    engine = VerificationEngine()
    report = engine.verify(suite, manifests)
    assert not report.all_passed
    # Verify failure report includes causal diagnosis
    failed = [r for r in report.results if not r.passed]
    assert len(failed) > 0
    assert "reason" in failed[0].failure_reason.lower() or len(failed[0].failure_reason) > 0
```

### Step 4: Commit

```bash
just ci && cd python/nomai-sdk && pytest tests/ -v
git add tests/ python/nomai-sdk/ crates/nomai-wasm-host/
git commit -m "feat: B8 integration test — correct gameplay passes, buggy fails with diagnosis"
```

**Acceptance:** Verification engine detects the bug and provides a causal explanation.

---

## Task B9: WASM Overhead Benchmark

**GitHub Issue:** #18

**Depends on:** B2, B4

**Files:**
- Create: `crates/nomai-wasm-host/benches/wasm_benchmarks.rs`
- Create: `crates/nomai-wasm-host/tests/fixtures/bench_50_calls.wat`
- Modify: `crates/nomai-wasm-host/Cargo.toml` (add criterion dev-dep)

### Design

Benchmark 50 host calls per tick. Create a WAT module that makes exactly 50 host calls (mix of reads and writes). Compare with equivalent native Rust system.

### Step 1: Create benchmark fixture

`bench_50_calls.wat`: Module that calls 25 reads + 25 writes per tick.

### Step 2: Criterion benchmark

```rust
fn wasm_50_host_calls(c: &mut Criterion) {
    let wasm = std::fs::read("tests/fixtures/bench_50_calls.wat").unwrap();
    let mut module = WasmModule::from_bytes(&wasm, WasmConfig::default()).unwrap();
    // Pre-populate host state with entities

    c.bench_function("wasm_50_host_calls", |b| {
        b.iter(|| {
            module.host_state_mut().begin_tick(1, 0.016);
            module.call_tick().unwrap();
        })
    });
}

fn native_equivalent(c: &mut Criterion) {
    // Same logic as pure Rust for comparison
    c.bench_function("native_equivalent_50_ops", |b| {
        // ... 50 equivalent operations directly on CommandBuffer ...
    });
}
```

### Step 3: Run and record

```bash
cargo bench -p nomai-wasm-host
# Save results to benchmarks/spike_b.json
```

### Step 4: Commit

```bash
git add crates/nomai-wasm-host/benches/ benchmarks/
git commit -m "feat: B9 WASM overhead benchmark — 50 host calls/tick"
```

**Acceptance criteria:**
- 50 host calls/tick: **<1ms total WASM overhead**
- WASM vs native: **<5x slowdown**

**Kill criteria:**
- >1ms for 50 host calls → spike FAILS

---

## Spike B Gate

**GitHub Issue:** #19

**Depends on:** B8, B9

**Files:**
- Create: `benchmarks/spike_b.json`
- Modify: `NOMAI_MVP_PLAN.md` (update status)

### Gate Criteria

1. **WASM host calls <1ms for 50 calls** (from B9 benchmark)
2. **Causality unbroken across WASM boundary** (from B5 + B8)
3. **Verification engine detects bugs from manifest alone** (from B8)
4. **All tests pass** (from B8 integration)

### Gate Document Format

```json
{
  "spike": "B",
  "description": "WASM Gameplay + Verification feasibility spike",
  "date": "2026-02-XX",
  "decision": "PASS|FAIL",
  "benchmarks": {
    "wasm_50_host_calls": {
      "median_us": 0,
      "acceptance_us": 1000,
      "kill_us": 1000
    },
    "native_equivalent": {
      "median_us": 0
    },
    "wasm_vs_native_ratio": 0
  },
  "acceptance_criteria": {
    "wasm_overhead_under_1ms": { "result": "PASS|FAIL" },
    "causality_unbroken": { "result": "PASS|FAIL" },
    "verification_detects_bugs": { "result": "PASS|FAIL" }
  },
  "test_counts": {
    "rust_total": 0,
    "python_total": 0,
    "all_passing": true
  }
}
```

---

## Execution Summary

| Task | Agent | Est. Complexity | Parallel Group |
|------|-------|----------------|----------------|
| B1 | wasm-sandbox | Medium | A |
| B2 | wasm-sandbox | High | A (after B1) |
| B3 | wasm-sandbox | Low | A (after B2) |
| B4 | wasm-sandbox | Medium | C (after B1) |
| B5 | wasm-sandbox | Medium | A (after B2) |
| B6 | python-verification | Medium | B |
| B7 | python-verification | High | B (after B6) |
| B8 | orchestrator | Medium | D (after B5+B7) |
| B9 | spike-validator | Medium | D (after B2+B4) |
| Gate | orchestrator | Low | After all |

**Parallel execution strategy:**
- Start B1 (wasm-sandbox) and B6 (python-verification) simultaneously
- After B1: start B2 and B4 in parallel
- After B2: start B3 and B5 in parallel
- After B6: start B7
- After B5 + B7: start B8
- After B2 + B4: start B9
- After B8 + B9: Gate decision
