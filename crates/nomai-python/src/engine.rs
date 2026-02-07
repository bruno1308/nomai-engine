//! Python-facing engine wrapper.
//!
//! [`PyNomaiEngine`] is a `#[pyclass]` that wraps the Rust [`TickLoop`] and
//! exposes it to Python. Manifest data crosses the FFI boundary as Python
//! dicts via a JSON round-trip: Rust `TickManifest` -> `serde_json::to_string`
//! -> Python `json.loads` -> `dict`.

use nomai_ecs::command::CausalReason;
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::{EntityIdentity, SystemId};
use nomai_engine::physics::{
    ColliderShape, PhysicsBody, PhysicsBodyType, PhysicsWorld, Position, Velocity,
};
use nomai_engine::tick::{InputFrame, TickConfig, TickLoop};
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
            pyo3::exceptions::PyRuntimeError::new_err(format!("WASM module load failed: {e}"))
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
                    pyo3::exceptions::PyRuntimeError::new_err(format!("WASM hot-swap failed: {e}"))
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

    /// Initialize the physics world with zero gravity (for breakout).
    ///
    /// Must be called before `register_physics_entity()`. Creates a rapier2d
    /// physics world and attaches it to the tick loop. Also registers the
    /// `"position"` and `"velocity"` component types required by the physics
    /// step, if they are not already registered.
    ///
    /// Calling this again replaces the existing physics world (all registered
    /// physics entities are lost).
    fn init_physics(&mut self) -> PyResult<()> {
        if self.tick_loop.physics().is_some() {
            tracing::warn!("init_physics() called again -- replacing existing physics world");
        }
        self.tick_loop.set_physics(PhysicsWorld::new_zero_gravity());
        // Auto-register position/velocity so the physics step's commands
        // don't fail with "unknown component" errors.
        self.tick_loop
            .world_mut()
            .register_dynamic_component::<serde_json::Value>("position");
        self.tick_loop
            .world_mut()
            .register_dynamic_component::<serde_json::Value>("velocity");
        Ok(())
    }

    /// Register a physics entity with position, velocity, and body type.
    ///
    /// The entity must already exist in the ECS world. Spawn it with
    /// `spawn_entity()`, call `tick()` to apply the spawn command, then
    /// look up the entity ID via `entity_index()` or `get_entity()`.
    /// The physics world must be initialized (via `init_physics`).
    ///
    /// Args:
    ///     entity_id: Raw entity ID (look up via entity_index after spawning).
    ///     x: Horizontal position coordinate (must be finite).
    ///     y: Vertical position coordinate (must be finite).
    ///     dx: Horizontal velocity component (must be finite).
    ///     dy: Vertical velocity component (must be finite).
    ///     body_type: One of "dynamic", "kinematic", or "static".
    ///     collider_type: One of "circle" or "box".
    ///     collider_radius: Radius for circle colliders (must be > 0, required if "circle").
    ///     collider_half_width: Half-width for box colliders (must be > 0, required if "box").
    ///     collider_half_height: Half-height for box colliders (must be > 0, required if "box").
    ///     restitution: Bounciness coefficient (must be >= 0.0 and finite, default 0.5).
    ///     is_sensor: Whether this is a sensor (default false).
    #[pyo3(signature = (
        entity_id,
        x,
        y,
        dx,
        dy,
        body_type,
        collider_type,
        collider_radius = None,
        collider_half_width = None,
        collider_half_height = None,
        restitution = 0.5,
        is_sensor = false,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn register_physics_entity(
        &mut self,
        entity_id: u64,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
        body_type: &str,
        collider_type: &str,
        collider_radius: Option<f64>,
        collider_half_width: Option<f64>,
        collider_half_height: Option<f64>,
        restitution: f64,
        is_sensor: bool,
    ) -> PyResult<()> {
        // Validate position and velocity are finite.
        for (name, val) in [("x", x), ("y", y), ("dx", dx), ("dy", dy)] {
            if !val.is_finite() {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "{name} must be finite, got {val}"
                )));
            }
        }

        // Validate restitution.
        if restitution < 0.0 || !restitution.is_finite() {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "restitution must be >= 0.0 and finite, got {restitution}"
            )));
        }

        // Validate entity is alive.
        let eid = EntityId::from_raw(entity_id);
        if !self.tick_loop.world().is_alive(eid) {
            return Err(pyo3::exceptions::PyValueError::new_err(format!(
                "entity {entity_id} is not alive -- spawn it with spawn_entity() \
                 and call tick() before registering with physics"
            )));
        }

        // Ensure physics is initialized.
        let physics = self.tick_loop.physics_mut().ok_or_else(|| {
            pyo3::exceptions::PyRuntimeError::new_err(
                "physics not initialized -- call init_physics() first",
            )
        })?;

        // Parse body type.
        let parsed_body_type = match body_type {
            "dynamic" => PhysicsBodyType::Dynamic,
            "kinematic" => PhysicsBodyType::Kinematic,
            "static" => PhysicsBodyType::Static,
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown body_type '{other}' -- expected \"dynamic\", \"kinematic\", or \"static\""
                )));
            }
        };

        // Build collider shape.
        let collider = match collider_type {
            "circle" => {
                let radius = collider_radius.ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "collider_radius is required when collider_type is \"circle\"",
                    )
                })?;
                if radius <= 0.0 || !radius.is_finite() {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "collider_radius must be positive and finite, got {radius}"
                    )));
                }
                ColliderShape::Circle { radius }
            }
            "box" => {
                let half_width = collider_half_width.ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "collider_half_width is required when collider_type is \"box\"",
                    )
                })?;
                let half_height = collider_half_height.ok_or_else(|| {
                    pyo3::exceptions::PyValueError::new_err(
                        "collider_half_height is required when collider_type is \"box\"",
                    )
                })?;
                if half_width <= 0.0 || !half_width.is_finite() {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "collider_half_width must be positive and finite, got {half_width}"
                    )));
                }
                if half_height <= 0.0 || !half_height.is_finite() {
                    return Err(pyo3::exceptions::PyValueError::new_err(format!(
                        "collider_half_height must be positive and finite, got {half_height}"
                    )));
                }
                ColliderShape::Box {
                    half_width,
                    half_height,
                }
            }
            other => {
                return Err(pyo3::exceptions::PyValueError::new_err(format!(
                    "unknown collider_type '{other}' -- expected \"circle\" or \"box\""
                )));
            }
        };

        // Build physics body descriptor.
        let body = PhysicsBody {
            body_type: parsed_body_type,
            collider,
            restitution,
            is_sensor,
        };

        let position = Position { x, y };
        let velocity = Velocity { dx, dy };

        physics.register_entity(eid, &position, &velocity, &body);
        Ok(())
    }

    /// Get the total number of alive entities in the world.
    fn entity_count(&self) -> usize {
        self.tick_loop.world().entity_count()
    }

    /// Return all tracked entities from the manifest pipeline's entity index.
    ///
    /// Each entity is returned as a Python dict (via JSON round-trip) containing
    /// fields like `entity_id`, `tier`, `entity_type`, `role`, `alive`,
    /// `spawned_at_tick`, and `despawned_at_tick`.
    fn entity_index(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        let index = self.tick_loop.manifest().entity_index();
        let json_mod = py.import("json")?;
        let mut result = Vec::with_capacity(index.len());
        for entry in index.values() {
            let json_str = serde_json::to_string(entry).map_err(|e| {
                pyo3::exceptions::PyRuntimeError::new_err(format!(
                    "failed to serialize EntityEntry to JSON: {e} \
                     -- this is an internal bug, please file an issue"
                ))
            })?;
            let dict = json_mod.call_method1("loads", (json_str,))?;
            result.push(dict.unbind());
        }
        Ok(result)
    }

    /// Return a single entity's index entry by entity ID, or None if not found.
    ///
    /// The entry is returned as a Python dict (via JSON round-trip).
    ///
    /// Args:
    ///     entity_id: The raw entity ID (u64).
    fn get_entity(&self, py: Python<'_>, entity_id: u64) -> PyResult<Option<PyObject>> {
        let eid = EntityId::from_raw(entity_id);
        let index = self.tick_loop.manifest().entity_index();
        match index.get(&eid) {
            Some(entry) => {
                let json_str = serde_json::to_string(entry).map_err(|e| {
                    pyo3::exceptions::PyRuntimeError::new_err(format!(
                        "failed to serialize EntityEntry to JSON: {e} \
                         -- this is an internal bug, please file an issue"
                    ))
                })?;
                let json_mod = py.import("json")?;
                let dict = json_mod.call_method1("loads", (json_str,))?;
                Ok(Some(dict.unbind()))
            }
            None => Ok(None),
        }
    }

    /// Trace the causal chain for a component change on an entity at a given tick.
    ///
    /// Finds the most recent matching component change (highest command index)
    /// for the given entity/component pair in the specified tick's manifest,
    /// then builds and returns the full causal chain as a Python dict (via
    /// JSON round-trip). Returns None if the tick is not in the history
    /// window or no matching component change is found.
    ///
    /// Args:
    ///     entity_id: The raw entity ID (u64).
    ///     component: The component type name (e.g. "position").
    ///     tick: The tick number to look up.
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

        // Find the most recent matching component change (last in command order).
        let change = manifest
            .component_changes
            .iter()
            .rev()
            .find(|c| c.entity_id == eid && c.component_type_name == component);

        match change {
            Some(c) => {
                let chain = self.tick_loop.manifest().build_causal_chain(c);
                let json_str = serde_json::to_string(&chain).map_err(|e| {
                    pyo3::exceptions::PyRuntimeError::new_err(format!(
                        "failed to serialize CausalChain to JSON: {e} \
                         -- this is an internal bug, please file an issue"
                    ))
                })?;
                let json_mod = py.import("json")?;
                let dict = json_mod.call_method1("loads", (json_str,))?;
                Ok(Some(dict.unbind()))
            }
            None => Ok(None),
        }
    }

    // -- Snapshot/Restore ---------------------------------------------------

    /// Capture a snapshot of the current engine state.
    ///
    /// Returns a JSON string of the full ``EngineSnapshot``. Pass this string
    /// to ``restore_snapshot()`` to rewind the engine to this state.
    fn capture_snapshot(&self) -> PyResult<String> {
        let snapshot = self.tick_loop.capture_snapshot();
        serde_json::to_string(&snapshot).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!(
                "failed to serialize EngineSnapshot to JSON: {e}"
            ))
        })
    }

    /// Restore engine state from a JSON snapshot string.
    ///
    /// The snapshot must have been produced by ``capture_snapshot()``.
    /// After restore the tick counter and world state match the snapshot;
    /// the manifest pipeline is reset and the command buffer is cleared.
    ///
    /// **Note:** Systems, physics world, and WASM module are NOT restored.
    /// Re-attach them after calling this method if needed.
    fn restore_snapshot(&mut self, snapshot_json: &str) -> PyResult<()> {
        let snapshot: nomai_engine::snapshot::EngineSnapshot =
            serde_json::from_str(snapshot_json).map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "invalid snapshot JSON: {e} -- ensure the string was produced by capture_snapshot()"
                ))
            })?;
        self.tick_loop
            .restore_from_snapshot(&snapshot)
            .map_err(|e| {
                pyo3::exceptions::PyRuntimeError::new_err(format!("snapshot restore failed: {e}"))
            })
    }

    /// Get the BLAKE3 hex digest of the current engine state.
    ///
    /// Returns a 64-character lowercase hex string. Two engines with
    /// identical state (world, tick counter, fixed_dt, input frame)
    /// will produce the same hash.
    fn state_hash(&self) -> String {
        self.tick_loop.state_hash()
    }

    // -- Replay -------------------------------------------------------------

    /// Replay a recorded log (JSON string) and return the result as JSON.
    ///
    /// The log must have been produced by serializing a ``ReplayLog`` to JSON
    /// (e.g., from the Rust integration tests or via ``ReplayRecorder``).
    ///
    /// Returns a JSON string of the ``ReplayResult``, which includes:
    /// - ``completed``: whether the replay ran to completion.
    /// - ``ticks_replayed``: total ticks replayed.
    /// - ``first_divergence``: the first checkpoint mismatch, if any.
    fn replay_log(&mut self, replay_log_json: &str) -> PyResult<String> {
        let log: nomai_engine::replay::ReplayLog =
            serde_json::from_str(replay_log_json).map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "invalid replay log JSON: {e} -- ensure the string is a valid ReplayLog"
                ))
            })?;
        let result = nomai_engine::replay::replay(&mut self.tick_loop, &log).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!("replay failed: {e}"))
        })?;
        serde_json::to_string(&result).map_err(|e| {
            pyo3::exceptions::PyRuntimeError::new_err(format!(
                "failed to serialize ReplayResult to JSON: {e}"
            ))
        })
    }

    // -- Input --------------------------------------------------------------

    /// Set the input frame for simulation.
    ///
    /// Takes a dict of ``{str: json_value}``. Each value must be
    /// JSON-serializable. The input frame persists until overwritten
    /// by another ``set_input()`` call (or snapshot restore) and is
    /// included in snapshot/replay state hashing.
    fn set_input(&mut self, input: &Bound<'_, PyDict>, py: Python<'_>) -> PyResult<()> {
        let json_mod = py.import("json")?;
        let mut frame = InputFrame::default();
        for (key, value) in input.iter() {
            let name: String = key.extract()?;
            let json_str: String = json_mod.call_method1("dumps", (value,))?.extract()?;
            let json_val: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
                pyo3::exceptions::PyValueError::new_err(format!(
                    "failed to parse input '{name}' as JSON: {e}"
                ))
            })?;
            frame.inputs.insert(name, json_val);
        }
        self.tick_loop.set_input(frame);
        Ok(())
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
        let json_str: String = json_mod.call_method1("dumps", (value,))?.extract()?;
        let json_val: serde_json::Value = serde_json::from_str(&json_str).map_err(|e| {
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
    let json_str: String = json_mod.call_method1("dumps", (value,))?.extract()?;
    serde_json::from_str(&json_str).map_err(|e| {
        pyo3::exceptions::PyValueError::new_err(format!(
            "failed to parse Python value as JSON: {e}"
        ))
    })
}
