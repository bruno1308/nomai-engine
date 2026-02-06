# Nomai Engine MVP -- Implementation Plan

**Spec:** [NOMAI_ENGINE_v8_MVP.md](./NOMAI_ENGINE_v8_MVP.md)
**Goal:** Prove the verification thesis -- AI verifies its own game dev work through the manifest pipeline.
**Timeline:** 10-13 weeks (Phase 0: 4-5w, Phase 1: 6-8w)
**Deliverable:** `demo_breakout.py` runs the full write-verify-fix loop end-to-end without human intervention.

---

## Status Legend

- `[ ]` Not started
- `[~]` In progress
- `[x]` Done
- `[!]` Blocked
- `[K]` Killed (spike failed kill gate)

---

## Phase 0: Feasibility Spikes

Purpose: validate that the two riskiest assumptions hold before committing to the full build. Each spike has explicit kill criteria. If either spike fails, we stop and redesign.

---

### Spike A: ECS + Manifest (2-3 weeks)

**Question:** Can we generate a causal, queryable manifest from a custom ECS at <5% frame budget for 1K entities at 60Hz?

#### A1. Project Bootstrap
- [ ] Initialize cargo workspace (`nomai-engine`, `nomai-ecs`, `nomai-manifest`)
- [ ] Set up `rust-toolchain.toml` (1.83.0 stable)
- [ ] Set up `justfile` with `build`, `test`, `bench` targets
- [ ] Add core dependencies: `serde`, `bincode`, `serde_json`, `blake3`, `tracing`, `rand`, `rand_pcg`
- [ ] Set up `cargo-nextest` for test runner
- [ ] Basic CI: `just ci` runs check + test

**Acceptance:** `cargo build` succeeds, `just test` runs, workspace compiles clean.

#### A2. Archetype ECS Core
- [ ] Entity allocator with generational IDs (`EntityId = u64`, generation + index)
- [ ] Archetype storage (SoA-within-archetype): `DynamicArchetype` with `DynamicColumn`
- [ ] Component type registry (runtime registration with string name + schema)
- [ ] Spawn / despawn entities
- [ ] Insert / remove / get components
- [ ] Basic archetype queries (iterate entities matching component set)
- [ ] Unit tests: spawn 10K entities, query, modify, despawn

**Acceptance:** 10K entities spawn, query, modify, despawn with correct results. No unsafe outside of column storage.

#### A3. Tiered Entity Identity
- [ ] `IdentityTier` enum: `Semantic`, `Pooled`
- [ ] `EntityIdentity` struct: `entity_type`, `role`, `spawned_by`, `requirement_id`
- [ ] `PoolIdentity` struct: `pool_type`, `variant`
- [ ] `spawn_semantic()`, `spawn_pooled()` APIs -- tier is required, impossible to forget
- [ ] Identity stored as a built-in component on every entity
- [ ] Unit tests: spawn mixed tiers, verify identity retrieval

**Acceptance:** Cannot spawn an entity without declaring a tier. Identity queryable from entity ID.

#### A4. Command Buffer with Causality
- [ ] `Command` struct with: kind, target entity, data, `issued_by: SystemId`, `reason: CausalReason`
- [ ] `CausalReason` enum: `PlayerInput`, `CollisionResponse`, `GameRule`, `StateTransition`, `Timer`, `SystemInternal`
- [ ] `CommandBuffer`: append commands, apply in deterministic order at sync point
- [ ] Command application updates ECS state
- [ ] Unit tests: enqueue commands from multiple systems, verify deterministic application order

**Acceptance:** Commands applied in stable, deterministic order. Every command carries causality metadata.

#### A5. Fixed-Timestep Tick Loop
- [ ] `TickLoop` struct: fixed dt, tick counter, sim time
- [ ] System registry: ordered list of system functions
- [ ] Tick execution: run systems in order → apply command buffer → advance tick
- [ ] Deterministic: same initial state + same systems + same inputs = same final state
- [ ] Unit tests: run 100 ticks, verify tick counter, sim time, system execution order

**Acceptance:** Tick loop is deterministic. Running 100 ticks twice from same state produces identical results.

