# Phase 1, Week 3-4: Production Manifest Pipeline + Python Bindings

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Integrate the manifest pipeline into the tick loop so every tick produces a queryable `TickManifest`, then build PyO3 bindings so Python can fully control the engine, query manifests, and run the AI verification loop.

**Architecture:** Three tracks converge at the milestone test:
1. **Rust manifest integration** — Wire `ManifestPipeline` into `TickLoop::tick()` so manifests are produced automatically
2. **PyO3 bindings** — New `nomai-python` crate exposing engine control + manifest queries to Python
3. **Python SDK** — `engine.py` wrapper + SDK restructuring for the PyO3 native module

**Tech Stack:** Rust 1.83.0, PyO3 0.23.3, maturin 1.x, Python 3.12+, pytest, pyright strict

---

## Dependency Graph

```
Task 1 (Manifest Integration into Tick Loop)
  │
  ├── Task 2 (Manifest Hardening + Tests)
  │     │
  │     └────────────┐
  │                  │
  Task 3 (PyO3 Crate Setup + Engine Bindings)
        │
        ├── Task 4 (PyO3 Manifest Bindings)
        │     │
        │     └── Task 5 (Python SDK engine.py Wrapper)
        │           │
        │           └── Task 6 (Milestone Test: Python → Engine → Manifest)
        │
        └───────────────────────────────────────────────────────┘

Execution order:
  Task 1 → Task 2 → Task 3 → Task 4 → Task 5 → Task 6
```

## Agents

| Task | Agent | Files |
|------|-------|-------|
| Task 1 | `rust-engine` | `crates/nomai-engine/src/tick.rs` |
| Task 2 | `manifest-pipeline` | `crates/nomai-manifest/src/manifest.rs`, `crates/nomai-manifest/src/journal.rs` |
| Task 3 | `python-verification` | `crates/nomai-python/` (new), `Cargo.toml` |
| Task 4 | `python-verification` | `crates/nomai-python/src/manifest.rs` |
| Task 5 | `python-verification` | `python/nomai-sdk/nomai/engine.py` |
| Task 6 | `python-verification` | `python/nomai-sdk/tests/test_milestone_week3_4.py` |

---

## Task 1: Integrate ManifestPipeline into TickLoop (Issue #25, Part 1)

**GitHub Issue:** #25

**Files:**
- Modify: `crates/nomai-engine/src/tick.rs`
- Modify: `crates/nomai-engine/src/lib.rs` (re-export ManifestPipeline types)

### What to Build

The tick loop currently returns `Vec<Command>` from `tick()` but does NOT feed those commands into the manifest pipeline. We need to:

1. Add a `ManifestPipeline` field to `TickLoop`
2. Wire `tick()` to call `begin_tick()` → `process_commands()` → `end_tick()` automatically
3. Make `tick()` return `&TickManifest` (or store it for later query)
4. Add manifest accessor methods to `TickLoop`

### Step 1: Add ManifestPipeline to TickLoop

In `crates/nomai-engine/src/tick.rs`, add the import and field:

```rust
use nomai_manifest::manifest::{ManifestPipeline, TickManifest, GameEvent};
```

Add to the `TickLoop` struct:

```rust
pub struct TickLoop {
    // ... existing fields ...
    /// Manifest pipeline producing structured state snapshots per tick.
    manifest: ManifestPipeline,
}
```

Initialize in `TickLoop::new()`:

```rust
pub fn new(world: World, config: TickConfig) -> Self {
    Self {
        // ... existing fields ...
        manifest: ManifestPipeline::new(),
    }
}
```

### Step 2: Wire tick() to produce manifests

Modify `tick()` to integrate manifest generation after command application. The current `tick()` flow is:

```
1. Run systems (each writes to command buffer)
2. Apply command buffer → Vec<Command>
3. Advance tick counter + sim time
4. Return applied commands
```

Change it to:

```
1. manifest.begin_tick()
2. Run systems (each writes to command buffer)
3. Apply command buffer → Vec<Command>
4. manifest.process_commands(&applied, tick, &world)
5. manifest.end_tick(tick, sim_time, system_names, &world) → TickManifest
6. Advance tick counter + sim time
7. Return applied commands (still returned for backward compat)
```

Modify `tick()`:

```rust
pub fn tick(&mut self) -> Vec<nomai_ecs::command::Command> {
    let tick_start = std::time::Instant::now();
    let mut system_times = Vec::with_capacity(self.systems.len());

    // Phase 0: Begin manifest tick.
    self.manifest.begin_tick();

    // Phase 1: Run systems with timing.
    let system_names: Vec<String> = self.systems.iter().map(|s| s.name.clone()).collect();
    for system in &self.systems {
        let sys_start = std::time::Instant::now();
        (system.func)(&self.world, &mut self.command_buffer);
        system_times.push((system.name.clone(), sys_start.elapsed()));
    }

    // Phase 2: Apply commands with timing.
    let apply_start = std::time::Instant::now();
    let applied = self.command_buffer.apply(&mut self.world);
    let command_apply_time = apply_start.elapsed();

    // Phase 3: Feed commands into manifest pipeline.
    self.manifest.process_commands(&applied, self.tick_counter, &self.world);

    // Phase 4: End manifest tick (produces TickManifest, stored in history).
    self.manifest.end_tick(
        self.tick_counter,
        self.sim_time,
        system_names,
        &self.world,
    );

    // Phase 5: Advance tick counter and simulation time.
    self.tick_counter += 1;
    self.sim_time += self.config.fixed_dt;

    // Phase 6: Record diagnostics.
    self.last_diagnostics = TickDiagnostics {
        system_times,
        total_time: tick_start.elapsed(),
        command_apply_time,
    };

    applied
}
```

### Step 3: Add manifest accessor methods

