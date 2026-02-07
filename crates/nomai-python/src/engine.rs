//! Python-facing engine wrapper.
//!
//! [`PyNomaiEngine`] is a `#[pyclass]` that wraps the Rust [`TickLoop`] and
//! exposes it to Python. Manifest data crosses the FFI boundary as Python
//! dicts via a JSON round-trip: Rust `TickManifest` -> `serde_json::to_string`
//! -> Python `json.loads` -> `dict`.

use nomai_ecs::command::CausalReason;
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::{EntityIdentity, SystemId};
use nomai_engine::tick::{TickConfig, TickLoop};
use nomai_manifest::manifest::TickManifest;
use nomai_wasm_host::{WasmConfig, WasmModule};
use pyo3::prelude::*;
use pyo3::types::PyDict;

/// Converts a [`TickManifest`] to a Python dict via JSON round-trip.
///
/// Serializes the manifest to a JSON string using `serde_json`, then
/// deserializes it into a Python dict using Python's `json.loads`.
fn manifest_to_pyobject(py: Python<'_>, manifest: &TickManifest) -> PyResult<PyObject> {
    let json_str = serde_json::to_string(manifest).map_err(|e| {
        pyo3::exceptions::PyRuntimeError::new_err(format!(
            "failed to serialize TickManifest to JSON: {e}"
        ))
    })?;
    let json_mod = py.import("json")?;
    let dict = json_mod.call_method1("loads", (json_str,))?;
    Ok(dict.unbind())
}