#### A6. Change Journal
- [ ] `ComponentChange` struct: entity, component type, old value, new value, `changed_by: SystemId`, command index
- [ ] Journal populated during command buffer application
- [ ] Journal cleared at start of each tick
- [ ] Query: changes by entity, by component type, by system
- [ ] Unit tests: modify components, verify journal captures every change with correct causality

**Acceptance:** Every component mutation appears in the journal with correct old/new values and causality.

#### A7. Manifest Generation
- [ ] `TickManifest` struct: tick, sim_time, entity_spawns, entity_despawns, component_changes, events, aggregates, systems_executed
- [ ] Entity index: maintained across ticks, tracks all Semantic entities with identity
- [ ] Change journal → manifest Layer 2 (component changes with causality)
- [ ] Event log → manifest Layer 3 (game events with involved entities and causality)
- [ ] Aggregate computation: count-by-type, sum-by-component, custom aggregates
- [ ] `CausalChain` assembly: walk back from change → command → reason → prior event
- [ ] Manifest serialization to JSON (`serde_json`)
- [ ] Unit tests: run 10 ticks with spawns/modifications/despawns, verify manifest is complete and correct

**Acceptance:** Manifest correctly represents all state changes with causal chains. JSON output is parseable and complete.

#### A8. Manifest Performance Benchmark
- [ ] Benchmark: 1K Semantic entities, 10% modified per tick, 60Hz
- [ ] Measure: manifest generation time per tick
- [ ] Measure: command buffer overhead from causality tagging
- [ ] Measure: change journal memory per tick
- [ ] Report: median, p95, p99 generation time

**Acceptance criteria:**
- Manifest generation: **<5% of 16.67ms** (~833us) at 1K Semantic entities
- Causality tagging overhead: **<30%** of base command buffer application time

**Kill criteria:**
- Manifest generation >10% of frame budget → spike FAILS
- Causality overhead >50% → redesign causality model (e.g., system-level only)

#### Spike A Gate
- [ ] **PASS/FAIL decision documented**
- [ ] Benchmark results saved to `benchmarks/spike_a.json`
- [ ] If PASS: proceed to Spike B
- [ ] If FAIL: document failure, evaluate alternatives (simpler manifest, no full causality, use existing ECS)

---

### Spike B: WASM Gameplay + Verification (2-3 weeks)

**Question:** Can a WASM gameplay module emit causally-tagged commands, and can a verification engine detect behavioral correctness from the manifest alone?

**Prerequisite:** Spike A passed.

#### B1. Wasmtime Integration
- [ ] Add `wasmtime` dependency (27.0.0)
- [ ] Basic WASM module loading: load `.wasm` binary, instantiate, call exported `tick()` function
- [ ] Fuel-based metering: configurable instruction budget per tick
- [ ] Memory limit: 16MB per module
- [ ] Sandbox: no filesystem, no network, no wall-clock access
- [ ] Unit tests: load a trivial WASM module, call tick(), verify fuel consumption

**Acceptance:** WASM module loads, executes with fuel metering, traps on budget exceeded.

#### B2. Gameplay Host API
- [ ] Host functions exposed to WASM: `get_position`, `get_component`, `query_entities`, `get_aggregate`
- [ ] Host functions for commands: `spawn`, `despawn`, `set_component`, `emit_event` -- each takes `reason: &str`
- [ ] Host utilities: `random_f32`, `sim_time`, `tick_number`, `log`
- [ ] Commands from WASM flow into the same command buffer as native systems, with `SystemId::WASM_GAMEPLAY`
- [ ] Reason strings from WASM mapped to `CausalReason::GameRule(reason)`
- [ ] Unit tests: WASM module reads entity state, emits commands, verify commands appear in buffer with causality

**Acceptance:** WASM module reads state and emits causally-tagged commands. Commands flow through the same pipeline as native commands.

#### B3. Hot-Swap
- [ ] Unload WASM module at tick boundary
- [ ] Load new WASM module, validate exports
- [ ] New module executes starting next tick
- [ ] No state migration (all state in ECS)
- [ ] Unit test: swap module mid-simulation, verify state continuity

**Acceptance:** Module swap takes <100ms. Simulation continues without state loss.

#### B4. AssemblyScript Compilation Pipeline
- [ ] Set up AssemblyScript project (`npm` + `asc`)
- [ ] AS host bindings: TypeScript declarations for host functions
- [ ] Compile AS → WASM → load in wasmtime
- [ ] Example AS gameplay module: reads entities, emits commands
- [ ] Measure: compile time, WASM binary size