```rust
/// Access the manifest pipeline.
pub fn manifest(&self) -> &ManifestPipeline {
    &self.manifest
}

/// Access the manifest pipeline mutably (for recording events).
pub fn manifest_mut(&mut self) -> &mut ManifestPipeline {
    &mut self.manifest
}

/// Get the manifest for the most recent tick.
pub fn last_manifest(&self) -> Option<&TickManifest> {
    if self.tick_counter == 0 {
        return None;
    }
    self.manifest.manifest_at_tick(self.tick_counter - 1)
}

/// Get the manifest for a specific tick (within the history window).
pub fn manifest_at_tick(&self, tick: u64) -> Option<&TickManifest> {
    self.manifest.manifest_at_tick(tick)
}
```

### Step 4: Update lib.rs prelude

Add manifest types to the engine prelude in `crates/nomai-engine/src/lib.rs`:

```rust
pub mod prelude {
    pub use nomai_ecs::prelude::*;
    pub use crate::tick::{InputFrame, SystemFn, TickConfig, TickDiagnostics, TickLoop};
    // Manifest types re-exported for convenience.
    pub use nomai_manifest::manifest::{
        Aggregates, CausalChain, CausalStep, EntityEntry, GameEvent, ManifestPipeline, TickManifest,
    };
    pub use nomai_manifest::journal::{ChangeJournal, ComponentChange};
}
```

Also re-export the manifest crate:

```rust
pub use nomai_manifest;
```

### Step 5: Write tests

Add tests in tick.rs `mod tests`:

```rust
#[test]
fn tick_produces_manifest() {
    let mut world = setup_world();
    world.spawn_with(Counter(0));
    let mut tick_loop = TickLoop::new(world, TickConfig::default());
    tick_loop.add_system("counter", counter_system);

    tick_loop.tick();

    let manifest = tick_loop.last_manifest().expect("should have manifest after tick");
    assert_eq!(manifest.tick, 0);
    assert!(manifest.commands_processed > 0);
    assert_eq!(manifest.systems_executed, vec!["counter"]);
}

#[test]
fn manifest_history_accessible() {
    let mut world = setup_world();
    world.spawn_with(Counter(0));
    let mut tick_loop = TickLoop::new(world, TickConfig::default());
    tick_loop.add_system("counter", counter_system);

    tick_loop.run_ticks(10);

    // All 10 manifests should be accessible.
    for tick in 0..10 {
        let manifest = tick_loop.manifest_at_tick(tick);
        assert!(manifest.is_some(), "manifest at tick {tick} missing");
        assert_eq!(manifest.unwrap().tick, tick);
    }
}

#[test]
fn manifest_records_spawns_and_despawns() {
    let mut world = setup_world();
    let mut tick_loop = TickLoop::new(world, TickConfig::default());

    // System that spawns an entity on tick 0.
    tick_loop.add_system("spawner", |_world, cmds| {
        cmds.spawn_semantic(
            EntityIdentity {
                entity_type: "test".to_owned(),
                role: "unit".to_owned(),
                spawned_by: SystemId(0),
                requirement_id: None,
            },
            vec![],
            SystemId(0),
            CausalReason::GameRule("test_spawn".to_owned()),
        );
    });

    tick_loop.tick();
    let manifest = tick_loop.last_manifest().unwrap();
    assert!(!manifest.entity_spawns.is_empty());
}
```

### Step 6: Run tests

Run: `cargo test -p nomai-engine`
Expected: All existing tests pass + 3 new manifest integration tests.

Also: `cargo test --workspace` to check no cross-crate regressions.

### Step 7: Commit

```
feat: integrate ManifestPipeline into TickLoop — every tick produces manifest (#25)

- ManifestPipeline wired into tick(): begin_tick → process_commands → end_tick
- manifest(), last_manifest(), manifest_at_tick() accessors on TickLoop
- Manifest types re-exported in engine prelude
- 3 integration tests for manifest production
```

---

## Task 2: Manifest Pipeline Hardening + Production Tests (Issue #25, Part 2)

**GitHub Issue:** #25

**Files:**
- Modify: `crates/nomai-manifest/src/manifest.rs` (harden edge cases)
- Tests in: `crates/nomai-manifest/src/manifest.rs` (mod tests)

### What to Build

1. **Delta mode stub**: Add `changes_since_tick()` method that returns only manifests from the given tick onward (simple filter over history). Not a full diff — just the history slice.

2. **Custom aggregate registration**: Allow users to register named aggregate functions beyond the built-in count-by-tier/type. These run at `end_tick()` and their results appear in `Aggregates`.

3. **Edge case tests**:
   - Empty ticks (no commands) produce valid manifests
   - Manifests outside the history window return `None`
   - Causal chains handle missing predecessor ticks gracefully
   - Entity index correctly tracks re-use (despawn + respawn at same index)

4. **Performance re-validation**: Run the A8 benchmark to confirm manifest overhead is still <5% of frame budget after Week 1-2 changes.

### Step 1: Add changes_since_tick()

In `manifest.rs`, add to `ManifestPipeline`:

```rust
/// Return all manifests from the given tick onward (inclusive).
///
/// Returns `None` if the start tick is outside the history window.
/// This is the "delta mode" — query only what changed since your last read.
pub fn manifests_since(&self, since_tick: u64) -> Vec<&TickManifest> {
    self.history
        .iter()
        .filter(|m| m.tick >= since_tick)
        .collect()
}
```

### Step 2: Add custom aggregate registration

Add a custom aggregates map to `Aggregates`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aggregates {
    pub entity_count_by_tier: HashMap<String, usize>,
    pub entity_count_by_type: HashMap<String, usize>,
    pub total_entity_count: usize,
    /// Custom aggregates computed by user-registered functions.
    #[serde(default)]
    pub custom: HashMap<String, f64>,
}
```

Add a custom aggregate registry to `ManifestPipeline`:

```rust
/// Type alias for custom aggregate function: takes &World, returns f64.
type AggregateFn = Box<dyn Fn(&World) -> f64 + Send>;

