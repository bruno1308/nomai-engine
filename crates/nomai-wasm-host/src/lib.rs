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
pub mod integration;
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

    /// The module's imports don't match the available host API functions.
    #[error("WASM module has unsupported import '{module}::{name}' -- only 'nomai::*' and 'env::abort' are supported")]
    InvalidImport {
        /// The import module namespace.
        module: String,
        /// The import function name.
        name: String,
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

    // -- B3 Hot-Swap Tests -------------------------------------------------

    #[test]
    fn hot_swap_changes_behavior() {
        let config = WasmConfig::default();
        let v1 = fixture_bytes("counter.wat");
        let v2 = fixture_bytes("counter_v2.wat");

        let mut module = WasmModule::from_bytes(&config, &v1).unwrap();

        // Run v1: increments by 1
        module.call_tick().unwrap();
        let count_v1 = module.call_i32_export("get_count").unwrap();
        assert_eq!(count_v1, 1, "v1 should increment by 1");

        // Swap to v2
        module.swap(&v2).unwrap();

        // Run v2: increments by 10 (counter resets since WASM memory is replaced)
        module.call_tick().unwrap();
        let count_v2 = module.call_i32_export("get_count").unwrap();
        assert_eq!(
            count_v2, 10,
            "v2 should increment by 10 from 0 (WASM state resets on swap)"
        );
    }

    #[test]
    fn hot_swap_validates_tick_export() {
        let config = WasmConfig::default();
        let v1 = fixture_bytes("noop.wat");

        let mut module = WasmModule::from_bytes(&config, &v1).unwrap();

        // Try to swap with a module that has no tick export
        let bad = b"(module (func (export \"not_tick\")))";
        let result = module.swap(bad);
        assert!(result.is_err(), "swap with no tick() should fail");
        assert!(
            matches!(result.unwrap_err(), WasmError::MissingExport { ref name } if name == "tick"),
            "should be MissingExport for 'tick'"
        );

        // Original module should still work
        let fuel = module.call_tick().unwrap();
        assert!(fuel > 0, "original module should still work after failed swap");
    }

    #[test]
    fn hot_swap_completes_under_100ms() {
        let config = WasmConfig::default();
        let v1 = fixture_bytes("noop.wat");
        let v2 = fixture_bytes("counter.wat");

        let mut module = WasmModule::from_bytes(&config, &v1).unwrap();

        let start = std::time::Instant::now();
        module.swap(&v2).unwrap();
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < 100,
            "hot-swap should take <100ms, took {}ms",
            elapsed.as_millis()
        );
    }

    #[test]
    fn hot_swap_preserves_host_state() {
        let config = WasmConfig::default();
        let v1 = fixture_bytes("noop.wat");
        let v2 = fixture_bytes("counter.wat");

        let mut module = WasmModule::from_bytes(&config, &v1).unwrap();

        // Set up host state
        module.host_state_mut().begin_tick(42, 0.7);
        module.host_state_mut().entity_count = 99;

        // Swap
        module.swap(&v2).unwrap();

        // Host state should be preserved (it lives in the Store, not the Instance)
        assert_eq!(module.host_state().tick, 42, "tick should be preserved");
        assert_eq!(
            module.host_state().entity_count, 99,
            "entity_count should be preserved"
        );
    }
}

