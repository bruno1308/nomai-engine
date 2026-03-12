# Nomai Engine Evaluation Framework Design

**Date:** 2026-03-12
**Status:** Proposed
**Author:** AI (Claude + Codex collaborative design)

---

## 1. Purpose

Measure how well the Nomai engine enables autonomous AI game development. The eval framework answers one question: **"Can AI build, verify, and fix games using this engine without human intervention?"**

This is NOT just about the text scene representation — it evaluates the whole engine across all four pillars of the Nomai thesis: Observability, Controllability, Reproducibility, and Autonomy.

## 2. North-Star Metric: CW-ZTVCR

**Complexity-Weighted Zero-Touch Verified Completion Rate**

A task run counts as success only if ALL conditions hold:
1. Intent verification suite passes
2. Replay hashes match on rerun (deterministic)
3. Zero human intervention
4. Converges within attempt budget (<=5 iterations)
5. Core performance gates met (manifest <5% frame budget)

**Formula:** `CW-ZTVCR = sum(w_i * success_i) / sum(w_i)`

Where `w_i` is game complexity weight and `success_i` is 1 if all conditions above hold for task `i`.

This single number is the best proxy for "how good is this engine at enabling autonomous AI game development."

## 3. Evaluation Dimensions

### 3.1 Observability — Manifest Fidelity

**Question:** Can AI reconstruct full game state from manifest output?

| Metric | Definition | Target |
|--------|-----------|--------|
| `manifest_change_recall` | Meaningful changes captured / ground-truth changes | >= 99.5% |
| `state_reconstruction_fidelity` | Entities correctly reconstructed from manifest diffs | 100% for Semantic entities |
| `root_cause_recoverability@3` | Failures where true root cause in first 3 causal steps | >= 90% |

**Implementation:** Compare manifest output against known ground-truth ECS states. Pure Python, zero LLM cost.

### 3.2 Controllability — API Effectiveness

**Question:** Can AI effectively drive the engine to desired states?

| Metric | Definition | Target |
|--------|-----------|--------|
| `api_capability_coverage` | Required action primitives exposed / required | >= 95% |
| `command_semantic_reliability` | Commands producing intended manifest delta / valid commands | >= 99% |
| `action_effect_latency_p95` | P95 ticks between command and observable effect | <= 1 tick |

**Implementation:** Issue commands via Python API, verify manifest deltas match expectations.

### 3.3 Reproducibility — Determinism

**Question:** Are simulations reliably reproducible?

| Metric | Definition | Target |
|--------|-----------|--------|
| `replay_hash_match_rate` | Matching checkpoints / total checkpoints | 100% |
| `snapshot_fidelity` | Restore+replay hash matches original trajectory | 100% |
| `cross_platform_drift_rate` | Divergent checkpoints across platforms | Post-MVP |

**Implementation:** Uses existing `engine.capture_snapshot()`, `engine.replay()`, `engine.state_hash()` APIs.

### 3.4 Verification — Reasoning Quality Over Manifest

**Question:** Can AI verify game behavior from manifest alone?

| Metric | Definition | Target |
|--------|-----------|--------|
| `intent_expressibility_coverage` | Benchmark rules expressible as IntentSpecs / total | >= 90% |
| `bug_detection_precision` | True positives / (TP + FP) on seeded bug corpus | >= 95% |
| `bug_detection_recall` | True positives / (TP + FN) on seeded bug corpus | >= 95% |
| `diagnosis_to_fix_success@2` | Bugs fixed in <=2 attempts using report only | >= 70% |

**Implementation:** Seeded bug corpus with known bugs and clean scenarios. Run verification engine, measure detection accuracy.

### 3.5 Autonomy — End-to-End Capability

**Question:** Can AI go from GDD to verified game without human help?

| Metric | Definition | Target |
|--------|-----------|--------|
| `cw_ztvcr` | North-star metric (see Section 2) | Track by tier |
| `convergence_median` | Median write-verify-fix iterations to green | <= 5 |
| `human_intervention_count` | Manual interventions per run | 0 |

**Implementation:** Full GDD-to-verified-game pipeline runs across complexity tiers.

### 3.6 Efficiency — Cross-Cutting Constraint