pub struct ManifestPipeline {
    // ... existing fields ...
    /// User-registered custom aggregate functions.
    custom_aggregates: Vec<(String, AggregateFn)>,
}
```

Add registration method:

```rust
/// Register a named custom aggregate that runs at the end of each tick.
///
/// The function receives an immutable reference to the world and should
/// return a numeric value. The result is stored in
/// `Aggregates::custom[name]`.
pub fn register_aggregate<F>(&mut self, name: &str, func: F)
where
    F: Fn(&World) -> f64 + Send + 'static,
{
    self.custom_aggregates.push((name.to_owned(), Box::new(func)));
}
```

In `end_tick()`, compute custom aggregates during the aggregates-building phase:

```rust
let mut custom = HashMap::new();
for (name, func) in &self.custom_aggregates {
    custom.insert(name.clone(), func(world));
}
aggregates.custom = custom;
```

### Step 3: Write edge case tests

```rust
#[test]
fn empty_tick_produces_valid_manifest() {
    let mut world = World::new();
    world.register_component::<Position>("position");
    let mut pipeline = ManifestPipeline::new();

    pipeline.begin_tick();
    let commands: Vec<Command> = Vec::new();
    pipeline.process_commands(&commands, 0, &world);
    let manifest = pipeline.end_tick(0, 0.0, vec![], &world);

    assert_eq!(manifest.tick, 0);
    assert!(manifest.entity_spawns.is_empty());
    assert!(manifest.entity_despawns.is_empty());
    assert!(manifest.component_changes.is_empty());
    assert_eq!(manifest.commands_processed, 0);
}

#[test]
fn manifest_outside_window_returns_none() {
    let mut pipeline = ManifestPipeline::with_max_history(5);
    let mut world = World::new();
    world.register_component::<Position>("position");

    // Run 10 ticks to push early ones out of window.
    for tick in 0..10u64 {
        pipeline.begin_tick();
        pipeline.process_commands(&[], tick, &world);
        pipeline.end_tick(tick, tick as f64 * 0.016, vec![], &world);
    }

    // Tick 0-4 should be gone, 5-9 should exist.
    assert!(pipeline.manifest_at_tick(0).is_none());
    assert!(pipeline.manifest_at_tick(4).is_none());
    assert!(pipeline.manifest_at_tick(5).is_some());
    assert!(pipeline.manifest_at_tick(9).is_some());
}

#[test]
fn manifests_since_returns_delta() {
    let mut pipeline = ManifestPipeline::new();
    let mut world = World::new();
    world.register_component::<Position>("position");

    for tick in 0..10u64 {
        pipeline.begin_tick();
        pipeline.process_commands(&[], tick, &world);
        pipeline.end_tick(tick, tick as f64 * 0.016, vec![], &world);
    }

    let since_7 = pipeline.manifests_since(7);
    assert_eq!(since_7.len(), 3); // ticks 7, 8, 9
    assert_eq!(since_7[0].tick, 7);
}

#[test]
fn custom_aggregate_runs_at_end_tick() {
    let mut world = World::new();
    world.register_component::<Position>("position");
    for _ in 0..5 {
        world.spawn_with(Position { x: 0.0, y: 0.0 });
    }

    let mut pipeline = ManifestPipeline::new();
    pipeline.register_aggregate("entity_count_f64", |w| w.entity_count() as f64);

    pipeline.begin_tick();
    pipeline.process_commands(&[], 0, &world);
    let manifest = pipeline.end_tick(0, 0.0, vec![], &world);

    assert_eq!(manifest.aggregates.custom.get("entity_count_f64"), Some(&5.0));
}
```

### Step 4: Run tests

Run: `cargo test -p nomai-manifest`
Expected: All 32 existing tests pass + 4 new tests.

Run: `cargo test --workspace`

### Step 5: Commit

```
feat: manifest delta mode, custom aggregates, and edge case tests (#25)

- manifests_since(tick) for delta queries over history window
- register_aggregate() for user-defined per-tick aggregate functions
- Aggregates.custom HashMap for custom aggregate results
- Edge case tests: empty ticks, window expiry, delta queries
```

---

## Task 3: PyO3 Crate Setup + Engine Bindings (Issue #26)

**GitHub Issue:** #26

**Files:**
- Create: `crates/nomai-python/Cargo.toml`
- Create: `crates/nomai-python/src/lib.rs`
- Create: `crates/nomai-python/src/engine.rs`
- Modify: `Cargo.toml` (workspace root — add member + pyo3 dep)
- Create: `crates/nomai-python/pyproject.toml` (maturin config)

### What to Build

A new Rust crate that uses PyO3 to expose a `NomaiEngine` Python class. This class wraps the Rust `TickLoop` and provides Python methods for:

- Engine lifecycle: `new()`, `register_component()`, `add_system()`
- Simulation: `tick()`, `run_ticks(n)`
- World state: `spawn_entity()`, `despawn_entity()`, `set_component()`
- WASM: `load_gameplay_wasm()`
- Manifest: `last_manifest()`, `manifest_at_tick(tick)`, `manifest_history()`

The bindings return manifest data as Python dicts (JSON round-trip via serde_json → Python dict). This approach is simpler than wrapping every Rust struct as a PyO3 class, and the Python SDK already has `TickManifest.from_dict()` to reconstruct typed dataclasses from dicts.

### Step 1: Add pyo3 to workspace dependencies

In root `Cargo.toml`, add:

```toml
[workspace.dependencies]
pyo3 = { version = "0.23.3", features = ["extension-module"] }
```

Add new member:

```toml
[workspace]
members = [
    "crates/nomai-ecs",
    "crates/nomai-manifest",
    "crates/nomai-engine",
    "crates/nomai-wasm-host",
    "crates/nomai-python",
]
```

### Step 2: Create crate structure

`crates/nomai-python/Cargo.toml`:

```toml
[package]
name = "nomai-python"
version = "0.1.0"
edition.workspace = true

[lib]
name = "_engine"
crate-type = ["cdylib"]

[dependencies]
nomai-ecs = { path = "../nomai-ecs" }
nomai-engine = { path = "../nomai-engine" }
nomai-manifest = { path = "../nomai-manifest" }
nomai-wasm-host = { path = "../nomai-wasm-host" }
pyo3 = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
```

`crates/nomai-python/pyproject.toml`:

```toml
[build-system]
requires = ["maturin>=1.0,<2.0"]
build-backend = "maturin"

[project]
name = "nomai-engine"
version = "0.1.0"
description = "Nomai Engine Python bindings"
requires-python = ">=3.12"

[tool.maturin]
features = ["pyo3/extension-module"]
module-name = "nomai._engine"
```

### Step 3: Create lib.rs module entry

`crates/nomai-python/src/lib.rs`:

```rust
//! PyO3 Python bindings for the Nomai Engine.
//!
//! Exposes the engine's tick loop, manifest pipeline, and world manipulation
//! to Python. Manifest data is passed as Python dicts (JSON round-trip) for
//! compatibility with the existing `nomai-sdk` Python dataclasses.

use pyo3::prelude::*;

mod engine;

/// The `nomai._engine` native module.
#[pymodule]
fn _engine(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<engine::PyNomaiEngine>()?;
    Ok(())
}
```

### Step 4: Create engine.rs with PyNomaiEngine

`crates/nomai-python/src/engine.rs`:

```rust
//! Python-facing engine wrapper.

use nomai_ecs::prelude::*;
use nomai_engine::tick::{TickConfig, TickLoop};
use nomai_manifest::manifest::TickManifest;
use nomai_wasm_host::module::{WasmConfig, WasmModule};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Converts a TickManifest to a Python dict via JSON round-trip.
fn manifest_to_pyobject(py: Python<'_>, manifest: &TickManifest) -> PyResult<PyObject> {
    let json_str = serde_json::to_string(manifest)
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("JSON serialize: {e}")))?;
    let json_mod = py.import("json")?;
    let dict = json_mod.call_method1("loads", (json_str,))?;
    Ok(dict.into_pyobject(py)?.into_any().unbind())
}

