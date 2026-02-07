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