| Metric | Definition | Target |
|--------|-----------|--------|
| `manifest_overhead_pct` | Manifest generation time / frame budget | < 5% |
| `tokens_per_build` | Total LLM tokens consumed per verified build | Trend down |
| `time_to_first_verified_game` | Wall-clock from GDD input to first green | Track p50/p95 |

## 4. Seeded Bug Corpus

Six scenarios (5 bugs + 1 clean) for verification testing:

| Bug ID | Category | Severity | Description |
|--------|----------|----------|-------------|
| `ball_passes_through_paddle` | collision | critical | Ball crosses paddle Y without collision event |
| `score_not_incremented` | event | major | Brick despawned but score aggregate unchanged |
| `entity_wrong_position` | spawn | major | Entity position set to (0,0) instead of intended |
| `physics_body_missing` | physics | critical | Entity alive but position never changes |
| `brick_not_despawned` | lifecycle | major | Collision recorded but brick stays alive |
| `clean_scenario` | none | minor | Correctly working scenario (should NOT be flagged) |

## 5. Evaluation Levels

### Level 1 — Automated, fast, every CI run (zero LLM cost)
- Property-based tests verifying manifest invariants
- Seeded bug corpus detection precision/recall
- Determinism checks (replay hash matching)
- Manifest overhead benchmarks

### Level 2 — Per-session, medium cost
- Template-generated Scene QA (deterministic gating set)
- LLM-generated questions as fuzz layer (auto-validated)
- Action prediction tests
- Hidden held-out QA set to prevent overfitting

### Level 3 — Nightly/release regression (LLM-as-judge)
- G-Eval scoring on diagnostic quality
- Multi-hop spatial reasoning questions
- Cross-version regression comparison
- Canary for reasoning regressions, NOT a CI gate

### Level 4 — End-to-end autonomy benchmarks
- Graduated GDD complexity tiers
- Full write-verify-fix loop runs
- CW-ZTVCR computation

## 6. Anti-Gaming Safeguards

- Hidden test worlds and held-out QA sets
- Mutation testing (perturb game state, verify eval detects it)
- Adversarial negatives (incorrect manifests that should fail)
- Information density metric (penalize verbosity without added info)
- Anti-shortcut checks (same scores under entity reorder, paraphrase)

## 7. Implementation Priority

1. **Observability fidelity** + causal correctness + overhead gates
2. **Verification accuracy** on seeded bug corpus
3. **Controllability** reliability/latency under workload
4. **Reproducibility** (same-platform determinism + snapshot fidelity)
5. **End-to-end autonomy** benchmark across complexity tiers
6. **Efficiency** optimization and cross-platform consistency (post-MVP)

## 8. File Structure

```
python/nomai-sdk/nomai/eval/
  __init__.py          — Package exports
  metrics.py           — MetricResult, DimensionScore, EvalDimension
  report.py            — EvalReport with JSON serialization
  observability.py     — Manifest fidelity metrics
  controllability.py   — API effectiveness metrics
  reproducibility.py   — Determinism metrics
  verification.py      — Verification quality metrics
  autonomy.py          — End-to-end capability metrics
  bug_corpus.py        — Seeded bug scenarios
  runner.py            — EvalRunner orchestrator
tests/
  test_eval.py         — Framework self-tests
```

## 9. Key Types

All types mirror the existing Nomai SDK conventions: frozen dataclasses, `to_dict()`/`from_dict()` for JSON serialization, no `Any` in public APIs.

- `MetricResult` — single metric outcome with name, value, target, pass/fail
- `DimensionScore` — aggregated score for one dimension
- `EvalReport` — complete eval run output with CW-ZTVCR
- `SeededBug` — bug scenario with manifests and ground truth
- `TaskResult` — GDD-to-game task outcome for autonomy eval
- `EvalRunner` — orchestrator that produces `EvalReport`

## 10. References

- `NOMAI_ENGINE_v8_MVP.md` — Section 15 (Success Criteria)
- `CLAUDE.md` — Verification thesis, manifest-as-product principle
- ALFWorld (dual visual+text from shared state)
- GAMEBoT (sub-problem decomposition with ground truth)
- RPGBench (LLMs as game engines — state tracking metrics)
- SGBench (scene graph entity/relation metrics)
- TextWorld/TALES (observation sufficiency via task completion)
- G-Eval/DeepEval (LLM-as-judge with custom criteria)