/// The main Nomai Engine exposed to Python.
///
/// Wraps the Rust [`TickLoop`] and provides methods for engine lifecycle,
/// simulation, world state manipulation, WASM loading, and manifest queries.
///
/// Usage from Python:
/// ```python
/// from nomai._engine import NomaiEngine
/// engine = NomaiEngine()
/// engine.register_component("position")
/// engine.register_component("velocity")
/// manifest = engine.tick()
/// ```
#[pyclass(name = "NomaiEngine", unsendable)]
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
        let dt = fixed_dt.unwrap_or(1.0 / 60.0);
        if dt <= 0.0 || !dt.is_finite() {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "fixed_dt must be positive and finite",
            ));
        }
        let world = nomai_ecs::world::World::new();
        let config = TickConfig {
            fixed_dt: dt,
            headless,
        };
        Ok(Self {
            tick_loop: TickLoop::new(world, config),
            wasm_module: None,
        })
    }

    /// Register a component type by name.
    ///
    /// Components are stored as JSON values internally. Each name gets
    /// a unique component type, so `register_component("position")` and
    /// `register_component("velocity")` create distinct component types.
    fn register_component(&mut self, name: &str) -> PyResult<()> {
        self.tick_loop
            .world_mut()
            .register_dynamic_component::<serde_json::Value>(name);
        Ok(())
    }

    /// Run one tick and return the manifest as a Python dict.
    fn tick(&mut self, py: Python<'_>) -> PyResult<PyObject> {
        self.tick_loop.tick();
        let manifest = self.tick_loop.last_manifest().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(
                "no manifest produced after tick -- this should not happen",
            )
        })?;
        manifest_to_pyobject(py, manifest)
    }

    /// Run N ticks and return a list of manifest dicts.
    ///
    /// Args:
    ///     n: Number of ticks to run (max 100,000).
    fn run_ticks(&mut self, py: Python<'_>, n: u64) -> PyResult<Vec<PyObject>> {
        if n > 100_000 {
            return Err(pyo3::exceptions::PyValueError::new_err(
                "run_ticks: n must be <= 100,000 to prevent excessive memory allocation",
            ));
        }
        let mut manifests = Vec::with_capacity(n as usize);
        for _ in 0..n {
            self.tick_loop.tick();
            let manifest = self.tick_loop.last_manifest().ok_or_else(|| {
                pyo3::exceptions::PyRuntimeError::new_err(
                    "no manifest produced after tick -- this should not happen",
                )
            })?;
            manifests.push(manifest_to_pyobject(py, manifest)?);
        }
        Ok(manifests)
    }

    /// Get the manifest for the most recent tick as a Python dict.
    ///
    /// Returns None if no ticks have been executed yet.
    fn last_manifest(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        match self.tick_loop.last_manifest() {
            Some(m) => Ok(Some(manifest_to_pyobject(py, m)?)),
            None => Ok(None),
        }
    }

    /// Get manifest at a specific tick (within history window).
    ///
    /// Returns None if the tick is not in the rolling history window.
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

    /// Current simulation time in seconds.
    fn sim_time(&self) -> f64 {
        self.tick_loop.sim_time()
    }

    /// Spawn a semantic entity via the command buffer.
    ///
    /// The entity will be created when the next tick's command buffer is
    /// applied. Components should be a dict of `{name: json_value}`.
    ///
    /// Args:
    ///     entity_type: The entity type string (e.g. "character").
    ///     role: The entity role string (e.g. "player").
    ///     components: Dict of component_name -> JSON-serializable value.
    fn spawn_entity(
        &mut self,
        entity_type: &str,
        role: &str,
        components: &Bound<'_, PyDict>,
        py: Python<'_>,
    ) -> PyResult<()> {
        let comp_vec = pydict_to_component_vec(components, py)?;

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
    ///
    /// The despawn happens when the next tick's command buffer is applied.
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
    /// The value should be a JSON-serializable Python object. The component
    /// is updated when the next tick's command buffer is applied.
    fn set_component(
        &mut self,
        entity_id: u64,
        component_name: &str,
        value: &Bound<'_, pyo3::PyAny>,
        py: Python<'_>,
    ) -> PyResult<()> {
        let json_val = pyobj_to_json_value(value, py)?;

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
    ///
    /// The module must export a `tick()` function.
    fn load_gameplay_wasm(&mut self, wasm_bytes: Vec<u8>) -> PyResult<()> {
        let config = WasmConfig::default();
        let module = WasmModule::from_bytes(&config, &wasm_bytes).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!(
                "WASM module load failed: {e}"
            ))
        })?;
        self.wasm_module = Some(module);
        tracing::info!("WASM gameplay module loaded ({} bytes)", wasm_bytes.len());
        Ok(())
    }

    /// Hot-swap the current WASM gameplay module with new bytes.
    ///
    /// The new module must export a `tick()` function. If the swap fails,
    /// the original module remains functional.
    fn hot_swap_gameplay_wasm(&mut self, wasm_bytes: Vec<u8>) -> PyResult<()> {
        match &mut self.wasm_module {
            Some(module) => {
                module.swap(&wasm_bytes).map_err(|e| {
                    pyo3::exceptions::PyRuntimeError::new_err(format!(
                        "WASM hot-swap failed: {e}"
                    ))
                })?;
                tracing::info!(
                    "WASM gameplay module hot-swapped ({} bytes)",
                    wasm_bytes.len()
                );
                Ok(())
            }
            None => Err(pyo3::exceptions::PyRuntimeError::new_err(
                "no WASM module loaded -- call load_gameplay_wasm() first",
            )),
        }
    }

    /// Get the total number of alive entities in the world.
    fn entity_count(&self) -> usize {
        self.tick_loop.world().entity_count()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a Python dict of `{str: any}` to a vec of `(String, serde_json::Value)`.
///
/// Each value is serialized to JSON via Python's `json.dumps`, then parsed
/// back into `serde_json::Value`.
fn pydict_to_component_vec(
    dict: &Bound<'_, PyDict>,
    py: Python<'_>,
) -> PyResult<Vec<(String, serde_json::Value)>> {
    let json_mod = py.import("json")?;
    let mut result = Vec::with_capacity(dict.len());
    for (key, value) in dict.iter() {
        let name: String = key.extract()?;
        let json_str: String = json_mod
            .call_method1("dumps", (value,))?
            .extract()?;
        let json_val: serde_json::Value =
            serde_json::from_str(&json_str).map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "failed to parse component '{name}' as JSON: {e}"
                ))
            })?;
        result.push((name, json_val));
    }
    Ok(result)
}

/// Convert a single Python object to a `serde_json::Value` via `json.dumps`.
fn pyobj_to_json_value(
    value: &Bound<'_, pyo3::PyAny>,
    py: Python<'_>,
) -> PyResult<serde_json::Value> {
    let json_mod = py.import("json")?;
    let json_str: String = json_mod
        .call_method1("dumps", (value,))?
        .extract()?;
    serde_json::from_str(&json_str).map_err(|e| {
        pyo3::exceptions::PyValueError::new_err(format!(
            "failed to parse Python value as JSON: {e}"
        ))
    })
}
