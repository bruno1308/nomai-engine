//! WASM module loading, validation, and execution.
//!
//! [`WasmModule`] wraps a Wasmtime instance of a WASM gameplay module. It
//! enforces fuel metering, memory limits, and validates that the required
//! `tick()` export exists before allowing execution.
//!
//! The store holds a [`HostState`] that provides the bridge between WASM
//! gameplay code and the ECS command buffer. Host functions registered via
//! [`register_host_api`](crate::host_api::register_host_api) allow WASM
//! modules to read world state and emit deferred commands with causality
//! metadata.

use crate::host_api::{register_host_api, HostState};
use crate::WasmError;
use wasmtime::{Engine, Instance, Linker, Module, Store};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the WASM sandbox.
///
/// Controls fuel budgets (for deterministic execution limits) and memory
/// caps (to prevent runaway allocation).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WasmConfig {
    /// Fuel units granted per `tick()` call. When fuel runs out, execution
    /// traps with [`WasmError::OutOfFuel`]. Default: 1,000,000.
    pub fuel_per_tick: u64,

    /// Maximum linear memory a module may allocate, in bytes.
    /// Default: 16 MiB (16,777,216 bytes).
    pub memory_limit_bytes: usize,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            fuel_per_tick: 1_000_000,
            memory_limit_bytes: 16 * 1024 * 1024, // 16 MiB
        }
    }
}

// ---------------------------------------------------------------------------
// WasmModule
// ---------------------------------------------------------------------------

/// A loaded and validated WASM gameplay module.
///
/// Wraps a Wasmtime [`Store`], [`Instance`], and the resolved `tick()` export.
/// Fuel metering is enabled: each call to [`call_tick`](WasmModule::call_tick)
/// resets the fuel budget and returns the amount consumed.
///
/// The store holds a [`HostState`] that accumulates commands and events from
/// WASM host function calls. Use [`host_state`](WasmModule::host_state) and
/// [`host_state_mut`](WasmModule::host_state_mut) to access it, or
/// convenience methods like [`drain_commands`](WasmModule::drain_commands).
///
/// # Sandbox Guarantees
///
/// - No WASI (no filesystem, no network, no wall-clock time)
/// - Host functions restricted to the `"nomai"` namespace
/// - Fuel metering prevents infinite loops
/// - Memory is capped at [`WasmConfig::memory_limit_bytes`]
pub struct WasmModule {
    /// The Wasmtime store, holding the module's [`HostState`].
    store: Store<HostState>,
    /// The instantiated module.
    instance: Instance,
    /// Snapshot of the configuration used to create this module.
    config: WasmConfig,
}

impl WasmModule {
    /// Load and instantiate a WASM module from raw bytes.
    ///
    /// The bytes may be either a binary `.wasm` file or a text `.wat` file
    /// (Wasmtime handles both transparently).
    ///
    /// Host functions from the `"nomai"` namespace are automatically
    /// registered. Modules that do not import any `"nomai"` functions will
    /// still work -- the host state simply sits unused.
    ///
    /// # Validation
    ///
    /// - The module must compile successfully.
    /// - The module must export a function named `tick` with signature `() -> ()`.
    /// - The module must not import any functions not provided by the linker.
    ///
    /// # Errors
    ///
    /// - [`WasmError::CompileError`] if the bytes are not valid WASM/WAT.
    /// - [`WasmError::MissingExport`] if `tick()` is not exported.
    /// - [`WasmError::Runtime`] if instantiation fails (e.g. unsatisfied imports).
    pub fn from_bytes(config: &WasmConfig, bytes: &[u8]) -> Result<Self, WasmError> {
        // Build engine with fuel metering enabled.
        let mut engine_config = wasmtime::Config::new();
        engine_config.consume_fuel(true);

        let engine = Engine::new(&engine_config)
            .map_err(|e| WasmError::Runtime(format!("failed to create Wasmtime engine: {e}")))?;

        // Compile the module.
        let module =
            Module::new(&engine, bytes).map_err(|e| WasmError::CompileError(format!("{e}")))?;

        // Validate that `tick` is exported before instantiation, so we give a
        // clean MissingExport error rather than a generic instantiation failure.
        let has_tick = module.exports().any(|export| export.name() == "tick");
        if !has_tick {
            return Err(WasmError::MissingExport {
                name: "tick".to_owned(),
            });
        }

        // Create store with HostState and fuel.
        let mut store = Store::new(&engine, HostState::new());
        store
            .set_fuel(config.fuel_per_tick)
            .map_err(|e| WasmError::Runtime(format!("failed to set fuel: {e}")))?;

        // Create linker with host API functions registered.
        let mut linker = Linker::new(&engine);
        register_host_api(&mut linker)
            .map_err(|e| WasmError::Runtime(format!("failed to register host API: {e}")))?;

        // Instantiate. This will fail if the module imports anything we don't provide.
        let instance = linker.instantiate(&mut store, &module).map_err(|e| {
            let msg = format!("{e}");
            WasmError::Runtime(msg)
        })?;

        tracing::debug!(
            fuel_per_tick = config.fuel_per_tick,
            memory_limit = config.memory_limit_bytes,
            "WASM module loaded and instantiated"
        );

        Ok(Self {
            store,
            instance,
            config: config.clone(),
        })
    }

