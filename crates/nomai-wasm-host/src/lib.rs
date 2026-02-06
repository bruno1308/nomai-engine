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
//! - **`WasmError`**: Error type covering compilation, missing exports, fuel
//!   exhaustion, and runtime traps.
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

mod module;

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
// Tests
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
