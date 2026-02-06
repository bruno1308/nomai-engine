//! Nomai WASM Host -- Wasmtime integration for sandboxed gameplay logic.
//!
//! This crate provides the WASM sandbox for AI-generated gameplay modules in
//! the Nomai Engine. Gameplay scripts (compiled from AssemblyScript to WASM)
//! run inside a fuel-metered Wasmtime instance with no filesystem, network,
//! threading, or wall-clock access.
//!
//! # Architecture
//!
//! - **`WasmConfig`**: Configuration for fuel budgets and memory limits.
//! - **`WasmModule`**: Loads, validates, and executes a WASM module with a
//!   required `tick()` export.
//! - **`HostState`**: State held inside the Wasmtime store; accumulates
//!   commands and events from WASM host function calls.
//! - **`WasmError`**: Error type covering compilation, missing exports, fuel
//!   exhaustion, and runtime traps.
//!
//! # Host API
//!
//! WASM modules can import functions from the `"nomai"` namespace to read
//! world state and emit deferred commands with causality metadata. All
//! mutations use [`SystemId::WASM_GAMEPLAY`] and carry a reason string for
//! manifest causality tracking.
//!
//! # Example
//!
//! ```no_run
//! use nomai_wasm_host::{WasmConfig, WasmModule};
//!
//! let config = WasmConfig::default();
//! let wat = r#"(module (func (export "tick") nop))"#;
//! let mut module = WasmModule::from_bytes(&config, wat.as_bytes()).unwrap();
//! let fuel_consumed = module.call_tick().unwrap();
//! assert!(fuel_consumed > 0);
//! ```

#![deny(unsafe_code)]

pub mod host_api;
mod module;

pub use host_api::HostState;
pub use module::{WasmConfig, WasmModule};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors produced by WASM module operations.
#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    /// The WASM binary could not be compiled (invalid bytecode or WAT syntax).
    #[error("WASM compilation failed: {0}")]
    CompileError(String),

    /// The module does not export a required function (e.g. `tick()`).
    #[error("missing required export '{name}' -- the WASM module must export a `tick()` function")]
    MissingExport {
        /// The name of the missing export.
        name: String,
    },

    /// The module exhausted its fuel budget during execution.
    #[error("WASM module ran out of fuel (budget: {budget} units) -- possible infinite loop or excessive computation")]
    OutOfFuel {
        /// The fuel budget that was exceeded.
        budget: u64,
    },

    /// A WASM trap occurred during execution (e.g. unreachable instruction,
    /// division by zero, out-of-bounds memory access).
    #[error("WASM trap: {0}")]
    Trap(String),

    /// The module attempted to exceed its memory limit.
    #[error("WASM module exceeded memory limit of {limit_bytes} bytes")]
    MemoryLimitExceeded {
        /// The configured memory limit in bytes.
        limit_bytes: usize,
    },

    /// A general runtime error from the Wasmtime engine.
    #[error("WASM runtime error: {0}")]
    Runtime(String),
}