    /// Execute the module's `tick()` function.
    ///
    /// Resets fuel to [`WasmConfig::fuel_per_tick`] before calling, ensuring
    /// deterministic budgets across ticks. Returns the amount of fuel consumed.
    ///
    /// # Errors
    ///
    /// - [`WasmError::OutOfFuel`] if the function exhausts the fuel budget.
    /// - [`WasmError::Trap`] if a WASM trap occurs (e.g. unreachable).
    /// - [`WasmError::Runtime`] if the `tick` export cannot be resolved.
    pub fn call_tick(&mut self) -> Result<u64, WasmError> {
        // Reset fuel for this tick.
        self.reset_fuel()?;

        let tick_fn = self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, "tick")
            .map_err(|e| WasmError::Runtime(format!("failed to resolve tick(): {e}")))?;

        tick_fn
            .call(&mut self.store, ())
            .map_err(|e| self.classify_trap(e))?;

        // Calculate fuel consumed.
        let remaining = self
            .store
            .get_fuel()
            .map_err(|e| WasmError::Runtime(format!("failed to read fuel: {e}")))?;

        let consumed = self.config.fuel_per_tick.saturating_sub(remaining);

        tracing::trace!(
            fuel_consumed = consumed,
            fuel_remaining = remaining,
            "tick() completed"
        );

