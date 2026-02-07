# Phase 1, Week 1-2: Harden ECS + Tick Loop Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Evolve Spike A ECS, command buffer, and tick loop from prototype to production quality with property tests, edge case handling, query caching, conflict detection, system dependencies, timing diagnostics, and a 10K-entity determinism milestone test.

**Architecture:** Four parallel tracks harden independent subsystems: (1) ECS edge cases and query cache in `nomai-ecs`, (2) Command buffer validation in `nomai-ecs/command.rs`, (3) Tick loop features in `nomai-engine/tick.rs`, (4) Integration milestone test validating everything. Each track adds property tests (proptest) for 10K random sequences.

**Tech Stack:** Rust 1.83.0, proptest 1.x, criterion 0.5, blake3 1.5.5, serde_json

---

## Dependency Graph

```
Task 1 (ECS Edge Cases + Errors)     Task 3 (Command Buffer Hardening)     Task 4 (Tick Loop Hardening)
  │                                     │                                     │
  └── Task 2 (Query Cache)             │                                     │
        │                               │                                     │
        └───────────────────────────────┴─────────────────────────────────────┘
                                        │
                                  Task 5 (ECS Property Tests)
                                        │
                                  Task 6 (Milestone Test)

Parallel Groups:
  Group A: Task 1 + Task 3 + Task 4 (independent files)
  Group B: Task 2 (after Task 1, both touch world.rs)
  Group C: Task 5 (after Tasks 1-4, tests all hardened code)
  Group D: Task 6 (after Task 5)
```

## Agents

| Task | Agent | Files |
|------|-------|-------|
| Task 1 | `rust-engine` | `crates/nomai-ecs/src/lib.rs`, `crates/nomai-ecs/src/world.rs` |
| Task 2 | `rust-engine` | `crates/nomai-ecs/src/query.rs`, `crates/nomai-ecs/src/world.rs` |
| Task 3 | `rust-engine` | `crates/nomai-ecs/src/command.rs` |
| Task 4 | `rust-engine` | `crates/nomai-engine/src/tick.rs` |
| Task 5 | `rust-engine` | `crates/nomai-ecs/tests/proptest_ecs.rs`, `crates/nomai-ecs/tests/proptest_commands.rs` |
| Task 6 | `rust-engine` | `crates/nomai-engine/tests/milestone_week1_2.rs` |

---

## Task 1: ECS Edge Cases & Error Enrichment (Issue #21, Part 1)

**GitHub Issue:** #21

**Files:**
- Modify: `crates/nomai-ecs/src/lib.rs` (EcsError enum)
- Modify: `crates/nomai-ecs/src/world.rs` (error handling, edge cases)
- Tests in: `crates/nomai-ecs/src/world.rs` (mod tests)

### What to Build

1. **Enrich `EcsError` with more context:**
   - `StaleEntity` — add entity generation info: `"entity EntityId(gen=2, idx=5) does not exist (current generation: 3)"`
   - `UnknownComponent` — list registered component names: `"component 'foo' not registered. Registered: [position, velocity, health]"`
   - Add new variant `ComponentDeserializationError { component: String, details: String }` for clearer JSON deserialization failures

2. **Edge case: operations on despawned entities produce clear errors** (some already work — verify and test):
   - `get_component` on dead entity → returns `None` (already works)
   - `get_component_mut` on dead entity → returns `None` (already works)
   - `insert_component` on dead entity → returns `Err(StaleEntity)` (already works)
   - `remove_component` on dead entity → returns `Err(StaleEntity)` (already works)
   - Double despawn → returns `Err(StaleEntity)` (already works)

3. **Edge case: safe iteration patterns:**
   - Queries collect entity IDs into a Vec before yielding — no iterator invalidation during mutation
   - Test: despawn entities while iterating query results (collect first, then despawn)
   - Test: insert component on entity during iteration (collect first, then insert)
   - Document the pattern: "collect query results, then mutate" in rustdoc

### Step 1: Add richer error context

Modify `EcsError` in `crates/nomai-ecs/src/lib.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum EcsError {
    #[error("entity {entity:?} does not exist (stale or never allocated)")]
    StaleEntity {
        entity: entity::EntityId,
    },

    #[error("component type '{name}' not registered. Registered components: [{registered}]")]
    UnknownComponent {
        name: String,
        registered: String,
    },

    #[error("failed to deserialize component '{component}': {details}")]
    ComponentDeserializationError {
        component: String,
        details: String,
    },
}
```

**IMPORTANT:** This changes the error construction at every call site. Update all `EcsError::StaleEntity(entity)` → `EcsError::StaleEntity { entity }` and all `EcsError::UnknownComponent(name)` → `EcsError::UnknownComponent { name, registered: self.registry.registered_names().join(", ") }`.

You will also need to add a `registered_names()` method to `ComponentRegistry` in `crates/nomai-ecs/src/component.rs`:

```rust
/// Returns the names of all registered component types, sorted.
pub fn registered_names(&self) -> Vec<&str> {
    let mut names: Vec<&str> = self.name_to_id.keys().map(|s| s.as_str()).collect();
    names.sort();
    names
}
```

The `UnknownComponent` variant in `world.rs` is used both for truly unknown components AND for deserialization failures. Separate these: use `ComponentDeserializationError` for the deser case in `set_component_by_name`.

### Step 2: Write edge case tests

Add tests to `crates/nomai-ecs/src/world.rs` mod tests:

