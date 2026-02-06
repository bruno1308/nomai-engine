# Nomai Engine -- AI Development Context

> **One-line thesis:** Structured semantic access to rendered game state (via manifest) enables AI to verify its own game development work, closing the loop without human intervention.

## What This Project Is

Nomai is a game engine built for AI-driven development. The engine's primary output is not pixels -- it's a **manifest**: a structured, queryable, causal representation of the game world that AI can reason about. The verification loop (write code -> run simulation -> read manifest -> verify intent -> fix -> repeat) is the entire product.

**MVP deliverable:** `demo_breakout.py` runs an AI through the full write-verify-fix loop for a breakout clone, end-to-end, no human engineering intervention.

## Architecture

```
Layer 3: Python orchestration       (AI agent commands, verification loop, intent specs)
Layer 2: WASM gameplay sandbox      (AI-generated game logic ONLY -- AssemblyScript)
Layer 1: Rust engine core           (ECS, tick loop, physics, rendering, manifest pipeline)
```

- **Rust** owns all stateful, performance-critical subsystems (ECS, physics, rendering, manifest).
- **WASM** sandboxes only AI-generated gameplay logic (~10-50 host calls/tick).
- **Python** is the AI's control surface -- manifest queries, verification, intent specs, engine control.

## Tech Stack (pinned versions)

| Component | Version | Notes |
|-----------|---------|-------|
| Rust | 1.83.0 stable | `rust-toolchain.toml` |
| rapier2d | 0.22.0 | `enhanced-determinism` feature required |
| wgpu | 23.0.0 | Debug renderer only for MVP |
| winit | 0.30.8 | Windowing |
| wasmtime | 27.0.0 | Fuel metering enabled |
| AssemblyScript | 0.28.2 | Primary WASM target |
| Python | 3.12.8 | AI interface |
| PyO3 | 0.23.3 | Rust <-> Python FFI |
| serde_json | 1.0.134 | Manifest JSON output |
| bincode | 2.0.0-rc.3 | Binary serialization |
| blake3 | 1.5.5 | Content hashing |
| rand / rand_pcg | 0.8.5 / 0.3.1 | Deterministic RNG |
| tracing | 0.1.41 | Structured logging |
| just | 1.38.0 | Task runner |

## Workspace Structure

```
nomai/
  Cargo.toml              # Workspace root
  justfile                # Build/test/bench/demo targets
  crates/
    nomai-ecs/            # Custom ECS with tiered identity
    nomai-manifest/       # Manifest pipeline (change journal, event log, causality)
    nomai-engine/         # Engine core (tick loop, command buffer, physics, renderer)
    nomai-wasm-host/      # Wasmtime integration, gameplay host API
    nomai-python/         # PyO3 bindings
  gameplay/               # AssemblyScript gameplay modules
    assembly/             # AS source
    build/                # Compiled WASM output
  python/
    nomai-sdk/            # Python SDK (intent specs, verification engine)
      nomai/
        engine.py         # Engine control wrapper
        manifest.py       # Manifest query types
        intents.py        # Intent spec DSL
        verify.py         # Verification engine
  benchmarks/             # Criterion benchmarks + spike results
  tests/                  # Integration tests
  assets/                 # Game assets (sprites, etc.) with convention-based annotation
```

## Key Design Decisions

### The Manifest Is the Product
Every architectural choice is evaluated against: *does this make the manifest more useful for AI verification?* The manifest is co-equal with the framebuffer as engine output.

### Native Subsystems, Not WASM Plugins
v7 proposed WASM plugins for physics/audio/rendering. v8 keeps them native because:
- Physics needs 2,400-10,000 host calls/tick (WASM boundary overhead kills it)
- Stateful plugins (rapier, wgpu) can't be cleanly serialized across WASM
- Cross-language determinism breaks on f32/f64 promotion, HashMap ordering, FMA

### Every Mutation Carries Causality
All state changes flow through the command buffer. Every command is tagged with `SystemId` + `CausalReason`. This is non-negotiable -- it's what makes the manifest useful.

