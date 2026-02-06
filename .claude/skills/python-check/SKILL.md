---
name: python-check
description: Run the full Python type check and test cycle. Use after any Python code changes.
---

# Python Check

Run the full Python quality gate. Execute these commands in order and report results:

## Steps

1. **Type check** (pyright strict):
   ```bash
   cd python/nomai-sdk && pyright --pythonversion 3.12
   ```

2. **Tests** (pytest):
   ```bash
   cd python/nomai-sdk && pytest -v
   ```

## On Failure

- **Type check**: Fix the type annotation. Do not use `# type: ignore` unless there's a documented reason (e.g., PyO3 limitation).
- **Tests**: Read the failure, fix the code, re-run. Do not delete failing tests.

## Report

After all steps pass, report:
```
Python check: PASS
  pyright: ok (0 errors, 0 warnings)
  pytest: ok (N passed, 0 failed)
```