/// The main Nomai Engine exposed to Python.
///
/// Usage from Python:
/// ```python
/// from nomai._engine import NomaiEngine
/// engine = NomaiEngine()
/// engine.register_component("position")
/// engine.register_component("velocity")
/// manifest = engine.tick()
/// ```
#[pyclass]
pub struct PyNomaiEngine {
    tick_loop: TickLoop,
    wasm_module: Option<WasmModule>,
}

#[pymethods]
impl PyNomaiEngine {
    /// Create a new engine instance.
    ///
    /// Args:
    ///     headless: Run without rendering (default True).
    ///     fixed_dt: Fixed timestep in seconds (default 1/60).
    #[new]
    #[pyo3(signature = (headless=true, fixed_dt=None))]
    fn new(headless: bool, fixed_dt: Option<f64>) -> PyResult<Self> {
        let world = World::new();
        let config = TickConfig {
            fixed_dt: fixed_dt.unwrap_or(1.0 / 60.0),
            headless,
        };
        Ok(Self {
            tick_loop: TickLoop::new(world, config),
            wasm_module: None,
        })
    }

    /// Register a component type by name.
    ///
    /// Components are stored as JSON values internally. Registration
    /// makes the name known to the ECS so commands can reference it.
    fn register_component(&mut self, name: &str) -> PyResult<()> {
        // Register as serde_json::Value — Python side sends JSON.
        self.tick_loop
            .world_mut()
            .register_component::<serde_json::Value>(name);
        Ok(())
    }