**Acceptance:** AS gameplay module compiles in <500ms, loads and executes correctly.

#### B5. Causality Across WASM Boundary
- [ ] WASM-emitted commands carry reason strings → manifest causal chains
- [ ] Causal chain: manifest change → command → WASM reason → (optionally) triggering event
- [ ] Unit test: WASM module reacts to collision event, emits command → verify full causal chain in manifest

**Acceptance:** Causal chains are unbroken from manifest observation through WASM boundary to root cause.

#### B6. Intent Spec Data Structures (Python)
- [ ] Set up Python project (`nomai-sdk`), `pyproject.toml`, `pytest`
- [ ] `IntentSpec`, `EntityIntent`, `BehaviorIntent`, `MetricIntent`, `InvariantIntent` dataclasses
- [ ] Trigger expressions: `Collision`, `StateTransition`, `AggregateCondition`, `ComponentCondition`, `And`/`Or`
- [ ] Expected outcomes: `ComponentChanged`, `EntityDespawned`, `AggregateChanged`, `InState`, `All`
- [ ] Serialization: intent specs to/from JSON
- [ ] Unit tests: construct intent specs programmatically, serialize, deserialize

**Acceptance:** Intent specs can express all breakout verification assertions. Round-trip serialization works.

#### B7. Verification Engine
- [ ] `VerificationEngine.verify()`: runs simulation, checks intents against manifest
- [ ] Entity verification: existence, visibility, required components
- [ ] Behavior verification: wait for trigger, check expected outcome
- [ ] Metric verification: component value within range across all ticks
- [ ] Invariant verification: condition holds every tick
- [ ] `VerificationReport`: per-intent results, pass/fail, failure reason, causal chain, suggestion
- [ ] Unit tests with mock manifests: pass cases and fail cases

**Acceptance:** Verification engine correctly distinguishes correct from incorrect behavior using only manifest data.

#### B8. Integration Test: Intentional Failures
- [ ] Create a simple test scenario: 3 entities, 1 behavior (entity A approaches entity B)
- [ ] Write intent spec for the behavior
- [ ] Run with correct WASM gameplay: verification passes
- [ ] Run with buggy WASM gameplay (entity moves wrong direction): verification fails with correct diagnosis
- [ ] Verify: failure report includes causal chain explaining WHY the behavior was wrong

**Acceptance:** The verification engine detects the bug and provides a causal explanation that an AI could use to generate a fix.

#### B9. WASM Overhead Benchmark
- [ ] Benchmark: 50 host calls per tick (reads + commands) at 60Hz
- [ ] Measure: total WASM execution time per tick (host calls + guest computation)
- [ ] Compare: same logic as native Rust system

**Acceptance criteria:**
- 50 host calls/tick: **<1ms total WASM overhead**
- WASM vs native: **<5x slowdown** for equivalent logic

**Kill criteria:**
- >1ms for 50 host calls → spike FAILS
- Causality breaks across WASM boundary → spike FAILS

#### Spike B Gate
- [ ] **PASS/FAIL decision documented**
- [ ] Benchmark results saved to `benchmarks/spike_b.json`
- [ ] If PASS: proceed to Phase 1
- [ ] If FAIL: document failure, evaluate alternatives (native gameplay instead of WASM, simplified verification)

---

### Phase 0 Exit Gate
- [ ] Both spikes passed
- [ ] All benchmark results documented
- [ ] Kill criteria evaluated honestly
- [ ] **GO / NO-GO decision for Phase 1**

---

## Phase 1: MVP Build

**Prerequisite:** Phase 0 passed. Spike code becomes the foundation -- not thrown away, evolved.

Phase 1 takes the spike prototypes and builds them into the complete MVP. Each week has concrete deliverables and acceptance criteria.

---

### Week 1-2: ECS Core + Tick Loop (production quality)

Evolve Spike A code from prototype to production.

#### 1.1 Harden ECS
- [ ] Edge cases: component removal during iteration, despawn during iteration, archetype migration
- [ ] Error handling: meaningful errors for invalid entity IDs, missing components, type mismatches
- [ ] Archetype caching: query-to-archetype mappings cached and invalidated on structural changes
- [ ] Property tests (`proptest`): random sequences of spawn/despawn/insert/remove/query
- [ ] Documentation: rustdoc on all public types