        Ok(consumed)
    }

    /// Call a named export that takes no arguments and returns an `i32`.
    ///
    /// This is a utility for testing (e.g. calling `get_count()` on the
    /// counter fixture). Fuel is NOT reset before this call -- it uses
    /// whatever fuel remains from the last `call_tick()` or initial load.
    ///
    /// # Errors
    ///
    /// - [`WasmError::Runtime`] if the export does not exist or has wrong signature.
    /// - [`WasmError::Trap`] or [`WasmError::OutOfFuel`] on execution failure.
    pub fn call_i32_export(&mut self, name: &str) -> Result<i32, WasmError> {
        let func = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, name)
            .map_err(|e| WasmError::Runtime(format!("failed to resolve export '{name}': {e}")))?;

        let result = func
            .call(&mut self.store, ())
            .map_err(|e| self.classify_trap(e))?;

        Ok(result)
    }

    /// Call a named export that takes no arguments and returns an `i64`.
    ///
    /// This is a utility for testing host API return values. Fuel is NOT
    /// reset before this call.
    ///
    /// # Errors
    ///
    /// - [`WasmError::Runtime`] if the export does not exist or has wrong signature.
    /// - [`WasmError::Trap`] or [`WasmError::OutOfFuel`] on execution failure.
    pub fn call_i64_export(&mut self, name: &str) -> Result<i64, WasmError> {
        let func = self
            .instance
            .get_typed_func::<(), i64>(&mut self.store, name)
            .map_err(|e| WasmError::Runtime(format!("failed to resolve export '{name}': {e}")))?;

        let result = func
            .call(&mut self.store, ())
            .map_err(|e| self.classify_trap(e))?;

        Ok(result)
    }

    /// Returns the amount of fuel remaining in the store.
    pub fn fuel_remaining(&self) -> u64 {
        self.store.get_fuel().unwrap_or(0)
    }

    /// Returns the configuration used to create this module.
    pub fn config(&self) -> &WasmConfig {
        &self.config
    }

    /// Provides immutable access to the [`HostState`] inside the store.
    pub fn host_state(&self) -> &HostState {
        self.store.data()
    }

    /// Provides mutable access to the [`HostState`] inside the store.
    pub fn host_state_mut(&mut self) -> &mut HostState {
        self.store.data_mut()
    }

    /// Convenience: drain accumulated commands from the host state.
    ///
    /// Returns the [`CommandBuffer`] containing all commands emitted by
    /// WASM host function calls since the last drain.
    pub fn drain_commands(&mut self) -> nomai_ecs::command::CommandBuffer {
        self.store.data_mut().drain_commands()
    }

    /// Convenience: drain accumulated events from the host state.
    ///
    /// Returns all [`GameEvent`]s emitted by WASM host function calls
    /// since the last drain.
    pub fn drain_events(&mut self) -> Vec<nomai_manifest::manifest::GameEvent> {
        self.store.data_mut().drain_events()
    }

    /// Provides immutable access to the Wasmtime [`Store`].
    pub fn store(&self) -> &Store<HostState> {
        &self.store
    }

    /// Provides mutable access to the Wasmtime [`Store`].
    pub fn store_mut(&mut self) -> &mut Store<HostState> {
        &mut self.store
    }

    /// Provides access to the Wasmtime [`Instance`].
    pub fn instance(&self) -> &Instance {
        &self.instance
    }

    // -- Internal helpers ---------------------------------------------------

    /// Reset fuel to the configured per-tick budget.
    fn reset_fuel(&mut self) -> Result<(), WasmError> {
        // First consume all remaining fuel to reset to zero, then set to budget.
        // Wasmtime's set_fuel adds to existing fuel, so we need to zero it first.
        let remaining = self
            .store
            .get_fuel()
            .map_err(|e| WasmError::Runtime(format!("failed to read fuel: {e}")))?;

        if remaining > 0 {
            // Consume all remaining fuel so we start from zero.
            self.store
                .set_fuel(0)
                .map_err(|e| WasmError::Runtime(format!("failed to reset fuel: {e}")))?;
        }

        self.store
            .set_fuel(self.config.fuel_per_tick)
            .map_err(|e| WasmError::Runtime(format!("failed to set fuel: {e}")))?;

        Ok(())
    }

    /// Classify a Wasmtime error into the appropriate [`WasmError`] variant.
    fn classify_trap(&self, error: anyhow::Error) -> WasmError {
        // Check if this is a Wasmtime Trap (includes fuel exhaustion).
        if let Some(trap) = error.downcast_ref::<wasmtime::Trap>() {
            if *trap == wasmtime::Trap::OutOfFuel {
                return WasmError::OutOfFuel {
                    budget: self.config.fuel_per_tick,
                };
            }
            return WasmError::Trap(format!("{error}"));
        }

        // Also check if the error chain contains a Trap (sometimes wrapped).
        for cause in error.chain() {
            if let Some(trap) = cause.downcast_ref::<wasmtime::Trap>() {
                if *trap == wasmtime::Trap::OutOfFuel {
                    return WasmError::OutOfFuel {
                        budget: self.config.fuel_per_tick,
                    };
                }
                return WasmError::Trap(format!("{error}"));
            }
        }

        WasmError::Runtime(format!("{error}"))
    }
}

impl std::fmt::Debug for WasmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmModule")
            .field("config", &self.config)
            .field("fuel_remaining", &self.fuel_remaining())
            .finish_non_exhaustive()
    }
}