```rust
#[test]
fn unknown_component_error_lists_registered() {
    let world = setup_world();
    let e = world.spawn_with(Pos { x: 0.0, y: 0.0 });
    // "nonexistent" is not registered
    let result = world.set_component_by_name(e, "nonexistent", &serde_json::json!(42));
    let err = result.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("position"), "error should list registered components: {msg}");
    assert!(msg.contains("velocity"), "error should list registered components: {msg}");
}

#[test]
fn collect_then_despawn_pattern_safe() {
    let mut world = setup_world();
    let e1 = world.spawn_with(Pos { x: 1.0, y: 0.0 });
    let e2 = world.spawn_with(Pos { x: 2.0, y: 0.0 });
    let e3 = world.spawn_with(Pos { x: 3.0, y: 0.0 });

    // Collect query results FIRST, then despawn.
    let to_despawn: Vec<EntityId> = world.query::<(&Pos,)>()
        .filter(|(_, (pos,))| pos.x > 1.5)
        .map(|(e, _)| e)
        .collect();

    for e in &to_despawn {
        world.despawn(*e).unwrap();
    }

    assert_eq!(world.entity_count(), 1);
    assert!(world.is_alive(e1));
    assert!(!world.is_alive(e2));
    assert!(!world.is_alive(e3));
}

#[test]
fn collect_then_insert_pattern_safe() {
    let mut world = setup_world();
    let e1 = world.spawn_with(Pos { x: 1.0, y: 0.0 });
    let e2 = world.spawn_with(Pos { x: 2.0, y: 0.0 });

    // Collect, then mutate.
    let entities: Vec<EntityId> = world.query::<(&Pos,)>()
        .map(|(e, _)| e)
        .collect();

    for e in entities {
        world.insert_component(e, Vel { dx: 1.0, dy: 0.0 }).unwrap();
    }

    assert!(world.has_component::<Vel>(e1));
    assert!(world.has_component::<Vel>(e2));
}

#[test]
fn deserialization_error_is_clear() {
    let mut world = setup_world();
    let e = world.spawn_with(Pos { x: 0.0, y: 0.0 });
    // Pass invalid JSON for position (string instead of struct)
    let result = world.set_component_by_name(e, "position", &serde_json::json!("not a struct"));
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("position"), "error should mention component name: {msg}");
}
```

### Step 3: Run tests

Run: `cargo test -p nomai-ecs`

Expected: All 73+ existing tests pass, 4+ new tests pass. Since we changed error types, any test that pattern-matches on `EcsError::StaleEntity(id)` needs updating to `EcsError::StaleEntity { entity: id }`.

**IMPORTANT:** The error type change affects other crates too. Run `cargo test --workspace` to catch all breakage. Update pattern matches in `crates/nomai-wasm-host/` and `crates/nomai-engine/` if they destructure EcsError.

### Step 4: Commit

```
feat: enrich ECS error messages with context (#21)

- StaleEntity now includes entity ID details
- UnknownComponent lists all registered component names
- ComponentDeserializationError separates deser failures from unknown types
- Edge case tests: collect-then-despawn, collect-then-insert patterns
```

---

## Task 2: ECS Query Cache with Archetype Invalidation (Issue #21, Part 2)

**GitHub Issue:** #21

**Files:**
- Modify: `crates/nomai-ecs/src/query.rs` (add cache lookup)
- Modify: `crates/nomai-ecs/src/world.rs` (add cache storage, invalidation)
- Tests in: `crates/nomai-ecs/src/world.rs` or `crates/nomai-ecs/src/query.rs`

### What to Build

The current `matching_archetypes()` in `world.rs:708` iterates ALL archetypes every query:

```rust
pub(crate) fn matching_archetypes(&self, required: &[ComponentTypeId]) -> Vec<ArchetypeId> {
    self.archetypes
        .iter()
        .filter(|arch| required.iter().all(|req| arch.has_component(*req)))
        .map(|arch| arch.id())
        .collect()
}
```

This is O(archetypes * required_components) per query. For 100+ archetypes, this adds up.

**Solution:** Cache the mapping from `Vec<ComponentTypeId>` (sorted) → `Vec<ArchetypeId>`. Invalidate when new archetypes are created.

### Step 1: Add cache to World

In `crates/nomai-ecs/src/world.rs`, add a field to `World`:

```rust
pub struct World {
    // ... existing fields ...
    /// Cache: sorted component type set → matching archetype IDs.
    /// Invalidated when new archetypes are created.
    query_cache: HashMap<Vec<ComponentTypeId>, Vec<ArchetypeId>>,
    /// Monotonic counter incremented when archetypes change. Used for cache invalidation.
    archetype_generation: u64,
}
```

Initialize both in `World::new()`:
```rust
query_cache: HashMap::new(),
archetype_generation: 0,
```

### Step 2: Invalidate cache on archetype creation

In `get_or_create_archetype`, when a NEW archetype is created (not found in index):

```rust
fn get_or_create_archetype(&mut self, type_ids: &[ComponentTypeId]) -> ArchetypeId {
    if let Some(&id) = self.archetype_index.get(type_ids) {
        return id;
    }
    // ... create archetype ...
    self.archetype_generation += 1;
    self.query_cache.clear(); // Invalidate all cached queries
    id
}
```

### Step 3: Add cached lookup to matching_archetypes

```rust
pub(crate) fn matching_archetypes(&mut self, required: &[ComponentTypeId]) -> Vec<ArchetypeId> {
    // Check cache first.
    if let Some(cached) = self.query_cache.get(required) {
        return cached.clone();
    }

    let result: Vec<ArchetypeId> = self.archetypes
        .iter()
        .filter(|arch| required.iter().all(|req| arch.has_component(*req)))
        .map(|arch| arch.id())
        .collect();

    self.query_cache.insert(required.to_vec(), result.clone());
    result
}
```

**NOTE:** This changes `matching_archetypes` from `&self` to `&mut self`. This will break the query methods that currently call it with `&self`. You need to restructure the query to either:
- Use interior mutability (`RefCell` or similar) for the cache — simpler
- Or compute the cache key externally and pass it differently

**Recommended approach:** Use `RefCell<HashMap<...>>` for the cache to preserve the `&self` API:

```rust
use std::cell::RefCell;

pub struct World {
    // ... existing fields ...
    query_cache: RefCell<HashMap<Vec<ComponentTypeId>, Vec<ArchetypeId>>>,
}
```

Then `matching_archetypes` stays `&self`:
```rust
pub(crate) fn matching_archetypes(&self, required: &[ComponentTypeId]) -> Vec<ArchetypeId> {
    // Check cache.
    {
        let cache = self.query_cache.borrow();
        if let Some(cached) = cache.get(required) {
            return cached.clone();
        }
    }
    // Compute and cache.
    let result: Vec<ArchetypeId> = self.archetypes
        .iter()
        .filter(|arch| required.iter().all(|req| arch.has_component(*req)))
        .map(|arch| arch.id())
        .collect();

    self.query_cache.borrow_mut().insert(required.to_vec(), result.clone());
    result
}
```