// ---------------------------------------------------------------------------
// Tests -- B1 tests (unchanged, still passing with HostState store)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: load a WAT fixture file from the tests/fixtures directory.
    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e))
    }

    // -- Test 1: Load noop module -------------------------------------------

    #[test]
    fn load_noop_module() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("noop.wat");
        let module = WasmModule::from_bytes(&config, &bytes);
        assert!(
            module.is_ok(),
            "noop module should load: {:?}",
            module.err()
        );
    }

    // -- Test 2: Call noop tick, verify fuel consumed ------------------------

    #[test]
    fn noop_tick_consumes_fuel() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("noop.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let fuel_consumed = module.call_tick().unwrap();
        assert!(
            fuel_consumed > 0,
            "even a noop tick should consume some fuel for call overhead"
        );
    }

    // -- Test 3: Counter module: 5 ticks, get_count == 5 --------------------

    #[test]
    fn counter_increments_over_five_ticks() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("counter.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        for _ in 0..5 {
            module.call_tick().unwrap();
        }

        let count = module.call_i32_export("get_count").unwrap();
        assert_eq!(count, 5, "counter should be 5 after 5 ticks");
    }

    // -- Test 4: Fuel exhaustion traps with OutOfFuel -----------------------

    #[test]
    fn fuel_exhaustion_returns_out_of_fuel() {
        let config = WasmConfig {
            fuel_per_tick: 10_000, // Very low budget for the infinite loop.
            ..WasmConfig::default()
        };
        let bytes = fixture_bytes("fuel_hog.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let result = module.call_tick();
        assert!(result.is_err(), "infinite loop should fail");

        let err = result.unwrap_err();
        assert!(
            matches!(err, WasmError::OutOfFuel { .. }),
            "expected OutOfFuel, got: {err:?}"
        );
    }

    // -- Test 5: Fuel resets between ticks (deterministic) ------------------

    #[test]
    fn fuel_resets_between_ticks() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("noop.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let fuel1 = module.call_tick().unwrap();
        let fuel2 = module.call_tick().unwrap();

        assert_eq!(
            fuel1, fuel2,
            "fuel consumed should be identical for identical noop ticks (deterministic): tick1={fuel1}, tick2={fuel2}"
        );
    }

    // -- Test 6: Missing tick export -> MissingExport error -----------------

    #[test]
    fn missing_tick_export_returns_error() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("no_tick.wat");
        let result = WasmModule::from_bytes(&config, &bytes);

        assert!(result.is_err(), "module without tick() should fail");

        let err = result.unwrap_err();
        assert!(
            matches!(err, WasmError::MissingExport { ref name } if name == "tick"),
            "expected MissingExport for 'tick', got: {err:?}"
        );
    }

    // -- Test 7: WASI import module fails (no WASI provided) ----------------

    #[test]
    fn wasi_import_fails_without_wasi() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("wasi_import.wat");
        let result = WasmModule::from_bytes(&config, &bytes);

        assert!(
            result.is_err(),
            "module requiring WASI imports should fail to instantiate"
        );
    }

    // -- Test 8: Config accessors -------------------------------------------

    #[test]
    fn config_accessors_return_configured_values() {
        let config = WasmConfig {
            fuel_per_tick: 500_000,
            memory_limit_bytes: 8 * 1024 * 1024,
        };
        let bytes = fixture_bytes("noop.wat");
        let module = WasmModule::from_bytes(&config, &bytes).unwrap();

        assert_eq!(module.config().fuel_per_tick, 500_000);
        assert_eq!(module.config().memory_limit_bytes, 8 * 1024 * 1024);
    }

    // -- Test 9: Fuel remaining after tick -----------------------------------

    #[test]
    fn fuel_remaining_decreases_after_tick() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("noop.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Before any tick, fuel_remaining should be 0 (fuel not yet loaded for a tick).
        // After call_tick, fuel_remaining reflects what's left.
        module.call_tick().unwrap();
        let remaining = module.fuel_remaining();
        assert!(
            remaining < config.fuel_per_tick,
            "fuel remaining ({remaining}) should be less than budget ({})",
            config.fuel_per_tick
        );
    }
}

// ---------------------------------------------------------------------------
// B2 Tests -- Host API
// ---------------------------------------------------------------------------

#[cfg(test)]
mod host_api_tests {
    use super::*;
    use nomai_ecs::command::{CausalReason, CommandKind};
    use nomai_ecs::identity::SystemId;