    /// Run one tick and return the manifest as a Python dict.
    fn tick(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        self.tick_loop.tick();
        let manifest = self.tick_loop.last_manifest().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err("no manifest after tick")
        })?;
        manifest_to_pyobject(py, manifest)
    }

    /// Run N ticks and return a list of manifest dicts.
    fn run_ticks(&mut self, py: Python<'_>, n: u64) -> PyResult<Vec<PyObject>> {
        let mut manifests = Vec::with_capacity(n as usize);
        for _ in 0..n {
            self.tick_loop.tick();
            if let Some(m) = self.tick_loop.last_manifest() {
                manifests.push(manifest_to_pyobject(py, m)?);
            }
        }
        Ok(manifests)
    }

    /// Get manifest at a specific tick (within history window).
    fn manifest_at_tick(&self, py: Python<'_>, tick: u64) -> PyResult<Option<PyObject>> {
        match self.tick_loop.manifest_at_tick(tick) {
            Some(m) => Ok(Some(manifest_to_pyobject(py, m)?)),
            None => Ok(None),
        }
    }

    /// Get all manifests in the history window as a list of dicts.
    fn manifest_history(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        let history = self.tick_loop.manifest().history();
        let mut result = Vec::with_capacity(history.len());
        for m in history {
            result.push(manifest_to_pyobject(py, m)?);
        }
        Ok(result)
    }

    /// Current tick count.
    fn tick_count(&self) -> u64 {
        self.tick_loop.tick_count()
    }

    /// Current simulation time.
    fn sim_time(&self) -> f64 {
        self.tick_loop.sim_time()
    }

    /// Spawn a semantic entity via the command buffer.
    ///
    /// Args:
    ///     entity_type: The entity type string.
    ///     role: The entity role string.
    ///     components: Dict of component_name -> JSON-serializable value.
    ///
    /// Returns: None (entity is spawned on next tick).
    fn spawn_entity(
        &mut self,
        entity_type: &str,
        role: &str,
        components: &Bound<'_, PyDict>,
        py: Python<'_>,
    ) -> PyResult<()> {
        let mut comp_vec: Vec<(String, serde_json::Value)> = Vec::new();
        for (key, value) in components.iter() {
            let name: String = key.extract()?;
            let json_str = py.import("json")?.call_method1("dumps", (value,))?;
            let json_val: serde_json::Value = serde_json::from_str(&json_str.extract::<String>()?)
                .map_err(|e| {
                    pyo3::exceptions::PyValueError::new_err(format!("invalid JSON: {e}"))
                })?;
            comp_vec.push((name, json_val));
        }

        self.tick_loop.command_buffer_mut().spawn_semantic(
            EntityIdentity {
                entity_type: entity_type.to_owned(),
                role: role.to_owned(),
                spawned_by: SystemId(0),
                requirement_id: None,
            },
            comp_vec,
            SystemId(0),
            CausalReason::SystemInternal("python_spawn".to_owned()),
        );
        Ok(())
    }

    /// Despawn an entity via the command buffer.
    fn despawn_entity(&mut self, entity_id: u64) -> PyResult<()> {
        let eid = EntityId::from_raw(entity_id);
        self.tick_loop.command_buffer_mut().despawn(
            eid,
            SystemId(0),
            CausalReason::SystemInternal("python_despawn".to_owned()),
        );
        Ok(())
    }

    /// Set a component value via the command buffer.
    ///
    /// The value should be a JSON-serializable Python object.
    fn set_component(
        &mut self,
        entity_id: u64,
        component_name: &str,
        value: &Bound<'_, pyo3::PyAny>,
        py: Python<'_>,
    ) -> PyResult<()> {
        let json_str = py.import("json")?.call_method1("dumps", (value,))?;
        let json_val: serde_json::Value = serde_json::from_str(&json_str.extract::<String>()?)
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("invalid JSON: {e}")))?;

        let eid = EntityId::from_raw(entity_id);
        self.tick_loop.command_buffer_mut().set_component(
            eid,
            component_name,
            json_val,
            SystemId(0),
            CausalReason::SystemInternal("python_set".to_owned()),
        );
        Ok(())
    }

    /// Load a WASM gameplay module from bytes.
    fn load_gameplay_wasm(&mut self, wasm_bytes: Vec<u8>) -> PyResult<()> {
        let config = WasmConfig::default();
        let module = WasmModule::from_bytes(&config, &wasm_bytes).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!("WASM load failed: {e}"))
        })?;
        self.wasm_module = Some(module);
        Ok(())
    }

    /// Hot-swap the current WASM gameplay module.
    fn hot_swap_gameplay_wasm(&mut self, wasm_bytes: Vec<u8>) -> PyResult<()> {
        match &mut self.wasm_module {
            Some(module) => {
                module.swap(&wasm_bytes).map_err(|e| {
                    pyo3::exceptions::PyRuntimeError::new_err(format!("WASM swap failed: {e}"))
                })?;
                Ok(())
            }
            None => Err(pyo3::exceptions::PyRuntimeError::new_err(
                "no WASM module loaded — call load_gameplay_wasm() first",
            )),
        }
    }

    /// Get the entity count in the world.
    fn entity_count(&self) -> usize {
        self.tick_loop.world().entity_count()
    }
}
```

### Step 5: Add command_buffer_mut() to TickLoop

The PyO3 engine needs to push commands externally (from Python). Add to `tick.rs`:

```rust
/// Mutable access to the command buffer for external command injection.
///
/// Used by Python bindings to spawn/despawn/set components before the
/// next tick's apply phase.
pub fn command_buffer_mut(&mut self) -> &mut CommandBuffer {
    &mut self.command_buffer
}
```

Also need `EntityId::from_raw()` — check if it exists, add if not:

```rust
// In entity.rs
pub fn from_raw(raw: u64) -> Self {
    Self {
        index: (raw & 0xFFFF_FFFF) as u32,
        generation: (raw >> 32) as u32,
    }
}
```

### Step 6: Build test

Run: `cargo build -p nomai-python`
Expected: Compiles successfully (cdylib target).

Run: `cargo test --workspace` (nomai-python won't have Rust tests yet, but workspace should still pass).

Run: `cargo clippy --workspace -- -D warnings`

### Step 7: Commit

```
feat: PyO3 crate with NomaiEngine Python bindings (#26)

- New crate: nomai-python with PyO3 + maturin configuration
- NomaiEngine class: tick(), run_ticks(), spawn/despawn/set_component
- Manifest queries: last_manifest(), manifest_at_tick(), manifest_history()
- WASM loading: load_gameplay_wasm(), hot_swap_gameplay_wasm()
- Manifest data returned as Python dicts (JSON round-trip)
```

---

## Task 4: PyO3 Manifest Bindings — Entity Index + Causal Chains (Issue #26, Part 2)

**GitHub Issue:** #26

**Files:**
- Create: `crates/nomai-python/src/manifest.rs`
- Modify: `crates/nomai-python/src/lib.rs` (add module)
- Modify: `crates/nomai-python/src/engine.rs` (add entity_index, trace_causality methods)

### What to Build

Add methods to `PyNomaiEngine` for:

1. **Entity index queries**: `entity_index()` returns all tracked entities
2. **Causal chain tracing**: `trace_causality(entity_id, component, tick)` builds and returns a causal chain
3. **Entity details**: `get_entity(entity_id)` returns entity entry from the index

### Step 1: Add entity index and causality methods to engine.rs

```rust
/// Get the full entity index as a list of dicts.
fn entity_index(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
    let index = self.tick_loop.manifest().entity_index();
    let mut result = Vec::new();
    for entry in index.values() {
        let json_str = serde_json::to_string(entry).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!("JSON serialize: {e}"))
        })?;
        let json_mod = py.import("json")?;
        let dict = json_mod.call_method1("loads", (json_str,))?;
        result.push(dict.into_pyobject(py)?.into_any().unbind());
    }
    Ok(result)
}

