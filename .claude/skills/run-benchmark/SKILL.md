---
name: run-benchmark
description: Run criterion benchmarks and compare against performance budgets. Use to validate spike criteria or after performance-sensitive changes.
---

# Run Benchmarks

Execute criterion benchmarks and evaluate results against the documented performance budgets.

## Steps

1. **Run benchmarks**:
   ```bash
   cargo bench --workspace
   ```

2. **Evaluate against budgets**:

   ### Manifest Pipeline
   | Metric | Budget | Kill |
   |--------|--------|------|
   | Manifest generation (1K entities) | <833us (5% of 16.67ms) | >1.67ms |
   | Single entity query | <10us | >100us |
   | Full-tick scan (10K entities) | <1ms | >5ms |
   | Causality overhead | <30% of base | >50% |

   ### WASM Sandbox
   | Metric | Budget | Kill |
   |--------|--------|------|
   | 50 host calls/tick | <1ms | >1ms |
   | WASM vs native ratio | <5x | >10x |
   | Hot-swap time | <100ms | >500ms |

   ### Engine Core
   | Metric | Budget |
   |--------|--------|
   | Snapshot/restore (1K entities) | <50ms |
   | Verification run (6K ticks) | <10s wall-clock |

3. **Save results** (for spike gates):
   ```bash
   # Results auto-saved to target/criterion/
   # For spike gates, also save summary to benchmarks/spike_X.json
   ```

## Report

```
Benchmark results:
  manifest_generation: 420us median (budget: <833us) PASS
  entity_query: 3.2us median (budget: <10us) PASS
  causality_overhead: 18% (budget: <30%) PASS
  wasm_host_calls: 0.6ms (budget: <1ms) PASS

Overall: PASS (all within budget)
```

## On Budget Exceeded

If any metric exceeds its budget but is below the kill threshold:
- Flag it as a warning
- Profile the hot path to identify the bottleneck
- File a GitHub issue for optimization

If any metric hits the kill threshold:
- STOP. This is a spike failure.
- Document the result honestly in the spike gate report.
- Do not rationalize past it.
