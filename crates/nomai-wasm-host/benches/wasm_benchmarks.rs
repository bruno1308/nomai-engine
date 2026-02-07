//! B9: WASM Overhead Benchmark for Spike B gate evaluation.
//!
//! Benchmarks the WASM host call overhead against the Spike B acceptance and
//! kill criteria:
//!
//! - **Acceptance:** 50 host calls per tick must complete in <1ms.
//! - **Kill:** 50 host calls >1ms -> spike FAILS, drop WASM, go native.
//!
//! Also measures:
//! - WASM vs native comparison ratio (target: <5x, kill: >10x)
//! - Noop tick baseline (measures raw WASM call overhead without host calls)
//! - Hot-swap latency (target: <100ms, kill: >500ms)
//!
//! Run with: `cargo bench --bench wasm_benchmarks`

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nomai_ecs::command::{CausalReason, CommandBuffer};
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::SystemId;
use nomai_wasm_host::{WasmConfig, WasmModule};

/// Load a WAT/WASM fixture from the tests/fixtures directory.
fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {}: {}", path.display(), e))
}

// ---------------------------------------------------------------------------
// Benchmark 1: 50 WASM host calls per tick (kill criterion)
// ---------------------------------------------------------------------------

/// Measures the time for a WASM module to make exactly 50 host calls per tick:
/// 25 read calls (get_entity_count, tick_number) + 25 write calls (set_component).
///
/// This is the primary kill-criterion benchmark for Spike B.
/// Budget: <1ms. Kill: >1ms.
fn bench_wasm_50_host_calls(c: &mut Criterion) {
    let bytes = fixture_bytes("bench_50_calls.wat");
    let config = WasmConfig::default();
    let mut module = WasmModule::from_bytes(&config, &bytes)
        .expect("bench_50_calls.wat should compile and instantiate");

    c.bench_function("wasm_50_host_calls", |b| {
        b.iter(|| {
            module.host_state_mut().begin_tick(1, 0.016);
            module.call_tick().expect("tick should not trap");
            // Drain commands to prevent unbounded growth across iterations.
            let cmds = module.drain_commands();
            black_box(cmds.len());
        });
    });
}

// ---------------------------------------------------------------------------
// Benchmark 2: Native equivalent of 50 operations (comparison baseline)
// ---------------------------------------------------------------------------

/// Performs the same logical operations as the WASM benchmark but in pure Rust.
///
/// This gives us the WASM-vs-native slowdown ratio.
/// Target: <5x slowdown. Kill: >10x slowdown.
fn bench_native_equivalent_50_ops(c: &mut Criterion) {
    c.bench_function("native_equivalent_50_ops", |b| {
        b.iter(|| {
            let mut cmd_buf = CommandBuffer::new();
            let entity_count: usize = 10;
            let tick: u64 = 1;

            // 25 "read" operations (simulated -- just access the values).
            for i in 0..25 {
                if i % 2 == 0 {
                    let _ = black_box(entity_count);
                } else {
                    let _ = black_box(tick);
                }
            }

            // 25 "write" operations -- build SetComponent commands, matching
            // what the WASM module does via host_set_component.
            for _ in 0..25 {
                cmd_buf.set_component(
                    EntityId::from_raw(0),
                    "health",
                    serde_json::json!(42),
                    SystemId::WASM_GAMEPLAY,
                    CausalReason::GameRule("bench_reason".to_owned()),
                );
            }

            black_box(cmd_buf.len());
        });
    });
}

// ---------------------------------------------------------------------------
// Benchmark 3: Noop tick (WASM call overhead baseline)
// ---------------------------------------------------------------------------

/// Measures the overhead of just calling into WASM and returning, with no host
/// calls at all. This isolates the Wasmtime call/return boundary cost.
fn bench_wasm_noop_tick(c: &mut Criterion) {
    let bytes = fixture_bytes("noop.wat");
    let config = WasmConfig::default();
    let mut module =
        WasmModule::from_bytes(&config, &bytes).expect("noop.wat should compile and instantiate");

    c.bench_function("wasm_noop_tick", |b| {
        b.iter(|| {
            module.host_state_mut().begin_tick(1, 0.016);
            let fuel = module.call_tick().expect("noop tick should not trap");
            black_box(fuel);
        });
    });
}

// ---------------------------------------------------------------------------
// Benchmark 4: Hot-swap latency
// ---------------------------------------------------------------------------

/// Measures the time to swap a running WASM module for a new one.
///
/// Target: <100ms. Kill: >500ms.
fn bench_wasm_hot_swap(c: &mut Criterion) {
    let v1_bytes = fixture_bytes("noop.wat");
    let v2_bytes = fixture_bytes("bench_50_calls.wat");
    let config = WasmConfig::default();

    c.bench_function("wasm_hot_swap", |b| {
        // We need a fresh module for each iteration because swap replaces
        // the instance in-place. We pre-create v1 once and swap to v2 each
        // iteration, then swap back.
        let mut module =
            WasmModule::from_bytes(&config, &v1_bytes).expect("noop.wat should compile");

        b.iter(|| {
            module.swap(&v2_bytes).expect("swap to v2 should succeed");
            module
                .swap(&v1_bytes)
                .expect("swap back to v1 should succeed");
        });
    });
}

// ---------------------------------------------------------------------------
// Benchmark 5: 50 host calls with world snapshot (realistic scenario)
// ---------------------------------------------------------------------------

/// Same as benchmark 1 but with a populated world snapshot in host state,
/// simulating a real game scenario where WASM reads from actual entity data.
fn bench_wasm_50_calls_with_snapshot(c: &mut Criterion) {
    let bytes = fixture_bytes("bench_50_calls.wat");
    let config = WasmConfig::default();
    let mut module = WasmModule::from_bytes(&config, &bytes)
        .expect("bench_50_calls.wat should compile and instantiate");

    // Build a snapshot with 100 entities, each with health and position.
    let mut snapshot = std::collections::HashMap::new();
    for i in 0u64..100 {
        let mut components = std::collections::HashMap::new();
        components.insert("health".to_owned(), serde_json::json!(100));
        components.insert(
            "position".to_owned(),
            serde_json::json!({"x": i as f64, "y": 0.0}),
        );
        snapshot.insert(i, components);
    }

    c.bench_function("wasm_50_calls_with_snapshot", |b| {
        b.iter(|| {
            module
                .host_state_mut()
                .snapshot_world(snapshot.clone(), 100);
            module.host_state_mut().begin_tick(1, 0.016);
            module.call_tick().expect("tick should not trap");
            let cmds = module.drain_commands();
            black_box(cmds.len());
        });
    });
}

// ---------------------------------------------------------------------------
// Criterion groups and main
// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_wasm_50_host_calls,
    bench_native_equivalent_50_ops,
    bench_wasm_noop_tick,
    bench_wasm_hot_swap,
    bench_wasm_50_calls_with_snapshot,
);
criterion_main!(benches);