### Tiered Entity Identity
Entities declare identity at spawn:
- **Semantic**: Full manifest presence, full causality (player, enemies, items)
- **Pooled**: Type-level aggregation in manifest (bullets, coins, tiles)
- **Ephemeral**: Count only, no tracking (particles -- post-MVP)

### Deterministic Simulation
Same snapshot + same inputs + same WASM module = identical state at every tick. This enables replay, regression testing, and snapshot branching.

## Code Conventions

### Rust
- Edition 2021, stable toolchain only
- `#[deny(unsafe_code)]` at crate level except `nomai-ecs` column storage
- All public types have rustdoc
- Use `thiserror` for error types, `anyhow` only in binary/test code
- Prefer `&str` over `String` in function parameters
- Component types implement `serde::Serialize + serde::Deserialize + Clone`
- System functions are plain `fn` taking `&World` or `&mut World` -- no trait gymnastics
- Property tests (`proptest`) for all data structure invariants
- Benchmarks (`criterion`) for anything in the hot path

### Python
- Python 3.12+, strict `pyright` type checking
- Dataclasses for all data types (not dicts)
- `pytest` for testing with clear arrange/act/assert
- No `Any` types in public APIs -- everything typed
- Intent specs and verification reports are serializable to JSON

### AssemblyScript
- TypeScript-like syntax targeting WASM
- Host function bindings via `@external` decorators
- Every command emission includes a `reason` string -- this feeds causality
- No global mutable state -- all state lives in ECS components

### General
- No `println!` / `print()` -- use `tracing` (Rust) or `logging` (Python)
- Error messages must be actionable: say what went wrong AND what to do about it
- Tests are not optional. Every task includes tests. No "add tests later."
- Benchmark early. Performance surprises in week 2 cost less than in week 10.

## Agent Roles

This project uses specialized AI agents for parallel development. The orchestrator (you, reading this in the main session) **never writes code directly** -- it dispatches to specialists.

| Agent | Domain | Files They Own |
|-------|--------|----------------|
| `rust-engine` | ECS, tick loop, command buffer, engine glue | `crates/nomai-ecs/`, `crates/nomai-engine/` |
| `manifest-pipeline` | Manifest generation, change journal, event log, causality, queries | `crates/nomai-manifest/` |
| `wasm-sandbox` | Wasmtime host, gameplay API, AssemblyScript pipeline, hot-swap | `crates/nomai-wasm-host/`, `gameplay/` |
| `python-verification` | PyO3 bindings, Python SDK, intent specs, verification engine | `crates/nomai-python/`, `python/nomai-sdk/` |
| `renderer` | wgpu debug renderer, semantic art annotation | Renderer module in `crates/nomai-engine/src/render/` |
| `spike-validator` | Benchmarks, spike gate evaluation, performance validation | `benchmarks/` |

### Agent Rules
1. **Stay in your lane.** Only modify files in your domain. If you need a change in another domain, describe what you need and the orchestrator will dispatch it.
2. **Every mutation needs causality.** If you're adding code that modifies ECS state, it MUST go through the command buffer with a `CausalReason`. No backdoors.
3. **Test as you go.** No PR without tests. No "I'll add tests later."
4. **Benchmark the hot path.** If you're touching manifest generation, command application, or ECS queries, include a benchmark.
5. **JSON-serializable outputs.** If the manifest or verification engine produces it, it must serialize to JSON cleanly.

## Commit Review Policy (Mandatory)

**Every commit MUST pass dual review before being finalized.** No exceptions.

### The Two Reviewers

1. **Claude Code subagent** — Use the `superpowers:code-reviewer` agent (or a `general-purpose` agent with review instructions) to review the diff against the plan, coding standards, and acceptance criteria.
2. **Codex agent** — Use the `codex` skill to get peer feedback from OpenAI Codex CLI on the same diff.

### Process