#### 1.2 Harden Command Buffer
- [ ] Conflict detection: warn on multiple commands targeting same component in same tick
- [ ] Command validation: reject commands on despawned entities, invalid component types
- [ ] Batch command application: optimized for bulk operations
- [ ] Property tests: random command sequences preserve consistency

#### 1.3 Harden Tick Loop
- [ ] System dependency declaration (explicit ordering, not implicit)
- [ ] Tick timing diagnostics: per-system execution time tracked
- [ ] Headless mode: tick as fast as possible (no frame limiting)
- [ ] Replay input injection: tick loop accepts recorded inputs

#### 1.4 Milestone Test
- [ ] 10K entities, 5 systems, 1000 ticks: deterministic (hash match on re-run)
- [ ] All property tests pass with 10K cases

**Week 1-2 acceptance:** Production-quality ECS + tick loop with property tests passing.

---

### Week 3-4: Manifest Pipeline + Python Bindings

#### 3.1 Production Manifest Pipeline
- [ ] Entity index (Layer 1): maintained incrementally across ticks
- [ ] Change journal (Layer 2): captures all component mutations with full causality
- [ ] Event log (Layer 3): typed events with entity refs and causal chains
- [ ] Aggregate engine: configurable aggregates (count-by-type, sum-by-field, custom)
- [ ] Causal chain assembly: multi-hop chains (change → command → reason → triggering event → prior change)
- [ ] Manifest delta mode: only emit changes since last query
- [ ] Manifest history: rolling window of N ticks (configurable, default 60)
- [ ] Performance: verify <5% overhead at 1K entities (from Spike A benchmark, now in production code)

#### 3.2 PyO3 Bindings
- [ ] Set up `pyo3` + `maturin` build for `nomai-engine` Python module
- [ ] `NomaiEngine` Python class: `start()`, `shutdown()`, `tick()`, `run_ticks(n)`
- [ ] `TickManifest` Python class: `entity()`, `entities()`, `events()`, `changes()`, `aggregate()`
- [ ] `EntityView` Python class: `get_component()`, `has_component()`, `changed_this_tick()`, `visible`
- [ ] `ManifestRange` Python class: `component_over_time()`, `events_in_range()`, `first_tick_where()`
- [ ] Snapshot/restore: `capture_snapshot()`, `restore_snapshot()`
- [ ] Entity manipulation: `spawn_entity()`, `despawn_entity()`, `set_component()`
- [ ] WASM loading: `load_gameplay_wasm()`, `hot_swap_gameplay_wasm()`

#### 3.3 Python SDK Structure
- [ ] `nomai-sdk` package: `nomai.engine`, `nomai.manifest`, `nomai.intents`, `nomai.verify`
- [ ] Install via `pip install -e .` (editable mode for development)
- [ ] Type stubs for IDE support
- [ ] Basic test: Python creates engine, ticks 10 times, queries manifest

#### 3.4 Milestone Test
- [ ] Python script spawns entities, runs 100 ticks, queries manifest, verifies content
- [ ] Manifest JSON output validates against expected schema
- [ ] Causal chains traverse correctly in Python API

**Week 3-4 acceptance:** Python can fully control the engine and query manifests with causal chains.

---

### Week 4-5: Physics + WASM Sandbox

#### 4.1 rapier2d Integration
- [ ] Add `rapier2d` with `enhanced-determinism` feature
- [ ] Physics system: reads `Position`, `Velocity`, `RigidBody`, `Collider` components
- [ ] Physics step: fixed dt, deterministic
- [ ] Collision events → `GameEvent` with entity pairs, contact points, normal
- [ ] Collision events carry `CausalReason::CollisionResponse(entity_a, entity_b)`
- [ ] Position/velocity updates → commands with physics causality
- [ ] Unit tests: two bodies collide, verify collision event in manifest with correct causality

#### 4.2 Production WASM Sandbox
- [ ] Evolve Spike B wasmtime integration to production quality
- [ ] Host API error handling: meaningful errors for invalid entity IDs, missing components
- [ ] Module validation: verify exports (tick), verify imports match host API
- [ ] Crash handling: trap → log → skip for tick → notify
- [ ] Hot-swap at tick boundary (from Spike B, hardened)