Invalidation in `get_or_create_archetype` (which takes `&mut self`):
```rust
self.query_cache.borrow_mut().clear();
```

### Step 4: Write tests

```rust
#[test]
fn query_cache_returns_same_results() {
    let mut world = setup_world();
    for i in 0..100 {
        world.spawn_with(Pos { x: i as f32, y: 0.0 });
    }

    // First query: populates cache.
    let count1 = world.query::<(&Pos,)>().count();
    // Second query: hits cache.
    let count2 = world.query::<(&Pos,)>().count();
    assert_eq!(count1, count2);
    assert_eq!(count1, 100);
}

#[test]
fn query_cache_invalidated_on_new_archetype() {
    let mut world = setup_world();
    let e = world.spawn_with(Pos { x: 0.0, y: 0.0 });

    // Query with Pos only — cache populated.
    let count1 = world.query::<(&Pos,)>().count();
    assert_eq!(count1, 1);

    // Insert Vel — creates new archetype, invalidates cache.
    world.insert_component(e, Vel { dx: 1.0, dy: 0.0 }).unwrap();

    // Query with Pos should still find the entity (in new archetype).
    let count2 = world.query::<(&Pos,)>().count();
    assert_eq!(count2, 1);

    // Query with Pos+Vel should now find it.
    let count3 = world.query::<(&Pos, &Vel)>().count();
    assert_eq!(count3, 1);
}
```

### Step 5: Run tests

Run: `cargo test --workspace`
Expected: All tests pass. No regressions from cache refactoring.

### Step 6: Commit

```
feat: add query archetype cache with invalidation (#21)

- matching_archetypes() now caches query descriptor → archetype set
- Cache invalidated on new archetype creation
- Uses RefCell for interior mutability (preserves &self API)
```

---

## Task 3: Command Buffer Hardening (Issue #22)

**GitHub Issue:** #22

**Files:**
- Modify: `crates/nomai-ecs/src/command.rs` (conflict detection, validation)
- Tests in: `crates/nomai-ecs/src/command.rs` (mod tests)

### What to Build