```
Code written by specialist agent
    │
    ▼
Orchestrator stages the changes (DO NOT commit yet)
    │
    ├──► Claude Code review (subagent)
    │       - Checks: correctness, conventions, test coverage, anti-patterns
    │       - Returns: APPROVE or REQUEST_CHANGES with specifics
    │
    ├──► Codex review (skill)
    │       - Checks: code quality, bugs, design issues
    │       - Returns: APPROVE or REQUEST_CHANGES with specifics
    │
    ▼
Both approved?
    │
    ├── YES ──► Commit
    │
    └── NO ──► Address feedback
                │
                ├── Feedback is correct ──► Fix the code, re-review
                │
                └── Feedback is incorrect ──► Respond with reasoning
                     explaining WHY the feedback doesn't apply,
                     and re-submit for approval. Do NOT skip the
                     reviewer. Get their explicit approval even
                     if you disagree — provide the extra context
                     needed to convince them.
```

### Rules

- **No commits without both approvals.** Period.
- **Disagreement is fine, skipping is not.** If you believe a reviewer's feedback is wrong, explain your reasoning and re-request review. The reviewer may update their assessment with the new context.
- **Review scope matches commit scope.** Reviewers see the full diff of what's being committed, not just individual files.
- **Reviewers check against CLAUDE.md conventions.** The coding standards, anti-patterns, and performance budgets documented here are the review criteria.

## Anti-Patterns (Named for Memorability)

### The Backdoor Mutation
Modifying ECS state directly instead of through the command buffer. This breaks causality tracking and makes the manifest incomplete. **Every state change goes through commands.**

### The Pixel Peeking Fallacy
Trying to verify game behavior by looking at rendered output. The manifest exists so we never need to do this. If the manifest can't answer a verification question, the manifest needs to be improved -- not bypassed.

### The Premature Optimization
Optimizing before benchmarking. We have explicit performance budgets (manifest <5% of frame budget, WASM <1ms for 50 calls). Measure first, optimize against the budget, not against feelings.

### The Leaky Sandbox
WASM gameplay code accessing anything outside the host API. No filesystem, no network, no wall-clock time, no direct ECS access. Everything goes through the `GameplayHost` trait.

### The Silent Failure
Errors that log and continue without surfacing to the manifest or verification report. If something goes wrong during simulation, the manifest must reflect it. Silent swallowing of errors makes verification impossible.

### The Orphaned Entity
Spawning entities without identity tier declaration. Every entity must be Semantic or Pooled (Ephemeral post-MVP). The spawn API makes this impossible to forget by requiring the tier parameter.

### The Broken Chain
A causal chain that terminates at "SystemInternal" when it should trace back to a player input, game rule, or collision. Causal chains should be as deep as possible -- they're what makes the manifest useful for diagnosis.

## Development Phases

### Phase 0: Feasibility Spikes (4-5 weeks)
- **Spike A** (#1-#9): ECS + Manifest -- can we generate causal manifests at <5% frame budget?
- **Spike B** (#10-#19): WASM + Verification -- can verification work from manifest alone?
- **Gate** (#20): GO/NO-GO for Phase 1

### Phase 1: MVP Build (6-8 weeks)
- Week 1-2 (#21-#24): Harden ECS + tick loop
- Week 3-4 (#25-#28): Production manifest + Python bindings
- Week 4-5 (#29-#32): Physics + WASM sandbox
- Week 5-6 (#33-#36): Verification engine + intent specs
- Week 6-7 (#37-#40): Snapshot/restore + debug renderer
- Week 7-8 (#41-#45): End-to-end integration + demo

### Kill Criteria (be honest)
- Manifest generation >10% of frame budget -> redesign
- WASM host call overhead >1ms for 50 calls -> drop WASM, go native
- Causality overhead >50% of command application -> simplify to system-level causality

## Spec References

- **Full engine spec:** `NOMAI_ENGINE_v8_MVP.md` (authoritative, 1587 lines)
- **Implementation plan:** `NOMAI_MVP_PLAN.md` (task breakdown with acceptance criteria)
- **GitHub issues:** All 45 tasks tracked at https://github.com/bruno1308/nomai-engine/issues

## Quick Commands

```bash
just build          # cargo build --workspace
just test           # cargo nextest run + pytest
just bench          # criterion benchmarks
just build-gameplay # compile AS -> WASM
just demo           # run demo_breakout.py
just ci             # full check + test + clippy + fmt
```
