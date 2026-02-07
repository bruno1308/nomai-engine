# Nomai Engine -- build and development tasks

# Build the entire workspace
build:
    cargo build --workspace

# Run tests with cargo-nextest (preferred)
test:
    @cargo nextest run --workspace 2>/dev/null || (echo "nextest not installed, falling back to cargo test" && cargo test --workspace)

# Run tests with standard cargo test (fallback)
test-cargo:
    cargo test --workspace

# Run all benchmarks
bench:
    cargo bench --workspace

# Lint with clippy, deny all warnings
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Format all code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Full CI pipeline: format check, lint, test
ci: fmt-check clippy test

# Quick workspace-wide type/syntax check (no codegen)
check:
    cargo check --workspace

# Install gameplay AssemblyScript dependencies
install-gameplay:
    cd gameplay && npm install

# Compile AssemblyScript gameplay to WASM
build-gameplay:
    cd gameplay && npm run build

# Compile AssemblyScript gameplay to WASM (debug mode with source maps)
build-gameplay-debug:
    cd gameplay && npm run build:debug

# Compile all WASM modules (fixed + buggy for demo)
build-gameplay-all:
    cd gameplay && npm run build:all

# Build the Python native extension (nomai._engine)
build-python:
    cargo build -p nomai-python --release

# Install Python extension into nomai-sdk (copies the built .pyd/.so)
install-python: build-python
    #!/usr/bin/env bash
    set -euo pipefail
    EXT_DIR="python/nomai-sdk/nomai"
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "win32" ]]; then
        cp target/release/_engine.dll "$EXT_DIR/_engine.pyd"
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        cp target/release/lib_engine.dylib "$EXT_DIR/_engine.so"
    else
        cp target/release/lib_engine.so "$EXT_DIR/_engine.so"
    fi
    echo "Installed native extension to $EXT_DIR"

# Run the breakout verification demo
demo: install-python build-gameplay-all
    python demo_breakout.py

# Full CI pipeline with Python tests
ci-full: ci install-python
    cd python/nomai-sdk && python -m pytest -x -q
