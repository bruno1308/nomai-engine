---
name: rust-check
description: Run the full Rust build, test, clippy, and format check cycle. Use after any Rust code changes.
---

# Rust Check

Run the full Rust quality gate. Execute these commands in order and report results:

## Steps

1. **Format check** (fail-fast):
   ```bash
   cargo fmt --all -- --check
   ```

2. **Clippy lint** (fail-fast):
   ```bash
   cargo clippy --workspace --all-targets -- -D warnings
   ```

3. **Build**:
   ```bash
   cargo build --workspace
   ```

4. **Tests** (via nextest):
   ```bash
   cargo nextest run --workspace
   ```

## On Failure

- **Format**: Run `cargo fmt --all` to fix, then re-check.
- **Clippy**: Fix the warning. Do not suppress with `#[allow(...)]` unless there's a documented reason.
- **Build**: Read the error, fix the code, re-run.
- **Tests**: Read the failure, fix the code, re-run. Do not delete failing tests.

## Report

After all steps pass, report:
```
Rust check: PASS
  fmt: ok
  clippy: ok (N warnings suppressed with reason)
  build: ok (N crates)
  tests: ok (N passed, 0 failed)
```
