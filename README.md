# Nomai Engine

A game engine built for AI-driven development. The engine's primary output is not pixels -- it's a **manifest**: a structured, queryable, causal representation of the game world that AI can reason about.

The verification loop -- write code, run simulation, read manifest, verify intent, fix, repeat -- is the entire product.

## Architecture

```
Layer 3: Python orchestration       AI agent commands, verification loop, intent specs
Layer 2: WASM gameplay sandbox      AI-generated game logic (AssemblyScript)
Layer 1: Rust engine core           ECS, tick loop, physics, rendering, manifest pipeline
```

- **Rust** owns all stateful, performance-critical subsystems (ECS, physics, rendering, manifest).
- **WASM** sandboxes AI-generated gameplay logic via Wasmtime with fuel metering.
- **Python** is the AI's control surface -- manifest queries, verification, intent specs, engine control.

## Workspace Layout

```
crates/
  nomai-ecs/           Custom ECS with tiered entity identity
  nomai-manifest/      Manifest pipeline (change journal, event log, causality)
  nomai-engine/        Engine core (tick loop, command buffer, physics, renderer)
  nomai-wasm-host/     Wasmtime integration, gameplay host API
  nomai-python/        PyO3 bindings
gameplay/              AssemblyScript gameplay modules
python/nomai-sdk/      Python SDK (intent specs, verification engine)
```

## Prerequisites

| Tool | Version | Notes |
|------|---------|-------|
| Rust | 1.83.0 | Pinned via `rust-toolchain.toml` |
| Python | 3.12+ | For the SDK and demo |
| Node.js | 18+ | For AssemblyScript compilation |
| [just](https://github.com/casey/just) | 1.38+ | Task runner |

## Getting Started

```bash
# Build the Rust workspace
just build

# Install AssemblyScript dependencies and compile gameplay WASM
just install-gameplay
just build-gameplay-all

# Install Python SDK dependencies
cd python/nomai-sdk && pip install -e ".[dev]" && cd ../..

# Run the end-to-end breakout verification demo
just demo
```

## Development Commands

```bash
just build              # Build the Rust workspace
just test               # Run all tests (Rust + Python)
just test-rust          # Run Rust tests only
just test-python        # Run Python tests only
just bench              # Run criterion benchmarks
just clippy             # Lint with clippy
just fmt                # Format all Rust code
just ci                 # Full Rust CI: format check + clippy + tests
just ci-full            # Full CI: Rust CI + Python tests
just demo               # Run the breakout verification demo
just build-gameplay     # Compile AssemblyScript gameplay to WASM
just build-gameplay-all # Compile all WASM modules (fixed + buggy for demo)
```

## The Demo

`demo_breakout.py` runs an AI through the full write-verify-fix loop for a Breakout clone:

1. **Buggy run** -- simulation without collision response; verification detects failures from manifest data alone.
2. **Fixed run** -- manifest-driven collision response from Python; verification confirms the fix.
3. **Regression** -- saves the passing run as a regression baseline and replays it.
4. **Report** -- structured comparison of buggy vs. fixed, proving the verification thesis.

No pixels are inspected. All verification is manifest-based.

## License

MIT