// ---------------------------------------------------------------------------
// B9 Tests -- Sandbox Hardening (#30)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod hardening_tests {
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

    // -- Import validation tests -----------------------------------------------

    #[test]
    fn invalid_import_rejected() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("bad_import.wat");
        let result = WasmModule::from_bytes(&config, &bytes);

        assert!(result.is_err(), "module with unknown import should fail");

        let err = result.unwrap_err();
        match err {
            WasmError::InvalidImport { ref module, ref name } => {
                assert_eq!(module, "unknown_module");
                assert_eq!(name, "unknown_func");
            }
            other => panic!("expected InvalidImport, got: {other:?}"),
        }
    }

    #[test]
    fn wasi_import_rejected_with_invalid_import() {
        // The existing wasi_import.wat should now be rejected at the import
        // validation stage (InvalidImport) rather than at instantiation
        // (Runtime error), because "wasi_snapshot_preview1" is not an
        // allowed namespace.
        let config = WasmConfig::default();
        let bytes = fixture_bytes("wasi_import.wat");
        let result = WasmModule::from_bytes(&config, &bytes);

        assert!(result.is_err(), "WASI import should be rejected");

        let err = result.unwrap_err();
        assert!(
            matches!(err, WasmError::InvalidImport { ref module, .. } if module == "wasi_snapshot_preview1"),
            "expected InvalidImport for wasi_snapshot_preview1, got: {err:?}"
        );
    }

    #[test]
    fn nomai_and_env_imports_allowed() {
        // The host_api_test.wat imports from "nomai" and the env::abort is
        // also registered. Both should be allowed.
        let config = WasmConfig::default();
        let bytes = fixture_bytes("host_api_test.wat");
        let result = WasmModule::from_bytes(&config, &bytes);
        assert!(
            result.is_ok(),
            "module importing 'nomai' functions should load: {:?}",
            result.err()
        );
    }

    // -- Trap recovery tests ---------------------------------------------------

    #[test]
    fn trap_recovers_gracefully() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("trap_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let result = module.call_tick();
        assert!(result.is_err(), "unreachable should trap");

        let err = result.unwrap_err();
        assert!(
            matches!(err, WasmError::Trap(_)),
            "expected Trap, got: {err:?}"
        );

        assert_eq!(
            module.consecutive_traps(),
            1,
            "consecutive_traps should be 1 after first trap"
        );
    }

    #[test]
    fn consecutive_traps_increment() {
        let config = WasmConfig::default();
        let bytes = fixture_bytes("trap_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        for i in 1..=3 {
            let result = module.call_tick();
            assert!(result.is_err(), "tick {i} should trap");
            assert_eq!(
                module.consecutive_traps(),
                i,
                "consecutive_traps should be {i} after {i} traps"
            );
        }
    }

    #[test]
    fn consecutive_traps_reset_on_success() {
        // This test verifies that consecutive_traps resets to 0 when a
        // successful tick occurs. We can only do this by swapping to a
        // working module after trapping.
        let config = WasmConfig::default();
        let trap_bytes = fixture_bytes("trap_test.wat");
        let noop_bytes = fixture_bytes("noop.wat");

        let mut module = WasmModule::from_bytes(&config, &trap_bytes).unwrap();

        // Trap twice.
        let _ = module.call_tick();
        let _ = module.call_tick();
        assert_eq!(module.consecutive_traps(), 2);

        // Swap to a working module.
        module.swap(&noop_bytes).unwrap();

        // The consecutive_traps should still be 2 (swap doesn't reset it).
        assert_eq!(
            module.consecutive_traps(),
            2,
            "swap should not reset consecutive_traps"
        );

        // A successful tick should reset it.
        module.call_tick().unwrap();
        assert_eq!(
            module.consecutive_traps(),
            0,
            "consecutive_traps should reset to 0 after successful tick"
        );
    }

    // -- Swap import validation tests ------------------------------------------

    #[test]
    fn swap_validates_imports() {
        let config = WasmConfig::default();
        let noop_bytes = fixture_bytes("noop.wat");
        let bad_bytes = fixture_bytes("bad_import.wat");

        let mut module = WasmModule::from_bytes(&config, &noop_bytes).unwrap();

        // Verify original module works.
        let fuel = module.call_tick().unwrap();
        assert!(fuel > 0, "original module should work before swap");

        // Try to swap with a module that has invalid imports.
        let result = module.swap(&bad_bytes);
        assert!(result.is_err(), "swap with bad imports should fail");

        let err = result.unwrap_err();
        assert!(
            matches!(err, WasmError::InvalidImport { ref module, ref name }
                if module == "unknown_module" && name == "unknown_func"),
            "should be InvalidImport, got: {err:?}"
        );

        // Original module should still work after failed swap.
        let fuel = module.call_tick().unwrap();
        assert!(fuel > 0, "original module should still work after failed swap");
    }

    #[test]
    fn swap_with_wasi_import_rejected() {
        let config = WasmConfig::default();
        let noop_bytes = fixture_bytes("noop.wat");
        let wasi_bytes = fixture_bytes("wasi_import.wat");

        let mut module = WasmModule::from_bytes(&config, &noop_bytes).unwrap();

        let result = module.swap(&wasi_bytes);
        assert!(result.is_err(), "swap with WASI imports should fail");

        assert!(
            matches!(result.unwrap_err(), WasmError::InvalidImport { .. }),
            "should be InvalidImport for WASI namespace"
        );

        // Original still works.
        module.call_tick().unwrap();
    }

    // -- Edge case: fuel exhaustion also increments consecutive_traps ----------

    #[test]
    fn fuel_exhaustion_increments_consecutive_traps() {
        let config = WasmConfig {
            fuel_per_tick: 10_000, // Very low budget for the infinite loop.
            ..WasmConfig::default()
        };
        let bytes = fixture_bytes("fuel_hog.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let result = module.call_tick();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WasmError::OutOfFuel { .. }));

        assert_eq!(
            module.consecutive_traps(),
            1,
            "fuel exhaustion should increment consecutive_traps"
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

    // -- B4: Load AS-compiled module (requires `just build-gameplay`) -------

    #[test]
    #[ignore] // Requires npm + AS build; run with `cargo test -- --ignored`
    fn load_assemblyscript_gameplay_module() {
        let wasm_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("gameplay")
            .join("build")
            .join("gameplay.wasm");

        let wasm = std::fs::read(&wasm_path).unwrap_or_else(|e| {
            panic!(
                "AS gameplay.wasm not found at {} -- run `just build-gameplay` first: {}",
                wasm_path.display(),
                e
            )
        });

        let config = WasmConfig::default();
        let mut module = WasmModule::from_bytes(&config, &wasm).unwrap();

        // Set up host state before tick.
        module.host_state_mut().begin_tick(1, 0.016);
        module.host_state_mut().entity_count = 3;

        // Execute tick.
        let fuel = module.call_tick().unwrap();
        assert!(fuel > 0, "AS module should consume fuel");

        // Check that commands were emitted (set_component for position).
        let cmds = module.drain_commands();
        assert!(
            !cmds.is_empty(),
            "AS module should emit at least one command (set_component for position)"
        );

        // Verify the command is a SetComponent targeting entity 0 with
        // component name "position" and reason "move_right_each_tick".
        let commands = cmds.commands();
        let cmd = &commands[0];
        assert_eq!(
            cmd.issued_by,
            nomai_ecs::identity::SystemId::WASM_GAMEPLAY,
            "command must be issued by SystemId::WASM_GAMEPLAY"
        );
        assert_eq!(
            cmd.reason,
            nomai_ecs::command::CausalReason::GameRule("move_right_each_tick".to_owned()),
            "command reason must carry the AS-provided reason string"
        );
        match &cmd.kind {
            nomai_ecs::command::CommandKind::SetComponent {
                component_name,
                value,
            } => {
                assert_eq!(component_name, "position");
                // tick=1, so x=1.0
                let x = value.get("x").expect("position should have x");
                assert_eq!(*x, serde_json::json!(1.0));
            }
            other => panic!("expected SetComponent, got: {other:?}"),
        }
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

// ---------------------------------------------------------------------------
// B5 Tests -- Causality Across WASM Boundary
// ---------------------------------------------------------------------------

#[cfg(test)]
mod integration_tests {
    use super::*;
    use nomai_ecs::command::CausalReason;
    use nomai_ecs::identity::SystemId;
    use nomai_ecs::world::World;
    use nomai_manifest::manifest::ManifestPipeline;

    /// Helper: load a WAT fixture file from the tests/fixtures directory.
    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e))
    }

    // -- Component type for tests --

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Health(i32);

    // -- B5 Test 1: Full causal chain traces through WASM boundary -----------

    #[test]
    fn causal_chain_traces_through_wasm_boundary() {
        // Setup world with a registered "health" component.
        let mut world = World::new();
        world.register_component::<Health>("health");

        // Spawn an entity. The first entity gets EntityId(index=0, gen=0),
        // which has to_raw() == 0. The causality_test.wat targets entity_id=0.
        let entity = world.spawn_with(Health(100));
        assert_eq!(
            entity.to_raw(),
            0,
            "first spawned entity should have raw id 0"
        );

        // Setup manifest pipeline.
        let mut manifest = ManifestPipeline::new();

        // Load WASM module from the causality test fixture.
        let config = WasmConfig::default();
        let bytes = fixture_bytes("causality_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Run a tick through the full pipeline.
        let (tick_manifest, cmd_count) = crate::integration::run_wasm_tick(
            &mut module,
            &mut world,
            &mut manifest,
            1,
            1.0 / 60.0,
        )
        .unwrap();

        // Verify commands were emitted and applied.
        assert!(cmd_count > 0, "WASM should have emitted commands");

        // Verify manifest has component changes.
        assert!(
            !tick_manifest.component_changes.is_empty(),
            "manifest should record component changes from WASM"
        );

        // Find the health component change.
        let health_change = tick_manifest
            .component_changes
            .iter()
            .find(|c| c.component_type_name == "health")
            .expect("should have a health component change");

        // Verify system ID is WASM_GAMEPLAY.
        assert_eq!(
            health_change.changed_by,
            SystemId::WASM_GAMEPLAY,
            "component change should be attributed to WASM_GAMEPLAY"
        );

        // Verify causal reason carries the WASM reason string.
        assert_eq!(
            health_change.reason,
            CausalReason::GameRule("wasm_causality_test".to_owned()),
            "causal reason should carry the WASM-provided reason string"
        );

        // Build causal chain from the manifest.
        let chain = manifest.build_causal_chain(health_change);
        assert!(
            !chain.steps.is_empty(),
            "causal chain should have at least one step"
        );
        assert_eq!(
            chain.steps[0].system_id,
            SystemId::WASM_GAMEPLAY,
            "first step in causal chain should be WASM_GAMEPLAY"
        );
        assert_eq!(
            chain.steps[0].reason,
            CausalReason::GameRule("wasm_causality_test".to_owned()),
            "causal chain step should carry the reason"
        );
    }

    // -- B5 Test 2: Multiple ticks build causal history ----------------------

    #[test]
    fn multiple_ticks_build_causal_history() {
        let mut world = World::new();
        world.register_component::<Health>("health");
        let entity = world.spawn_with(Health(100));
        assert_eq!(entity.to_raw(), 0);

        let mut manifest = ManifestPipeline::new();
        let config = WasmConfig::default();
        let bytes = fixture_bytes("causality_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Run multiple ticks through the full pipeline.
        for tick in 1..=5 {
            let (_, cmd_count) = crate::integration::run_wasm_tick(
                &mut module,
                &mut world,
                &mut manifest,
                tick,
                tick as f64 / 60.0,
            )
            .unwrap();
            assert!(cmd_count > 0, "tick {tick} should apply commands");
        }

        // Get the last tick's manifest and build chain.
        let last_manifest = manifest
            .manifest_at_tick(5)
            .expect("tick 5 should be in history");
        let change = last_manifest
            .component_changes
            .iter()
            .find(|c| c.component_type_name == "health")
            .expect("should have health component change at tick 5");

        let chain = manifest.build_causal_chain(change);

        // Should have steps from tick 5 (the change itself) + prior ticks.
        // Ticks 1-4 each changed "health" on the same entity, so the chain
        // should trace back through all of them.
        assert!(
            chain.steps.len() >= 2,
            "causal chain should span multiple ticks, got {} steps",
            chain.steps.len()
        );

        // All steps should be WASM_GAMEPLAY.
        for step in &chain.steps {
            assert_eq!(
                step.system_id,
                SystemId::WASM_GAMEPLAY,
                "all causal chain steps should be WASM_GAMEPLAY"
            );
        }

        // First step should be from tick 5 (most recent), last from tick 1 (oldest).
        assert_eq!(chain.steps[0].tick, 5, "first step should be tick 5");
        assert_eq!(
            chain.steps.last().unwrap().tick,
            1,
            "last step should be tick 1"
        );
    }

    // -- B5 Test 3: Manifest records systems_executed ------------------------

    #[test]
    fn manifest_records_systems_executed() {
        let mut world = World::new();
        world.register_component::<Health>("health");
        let _entity = world.spawn_with(Health(100));

        let mut manifest = ManifestPipeline::new();
        let config = WasmConfig::default();
        let bytes = fixture_bytes("causality_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let (tick_manifest, _) = crate::integration::run_wasm_tick(
            &mut module,
            &mut world,
            &mut manifest,
            1,
            0.016,
        )
        .unwrap();

        assert!(
            tick_manifest
                .systems_executed
                .contains(&"wasm_gameplay".to_owned()),
            "manifest should record wasm_gameplay in systems_executed"
        );
    }

    // -- B5 Test 4: WASM events appear in manifest ---------------------------

    #[test]
    fn wasm_events_appear_in_manifest() {
        let mut world = World::new();
        world.register_component::<Health>("health");
        let _entity = world.spawn_with(Health(100));

        let mut manifest = ManifestPipeline::new();
        let config = WasmConfig::default();
        let bytes = fixture_bytes("causality_test.wat");
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        // Manually inject an event to simulate WASM emitting one.
        // (The causality_test.wat does not call emit_event, so we inject
        // directly into the host state to test the event path through
        // the integration orchestrator.)
        use nomai_manifest::manifest::GameEvent;
        module.host_state_mut().events.push(GameEvent {
            event_type: "test_event".to_owned(),
            description: "WASM emitted test event".to_owned(),
            involved_entities: vec![],
            caused_by: SystemId::WASM_GAMEPLAY,
            reason: CausalReason::GameRule("test_reason".to_owned()),
            tick: 1,
        });

        let (tick_manifest, _) = crate::integration::run_wasm_tick(
            &mut module,
            &mut world,
            &mut manifest,
            1,
            0.016,
        )
        .unwrap();

        assert!(
            !tick_manifest.events.is_empty(),
            "manifest should include events from WASM"
        );
        assert_eq!(
            tick_manifest.events[0].event_type, "test_event",
            "event type should match"
        );
        assert_eq!(
            tick_manifest.events[0].caused_by,
            SystemId::WASM_GAMEPLAY,
            "event should be attributed to WASM_GAMEPLAY"
        );
    }
}

// ---------------------------------------------------------------------------
// B8 Tests -- Integration: Intentional Failures
// ---------------------------------------------------------------------------

#[cfg(test)]
mod b8_integration_tests {
    use super::*;
    use nomai_ecs::world::World;
    use nomai_manifest::manifest::ManifestPipeline;

    /// Helper: load a WAT fixture file from the tests/fixtures directory.
    fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join(name);
        std::fs::read(&path)
            .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e))
    }

    /// Component type for position (matches the JSON {"x":..., "y":...}).
    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Position {
        x: f64,
        y: f64,
    }

    /// Run a WASM scenario through the full pipeline and return manifest JSON values.
    fn run_scenario(wasm_fixture: &str, num_ticks: u64) -> Vec<serde_json::Value> {
        let mut world = World::new();
        world.register_component::<Position>("position");
        let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });
        assert_eq!(
            entity.to_raw(),
            0,
            "first entity should have raw id 0"
        );

        let mut manifest = ManifestPipeline::new();
        let config = WasmConfig::default();
        let bytes = fixture_bytes(wasm_fixture);
        let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

        let mut manifests = Vec::new();
        for tick in 1..=num_ticks {
            let (tick_manifest, _) = crate::integration::run_wasm_tick(
                &mut module,
                &mut world,
                &mut manifest,
                tick,
                tick as f64 / 60.0,
            )
            .unwrap();

            let json = serde_json::to_value(&tick_manifest).unwrap();
            manifests.push(json);
        }

        manifests
    }

    // -- B8 Test 1: Export correct manifests to JSON -------------------------

    #[test]
    fn b8_export_correct_manifests() {
        let manifests = run_scenario("correct_movement.wat", 5);

        // Write to JSON files for Python consumption.
        let output_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("b8_manifests")
            .join("correct");
        std::fs::create_dir_all(&output_dir).unwrap();

        for (i, manifest) in manifests.iter().enumerate() {
            let path = output_dir.join(format!("tick_{}.json", i + 1));
            let json_str = serde_json::to_string_pretty(manifest).unwrap();
            std::fs::write(&path, json_str).unwrap();
        }

        // Basic sanity: manifests should have component changes.
        for manifest in &manifests {
            let changes = manifest["component_changes"]
                .as_array()
                .expect("component_changes should be an array");
            assert!(
                !changes.is_empty(),
                "each tick should have component changes"
            );

            // Position should have x > 0 (correct behavior: moving toward target).
            let pos_change = changes
                .iter()
                .find(|c| c["component_type_name"] == "position")
                .expect("should have position change");
            let new_value = &pos_change["new_value"];
            let x = new_value["x"]
                .as_f64()
                .expect("new_value.x should be a number");
            assert!(
                x > 0.0,
                "correct movement: x should be positive, got {x}"
            );
        }
    }

    // -- B8 Test 2: Export buggy manifests to JSON ---------------------------

    #[test]
    fn b8_export_buggy_manifests() {
        let manifests = run_scenario("buggy_movement.wat", 5);

        // Write to JSON files for Python consumption.
        let output_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("b8_manifests")
            .join("buggy");
        std::fs::create_dir_all(&output_dir).unwrap();

        for (i, manifest) in manifests.iter().enumerate() {
            let path = output_dir.join(format!("tick_{}.json", i + 1));
            let json_str = serde_json::to_string_pretty(manifest).unwrap();
            std::fs::write(&path, json_str).unwrap();
        }

        // Sanity: buggy manifests have position with x < 0.
        for manifest in &manifests {
            let changes = manifest["component_changes"]
                .as_array()
                .expect("component_changes should be an array");
            let pos_change = changes
                .iter()
                .find(|c| c["component_type_name"] == "position")
                .expect("should have position change");
            let new_value = &pos_change["new_value"];
            let x = new_value["x"]
                .as_f64()
                .expect("new_value.x should be a number");
            assert!(
                x < 0.0,
                "buggy movement: x should be negative, got {x}"
            );
        }
    }

    // -- B8 Test 3: Correct and buggy have different causality ---------------

    #[test]
    fn b8_correct_and_buggy_have_different_causality() {
        let correct = run_scenario("correct_movement.wat", 3);
        let buggy = run_scenario("buggy_movement.wat", 3);

        // Both should have component changes.
        assert!(!correct.is_empty());
        assert!(!buggy.is_empty());

        // Both should have WASM_GAMEPLAY as the system.
        for manifest in correct.iter().chain(buggy.iter()) {
            let changes = manifest["component_changes"]
                .as_array()
                .expect("component_changes should be an array");
            for change in changes {
                // SystemId(100) serializes as the integer 100 via serde newtype.
                let changed_by = &change["changed_by"];
                let sys_id = changed_by.as_u64().unwrap_or_else(|| {
                    // Newtype struct: {"0": 100}
                    changed_by
                        .get("0")
                        .and_then(|v| v.as_u64())
                        .expect("changed_by should contain SystemId value")
                });
                assert_eq!(
                    sys_id, 100,
                    "all changes should be from WASM_GAMEPLAY (SystemId=100)"
                );
            }
        }

        // The reason strings should differ between correct and buggy.
        let correct_reason = &correct[0]["component_changes"][0]["reason"];
        let buggy_reason = &buggy[0]["component_changes"][0]["reason"];
        assert_ne!(
            correct_reason, buggy_reason,
            "correct and buggy should have different reason strings"
        );
    }

    // -- B8 Test 4: Manifest JSON roundtrip through Python format -----------

    #[test]
    fn b8_manifest_json_has_required_fields() {
        let manifests = run_scenario("correct_movement.wat", 1);
        let manifest = &manifests[0];

        // Verify all fields required by Python TickManifest.from_json are present.
        assert!(manifest.get("tick").is_some(), "manifest must have 'tick'");
        assert!(
            manifest.get("sim_time").is_some(),
            "manifest must have 'sim_time'"
        );
        assert!(
            manifest.get("entity_spawns").is_some(),
            "manifest must have 'entity_spawns'"
        );
        assert!(
            manifest.get("entity_despawns").is_some(),
            "manifest must have 'entity_despawns'"
        );
        assert!(
            manifest.get("component_changes").is_some(),
            "manifest must have 'component_changes'"
        );
        assert!(
            manifest.get("events").is_some(),
            "manifest must have 'events'"
        );
        assert!(
            manifest.get("aggregates").is_some(),
            "manifest must have 'aggregates'"
        );
        assert!(
            manifest.get("systems_executed").is_some(),
            "manifest must have 'systems_executed'"
        );
        assert!(
            manifest.get("commands_processed").is_some(),
            "manifest must have 'commands_processed'"
        );
        assert!(
            manifest.get("commands_succeeded").is_some(),
            "manifest must have 'commands_succeeded'"
        );

        // Verify component change has all required fields for Python parsing.
        let change = &manifest["component_changes"][0];
        assert!(
            change.get("entity_id").is_some(),
            "change must have 'entity_id'"
        );
        assert!(
            change.get("component_type_name").is_some(),
            "change must have 'component_type_name'"
        );
        assert!(
            change.get("new_value").is_some(),
            "change must have 'new_value'"
        );
        assert!(
            change.get("changed_by").is_some(),
            "change must have 'changed_by'"
        );
        assert!(
            change.get("reason").is_some(),
            "change must have 'reason'"
        );
        assert!(
            change.get("command_index").is_some(),
            "change must have 'command_index'"
        );
        assert!(
            change.get("tick").is_some(),
            "change must have 'tick'"
        );

        // Verify the reason is in the expected serde enum format.
        let reason = &change["reason"];
        assert!(
            reason.get("GameRule").is_some(),
            "reason should be {{\"GameRule\": \"...\"}} format, got: {reason}"
        );
    }
}