#### 4.3 AS Gameplay Pipeline
- [ ] AssemblyScript project template with host bindings
- [ ] `just build-gameplay` compiles AS → WASM
- [ ] Example: breakout gameplay module in AS (paddle movement, ball physics response, brick destruction, scoring)
- [ ] Verify: AS module interacts correctly with rapier2d collision events via manifest

#### 4.4 Milestone Test
- [ ] Ball-paddle-brick scenario runs in headless mode
- [ ] Collision events appear in manifest with correct entity pairs
- [ ] AS gameplay module responds to collisions, emits commands, manifest shows results
- [ ] Full causal chain: player input → paddle move → ball collision → brick despawn → score change

**Week 4-5 acceptance:** Physics + WASM gameplay produce a manifest with full causal chains for collision scenarios.

---

### Week 5-6: Verification Engine + Intent Specs

#### 5.1 Production Intent Specs
- [ ] Evolve Spike B intent specs to production quality
- [ ] All trigger types: `Collision`, `StateTransition`, `AggregateCondition`, `ComponentCondition`, `EventOccurred`, `And`, `Or`, `After`
- [ ] All expected types: `ComponentChanged`, `EntityDespawned`, `AggregateChanged`, `InState`, `EventEmitted`, `All`, `Any`
- [ ] Intent spec validation: detect impossible triggers, warn on overly broad assertions
- [ ] Serialization: intents save to / load from JSON files (for regression tests)

#### 5.2 Production Verification Engine
- [ ] `verify()` runs full intent spec against simulation
- [ ] Per-intent timeout handling
- [ ] Structured report: pass/fail, trigger tick, manifest evidence, causal chain, failure reason
- [ ] `diagnosis()`: AI-readable summary of failures
- [ ] `suggested_fixes()`: heuristic suggestions (entity not found → add spawn, trigger never fired → check interaction)
- [ ] Regression test creation: intent + snapshot + input recording + expected hashes
- [ ] Regression test replay: deterministic re-run, verify same results

#### 5.3 Breakout Intent Spec
- [ ] Write the complete breakout intent spec (from v8 spec Section 11 example)
- [ ] Entity intents: paddle, ball, bricks
- [ ] Behavior intents: ball bounces off walls, ball bounces off paddle, brick destroyed on hit, game won
- [ ] Metric intents: ball speed range
- [ ] Invariant intents: ball in bounds, paddle in bounds

