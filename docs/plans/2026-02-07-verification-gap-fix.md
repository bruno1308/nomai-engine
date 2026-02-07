# Verification Gap Fix -- Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Close the verification gap that allowed a ball-not-bouncing-off-bricks bug to go undetected. Fix the physics bug, strengthen the verification engine's semantics, add VALUE_RELATION expected type, and implement Layer 0 automatic physics sanity checks.

**Architecture:** Four-layer fix — (1) fix the actual physics bug in rapier despawn timing, (2) fix existing verifier semantics bugs, (3) extend the DSL with relational assertions, (4) add automatic physics invariant derivation from entity configuration.

**Tech Stack:** Rust (rapier2d physics), Python (verification engine, intent DSL)

---

### Task 1: Fix physics deferred unregister

**Files:**
- Modify: `crates/nomai-engine/src/physics.rs`
- Modify: `crates/nomai-engine/examples/breakout_visual.rs`

**What to do:**

1. Add `pending_removals: Vec<u64>` field to `PhysicsWorld` struct.
2. Add `deferred_unregister(&mut self, entity_id: EntityId)` method that pushes to pending_removals.
3. At the START of `step()`, drain `pending_removals` and call the internal removal logic for each. This ensures the entity is present during the CURRENT step's solver pass (the one that follows the collision detection from the previous step) and removed before the next step.
4. Update `new()` to initialize `pending_removals: Vec::new()`.
5. Update `breakout_visual.rs` to use `deferred_unregister` instead of `unregister_entity` for bricks.
6. Convert the diagnostic tests (`ball_bounces_off_brick_no_despawn`, `ball_fails_bounce_when_brick_despawned_on_collision_tick`) into proper passing tests that verify the fix works.
7. Run: `cargo test -p nomai-engine` -- all must pass.

---

### Task 2: Add VALUE_RELATION expected type to DSL

**Files:**
- Modify: `python/nomai-sdk/nomai/intents.py`
- Modify: `python/nomai-sdk/nomai/verify.py`

**What to do:**

1. Add `VALUE_RELATION = "value_relation"` to `ExpectedType` enum.
2. Add constructor function `value_relation(entity, component, field, relation)` where `relation` is one of: `"sign_flipped"`, `"magnitude_preserved"`, `"increased"`, `"decreased"`, `"changed_by_more_than"`.
3. In `verify.py` `_check_expected()`, add handler for `VALUE_RELATION` that:
   - Finds `ComponentChange` entries matching the component
   - Compares `old_value[field]` to `new_value[field]` using the specified relation
   - `sign_flipped`: `old * new < 0` (opposite signs, neither zero)
   - `magnitude_preserved`: `abs(new) ≈ abs(old)` within 10% tolerance
   - `increased`: `new > old`
   - `decreased`: `new < old`
4. Add tests in `python/nomai-sdk/tests/` for the new expected type.
5. Run: `cd python/nomai-sdk && python -m pytest -x -q`

---

### Task 3: Fix verifier semantics bugs

**Files:**
- Modify: `python/nomai-sdk/nomai/verify.py`

**What to do:**

1. Fix `ENTITY_DESPAWNED` (line 1041-1043): Currently checks `len(manifest.entity_despawns) > 0` -- matches ANY despawn. Fix to filter by the entity name param: match `entity_despawns` entries whose identity/type contains the entity name from `expected.params["entity"]`.
2. Fix `COMPONENT_CHANGED` (line 1020-1038): Currently passes if the field exists in new_value. Add a real delta check: require that `old_value` and `new_value` differ (if both are present). The current implementation counts rapier's per-tick position updates as "changed" even when the physics response hasn't happened.
3. Add entity scoping to `COMPONENT_CHANGED`: Use `expected.params["entity"]` to filter changes to only the specified entity (currently ignores entity param).
4. Update existing tests to match new stricter semantics.
5. Run: `cd python/nomai-sdk && python -m pytest -x -q`

---

### Task 4: Implement Layer 0 physics sanity checker

**Files:**
- Create: `python/nomai-sdk/nomai/physics_sanity.py`
- Modify: `python/nomai-sdk/nomai/verify.py`

**What to do:**

1. Create `PhysicsSanityChecker` class that takes a physics entity registry (dict mapping entity_id -> {body_type, restitution, collider_shape}).
2. Method `check_collision_response(manifests) -> list[IntentResult]`:
   - For each collision event in manifests involving a dynamic body with restitution > 0:
   - Find the dynamic entity's velocity ComponentChange within the next 3 ticks
   - Verify at least one velocity component changed sign (bounce)
   - If restitution == 1.0, verify speed magnitude is approximately preserved
3. Method `check_static_immobility(manifests) -> list[IntentResult]`:
   - Verify no static body has position/velocity changes (except via explicit sync)
4. Method `check_no_tunneling(manifests) -> list[IntentResult]`:
   - For dynamic bodies, verify position doesn't jump more than velocity*dt*2 per tick
5. Integrate into `VerificationEngine.verify()`: accept optional `physics_registry` param, run sanity checks alongside intent checks, append results to report.
6. Add tests.
7. Run: `cd python/nomai-sdk && python -m pytest -x -q`

---

### Task 5: Update breakout intents and demo

**Files:**
- Modify: `python/nomai-sdk/nomai/breakout_intents.py`
- Modify: `demo_breakout.py`

**What to do:**

1. Add `ball_reflects_on_brick_collision` intent using new `value_relation()`:
   ```python
   trigger=collision("ball", "brick"),
   expected=value_relation("ball", "velocity", "dy", "sign_flipped"),
   ```
2. Add `ball_reflects_on_wall_collision` intent similarly.
3. In `demo_breakout.py`, pass physics_registry to the verification engine so Layer 0 checks run.
4. Update expected pass/fail counts.
5. Run: `just demo` -- full demo must pass.

---

### Task 6: Cleanup and final verification

1. Clean up diagnostic test code in physics.rs (remove eprintln, convert to proper assertions).
2. Run full CI: `just ci` + `cd python/nomai-sdk && python -m pytest -x -q`
3. Commit with dual review (Claude Code + Codex).