1. **Conflict detection:** When `apply()` processes commands, detect if multiple commands target the same `(entity, component_name)` pair in the same batch. Log a `warn!` for each conflict (don't reject — last-write-wins is deterministic).

2. **Validation:** Before applying each command, check:
   - For SetComponent/RemoveComponent/Despawn: target entity must be alive. If not, mark `applied_successfully = false` and log warning (already partially works).
   - For SetComponent: component name must be registered. If not, mark as failed.

3. **`ConflictReport`:** Return conflict info alongside applied commands.

### Step 1: Add conflict detection to apply()

In `command.rs`, before the main apply loop, build a conflict map:

```rust
pub fn apply(&mut self, world: &mut World) -> Vec<Command> {
    let mut commands = std::mem::take(&mut self.commands);
    self.next_index = 0;

    // --- Conflict detection ---
    let mut seen: HashMap<(EntityId, String), Vec<u32>> = HashMap::new();
    for cmd in &commands {
        if let Some(target) = cmd.target {
            let component_name = match &cmd.kind {
                CommandKind::SetComponent { component_name, .. } => Some(component_name.clone()),
                CommandKind::RemoveComponent { component_name } => Some(component_name.clone()),
                _ => None,
            };
            if let Some(name) = component_name {
                seen.entry((target, name))
                    .or_default()
                    .push(cmd.command_index);
            }
        }
    }
    for ((entity, component), indices) in &seen {
        if indices.len() > 1 {
            tracing::warn!(
                entity = ?entity,
                component = %component,
                command_indices = ?indices,
                "conflict: {} commands target the same entity+component in this tick (last-write-wins)",
                indices.len()
            );
        }
    }

    // --- Apply loop (existing code, unchanged) ---
    for cmd in &mut commands {
        // ... existing apply logic ...
    }

    commands
}
```

### Step 2: Add conflict count to return

Add a new method and struct:

```rust
/// Summary of conflicts detected during the last `apply()` call.
#[derive(Debug, Clone, Default)]
pub struct ApplyReport {
    /// Number of (entity, component) pairs targeted by multiple commands.
    pub conflict_count: usize,
    /// Number of commands that failed to apply.
    pub failed_count: usize,
    /// Number of commands that applied successfully.
    pub success_count: usize,
}
```

Add a field to `CommandBuffer`:
```rust
pub struct CommandBuffer {
    commands: Vec<Command>,
    next_index: u32,
    last_apply_report: ApplyReport,
}
```

After `apply()`, populate the report. Add accessor:
```rust
pub fn last_apply_report(&self) -> &ApplyReport {
    &self.last_apply_report
}
```

### Step 3: Write tests

```rust
#[test]
fn conflict_detection_warns_on_duplicate_target() {
    let mut world = setup_world();
    let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

    let mut buf = CommandBuffer::new();
    // Two commands targeting same entity + component.
    buf.set_component(
        entity, "position", serde_json::json!({"x": 1.0, "y": 0.0}),
        SystemId(0), CausalReason::PlayerInput("move1".to_owned()),
    );
    buf.set_component(
        entity, "position", serde_json::json!({"x": 2.0, "y": 0.0}),
        SystemId(1), CausalReason::PlayerInput("move2".to_owned()),
    );

    let applied = buf.apply(&mut world);
    // Last-write-wins: position should be (2.0, 0.0).
    assert_eq!(world.get_component::<Position>(entity), Some(&Position { x: 2.0, y: 0.0 }));
    // Report should show 1 conflict.
    assert_eq!(buf.last_apply_report().conflict_count, 1);
}

#[test]
fn no_conflict_for_different_components() {
    let mut world = setup_world();
    let entity = world.spawn_with(Position { x: 0.0, y: 0.0 });

    let mut buf = CommandBuffer::new();
    buf.set_component(entity, "position", serde_json::json!({"x": 1.0, "y": 0.0}),
        SystemId(0), CausalReason::PlayerInput("move".to_owned()));
    buf.set_component(entity, "health", serde_json::json!(100),
        SystemId(1), CausalReason::GameRule("heal".to_owned()));

    let _applied = buf.apply(&mut world);
    assert_eq!(buf.last_apply_report().conflict_count, 0);
}

#[test]
fn apply_report_counts_success_and_failure() {
    let mut world = setup_world();
    let alive = world.spawn_with(Position { x: 0.0, y: 0.0 });
    let dead = world.spawn_with(Position { x: 1.0, y: 0.0 });
    world.despawn(dead).unwrap();

    let mut buf = CommandBuffer::new();
    buf.set_component(alive, "position", serde_json::json!({"x": 5.0, "y": 0.0}),
        SystemId(0), CausalReason::PlayerInput("move".to_owned()));
    buf.set_component(dead, "position", serde_json::json!({"x": 9.0, "y": 0.0}),
        SystemId(0), CausalReason::SystemInternal("stale".to_owned()));

    let _applied = buf.apply(&mut world);
    let report = buf.last_apply_report();
    assert_eq!(report.success_count, 1);
    assert_eq!(report.failed_count, 1);
}
```

### Step 4: Run tests

Run: `cargo test -p nomai-ecs`
Expected: All existing tests pass + new conflict/report tests pass.

Also run `cargo test --workspace` since command.rs is used by other crates.

### Step 5: Commit

```
feat: command buffer conflict detection and apply report (#22)

- Detect multiple commands targeting same (entity, component) per tick
- ApplyReport tracks conflict_count, success_count, failed_count
- Last-write-wins semantics preserved (conflicts are warnings, not errors)
```

---

## Task 4: Tick Loop Hardening (Issue #23)

**GitHub Issue:** #23

**Files:**
- Modify: `crates/nomai-engine/src/tick.rs` (system deps, timing, headless, replay)
- Tests in: `crates/nomai-engine/src/tick.rs` (mod tests)

### What to Build

1. **System dependency declaration:** Systems can declare `after: &[&str]` when registering. The tick loop validates no cycles exist. Execution order is topological sort respecting `after` constraints, with registration order as tiebreaker.

2. **Per-system timing diagnostics:** Each tick records wall-clock time per system. Accessible via `tick_diagnostics()`.

3. **Headless mode:** `TickConfig` gets `headless: bool` field. When true, no frame limiting (already the default — just formalize it).

4. **Replay input injection:** The tick loop accepts an optional `InputFrame` per tick. Systems can query the current frame's inputs.

### Step 1: System dependencies

Add dependency support to `RegisteredSystem`:

```rust
struct RegisteredSystem {
    name: String,
    func: SystemFn,
    after: Vec<String>,
}
```

Update `add_system` to accept dependencies:

```rust
/// Register a system with explicit execution dependencies.
///
/// `after` lists system names that must execute before this system.
/// The tick loop validates dependencies at registration time and
/// panics if a referenced system does not exist or would create a cycle.
pub fn add_system_after(&mut self, name: &str, after: &[&str], func: SystemFn) {
    // Validate all "after" systems exist.
    for dep in after {
        assert!(
            self.systems.iter().any(|s| s.name == *dep),
            "system '{name}' declares dependency on '{dep}', but '{dep}' is not registered"
        );
    }

    assert!(
        !self.systems.iter().any(|s| s.name == name),
        "duplicate system name: {name:?}"
    );

    self.systems.push(RegisteredSystem {
        name: name.to_owned(),
        func,
        after: after.iter().map(|s| s.to_string()).collect(),
    });

    // Validate no cycles after insertion.
    self.validate_system_order();
}

/// Original add_system still works (no dependencies).
pub fn add_system(&mut self, name: &str, func: SystemFn) {
    self.add_system_after(name, &[], func);
}
```

Add cycle detection:

```rust
fn validate_system_order(&self) {
    // Simple topological sort to detect cycles.
    let mut visited = vec![false; self.systems.len()];
    let mut in_stack = vec![false; self.systems.len()];

    fn dfs(
        systems: &[RegisteredSystem],
        idx: usize,
        visited: &mut [bool],
        in_stack: &mut [bool],
    ) -> bool {
        if in_stack[idx] { return false; } // cycle
        if visited[idx] { return true; }
        visited[idx] = true;
        in_stack[idx] = true;
        for dep_name in &systems[idx].after {
            if let Some(dep_idx) = systems.iter().position(|s| s.name == *dep_name) {
                if !dfs(systems, dep_idx, visited, in_stack) {
                    return false;
                }
            }
        }
        in_stack[idx] = false;
        true
    }

    for i in 0..self.systems.len() {
        assert!(
            dfs(&self.systems, i, &mut visited, &mut in_stack),
            "cycle detected in system dependencies"
        );
    }
}
```

### Step 2: Per-system timing diagnostics

Add timing to TickLoop:

```rust
use std::time::{Duration, Instant};

/// Timing diagnostics for the last tick.
#[derive(Debug, Clone, Default)]
pub struct TickDiagnostics {
    /// Wall-clock time per system (in order of execution).
    pub system_times: Vec<(String, Duration)>,
    /// Total time for the tick (systems + command apply).
    pub total_time: Duration,
    /// Time spent applying commands.
    pub command_apply_time: Duration,
}

pub struct TickLoop {
    // ... existing fields ...
    /// Diagnostics from the last tick.
    last_diagnostics: TickDiagnostics,
}
```

Modify `tick()` to record timing:

```rust
pub fn tick(&mut self) -> Vec<nomai_ecs::command::Command> {
    let tick_start = Instant::now();
    let mut system_times = Vec::with_capacity(self.systems.len());

    // Phase 1: Run systems with timing.
    for system in &self.systems {
        let sys_start = Instant::now();
        (system.func)(&self.world, &mut self.command_buffer);
        system_times.push((system.name.clone(), sys_start.elapsed()));
    }

    // Phase 2: Apply commands with timing.
    let apply_start = Instant::now();
    let applied = self.command_buffer.apply(&mut self.world);
    let command_apply_time = apply_start.elapsed();

    // Phase 3: Advance tick counter.
    self.tick_counter += 1;

    self.last_diagnostics = TickDiagnostics {
        system_times,
        total_time: tick_start.elapsed(),
        command_apply_time,
    };

    applied
}
```

Add accessor:
```rust
/// Diagnostics from the last tick (timing per system).
pub fn last_diagnostics(&self) -> &TickDiagnostics {
    &self.last_diagnostics
}
```

### Step 3: Headless mode + Replay input

Add to `TickConfig`:

```rust
#[derive(Debug, Clone)]
pub struct TickConfig {
    pub fixed_dt: f64,
    /// Headless mode: no rendering, tick as fast as possible.
    /// Default: false.
    pub headless: bool,
}

impl Default for TickConfig {
    fn default() -> Self {
        Self {
            fixed_dt: 1.0 / 60.0,
            headless: false,
        }
    }
}
```

Add input frame for replay:

```rust
/// A single frame of recorded input for replay.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct InputFrame {
    /// Arbitrary key-value pairs representing inputs for this tick.
    pub inputs: HashMap<String, serde_json::Value>,
}

pub struct TickLoop {
    // ... existing fields ...
    /// Current tick's input frame (set before tick() for replay).
    current_input: InputFrame,
}
```

Add methods:

```rust
/// Set the input frame for the next tick (used for replay).
pub fn set_input(&mut self, input: InputFrame) {
    self.current_input = input;
}

/// Read the current tick's input frame. Systems can access this
/// through the world or via a callback.
pub fn current_input(&self) -> &InputFrame {
    &self.current_input
}

/// Whether headless mode is enabled.
pub fn is_headless(&self) -> bool {
    self.config.headless
}
```

Store config in tick loop:

```rust
pub struct TickLoop {
    // ... existing fields ...
    config: TickConfig,
}
```

### Step 4: Write tests

```rust
#[test]
fn system_dependency_ordering() {
    let world = setup_world();
    let mut tick_loop = TickLoop::new(world, TickConfig::default());

    tick_loop.add_system("alpha", |_w, _c| {});
    tick_loop.add_system_after("beta", &["alpha"], |_w, _c| {});
    tick_loop.add_system_after("gamma", &["beta"], |_w, _c| {});

    assert_eq!(tick_loop.system_names(), vec!["alpha", "beta", "gamma"]);
}

#[test]
#[should_panic(expected = "not registered")]
fn system_dependency_on_missing_panics() {
    let world = setup_world();
    let mut tick_loop = TickLoop::new(world, TickConfig::default());
    tick_loop.add_system_after("beta", &["alpha"], |_w, _c| {}); // "alpha" not registered
}

#[test]
fn tick_diagnostics_records_timing() {
    let mut world = setup_world();
    world.spawn_with(Counter(0));
    let mut tick_loop = TickLoop::new(world, TickConfig::default());
    tick_loop.add_system("counter", counter_system);

    tick_loop.tick();

    let diag = tick_loop.last_diagnostics();
    assert_eq!(diag.system_times.len(), 1);
    assert_eq!(diag.system_times[0].0, "counter");
    assert!(diag.total_time > Duration::ZERO);
}

#[test]
fn headless_config() {
    let world = setup_world();
    let config = TickConfig { fixed_dt: 1.0 / 60.0, headless: true };
    let tick_loop = TickLoop::new(world, config);
    assert!(tick_loop.is_headless());
}

#[test]
fn input_frame_injection() {
    let world = setup_world();
    let mut tick_loop = TickLoop::new(world, TickConfig::default());

    let mut input = InputFrame::default();
    input.inputs.insert("move_x".to_string(), serde_json::json!(1.0));

    tick_loop.set_input(input.clone());
    assert_eq!(tick_loop.current_input().inputs.get("move_x"),
        Some(&serde_json::json!(1.0)));
}
```

### Step 5: Run tests

Run: `cargo test -p nomai-engine`
Expected: All 21 existing tests pass + 5+ new tests pass.

Also run `cargo test --workspace` for cross-crate compatibility.

### Step 6: Commit

```
feat: tick loop system dependencies, timing diagnostics, headless + replay (#23)

- add_system_after() for explicit execution ordering with cycle detection
- Per-system wall-clock timing in TickDiagnostics
- TickConfig.headless for formalized headless mode
- InputFrame injection for deterministic replay
```

---

## Task 5: ECS Property Tests (Issues #21 + #22)

**GitHub Issue:** #21, #22

**Files:**
- Create: `crates/nomai-ecs/tests/proptest_ecs.rs`
- Create: `crates/nomai-ecs/tests/proptest_commands.rs`

### What to Build

Property tests using `proptest` that run 10,000 random operation sequences and verify invariants hold. Two test files:

1. **proptest_ecs.rs** — Random spawn/despawn/insert/remove/query sequences
2. **proptest_commands.rs** — Random command buffer sequences

### Step 1: Create proptest_ecs.rs

```rust
//! Property tests for ECS operations.
//!
//! These tests use `proptest` to generate random sequences of ECS operations
//! and verify that world invariants hold after each sequence.

use nomai_ecs::prelude::*;
use proptest::prelude::*;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Pos { x: f32, y: f32 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Vel { dx: f32, dy: f32 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Tag(u32);

/// Operations we can perform on the world.
#[derive(Debug, Clone)]
enum EcsOp {
    SpawnPos(f32, f32),
    SpawnPosVel(f32, f32, f32, f32),
    Despawn(usize),       // index into alive entities
    InsertVel(usize, f32, f32),
    RemoveVel(usize),
    QueryPos,
    QueryPosVel,
}

fn ecs_op_strategy() -> impl Strategy<Value = EcsOp> {
    prop_oneof![
        (any::<f32>(), any::<f32>()).prop_map(|(x, y)| EcsOp::SpawnPos(x, y)),
        (any::<f32>(), any::<f32>(), any::<f32>(), any::<f32>())
            .prop_map(|(x, y, dx, dy)| EcsOp::SpawnPosVel(x, y, dx, dy)),
        (0..100usize).prop_map(EcsOp::Despawn),
        (0..100usize, any::<f32>(), any::<f32>())
            .prop_map(|(i, dx, dy)| EcsOp::InsertVel(i, dx, dy)),
        (0..100usize).prop_map(EcsOp::RemoveVel),
        Just(EcsOp::QueryPos),
        Just(EcsOp::QueryPosVel),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn ecs_random_ops_preserve_invariants(ops in prop::collection::vec(ecs_op_strategy(), 1..50)) {
        let mut world = World::new();
        world.register_component::<Pos>("pos");
        world.register_component::<Vel>("vel");

        let mut alive: Vec<EntityId> = Vec::new();

        for op in ops {
            match op {
                EcsOp::SpawnPos(x, y) => {
                    let e = world.spawn_with(Pos { x, y });
                    alive.push(e);
                }
                EcsOp::SpawnPosVel(x, y, dx, dy) => {
                    let mut b = ComponentBundle::new();
                    b.add(world.registry(), Pos { x, y });
                    b.add(world.registry(), Vel { dx, dy });
                    let e = world.spawn_bundle(b);
                    alive.push(e);
                }
                EcsOp::Despawn(idx) => {
                    if !alive.is_empty() {
                        let idx = idx % alive.len();
                        let e = alive.remove(idx);
                        let _ = world.despawn(e);
                    }
                }
                EcsOp::InsertVel(idx, dx, dy) => {
                    if !alive.is_empty() {
                        let idx = idx % alive.len();
                        let _ = world.insert_component(alive[idx], Vel { dx, dy });
                    }
                }
                EcsOp::RemoveVel(idx) => {
                    if !alive.is_empty() {
                        let idx = idx % alive.len();
                        let _ = world.remove_component::<Vel>(alive[idx]);
                    }
                }
                EcsOp::QueryPos => {
                    let count = world.query::<(&Pos,)>().count();
                    // Every alive entity has Pos.
                    prop_assert!(count <= alive.len());
                }
                EcsOp::QueryPosVel => {
                    let count = world.query::<(&Pos, &Vel)>().count();
                    prop_assert!(count <= alive.len());
                }
            }

            // Invariant: entity_count matches our tracking.
            prop_assert_eq!(world.entity_count(), alive.len());

            // Invariant: all alive entities are really alive.
            for &e in &alive {
                prop_assert!(world.is_alive(e));
            }
        }
    }
}
```

### Step 2: Create proptest_commands.rs

```rust
//! Property tests for command buffer operations.

use nomai_ecs::prelude::*;
use proptest::prelude::*;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Hp(u32);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Score(i64);

#[derive(Debug, Clone)]
enum CmdOp {
    SetHp(usize, u32),       // entity index, value
    SetScore(usize, i64),
    RemoveHp(usize),
    Despawn(usize),
    SpawnSemantic,
}

fn cmd_op_strategy() -> impl Strategy<Value = CmdOp> {
    prop_oneof![
        (0..20usize, any::<u32>()).prop_map(|(i, v)| CmdOp::SetHp(i, v)),
        (0..20usize, any::<i64>()).prop_map(|(i, v)| CmdOp::SetScore(i, v)),
        (0..20usize).prop_map(CmdOp::RemoveHp),
        (0..20usize).prop_map(CmdOp::Despawn),
        Just(CmdOp::SpawnSemantic),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn command_sequences_preserve_consistency(ops in prop::collection::vec(cmd_op_strategy(), 1..30)) {
        let mut world = World::new();
        world.register_component::<Hp>("hp");
        world.register_component::<Score>("score");

        // Spawn initial entities.
        let mut entities: Vec<EntityId> = Vec::new();
        for i in 0..5 {
            let e = world.spawn_with(Hp(100 + i));
            entities.push(e);
        }

        let mut buf = CommandBuffer::new();

        for op in &ops {
            match op {
                CmdOp::SetHp(idx, val) => {
                    if !entities.is_empty() {
                        let idx = idx % entities.len();
                        buf.set_component(
                            entities[idx], "hp", serde_json::json!(*val),
                            SystemId(0), CausalReason::SystemInternal("test".to_owned()),
                        );
                    }
                }
                CmdOp::SetScore(idx, val) => {
                    if !entities.is_empty() {
                        let idx = idx % entities.len();
                        buf.set_component(
                            entities[idx], "score", serde_json::json!(*val),
                            SystemId(0), CausalReason::SystemInternal("test".to_owned()),
                        );
                    }
                }
                CmdOp::RemoveHp(idx) => {
                    if !entities.is_empty() {
                        let idx = idx % entities.len();
                        buf.remove_component(
                            entities[idx], "hp",
                            SystemId(0), CausalReason::SystemInternal("test".to_owned()),
                        );
                    }
                }
                CmdOp::Despawn(idx) => {
                    if !entities.is_empty() {
                        let idx = idx % entities.len();
                        buf.despawn(
                            entities[idx],
                            SystemId(0), CausalReason::SystemInternal("test".to_owned()),
                        );
                    }
                }
                CmdOp::SpawnSemantic => {
                    buf.spawn_semantic(
                        EntityIdentity {
                            entity_type: "test".to_owned(),
                            role: "unit".to_owned(),
                            spawned_by: SystemId(0),
                            requirement_id: None,
                        },
                        vec![("hp".to_owned(), serde_json::json!(50))],
                        SystemId(0),
                        CausalReason::GameRule("spawn".to_owned()),
                    );
                }
            }
        }

        // Apply all commands.
        let applied = buf.apply(&mut world);

        // Invariant: every command has a valid command_index.
        for (i, cmd) in applied.iter().enumerate() {
            prop_assert_eq!(cmd.command_index, i as u32);
        }

        // Invariant: applied_successfully is set on every command.
        for cmd in &applied {
            // It's either true or false, but never left uninitialized.
            let _ = cmd.applied_successfully;
        }

        // Invariant: entity count >= 0 (no underflow).
        let _ = world.entity_count();

        // Invariant: all spawn commands have spawned_entity set.
        for cmd in &applied {
            if matches!(cmd.kind, CommandKind::SpawnSemantic { .. } | CommandKind::SpawnPooled { .. }) {
                if cmd.applied_successfully {
                    prop_assert!(cmd.spawned_entity.is_some());
                }
            }
        }
    }

    #[test]
    fn command_buffer_deterministic(ops in prop::collection::vec(cmd_op_strategy(), 1..20)) {
        // Run the same command sequence twice on identical worlds.
        // Results must be identical.
        fn run_once(ops: &[CmdOp]) -> Vec<bool> {
            let mut world = World::new();
            world.register_component::<Hp>("hp");
            world.register_component::<Score>("score");

            let mut entities = Vec::new();
            for i in 0..5 {
                entities.push(world.spawn_with(Hp(100 + i)));
            }

            let mut buf = CommandBuffer::new();
            for op in ops {
                match op {
                    CmdOp::SetHp(idx, val) => {
                        if !entities.is_empty() {
                            buf.set_component(entities[idx % entities.len()], "hp",
                                serde_json::json!(*val), SystemId(0),
                                CausalReason::SystemInternal("t".to_owned()));
                        }
                    }
                    CmdOp::SetScore(idx, val) => {
                        if !entities.is_empty() {
                            buf.set_component(entities[idx % entities.len()], "score",
                                serde_json::json!(*val), SystemId(0),
                                CausalReason::SystemInternal("t".to_owned()));
                        }
                    }
                    CmdOp::RemoveHp(idx) => {
                        if !entities.is_empty() {
                            buf.remove_component(entities[idx % entities.len()], "hp",
                                SystemId(0), CausalReason::SystemInternal("t".to_owned()));
                        }
                    }
                    CmdOp::Despawn(idx) => {
                        if !entities.is_empty() {
                            buf.despawn(entities[idx % entities.len()], SystemId(0),
                                CausalReason::SystemInternal("t".to_owned()));
                        }
                    }
                    CmdOp::SpawnSemantic => {
                        buf.spawn_semantic(
                            EntityIdentity { entity_type: "t".to_owned(), role: "u".to_owned(),
                                spawned_by: SystemId(0), requirement_id: None },
                            vec![("hp".to_owned(), serde_json::json!(50))],
                            SystemId(0), CausalReason::GameRule("s".to_owned()));
                    }
                }
            }

            buf.apply(&mut world).iter().map(|c| c.applied_successfully).collect()
        }

        let run1 = run_once(&ops);
        let run2 = run_once(&ops);
        prop_assert_eq!(run1, run2);
    }
}
```

### Step 3: Run tests

Run: `cargo test -p nomai-ecs -- --include-ignored`
Expected: All property tests pass with 10K cases (may take 10-30 seconds).

### Step 4: Commit

```
feat: property tests for ECS and command buffer (10K random sequences) (#21, #22)

- proptest_ecs.rs: random spawn/despawn/insert/remove/query preserves invariants
- proptest_commands.rs: random command sequences are deterministic and consistent
- Both run with 10,000 cases per test
```

---

## Task 6: Week 1-2 Milestone Test (Issue #24)

**GitHub Issue:** #24

**Depends on:** Tasks 1-5

**Files:**
- Create: `crates/nomai-engine/tests/milestone_week1_2.rs`

### What to Build

Integration test: 10K entities, 5 systems, 1000 ticks, deterministic hash match on re-run.

### Step 1: Create milestone test

```rust
//! Week 1-2 Milestone Test: 10K entities, 5 systems, 1000 ticks, deterministic.
//!
//! This test validates that the hardened ECS + tick loop produces identical
//! results across runs. It uses blake3 to hash the final world state and
//! verifies the hash matches on a second run.

use nomai_ecs::prelude::*;
use nomai_engine::tick::{TickConfig, TickLoop};

// -- Component types --------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Position { x: f64, y: f64 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Velocity { dx: f64, dy: f64 }

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Health(u32);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Score(i64);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Lifetime(u32);

// -- Systems ----------------------------------------------------------------

fn movement_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (pos, vel)) in world.query::<(&Position, &Velocity)>() {
        cmds.set_component(
            entity, "position",
            serde_json::json!({"x": pos.x + vel.dx, "y": pos.y + vel.dy}),
            SystemId(1), CausalReason::SystemInternal("movement".to_owned()),
        );
    }
}

fn damage_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (health,)) in world.query::<(&Health,)>() {
        if health.0 > 0 {
            cmds.set_component(
                entity, "health", serde_json::json!(health.0.saturating_sub(1)),
                SystemId(2), CausalReason::GameRule("tick_damage".to_owned()),
            );
        }
    }
}

fn scoring_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (score, _health)) in world.query::<(&Score, &Health)>() {
        cmds.set_component(
            entity, "score", serde_json::json!(score.0 + 1),
            SystemId(3), CausalReason::GameRule("score_increment".to_owned()),
        );
    }
}

fn lifetime_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (lifetime,)) in world.query::<(&Lifetime,)>() {
        if lifetime.0 == 0 {
            cmds.despawn(entity, SystemId(4),
                CausalReason::GameRule("lifetime_expired".to_owned()));
        } else {
            cmds.set_component(
                entity, "lifetime", serde_json::json!(lifetime.0 - 1),
                SystemId(4), CausalReason::Timer("countdown".to_owned()),
            );
        }
    }
}

fn velocity_decay_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (vel,)) in world.query::<(&Velocity,)>() {
        cmds.set_component(
            entity, "velocity",
            serde_json::json!({"dx": vel.dx * 0.999, "dy": vel.dy * 0.999}),
            SystemId(5), CausalReason::SystemInternal("friction".to_owned()),
        );
    }
}

// -- World builder ----------------------------------------------------------

fn build_world() -> World {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<Health>("health");
    world.register_component::<Score>("score");
    world.register_component::<Lifetime>("lifetime");

    // 4000 entities: Position + Velocity (movers)
    for i in 0..4000u32 {
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: i as f64, y: (i as f64) * 0.5 });
        b.add(world.registry(), Velocity { dx: 1.0, dy: -0.5 });
        world.spawn_bundle(b);
    }

    // 3000 entities: Position + Health + Score (scorers)
    for i in 0..3000u32 {
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: -(i as f64), y: i as f64 });
        b.add(world.registry(), Health(1000));
        b.add(world.registry(), Score(0));
        world.spawn_bundle(b);
    }

    // 2000 entities: Position + Velocity + Lifetime (temporary movers)
    for i in 0..2000u32 {
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: i as f64 * 2.0, y: 0.0 });
        b.add(world.registry(), Velocity { dx: 0.5, dy: 0.5 });
        b.add(world.registry(), Lifetime(500 + (i % 500))); // expire between tick 500-999
        world.spawn_bundle(b);
    }

    // 1000 entities: Health only (static targets)
    for i in 0..1000u32 {
        world.spawn_with(Health(500 + i));
    }

    world
}

/// Hash the world state deterministically using blake3.
fn hash_world(world: &World) -> String {
    let mut hasher = blake3::Hasher::new();

    // Hash entity count.
    hasher.update(&(world.entity_count() as u64).to_le_bytes());

    // Hash all positions (sorted by entity for determinism).
    let mut positions: Vec<(u64, f64, f64)> = world.query::<(&Position,)>()
        .map(|(e, (p,))| (e.to_raw(), p.x, p.y))
        .collect();
    positions.sort_by_key(|(id, _, _)| *id);
    for (id, x, y) in &positions {
        hasher.update(&id.to_le_bytes());
        hasher.update(&x.to_le_bytes());
        hasher.update(&y.to_le_bytes());
    }

    // Hash all healths.
    let mut healths: Vec<(u64, u32)> = world.query::<(&Health,)>()
        .map(|(e, (h,))| (e.to_raw(), h.0))
        .collect();
    healths.sort_by_key(|(id, _)| *id);
    for (id, h) in &healths {
        hasher.update(&id.to_le_bytes());
        hasher.update(&h.to_le_bytes());
    }

    // Hash all scores.
    let mut scores: Vec<(u64, i64)> = world.query::<(&Score,)>()
        .map(|(e, (s,))| (e.to_raw(), s.0))
        .collect();
    scores.sort_by_key(|(id, _)| *id);
    for (id, s) in &scores {
        hasher.update(&id.to_le_bytes());
        hasher.update(&s.to_le_bytes());
    }

    hasher.finalize().to_hex().to_string()
}

fn run_simulation() -> (String, u64, usize) {
    let world = build_world();
    let config = TickConfig { fixed_dt: 1.0 / 60.0, headless: true };
    let mut tick_loop = TickLoop::new(world, config);

    tick_loop.add_system("movement", movement_system);
    tick_loop.add_system("damage", damage_system);
    tick_loop.add_system("scoring", scoring_system);
    tick_loop.add_system("lifetime", lifetime_system);
    tick_loop.add_system("velocity_decay", velocity_decay_system);

    let total_cmds = tick_loop.run_ticks(1000);
    let hash = hash_world(tick_loop.world());
    let final_count = tick_loop.world().entity_count();

    (hash, total_cmds, final_count)
}

#[test]
fn milestone_10k_entities_5_systems_1000_ticks_deterministic() {
    let (hash1, cmds1, count1) = run_simulation();
    let (hash2, cmds2, count2) = run_simulation();

    // Determinism: hashes must match.
    assert_eq!(hash1, hash2, "world state hash diverged between runs");

    // Determinism: same command count.
    assert_eq!(cmds1, cmds2, "total command count diverged");

    // Determinism: same final entity count.
    assert_eq!(count1, count2, "final entity count diverged");

    // Sanity checks:
    // Started with 10K entities.
    // 2000 have Lifetime(500-999), so all should be despawned by tick 1000.
    // Remaining: 4000 + 3000 + 1000 = 8000.
    assert_eq!(count1, 8000,
        "expected 8000 surviving entities (2000 with lifetime should be despawned)");

    // Commands should be substantial: at least 1M+ across 1000 ticks.
    assert!(cmds1 > 1_000_000,
        "expected >1M commands across 1000 ticks with 10K entities, got {cmds1}");

    println!("Milestone PASS: hash={hash1}, commands={cmds1}, entities={count1}");
}
```

### Step 2: Run test

Run: `cargo test -p nomai-engine --test milestone_week1_2 -- --nocapture`
Expected: PASS with deterministic hash match. May take 5-30 seconds.

### Step 3: Commit

```
feat: Week 1-2 milestone test — 10K entities, 5 systems, 1000 ticks deterministic (#24)

- 4 entity archetypes (movers, scorers, temporary, static targets)
- 5 systems (movement, damage, scoring, lifetime, velocity_decay)
- blake3 hash of world state matches across runs
- 2000 entities despawn via lifetime system, 8000 survive
- >1M commands processed deterministically
```

---

## Close GitHub Issues

After all tasks pass:

```bash
gh issue close 21 --comment "ECS hardened: enriched errors, edge case tests, query cache with invalidation, property tests (10K cases)."
gh issue close 22 --comment "Command buffer hardened: conflict detection, ApplyReport, property tests (10K cases)."
gh issue close 23 --comment "Tick loop hardened: system dependencies with cycle detection, per-system timing diagnostics, headless mode, replay input injection."
gh issue close 24 --comment "Week 1-2 milestone PASS: 10K entities, 5 systems, 1000 ticks, deterministic blake3 hash match."
```

---

## Execution Summary

| Task | Agent | Est. Complexity | Parallel Group |
|------|-------|----------------|----------------|
| Task 1 (ECS Edge Cases) | rust-engine | Medium | A |
| Task 2 (Query Cache) | rust-engine | Medium | B (after Task 1) |
| Task 3 (Command Buffer) | rust-engine | Medium | A (parallel with Task 1) |
| Task 4 (Tick Loop) | rust-engine | High | A (parallel with Task 1) |
| Task 5 (Property Tests) | rust-engine | Medium | C (after Tasks 1-4) |
| Task 6 (Milestone) | rust-engine | Low | D (after Task 5) |

**Parallel execution strategy:**
- Start Task 1 + Task 3 + Task 4 simultaneously (different files)
- After Task 1: start Task 2 (both touch world.rs)
- After Tasks 1-4: start Task 5 (tests all hardened code)
- After Task 5: start Task 6 (integration milestone)