#### 5.4 Milestone Test
- [ ] Run breakout intent spec against correct gameplay: all pass
- [ ] Run against buggy gameplay (ball doesn't bounce): correct failure with causal diagnosis
- [ ] Run against buggy gameplay (bricks don't despawn): correct failure with causal diagnosis
- [ ] Regression test saved and replayed successfully

**Week 5-6 acceptance:** Verification engine correctly validates breakout behaviors and diagnoses failures.

---

### Week 6-7: Snapshot/Restore + Debug Renderer

#### 6.1 Production Snapshot/Restore
- [ ] Full world serialization via bincode + serde
- [ ] Snapshot includes: all entities, all components, tick counter, sim time, RNG state
- [ ] Restore to exact state: deterministic from snapshot forward
- [ ] Snapshot hash: BLAKE3 hash of serialized state (for replay verification)
- [ ] Snapshot branching: fork, run two branches, compare manifests
- [ ] Property tests: snapshot → restore → run N ticks == run N ticks from same starting point

#### 6.2 Deterministic Replay
- [ ] `ReplayLog`: initial snapshot + input recording + checkpoint hashes
- [ ] Record mode: capture inputs and checkpoint hashes during simulation
- [ ] Replay mode: restore snapshot, inject recorded inputs, verify checkpoint hashes match
- [ ] Replay failure: report first divergent tick with state diff
- [ ] Integration with verification: replay a recorded scenario + re-run intent spec assertions

#### 6.3 Debug 2D Renderer
- [ ] wgpu initialization: window, surface, device, queue
- [ ] Render colored rectangles (entities by type: paddle=blue, ball=white, bricks=colors)
- [ ] Basic text rendering (score display)
- [ ] Camera: fixed 2D orthographic
- [ ] Render loop: read ECS state, draw entities, present
- [ ] Headless/windowed toggle: renderer is optional, verification works without it
- [ ] Semantic art annotation: convention-based asset path parsing (from v8 spec Section 5)

#### 6.4 Milestone Test
- [ ] Save snapshot at tick 100, restore, run 200 more ticks: hash matches running straight through
- [ ] Record breakout session, replay, all checkpoint hashes match
- [ ] Windowed mode: human can see and play breakout (keyboard input for paddle)
- [ ] Headless mode: same simulation runs without window

**Week 6-7 acceptance:** Deterministic snapshot/restore/replay works. Debug renderer shows a playable breakout.

---

### Week 7-8: End-to-End Integration + Demo

#### 7.1 The Demo Script: `demo_breakout.py`
- [ ] Implement the full verification loop from v8 spec Section 11
- [ ] Step 1: Load a pre-written intent spec for breakout
- [ ] Step 2: Load a pre-written AS gameplay module (initial version with deliberate bugs)
- [ ] Step 3: Run verification → detect failures
- [ ] Step 4: (Simulated AI fix) Load corrected AS gameplay module
- [ ] Step 5: Re-run verification → all pass
- [ ] Step 6: Save regression tests
- [ ] Step 7: Run regression tests → all pass
- [ ] Output: structured report showing the full verification-fix cycle

#### 7.2 AI-Driven Demo (stretch goal)
- [ ] Connect to LLM API (Claude Opus 4.6)
- [ ] AI generates intent spec from text directive "make breakout"
- [ ] AI generates AS gameplay module
- [ ] AI reads verification report on failure
- [ ] AI generates fix from causal diagnosis
- [ ] Full loop runs without human intervention
- [ ] If LLM integration is flaky: document what works and what doesn't, proceed with scripted demo

#### 7.3 Polish
- [ ] README.md: project overview, how to build, how to run demo
- [ ] `just demo` runs the full demo
- [ ] `just test` runs all Rust + Python tests
- [ ] `just bench` runs performance benchmarks
- [ ] Clean up: remove dead code, fix warnings, format

#### 7.4 Final Acceptance
- [ ] `demo_breakout.py` completes the verification loop (scripted version at minimum)
- [ ] Human plays the breakout result and confirms it works
- [ ] All tests pass
- [ ] Performance targets met (from v8 spec Section 15)
- [ ] Total Rust LOC <10,000
- [ ] Total Python LOC <3,000

**Week 7-8 acceptance:** The thesis is proven or clearly disproven.

---

## Phase 1 Exit Gate

- [ ] `demo_breakout.py` runs end-to-end
- [ ] Verification engine detects intentional bugs and provides causal diagnosis
- [ ] Regression tests catch behavioral regressions
- [ ] Debug renderer shows playable breakout
- [ ] All performance targets met
- [ ] **THESIS VALIDATED / INVALIDATED decision documented**

---

## Decision Log

Track key decisions and their rationale as they come up during implementation.

| Date | Decision | Rationale | Alternatives Considered |
|------|----------|-----------|------------------------|
| | | | |

---

## Risk Register (Active)

Track risks as they materialize during implementation.

| Risk | Status | Impact | Action |
|------|--------|--------|--------|
| Manifest overhead >5% | Monitoring (Spike A) | Could require simpler manifest | Degrade causality to system-level |
| WASM host call overhead | Monitoring (Spike B) | Could require native gameplay | Drop WASM, use Luau or native Rust |
| PyO3 ergonomics | Not started | Could slow Python API | Fallback to JSON-over-FFI |
| rapier2d determinism | Not started | Could break replay | Same-platform only, fixed seed |
| AS compilation pipeline | Not started | Could be fragile | Fallback to Rust for gameplay WASM |

---

## Notes

- **Phase 0 code is not throwaway.** It evolves into Phase 1. Write spike code as if it will be kept, because it will be.
- **Test as you go.** Every task includes tests. Don't defer testing to "later."
- **Benchmark early.** Performance surprises are cheaper to find in week 2 than week 10.
- **Kill honestly.** If a spike fails its kill criteria, stop. Do not rationalize past it.
- **One thing at a time.** Resist the urge to build ahead. Week 3 code depends on week 1-2 being solid.