    /// Helper: load a WAT fixture file from the tests/fixtures directory.
    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e))
    }

    // -- B2 Test 1: set_component emits command with correct causality ------

    #[test]
    fn host_api_set_component_emits_command() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Set up host state before tick.
        module.host_state_mut().begin_tick(1, 0.016);
        module.host_state_mut().entity_count = 10;

        // Run tick -- the WAT module calls set_component during tick().
        module.call_tick().unwrap();

        // Drain and inspect commands.
        let cmd_buf = module.drain_commands();
        let commands = cmd_buf.commands();

        assert_eq!(
            commands.len(),
            1,
            "expected 1 command from WASM set_component call"
        );

        let cmd = &commands[0];

        // Verify system ID is WASM_GAMEPLAY.
        assert_eq!(
            cmd.issued_by,
            SystemId::WASM_GAMEPLAY,
            "command must be issued by SystemId::WASM_GAMEPLAY"
        );

        // Verify causal reason is GameRule with the reason string from WASM.
        assert_eq!(
            cmd.reason,
            CausalReason::GameRule("damage_from_wasm".to_owned()),
            "command reason must carry the WASM-provided reason string"
        );

        // Verify the command kind.
        match &cmd.kind {
            CommandKind::SetComponent {
                component_name,
                value,
            } => {
                assert_eq!(component_name, "health");
                assert_eq!(*value, serde_json::json!(42));
            }
            other => panic!("expected SetComponent, got: {other:?}"),
        }

        // Verify target entity.
        let target = cmd.target.expect("SetComponent must have a target");
        assert_eq!(target.to_raw(), 42, "entity_id should be 42 as set in WAT");
    }

    // -- B2 Test 2: WASM reads entity count from host state -----------------

    #[test]
    fn host_api_reads_entity_count() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Set entity_count to a known value.
        module.host_state_mut().entity_count = 42;

        module.call_tick().unwrap();

        // The WAT stores the result in a global, accessible via get_last_entity_count().
        let count = module.call_i32_export("get_last_entity_count").unwrap();
        assert_eq!(
            count, 42,
            "WASM should read entity_count=42 from host state"
        );
    }

    // -- B2 Test 3: WASM reads tick number from host state ------------------

    #[test]
    fn host_api_reads_tick_number() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Set tick to a known value.
        module.host_state_mut().tick = 99;

        module.call_tick().unwrap();

        // The WAT stores the result in a global, accessible via get_last_tick_number().
        let tick = module.call_i64_export("get_last_tick_number").unwrap();
        assert_eq!(tick, 99, "WASM should read tick=99 from host state");
    }

    // -- B2 Test 4: all commands carry SystemId::WASM_GAMEPLAY --------------

    #[test]
    fn host_api_commands_carry_wasm_system_id() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        module.host_state_mut().begin_tick(1, 0.016);

        // Run multiple ticks to generate multiple commands.
        for _ in 0..3 {
            module.call_tick().unwrap();
        }

        let cmd_buf = module.drain_commands();
        let commands = cmd_buf.commands();

        assert_eq!(commands.len(), 3, "expected 3 commands from 3 ticks");

        for (i, cmd) in commands.iter().enumerate() {
            assert_eq!(
                cmd.issued_by,
                SystemId::WASM_GAMEPLAY,
                "command {i} must have SystemId::WASM_GAMEPLAY, got {:?}",
                cmd.issued_by
            );
        }
    }

    // -- B2 Test 5: host_call_count increments on each host call ------------

    #[test]
    fn host_call_counter_increments() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        module.host_state_mut().begin_tick(1, 0.016);
        assert_eq!(
            module.host_state().host_call_count,
            0,
            "host_call_count should start at 0 after begin_tick"
        );

        module.call_tick().unwrap();

        // The WAT module calls:
        //   1. get_entity_count
        //   2. tick_number
        //   3. set_component
        //   4. log
        // Total: 4 host calls per tick
        let count = module.host_state().host_call_count;
        assert_eq!(
            count, 4,
            "expected 4 host calls (get_entity_count + tick_number + set_component + log), got {count}"
        );
    }

    // -- B2 Test 6: host state accessible via accessors ---------------------

    #[test]
    fn host_state_accessors_work() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("noop.wat");
        let module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Verify host state is accessible and has sensible defaults.
        let state = module.host_state();
        assert_eq!(state.tick, 0);
        assert_eq!(state.sim_time, 0.0);
        assert_eq!(state.entity_count, 0);
        assert_eq!(state.host_call_count, 0);
        assert!(state.commands.is_empty());
        assert!(state.events.is_empty());
    }

    // -- B2 Test 7: begin_tick resets host_call_count -----------------------

    #[test]
    fn begin_tick_resets_host_call_count() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // First tick.
        module.host_state_mut().begin_tick(1, 0.016);
        module.call_tick().unwrap();
        assert!(
            module.host_state().host_call_count > 0,
            "should have host calls after tick"
        );

        // begin_tick should reset the counter.
        module.host_state_mut().begin_tick(2, 0.032);
        assert_eq!(
            module.host_state().host_call_count,
            0,
            "begin_tick should reset host_call_count to 0"
        );
    }

    // -- B2 Test 8: drain_commands returns and clears buffer ----------------

    #[test]
    fn drain_commands_returns_and_clears() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        module.host_state_mut().begin_tick(1, 0.016);
        module.call_tick().unwrap();

        // First drain should return the commands.
        let buf = module.drain_commands();
        assert_eq!(buf.commands().len(), 1, "should have 1 command");

        // Second drain should return empty -- buffer was already drained.
        let buf2 = module.drain_commands();
        assert!(buf2.is_empty(), "second drain should return empty buffer");
    }

    // -- B2 Test 9: modules without nomai imports still work ----------------

    #[test]
    fn module_without_nomai_imports_still_works() {
        // The noop module does not import any nomai functions.
        // It should still load and run fine with the HostState store.
        let config = WasmConfig::default();
        let bytes = fixture_bytes("noop.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let fuel = module.call_tick().unwrap();
        assert!(fuel > 0);

        // Host state should be at defaults (no calls made).
        assert_eq!(module.host_state().host_call_count, 0);
        assert!(module.host_state().commands.is_empty());
    }

    // -- B2 Test 10: snapshot_world populates entity_components -------------

    #[test]
    fn snapshot_world_populates_state() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("noop.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let mut snapshot = std::collections::HashMap::new();
        let mut components = std::collections::HashMap::new();
        components.insert("health".to_owned(), serde_json::json!(100));
        components.insert(
            "position".to_owned(),
            serde_json::json!({"x": 1.0, "y": 2.0}),
        );
        snapshot.insert(42u64, components);

        module.host_state_mut().snapshot_world(snapshot, 5);

        assert_eq!(module.host_state().entity_count, 5);
        assert_eq!(module.host_state().entity_components.len(), 1);

        let entity_components = module.host_state().entity_components.get(&42u64).unwrap();
        assert_eq!(
            entity_components.get("health"),
            Some(&serde_json::json!(100))
        );
    }
}