/// Get a single entity's index entry.
fn get_entity(&self, py: Python<'_>, entity_id: u64) -> PyResult<Option<PyObject>> {
    let eid = EntityId::from_raw(entity_id);
    let index = self.tick_loop.manifest().entity_index();
    match index.get(&eid) {
        Some(entry) => {
            let json_str = serde_json::to_string(entry).map_err(|e| {
                pyo3::exceptions::PyRuntimeError::new_err(format!("JSON: {e}"))
            })?;
            let json_mod = py.import("json")?;
            let dict = json_mod.call_method1("loads", (json_str,))?;
            Ok(Some(dict.into_pyobject(py)?.into_any().unbind()))
        }
        None => Ok(None),
    }
}

/// Trace the causal chain for a component change.
///
/// Finds the most recent change to the specified component on the
/// specified entity at the given tick, then walks the causal chain
/// backwards through the manifest history.
fn trace_causality(
    &self,
    py: Python<'_>,
    entity_id: u64,
    component: &str,
    tick: u64,
) -> PyResult<Option<PyObject>> {
    let eid = EntityId::from_raw(entity_id);
    let manifest = match self.tick_loop.manifest_at_tick(tick) {
        Some(m) => m,
        None => return Ok(None),
    };

    // Find the matching change.
    let change = manifest
        .component_changes
        .iter()
        .find(|c| c.entity_id == eid && c.component_type_name == component);

    match change {
        Some(c) => {
            let chain = self.tick_loop.manifest().build_causal_chain(c);
            let json_str = serde_json::to_string(&chain).map_err(|e| {
                pyo3::exceptions::PyRuntimeError::new_err(format!("JSON: {e}"))
            })?;
            let json_mod = py.import("json")?;
            let dict = json_mod.call_method1("loads", (json_str,))?;
            Ok(Some(dict.into_pyobject(py)?.into_any().unbind()))
        }
        None => Ok(None),
    }
}
```

### Step 2: Build and test

Run: `cargo build -p nomai-python`
Run: `cargo clippy --workspace -- -D warnings`

### Step 3: Commit

```
feat: PyO3 entity index and causal chain tracing (#26)

- entity_index() returns all tracked entities as Python dicts
- get_entity(id) returns single entity entry
- trace_causality(entity_id, component, tick) builds causal chain
```

---

## Task 5: Python SDK engine.py Wrapper (Issue #27)

**GitHub Issue:** #27

**Files:**
- Create: `python/nomai-sdk/nomai/engine.py`
- Modify: `python/nomai-sdk/nomai/__init__.py` (add engine import)
- Modify: `python/nomai-sdk/pyproject.toml` (add nomai-engine dependency)

### What to Build

A Python wrapper around the PyO3 `_engine` native module. This provides:
- High-level API with type annotations and docstrings
- Automatic conversion of dict results to `TickManifest` dataclasses
- `run_until()` with callable condition
- Graceful error if native module isn't installed

### Step 1: Create engine.py

`python/nomai-sdk/nomai/engine.py`:

```python
"""High-level Python wrapper around the Rust NomaiEngine via PyO3.

The native extension module ``nomai._engine`` provides the raw FFI layer.
This module wraps it with typed Python APIs that return proper dataclasses
instead of raw dicts.
"""

from __future__ import annotations

import logging
from typing import Any, Callable

from nomai.manifest import (
    CausalChain,
    EntityEntry,
    TickManifest,
)

logger = logging.getLogger(__name__)


def _get_native_engine() -> type:
    """Import the native engine, raising a clear error if unavailable."""
    try:
        from nomai._engine import NomaiEngine  # type: ignore[import-not-found]

        return NomaiEngine
    except ImportError as exc:
        raise RuntimeError(
            "Nomai native engine not available. "
            "Build with: cd crates/nomai-python && maturin develop --release"
        ) from exc


class NomaiEngine:
    """High-level wrapper around the Rust NomaiEngine.

    All manifest results are returned as typed Python dataclasses
    (``TickManifest``, ``EntityEntry``, ``CausalChain``).

    Usage::

        engine = NomaiEngine()
        engine.register_component("position")
        manifest = engine.tick()
        print(manifest.tick, manifest.commands_processed)
    """

    def __init__(
        self,
        *,
        headless: bool = True,
        fixed_dt: float | None = None,
    ) -> None:
        cls = _get_native_engine()
        self._engine = cls(headless=headless, fixed_dt=fixed_dt)

    # -- Simulation control --------------------------------------------------

    def register_component(self, name: str) -> None:
        """Register a component type by name."""
        self._engine.register_component(name)

    def tick(self) -> TickManifest:
        """Run one tick and return the manifest."""
        raw = self._engine.tick()
        return TickManifest.from_dict(raw)

    def run_ticks(self, n: int) -> list[TickManifest]:
        """Run N ticks and return all manifests."""
        raws = self._engine.run_ticks(n)
        return [TickManifest.from_dict(r) for r in raws]

    def run_until(
        self,
        condition: Callable[[TickManifest], bool],
        max_ticks: int = 10_000,
    ) -> list[TickManifest]:
        """Run ticks until condition returns True or max_ticks reached."""
        manifests: list[TickManifest] = []
        for _ in range(max_ticks):
            m = self.tick()
            manifests.append(m)
            if condition(m):
                break
        return manifests

    # -- Manifest queries ----------------------------------------------------

    def manifest_at_tick(self, tick: int) -> TickManifest | None:
        """Get manifest at a specific tick (within history window)."""
        raw = self._engine.manifest_at_tick(tick)
        if raw is None:
            return None
        return TickManifest.from_dict(raw)

    def manifest_history(self) -> list[TickManifest]:
        """Get all manifests in the history window."""
        raws = self._engine.manifest_history()
        return [TickManifest.from_dict(r) for r in raws]

    def entity_index(self) -> list[EntityEntry]:
        """Get all tracked entities."""
        raws = self._engine.entity_index()
        return [EntityEntry.from_dict(r) for r in raws]

    def get_entity(self, entity_id: int) -> EntityEntry | None:
        """Get a single entity's index entry."""
        raw = self._engine.get_entity(entity_id)
        if raw is None:
            return None
        return EntityEntry.from_dict(raw)

    def trace_causality(
        self,
        entity_id: int,
        component: str,
        tick: int,
    ) -> CausalChain | None:
        """Trace the causal chain for a component change."""
        raw = self._engine.trace_causality(entity_id, component, tick)
        if raw is None:
            return None
        return CausalChain.from_dict(raw)

    # -- World manipulation --------------------------------------------------

    def spawn_entity(
        self,
        entity_type: str,
        role: str,
        components: dict[str, Any] | None = None,
    ) -> None:
        """Queue a semantic entity spawn (applied on next tick)."""
        self._engine.spawn_entity(
            entity_type, role, components or {}
        )

    def despawn_entity(self, entity_id: int) -> None:
        """Queue an entity despawn (applied on next tick)."""
        self._engine.despawn_entity(entity_id)

    def set_component(
        self,
        entity_id: int,
        component: str,
        value: Any,
    ) -> None:
        """Queue a component value change (applied on next tick)."""
        self._engine.set_component(entity_id, component, value)

    # -- WASM ----------------------------------------------------------------

    def load_gameplay_wasm(self, wasm_bytes: bytes) -> None:
        """Load a WASM gameplay module."""
        self._engine.load_gameplay_wasm(wasm_bytes)

    def hot_swap_gameplay_wasm(self, wasm_bytes: bytes) -> None:
        """Hot-swap the current WASM gameplay module."""
        self._engine.hot_swap_gameplay_wasm(wasm_bytes)

    # -- Info ----------------------------------------------------------------

    @property
    def tick_count(self) -> int:
        """Current tick count."""
        return self._engine.tick_count()

    @property
    def sim_time(self) -> float:
        """Current simulation time."""
        return self._engine.sim_time()

    @property
    def entity_count(self) -> int:
        """Current entity count in the world."""
        return self._engine.entity_count()
```

### Step 2: Update __init__.py

```python
"""Nomai SDK -- Python interface for the Nomai Engine.

Provides intent spec DSL, manifest data types, verification engine,
and engine control for AI-driven game development.
"""

__version__ = "0.1.0"

# Re-export key types for convenience.
from nomai.manifest import (
    Aggregates,
    CausalChain,
    CausalStep,
    ComponentChange,
    EntityEntry,
    GameEvent,
    TickManifest,
)

__all__ = [
    "Aggregates",
    "CausalChain",
    "CausalStep",
    "ComponentChange",
    "EntityEntry",
    "GameEvent",
    "TickManifest",
]
```

### Step 3: Build the native module and test import

```bash
cd crates/nomai-python && maturin develop --release
cd ../../python/nomai-sdk && pip install -e .
python -c "from nomai.engine import NomaiEngine; e = NomaiEngine(); print('OK:', e.tick_count)"
```

### Step 4: Commit

```
feat: Python SDK engine.py wrapper with typed manifest API (#27)

- NomaiEngine wrapper: tick(), run_ticks(), run_until() returning TickManifest
- Entity manipulation: spawn_entity(), despawn_entity(), set_component()
- Manifest queries: manifest_at_tick(), entity_index(), trace_causality()
- WASM loading: load_gameplay_wasm(), hot_swap_gameplay_wasm()
- Updated __init__.py with manifest type re-exports
```

---

## Task 6: Week 3-4 Milestone Test (Issue #28)

**GitHub Issue:** #28

**Files:**
- Create: `python/nomai-sdk/tests/test_milestone_week3_4.py`

### What to Build

End-to-end Python test: spawn entities → run ticks → query manifest → verify causal chains. This validates the entire Python → Rust → Manifest pipeline.

### Step 1: Create milestone test

`python/nomai-sdk/tests/test_milestone_week3_4.py`:

```python
"""Week 3-4 Milestone Test: Python → Engine → Manifest end-to-end.

Validates that Python can:
1. Create an engine and register components
2. Spawn entities with components
3. Run ticks and receive typed TickManifest objects
4. Query entity index
5. Trace causal chains through manifest history
"""

import pytest
from nomai.engine import NomaiEngine
from nomai.manifest import TickManifest, EntityEntry, CausalChain


class TestMilestoneWeek34:
    """End-to-end integration tests for Python → Rust engine."""

    def test_engine_creates_and_ticks(self) -> None:
        """Engine can be created and ticked without error."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")
        engine.register_component("health")
        manifest = engine.tick()
        assert isinstance(manifest, TickManifest)
        assert manifest.tick == 0
        assert engine.tick_count == 1

    def test_spawn_entities_appear_in_manifest(self) -> None:
        """Entities spawned from Python appear in the next tick's manifest."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")
        engine.register_component("health")

        engine.spawn_entity("unit", "warrior", {
            "position": {"x": 10.0, "y": 20.0},
            "health": 100,
        })
        engine.spawn_entity("unit", "mage", {
            "position": {"x": 30.0, "y": 40.0},
            "health": 80,
        })

        manifest = engine.tick()
        assert len(manifest.entity_spawns) == 2
        assert engine.entity_count == 2

    def test_run_ticks_returns_manifests(self) -> None:
        """run_ticks returns correct number of typed manifests."""
        engine = NomaiEngine(headless=True)
        engine.register_component("counter")

        manifests = engine.run_ticks(10)
        assert len(manifests) == 10
        for i, m in enumerate(manifests):
            assert isinstance(m, TickManifest)
            assert m.tick == i

    def test_entity_index_tracks_spawns(self) -> None:
        """Entity index tracks spawned entities with identity info."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")

        engine.spawn_entity("projectile", "bullet", {
            "position": {"x": 0.0, "y": 0.0},
        })
        engine.tick()

        index = engine.entity_index()
        assert len(index) >= 1
        bullet = [e for e in index if e.entity_type == "projectile"]
        assert len(bullet) == 1
        assert bullet[0].role == "bullet"
        assert bullet[0].alive is True
        assert bullet[0].tier == "Semantic"

    def test_despawn_reflected_in_manifest(self) -> None:
        """Despawned entities appear in manifest despawns list."""
        engine = NomaiEngine(headless=True)
        engine.register_component("health")

        engine.spawn_entity("unit", "target", {"health": 50})
        manifest_1 = engine.tick()
        assert len(manifest_1.entity_spawns) == 1
        entity_id = manifest_1.entity_spawns[0]

        engine.despawn_entity(entity_id)
        manifest_2 = engine.tick()
        assert entity_id in manifest_2.entity_despawns

    def test_set_component_produces_change(self) -> None:
        """set_component produces a ComponentChange in the manifest."""
        engine = NomaiEngine(headless=True)
        engine.register_component("score")

        engine.spawn_entity("player", "hero", {"score": 0})
        engine.tick()  # Apply spawn

        # Get entity ID from index.
        index = engine.entity_index()
        hero = [e for e in index if e.role == "hero"][0]

        engine.set_component(hero.entity_id, "score", 100)
        manifest = engine.tick()  # Apply set_component

        score_changes = [
            c for c in manifest.component_changes
            if c.component_type_name == "score"
        ]
        assert len(score_changes) >= 1

    def test_manifest_history_available(self) -> None:
        """All manifests within history window are accessible."""
        engine = NomaiEngine(headless=True)
        engine.register_component("counter")

        engine.run_ticks(20)

        history = engine.manifest_history()
        assert len(history) == 20

        # Each tick is accessible by number.
        for tick_num in range(20):
            m = engine.manifest_at_tick(tick_num)
            assert m is not None
            assert m.tick == tick_num

    def test_causal_chain_traced_through_manifest(self) -> None:
        """Causal chains can be traced from Python through the manifest."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")

        engine.spawn_entity("unit", "mover", {
            "position": {"x": 0.0, "y": 0.0},
        })
        engine.tick()  # Apply spawn

        # Get entity ID.
        index = engine.entity_index()
        mover = [e for e in index if e.role == "mover"][0]

        # Move the entity.
        engine.set_component(mover.entity_id, "position", {"x": 5.0, "y": 10.0})
        engine.tick()  # Apply set

        # Trace causality.
        chain = engine.trace_causality(mover.entity_id, "position", 1)
        assert chain is not None
        assert isinstance(chain, CausalChain)
        assert len(chain.steps) >= 1

    def test_run_until_with_condition(self) -> None:
        """run_until stops when the condition is met."""
        engine = NomaiEngine(headless=True)
        engine.register_component("counter")

        manifests = engine.run_until(
            condition=lambda m: m.tick >= 5,
            max_ticks=100,
        )
        assert len(manifests) == 6  # ticks 0-5
        assert manifests[-1].tick == 5

    def test_100_ticks_end_to_end(self) -> None:
        """Full 100-tick simulation with spawns, mutations, and manifest queries."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")
        engine.register_component("health")
        engine.register_component("score")

        # Spawn 10 entities.
        for i in range(10):
            engine.spawn_entity("unit", f"soldier_{i}", {
                "position": {"x": float(i), "y": 0.0},
                "health": 100,
                "score": 0,
            })

        manifests = engine.run_ticks(100)
        assert len(manifests) == 100

        # Verify spawns appeared in first tick.
        assert len(manifests[0].entity_spawns) == 10

        # Verify entity index has all 10.
        index = engine.entity_index()
        soldiers = [e for e in index if e.entity_type == "unit"]
        assert len(soldiers) == 10

        # Verify manifests have correct tick numbers.
        for i, m in enumerate(manifests):
            assert m.tick == i

        # All manifests have valid aggregates.
        for m in manifests:
            assert m.aggregates.total_entity_count >= 0

        print(f"Milestone PASS: 100 ticks, {len(index)} entities tracked")
```

### Step 2: Run the milestone test

```bash
cd crates/nomai-python && maturin develop --release
cd ../../python/nomai-sdk && pip install -e .[dev]
pytest tests/test_milestone_week3_4.py -v
```

Expected: All 10 tests pass.

### Step 3: Commit

```
feat: Week 3-4 milestone test — Python → Engine → Manifest end-to-end (#28)

- 10 integration tests covering full Python → Rust → Manifest pipeline
- Spawn entities, run ticks, query manifests, trace causal chains from Python
- 100-tick end-to-end simulation with 10 entities
- Validates entity index, manifest history, despawns, component mutations
```

---

## Close GitHub Issues

After all tasks pass:

```bash
gh issue close 25 --comment "Production manifest pipeline: integrated into tick loop, delta queries, custom aggregates, edge case tests."
gh issue close 26 --comment "PyO3 bindings: NomaiEngine class with tick/spawn/despawn/set_component, manifest queries, entity index, causal chain tracing, WASM loading."
gh issue close 27 --comment "Python SDK: engine.py wrapper with typed TickManifest API, run_until(), manifest history, entity index."
gh issue close 28 --comment "Week 3-4 milestone PASS: Python controls engine, spawns entities, queries manifests with causal chains, 100-tick end-to-end."
```

---

## Execution Summary

| Task | Agent | Est. Complexity | Dependencies |
|------|-------|----------------|--------------|
| Task 1 (Manifest in Tick Loop) | rust-engine | Medium | None |
| Task 2 (Manifest Hardening) | manifest-pipeline | Medium | Task 1 |
| Task 3 (PyO3 Crate + Engine) | python-verification | High | Task 1 |
| Task 4 (PyO3 Manifest) | python-verification | Medium | Task 3 |
| Task 5 (Python engine.py) | python-verification | Medium | Task 4 |
| Task 6 (Milestone Test) | python-verification | Medium | Task 5 |

**Execution order:** Strictly sequential (Task 1 → 2 → 3 → 4 → 5 → 6) since each builds on the previous.
