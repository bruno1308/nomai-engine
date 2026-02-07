# Week 6-7: Snapshot/Restore, Deterministic Replay, Debug Renderer

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement production snapshot/restore with branching (#37), deterministic replay (#38), debug 2D renderer (#39), and the week 6-7 milestone test (#40).

**Architecture:** Snapshot serializes ECS World state (entities, components, allocator) plus tick metadata via JSON (serde_json) for debuggability, with BLAKE3 hashing for verification. Physics state is reconstructed from ECS components rather than serialized directly (rapier2d internals aren't serializable). Replay records input frames and checkpoint hashes against a snapshot. The debug renderer uses wgpu 23.0 + winit 0.30 with a simple 2D orthographic pipeline reading ECS Position/ColliderShape for geometry, gated behind `headless` config. Semantic art annotation uses convention-based asset path parsing.

**Tech Stack:** Rust 1.83 stable, serde + serde_json (existing), bincode 2.0.0-rc.3 (workspace), blake3 1.5.5 (workspace), wgpu 23.0.0 (new dep), winit 0.30.8 (new dep), rapier2d 0.22.0 (existing)

---

## Task 1: World Snapshot Serialization (nomai-ecs) — `rust-engine` agent

**Owner:** `rust-engine`
**Files:**
- Create: `crates/nomai-ecs/src/snapshot.rs`
- Modify: `crates/nomai-ecs/src/lib.rs` (add `pub mod snapshot;`)
- Modify: `crates/nomai-ecs/src/world.rs` (add snapshot/restore methods)
- Modify: `crates/nomai-ecs/src/entity.rs` (expose allocator state for snapshot)
- Modify: `crates/nomai-ecs/src/archetype.rs` (add `clone_column_data` method)
- Modify: `crates/nomai-ecs/src/component.rs` (add `registered_names` if missing)
- Test: `crates/nomai-ecs/tests/snapshot_tests.rs`

**Context:**
- `World` (world.rs:316-337) owns: `allocator: EntityAllocator`, `registry: ComponentRegistry`, `vtable_registry`, `deserializer_registry`, `archetypes: Vec<Archetype>`, `archetype_index`, `entity_locations`, `query_cache`, `archetype_generation`
- `EntityAllocator` (entity.rs:74-80): `generations: Vec<u32>`, `alive: Vec<bool>`, `free_indices: VecDeque<u32>`
- `ComponentRegistry` (component.rs:56-63): `by_type`, `by_name`, `infos: Vec<ComponentInfo>`
- `Archetype` stores `Column` per component type (raw byte buffers), `entities: Vec<EntityId>`, `type_ids: Vec<ComponentTypeId>`
- `Column` (archetype.rs:100-110): `data: *mut u8`, `len`, `capacity`, `item_size`, `item_align`
- Components use JSON serialization via `DeserializerRegistry` (world.rs:69-71) — each registered component has a `SerializeFn` that can convert raw bytes → `serde_json::Value`
- `ComponentVtable` (archetype.rs:41-53) has `clone_fn` already implemented but marked `#[allow(dead_code)]`

**Design:**
The snapshot format serializes all entity data as JSON values keyed by component name. This avoids needing to serialize raw bytes (which are type-erased and not portable). The `World` already has deserializer functions registered per component type; we need to add *serializer* functions too.

```rust
// crates/nomai-ecs/src/snapshot.rs

/// Serializable snapshot of the entire ECS World state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSnapshot {
    /// Allocator state for entity ID generation.
    pub allocator: AllocatorSnapshot,
    /// All registered component names (ordered by ComponentTypeId).
    pub component_names: Vec<String>,
    /// All entities and their component data.
    pub entities: Vec<EntitySnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocatorSnapshot {
    pub generations: Vec<u32>,
    pub alive: Vec<bool>,
    pub free_indices: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySnapshot {
    pub entity_id: EntityId,
    /// Component name -> JSON value for each component on this entity.
    pub components: HashMap<String, serde_json::Value>,
}
```

**Step 1: Add serializer registry to World**

Add a `SerializerRegistry` alongside the existing `DeserializerRegistry`. Each registered component type gets a function `fn(*const u8) -> serde_json::Value` that reads raw column bytes and serializes to JSON.

In `crates/nomai-ecs/src/world.rs`, add:
```rust
type SerializeFn = Box<dyn Fn(*const u8) -> serde_json::Value + Send + Sync>;

pub(crate) struct SerializerRegistry {
    serializers: Vec<Option<SerializeFn>>,
}
```

Register serializers alongside deserializers in `register_component<T>()`.

**Step 2: Expose EntityAllocator internals for snapshot**

In `crates/nomai-ecs/src/entity.rs`, add public getter methods:
```rust
impl EntityAllocator {
    pub fn snapshot_state(&self) -> (Vec<u32>, Vec<bool>, Vec<u32>) {
        (self.generations.clone(), self.alive.clone(), self.free_indices.iter().copied().collect())
    }

    pub fn restore_from_snapshot(generations: Vec<u32>, alive: Vec<bool>, free_indices: Vec<u32>) -> Self {
        Self { generations, alive, free_indices: free_indices.into() }
    }
}
```

**Step 3: Add column data read method to Archetype**

In `crates/nomai-ecs/src/archetype.rs`, add a method to read raw column bytes for a given row as a typed value via a serialize function:
```rust
impl Archetype {
    /// Iterate over all entities and their components, calling serialize_fn for each.
    pub unsafe fn serialize_entities(&self, ...) -> Vec<(EntityId, Vec<(ComponentTypeId, serde_json::Value)>)>
}
```

**Step 4: Create snapshot.rs with WorldSnapshot types and snapshot/restore logic**

Create `crates/nomai-ecs/src/snapshot.rs` with the types above plus:
```rust
impl World {
    pub fn capture_snapshot(&self) -> WorldSnapshot { ... }
    pub fn restore_from_snapshot(&mut self, snapshot: &WorldSnapshot) -> Result<(), EcsError> { ... }
}
```

`capture_snapshot`:
1. Read allocator state via `snapshot_state()`
2. Collect registered component names in order
3. For each entity in `entity_locations`, iterate its archetype row and serialize each column value to JSON using the serializer registry
4. Return `WorldSnapshot`

`restore_from_snapshot`:
1. Clear all archetypes, entity_locations, query_cache
2. Restore allocator from snapshot
3. For each entity in snapshot, deserialize JSON values → spawn entity with those components using the deserializer registry
4. Rebuild archetype index

**Step 5: Write tests**

```rust
// crates/nomai-ecs/tests/snapshot_tests.rs

#[test]
fn snapshot_empty_world() { ... }

#[test]
fn snapshot_single_entity_roundtrip() { ... }

#[test]
fn snapshot_multiple_entities_multiple_archetypes() { ... }

#[test]
fn snapshot_preserves_entity_ids() { ... }

#[test]
fn snapshot_preserves_identity_tiers() { ... }

#[test]
fn snapshot_with_despawned_entities_preserves_allocator() { ... }

#[test]
fn snapshot_json_serializable() { ... }
```

**Step 6: Run tests**

Run: `cargo test -p nomai-ecs --test snapshot_tests`
Expected: All PASS

**Step 7: Commit**

```bash
git add crates/nomai-ecs/src/snapshot.rs crates/nomai-ecs/src/lib.rs \
  crates/nomai-ecs/src/world.rs crates/nomai-ecs/src/entity.rs \
  crates/nomai-ecs/src/archetype.rs crates/nomai-ecs/src/component.rs \
  crates/nomai-ecs/tests/snapshot_tests.rs
git commit -m "feat: ECS World snapshot/restore with JSON serialization (#37)"
```

---

## Task 2: TickLoop Snapshot with BLAKE3 Hashing + Branching (nomai-engine) — `rust-engine` agent

**Owner:** `rust-engine`
**Files:**
- Create: `crates/nomai-engine/src/snapshot.rs`
- Modify: `crates/nomai-engine/src/lib.rs` (add `pub mod snapshot;`, re-export in prelude)
- Modify: `crates/nomai-engine/src/tick.rs` (add capture/restore methods to TickLoop)
- Test: `crates/nomai-engine/tests/snapshot_engine_tests.rs`

**Context:**
- `TickLoop` (tick.rs:161-184) owns: `world`, `command_buffer`, `systems`, `tick_counter`, `fixed_dt`, `config`, `last_diagnostics`, `current_input`, `manifest`, `physics`, `wasm_module`
- `systems` contains closures (`SystemFn = Box<dyn Fn(&World, &mut CommandBuffer)>`) — NOT serializable
- `physics` owns rapier state — NOT serializable but reconstructable from ECS components
- `wasm_module` owns Wasmtime Store — NOT serializable, must be re-attached after restore
- `manifest` (ManifestPipeline) maintains rolling history — can be reset on restore
- `command_buffer` should be empty between ticks

**Design:**
The engine-level snapshot captures: `WorldSnapshot` + `tick_counter` + `fixed_dt` + `current_input`. Physics and WASM are NOT included; after restore the caller must re-attach physics (re-registering entities from ECS state) and re-load WASM. The manifest pipeline is reset. A BLAKE3 hash is computed over the serialized snapshot bytes for checkpoint verification.

```rust
// crates/nomai-engine/src/snapshot.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineSnapshot {
    pub world: WorldSnapshot,
    pub tick_counter: u64,
    pub fixed_dt: f64,
    pub current_input: InputFrame,
    pub hash: String,  // BLAKE3 hex digest
}

impl EngineSnapshot {
    pub fn hash_bytes(&self) -> [u8; 32] { ... }
}
```

**Step 1: Create engine snapshot module**

Create `crates/nomai-engine/src/snapshot.rs`:
- `EngineSnapshot` struct (above)
- `impl TickLoop`:
  - `capture_snapshot(&self) -> EngineSnapshot`: serialize world, tick state, hash with BLAKE3
  - `restore_from_snapshot(&mut self, snapshot: &EngineSnapshot) -> Result<(), anyhow::Error>`: restore world, tick counter, reset manifest, clear physics/WASM (caller re-attaches)

**Step 2: Add BLAKE3 hashing**

After serializing to JSON bytes, compute `blake3::hash(&json_bytes)` and store hex string in `EngineSnapshot.hash`.

**Step 3: Add snapshot branching helper**

```rust
impl TickLoop {
    /// Fork: capture snapshot, return it. Caller can restore later for branching.
    pub fn fork_snapshot(&self) -> EngineSnapshot { self.capture_snapshot() }
}
```

**Step 4: Property test — snapshot determinism**

```rust
// snapshot at tick N, restore, run M more ticks
// vs. run N+M ticks from scratch
// -> final world state hash should match
#[test]
fn snapshot_restore_determinism() {
    // Setup world with entities and systems
    // Run 50 ticks
    // Capture snapshot
    // Run 50 more ticks, capture final hash A
    // Restore snapshot
    // Run 50 more ticks, capture final hash B
    // assert_eq!(A, B)
}

#[test]
fn snapshot_branching_produces_different_states() {
    // Fork at tick 50
    // Branch A: modify velocity, run 50 ticks
    // Branch B: restore, different velocity, run 50 ticks
    // assert_ne!(hash_a, hash_b)
    // Both branches produce valid manifests
}

#[test]
fn snapshot_hash_changes_with_state() {
    // Snapshot at tick 10 and tick 20 should have different hashes
}
```

**Step 5: Run tests**

Run: `cargo test -p nomai-engine --test snapshot_engine_tests`
Expected: All PASS

**Step 6: Commit**

```bash
git add crates/nomai-engine/src/snapshot.rs crates/nomai-engine/src/lib.rs \
  crates/nomai-engine/src/tick.rs \
  crates/nomai-engine/tests/snapshot_engine_tests.rs
git commit -m "feat: engine snapshot with BLAKE3 hashing and branching (#37)"
```

---

## Task 3: Deterministic Replay (nomai-engine) — `rust-engine` agent

**Owner:** `rust-engine`
**Files:**
- Create: `crates/nomai-engine/src/replay.rs`
- Modify: `crates/nomai-engine/src/lib.rs` (add `pub mod replay;`, re-export in prelude)
- Modify: `crates/nomai-engine/src/tick.rs` (add `state_hash()` helper for checkpoint computation)
- Test: `crates/nomai-engine/tests/replay_tests.rs`

**Context:**
- `InputFrame` (tick.rs) is `HashMap<String, serde_json::Value>`, already `Serialize + Deserialize`
- `TickLoop::set_input(frame)` (tick.rs:528) sets input for next tick
- `TickLoop::current_input()` (tick.rs:533) reads current input
- The spec defines `ReplayLog` with `initial_snapshot`, `gameplay_module_hash`, `seed`, and `Vec<ReplayEntry>` where entries are `Input { tick, input }` or `Checkpoint { tick, state_hash }`

**Design:**
```rust
// crates/nomai-engine/src/replay.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayLog {
    pub initial_snapshot: EngineSnapshot,
    pub gameplay_module_hash: Option<String>,  // BLAKE3 hex of WASM binary, if any
    pub entries: Vec<ReplayEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplayEntry {
    Input { tick: u64, input: InputFrame },
    Checkpoint { tick: u64, state_hash: String },
}

#[derive(Debug, Clone)]
pub struct ReplayResult {
    pub completed: bool,
    pub ticks_replayed: u64,
    pub first_divergence: Option<ReplayDivergence>,
}

#[derive(Debug, Clone)]
pub struct ReplayDivergence {
    pub tick: u64,
    pub expected_hash: String,
    pub actual_hash: String,
}
```

**Step 1: Create replay.rs with types**

The replay module contains:
- `ReplayLog`, `ReplayEntry`, `ReplayResult`, `ReplayDivergence` structs
- Recording mode: `ReplayRecorder` that wraps a TickLoop reference and records inputs + periodic checkpoints
- Playback mode: `replay()` function that restores snapshot, injects recorded inputs, verifies checkpoints

**Step 2: Implement ReplayRecorder**

```rust
pub struct ReplayRecorder {
    log: ReplayLog,
    checkpoint_interval: u64,  // e.g. every 10 ticks
}

impl ReplayRecorder {
    pub fn new(snapshot: EngineSnapshot, checkpoint_interval: u64) -> Self { ... }

    /// Call before each tick to record the current input frame.
    pub fn record_tick(&mut self, tick: u64, input: &InputFrame, state_hash: Option<String>) {
        if !input.is_empty() {
            self.log.entries.push(ReplayEntry::Input { tick, input: input.clone() });
        }
        if tick % self.checkpoint_interval == 0 {
            if let Some(hash) = state_hash {
                self.log.entries.push(ReplayEntry::Checkpoint { tick, state_hash: hash });
            }
        }
    }

    pub fn finish(self) -> ReplayLog { self.log }
}
```

**Step 3: Implement replay playback**

```rust
/// Replay a recorded log against the tick loop.
/// Restores the initial snapshot, injects recorded inputs tick-by-tick,
/// and verifies checkpoint hashes. Returns first divergence if any.
pub fn replay(tick_loop: &mut TickLoop, log: &ReplayLog) -> Result<ReplayResult, anyhow::Error> {
    tick_loop.restore_from_snapshot(&log.initial_snapshot)?;

    // Build lookup maps for inputs and checkpoints by tick
    let mut input_map: HashMap<u64, InputFrame> = HashMap::new();
    let mut checkpoint_map: HashMap<u64, String> = HashMap::new();
    for entry in &log.entries {
        match entry {
            ReplayEntry::Input { tick, input } => { input_map.insert(*tick, input.clone()); }
            ReplayEntry::Checkpoint { tick, state_hash } => { checkpoint_map.insert(*tick, state_hash.clone()); }
        }
    }

    let max_tick = log.entries.iter().map(|e| match e {
        ReplayEntry::Input { tick, .. } | ReplayEntry::Checkpoint { tick, .. } => *tick,
    }).max().unwrap_or(0);

    let start_tick = tick_loop.tick_count();
    for tick in start_tick..=max_tick {
        if let Some(input) = input_map.get(&tick) {
            tick_loop.set_input(input.clone());
        } else {
            tick_loop.set_input(InputFrame::default());
        }

        tick_loop.tick();

        if let Some(expected_hash) = checkpoint_map.get(&tick) {
            let actual_hash = tick_loop.capture_snapshot().hash;
            if &actual_hash != expected_hash {
                return Ok(ReplayResult {
                    completed: false,
                    ticks_replayed: tick - start_tick,
                    first_divergence: Some(ReplayDivergence {
                        tick,
                        expected_hash: expected_hash.clone(),
                        actual_hash,
                    }),
                });
            }
        }
    }

    Ok(ReplayResult {
        completed: true,
        ticks_replayed: max_tick - start_tick + 1,
        first_divergence: None,
    })
}
```

**Step 4: Add state_hash helper to TickLoop**

In tick.rs, add a convenience method:
```rust
impl TickLoop {
    pub fn state_hash(&self) -> String {
        self.capture_snapshot().hash
    }
}
```

**Step 5: Write tests**

```rust
// crates/nomai-engine/tests/replay_tests.rs

#[test]
fn record_and_replay_empty_simulation() { ... }

#[test]
fn record_and_replay_with_inputs() {
    // Record 100 ticks with varying inputs
    // Replay from snapshot
    // All checkpoint hashes match
}

#[test]
fn replay_detects_divergence_when_systems_differ() {
    // Record with system A, replay with system B
    // Should report first divergent tick
}

#[test]
fn replay_log_serializable_to_json() {
    // Serialize ReplayLog to JSON and back
}
```

**Step 6: Run tests**

Run: `cargo test -p nomai-engine --test replay_tests`
Expected: All PASS

**Step 7: Commit**

```bash
git add crates/nomai-engine/src/replay.rs crates/nomai-engine/src/lib.rs \
  crates/nomai-engine/src/tick.rs \
  crates/nomai-engine/tests/replay_tests.rs
git commit -m "feat: deterministic replay with input recording and checkpoint verification (#38)"
```

---

## Task 4: Python Bindings for Snapshot/Replay — `python-verification` agent

**Owner:** `python-verification`
**Files:**
- Modify: `crates/nomai-python/src/engine.rs` (add snapshot/replay methods to PyNomaiEngine)
- Modify: `python/nomai-sdk/nomai/engine.py` (add snapshot/replay wrapper methods)
- Create: `python/nomai-sdk/nomai/replay.py` (ReplayLog, ReplayResult dataclasses)
- Test: `python/nomai-sdk/tests/test_snapshot_replay.py`

**Context:**
- `PyNomaiEngine` wraps `TickLoop` via PyO3
- Current pattern: Rust types → JSON string → Python dict → Python dataclasses
- Snapshot/replay types all derive `Serialize + Deserialize`

**Step 1: Add PyO3 methods for snapshot/replay**

In `crates/nomai-python/src/engine.rs`:
```rust
#[pymethods]
impl PyNomaiEngine {
    /// Capture a snapshot of the current engine state.
    /// Returns JSON string of EngineSnapshot.
    fn capture_snapshot(&self) -> PyResult<String> { ... }

    /// Restore engine state from a JSON snapshot string.
    fn restore_snapshot(&mut self, snapshot_json: &str) -> PyResult<()> { ... }

    /// Get BLAKE3 hash of current state.
    fn state_hash(&self) -> PyResult<String> { ... }

    /// Start recording a replay. Returns recorder handle.
    fn start_recording(&mut self, checkpoint_interval: u64) -> PyResult<()> { ... }

    /// Stop recording and return ReplayLog as JSON.
    fn stop_recording(&mut self) -> PyResult<String> { ... }

    /// Replay a recorded log (JSON string). Returns ReplayResult as JSON.
    fn replay(&mut self, replay_log_json: &str) -> PyResult<String> { ... }
}
```

**Step 2: Python SDK wrappers**

In `python/nomai-sdk/nomai/engine.py`:
```python
def capture_snapshot(self) -> "EngineSnapshot": ...
def restore_snapshot(self, snapshot: "EngineSnapshot") -> None: ...
def state_hash(self) -> str: ...
def start_recording(self, checkpoint_interval: int = 10) -> None: ...
def stop_recording(self) -> "ReplayLog": ...
def replay(self, log: "ReplayLog") -> "ReplayResult": ...
```

**Step 3: Python dataclasses**

In `python/nomai-sdk/nomai/replay.py`:
```python
@dataclass
class EngineSnapshot:
    tick_counter: int
    fixed_dt: float
    hash: str
    raw_json: str  # full JSON for restore

@dataclass
class ReplayResult:
    completed: bool
    ticks_replayed: int
    first_divergence: Optional["ReplayDivergence"]

@dataclass
class ReplayDivergence:
    tick: int
    expected_hash: str
    actual_hash: str
```

**Step 4: Tests**

```python
def test_snapshot_roundtrip():
    engine = NomaiEngine(headless=True)
    engine.register_component("score")
    engine.spawn_entity("item", "coin", {"score": 10})
    engine.run_ticks(10)

    snapshot = engine.capture_snapshot()
    assert snapshot.hash != ""

    engine.run_ticks(10)
    engine.restore_snapshot(snapshot)
    assert engine.tick_count == 10  # restored to snapshot state

def test_replay_deterministic():
    engine = NomaiEngine(headless=True)
    engine.start_recording(checkpoint_interval=5)
    engine.run_ticks(20)
    log = engine.stop_recording()

    result = engine.replay(log)
    assert result.completed
    assert result.first_divergence is None
```

**Step 5: Run tests**

Run: `cd python/nomai-sdk && python -m pytest tests/test_snapshot_replay.py -v`
Expected: All PASS

**Step 6: Commit**

```bash
git add crates/nomai-python/src/engine.rs \
  python/nomai-sdk/nomai/engine.py \
  python/nomai-sdk/nomai/replay.py \
  python/nomai-sdk/tests/test_snapshot_replay.py
git commit -m "feat: Python bindings for snapshot/restore and replay (#37, #38)"
```

---

## Task 5: Debug 2D Renderer — wgpu Setup + Rectangle Rendering — `renderer` agent

**Owner:** `renderer`
**Files:**
- Modify: `Cargo.toml` (workspace deps: add wgpu, winit)
- Modify: `crates/nomai-engine/Cargo.toml` (add wgpu, winit as optional deps)
- Create: `crates/nomai-engine/src/render/mod.rs`
- Create: `crates/nomai-engine/src/render/renderer.rs`
- Create: `crates/nomai-engine/src/render/shaders.wgsl`
- Modify: `crates/nomai-engine/src/lib.rs` (add `pub mod render;`)
- Test: `crates/nomai-engine/tests/renderer_tests.rs`

**Context:**
- No render module exists yet
- `TickConfig.headless` (tick.rs:71) controls whether rendering is enabled
- Physics types `Position { x, y }` and `ColliderShape { Box { half_width, half_height }, Circle { radius } }` define entity geometry
- `Identity` component carries entity type/role for color mapping
- wgpu 23.0.0 and winit 0.30.8 are specified in CLAUDE.md but NOT yet in workspace deps

**Design:**
The renderer is a standalone module that reads ECS world state and draws colored rectangles/circles. It does NOT own the event loop — the tick loop drives it. The renderer is optional (feature-gated or runtime-skipped when headless=true).

**Step 1: Add wgpu and winit to workspace deps**

In root `Cargo.toml`:
```toml
wgpu = "23.0.0"
winit = "0.30.8"
```

In `crates/nomai-engine/Cargo.toml`:
```toml
[dependencies]
wgpu = { workspace = true, optional = true }
winit = { workspace = true, optional = true }

[features]
default = []
renderer = ["dep:wgpu", "dep:winit"]
```

**Step 2: Create render module structure**

```rust
// crates/nomai-engine/src/render/mod.rs
#[cfg(feature = "renderer")]
pub mod renderer;

#[cfg(feature = "renderer")]
pub use renderer::DebugRenderer;
```

**Step 3: Implement DebugRenderer**

```rust
// crates/nomai-engine/src/render/renderer.rs

/// Debug 2D renderer using wgpu.
/// Reads ECS state and draws entities as colored rectangles/circles.
pub struct DebugRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    window: Arc<winit::window::Window>,
    camera: Camera2D,
}

pub struct Camera2D {
    pub width: f32,
    pub height: f32,
    pub x: f32,
    pub y: f32,
}

/// A drawable entity extracted from ECS state.
pub struct DrawCommand {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub color: [f32; 4],
}

impl DebugRenderer {
    /// Initialize wgpu: window, surface, device, queue, pipeline.
    pub async fn new(window: Arc<winit::window::Window>) -> Result<Self, anyhow::Error> { ... }

    /// Extract draw commands from ECS world state.
    pub fn extract_draw_commands(world: &World) -> Vec<DrawCommand> {
        // Query entities with Position + ColliderShape (via PhysicsBody)
        // Map entity type/role to color:
        //   paddle -> blue [0.2, 0.4, 0.9, 1.0]
        //   ball -> white [1.0, 1.0, 1.0, 1.0]
        //   brick -> varied colors based on row
        //   wall -> gray [0.5, 0.5, 0.5, 1.0]
    }

    /// Render a frame from draw commands.
    pub fn render(&mut self, commands: &[DrawCommand]) -> Result<(), wgpu::SurfaceError> { ... }

    /// Resize the surface.
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) { ... }
}
```

**Step 4: WGSL shader for colored rectangles**

```wgsl
// crates/nomai-engine/src/render/shaders.wgsl

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: mat4x4<f32>;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera * vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
```

**Step 5: Write tests (headless/GPU-less validation)**

```rust
#[test]
fn extract_draw_commands_from_world() {
    // Setup world with Position + PhysicsBody entities
    // Call extract_draw_commands
    // Verify correct number of commands with correct positions and colors
}

#[test]
fn camera_2d_orthographic_projection() {
    // Verify camera matrix produces correct clip coordinates
}
```

Note: Full GPU rendering tests require a display context. Unit tests focus on command extraction logic and camera math.

**Step 6: Run tests**

Run: `cargo test -p nomai-engine --features renderer --test renderer_tests`
Expected: All PASS

**Step 7: Commit**

```bash
git add Cargo.toml crates/nomai-engine/Cargo.toml \
  crates/nomai-engine/src/render/ \
  crates/nomai-engine/src/lib.rs \
  crates/nomai-engine/tests/renderer_tests.rs
git commit -m "feat: debug 2D renderer with wgpu colored rectangles (#39)"
```

---

## Task 6: Text Rendering + Semantic Art Annotation — `renderer` agent

**Owner:** `renderer`
**Files:**
- Modify: `crates/nomai-engine/src/render/renderer.rs` (add text rendering, HUD overlay)
- Create: `crates/nomai-engine/src/render/text.rs` (simple glyph-based text using rectangles)
- Create: `crates/nomai-engine/src/render/annotation.rs` (asset path parsing)
- Test: `crates/nomai-engine/tests/annotation_tests.rs`

**Context:**
- v8 spec Section 5 describes convention-based asset path parsing
- For MVP, text rendering can be simple bitmap/glyph rectangles (no font atlas needed)
- Asset paths follow: `assets/sprites/{entity_type}/{state}_{direction}.png`

**Design:**
For MVP text rendering, use a simple approach: render text as colored rectangle glyphs (5x7 pixel font bitmap). This avoids font loading complexity while providing basic score/tick display.

Semantic art annotation parses file paths:
```
assets/sprites/player/idle_right.png → AssetAnnotation {
    semantic_type: "character_sprite",
    represents: "player.idle.facing_right",
    tags: ["player", "idle", "right"],
    ...
}
```

**Step 1: Simple text rendering**

```rust
// crates/nomai-engine/src/render/text.rs

/// Render text as colored rectangles (5x7 glyph bitmap).
pub struct TextRenderer {
    glyphs: HashMap<char, [[bool; 5]; 7]>,
}

impl TextRenderer {
    pub fn new() -> Self { /* init ASCII 0-9, A-Z */ }

    pub fn text_to_draw_commands(
        &self,
        text: &str,
        x: f32, y: f32,
        scale: f32,
        color: [f32; 4],
    ) -> Vec<DrawCommand> { ... }
}
```

**Step 2: Semantic art annotation**

```rust
// crates/nomai-engine/src/render/annotation.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetAnnotation {
    pub asset_path: String,
    pub semantic_type: String,
    pub represents: String,
    pub tags: Vec<String>,
    pub entity_type: String,
}

/// Parse asset path to semantic annotation using directory convention.
pub fn parse_asset_path(path: &str) -> Option<AssetAnnotation> {
    // "assets/sprites/player/idle_right.png"
    // → entity_type: "player", state: "idle", direction: "right"
    // → represents: "player.idle.facing_right"
    // → tags: ["player", "idle", "right"]
}
```

**Step 3: Tests**

```rust
// crates/nomai-engine/tests/annotation_tests.rs

#[test]
fn parse_player_sprite_path() {
    let ann = parse_asset_path("assets/sprites/player/idle_right.png").unwrap();
    assert_eq!(ann.entity_type, "player");
    assert_eq!(ann.represents, "player.idle.facing_right");
    assert!(ann.tags.contains(&"idle".to_string()));
}

#[test]
fn parse_enemy_sprite_path() { ... }

#[test]
fn parse_tile_path() { ... }

#[test]
fn parse_invalid_path_returns_none() { ... }
```

**Step 4: Run tests**

Run: `cargo test -p nomai-engine --features renderer --test annotation_tests`
Expected: All PASS

**Step 5: Commit**

```bash
git add crates/nomai-engine/src/render/text.rs \
  crates/nomai-engine/src/render/annotation.rs \
  crates/nomai-engine/src/render/mod.rs \
  crates/nomai-engine/src/render/renderer.rs \
  crates/nomai-engine/tests/annotation_tests.rs
git commit -m "feat: text rendering and semantic art annotation (#39)"
```

---

## Task 7: Headless/Windowed Toggle + Render Loop Integration — `renderer` agent

**Owner:** `renderer`
**Files:**
- Modify: `crates/nomai-engine/src/tick.rs` (add optional render call after tick)
- Modify: `crates/nomai-engine/src/render/renderer.rs` (add `render_world` convenience)
- Create: `crates/nomai-engine/src/render/app.rs` (winit event loop integration)
- Test: `crates/nomai-engine/tests/headless_toggle_tests.rs`

**Context:**
- `TickConfig.headless` (tick.rs:71) is the toggle
- winit 0.30 uses `ApplicationHandler` trait for event loop
- The renderer must be optional: headless mode must work without GPU

**Design:**
The render integration provides a `run_windowed()` function that creates a winit event loop, initializes the renderer, and drives the tick loop with rendering after each tick. The existing `tick()` and `run_ticks()` remain unchanged for headless mode.

**Step 1: Implement windowed app runner**

```rust
// crates/nomai-engine/src/render/app.rs

/// Run the tick loop in a window with debug rendering.
/// This function takes ownership of the tick loop and drives it
/// from the winit event loop. Blocks until the window is closed.
pub fn run_windowed(
    tick_loop: TickLoop,
    window_title: &str,
    width: u32,
    height: u32,
) -> Result<(), anyhow::Error> { ... }
```

**Step 2: Add render_world convenience**

```rust
impl DebugRenderer {
    /// Extract draw commands from world + render in one call.
    pub fn render_world(&mut self, world: &World) -> Result<(), wgpu::SurfaceError> {
        let commands = Self::extract_draw_commands(world);
        self.render(&commands)
    }
}
```

**Step 3: Tests**

```rust
#[test]
fn headless_mode_runs_without_renderer() {
    let config = TickConfig { headless: true, ..Default::default() };
    let mut tick_loop = TickLoop::new(World::new(), config);
    tick_loop.run_ticks(100);
    assert_eq!(tick_loop.tick_count(), 100);
    // No GPU initialization, no panics
}

#[test]
fn tick_loop_works_without_renderer_feature() {
    // This test runs even without --features renderer
    let config = TickConfig { headless: true, ..Default::default() };
    let mut tick_loop = TickLoop::new(World::new(), config);
    tick_loop.run_ticks(50);
    assert_eq!(tick_loop.tick_count(), 50);
}
```

**Step 4: Run tests**

Run: `cargo test -p nomai-engine --test headless_toggle_tests` (without renderer feature)
Run: `cargo test -p nomai-engine --features renderer --test headless_toggle_tests` (with renderer feature)
Expected: Both PASS

**Step 5: Commit**

```bash
git add crates/nomai-engine/src/render/app.rs \
  crates/nomai-engine/src/render/renderer.rs \
  crates/nomai-engine/src/render/mod.rs \
  crates/nomai-engine/src/tick.rs \
  crates/nomai-engine/tests/headless_toggle_tests.rs
git commit -m "feat: headless/windowed toggle with render loop integration (#39)"
```

---

## Task 8: Physics Reconstruction After Snapshot Restore — `rust-engine` agent

**Owner:** `rust-engine`
**Files:**
- Modify: `crates/nomai-engine/src/physics.rs` (add `reconstruct_from_world` method)
- Modify: `crates/nomai-engine/src/snapshot.rs` (call physics reconstruction in restore)
- Test: `crates/nomai-engine/tests/snapshot_physics_tests.rs`

**Context:**
- rapier2d types (`PhysicsPipeline`, `RigidBodySet`, etc.) are NOT serializable
- `PhysicsWorld` (physics.rs:102-120) maintains `entity_to_body`, `body_to_entity`, `collider_to_entity` maps
- `PhysicsWorld::register_entity()` (physics.rs:148+) creates rapier bodies from ECS Position + PhysicsBody components
- After snapshot restore, the rapier world is stale. We need to rebuild it from ECS component data.

**Design:**
Add a `PhysicsWorld::reconstruct_from_world(&mut self, world: &World)` method that:
1. Clears all rapier bodies, colliders, and handle maps
2. Queries all entities with Position + PhysicsBody + Velocity components
3. Re-registers each entity with `register_entity()`
4. Result: rapier state matches ECS state

**Step 1: Implement reconstruct_from_world**

```rust
impl PhysicsWorld {
    /// Clear all physics state and rebuild from ECS components.
    pub fn reconstruct_from_world(&mut self, world: &World) {
        // Clear existing rapier state
        self.rigid_body_set = RigidBodySet::new();
        self.collider_set = ColliderSet::new();
        self.entity_to_body.clear();
        self.body_to_entity.clear();
        self.collider_to_entity.clear();
        self.island_manager = IslandManager::new();
        self.broad_phase = DefaultBroadPhase::new();
        self.narrow_phase = NarrowPhase::new();
        self.impulse_joint_set = ImpulseJointSet::new();
        self.multibody_joint_set = MultibodyJointSet::new();
        self.ccd_solver = CCDSolver::new();
        self.pipeline = PhysicsPipeline::new();

        // Re-register all physics entities from ECS
        // Query (Position, PhysicsBody) entities, optionally Velocity
        for (entity, (pos, body)) in world.query::<(&Position, &PhysicsBody)>() {
            let vel = world.get_component::<Velocity>(entity);
            self.register_entity(entity, pos, body, vel.map(|v| v.clone()).as_ref());
        }
    }
}
```

**Step 2: Integrate with snapshot restore**

In `crates/nomai-engine/src/snapshot.rs`, after restoring the world:
```rust
impl TickLoop {
    pub fn restore_from_snapshot(&mut self, snapshot: &EngineSnapshot) -> Result<(), anyhow::Error> {
        self.world.restore_from_snapshot(&snapshot.world)?;
        self.tick_counter = snapshot.tick_counter;
        self.current_input = snapshot.current_input.clone();
        self.manifest = ManifestPipeline::new(); // Reset manifest
        self.command_buffer = CommandBuffer::new(); // Clear command buffer

        // Reconstruct physics from restored ECS state
        if let Some(ref mut physics) = self.physics {
            physics.reconstruct_from_world(&self.world);
        }

        // WASM module must be re-attached by caller
        // (Wasmtime Store is not serializable)

        Ok(())
    }
}
```

**Step 3: Tests**

```rust
#[test]
fn snapshot_restore_preserves_physics_behavior() {
    // Setup: world with paddle, ball, walls (physics entities)
    // Run 50 ticks (ball bouncing)
    // Snapshot
    // Run 50 more ticks, record final positions hash_a
    // Restore snapshot
    // physics.reconstruct_from_world() is called automatically
    // Run 50 more ticks, record final positions hash_b
    // assert_eq!(hash_a, hash_b)
}

#[test]
fn physics_reconstruction_registers_all_entities() {
    // Setup world with 5 physics entities
    // Snapshot + restore
    // Verify physics.entity_count() == 5
    // Verify bodies have correct positions
}
```

**Step 4: Run tests**

Run: `cargo test -p nomai-engine --test snapshot_physics_tests`
Expected: All PASS

**Step 5: Commit**

```bash
git add crates/nomai-engine/src/physics.rs \
  crates/nomai-engine/src/snapshot.rs \
  crates/nomai-engine/tests/snapshot_physics_tests.rs
git commit -m "feat: physics reconstruction from ECS state after snapshot restore (#37)"
```

---

## Task 9: Week 6-7 Milestone Test (#40) — `rust-engine` agent

**Owner:** `rust-engine`
**Files:**
- Create: `crates/nomai-engine/tests/milestone_week6_7.rs`

**Context:**
- Milestone test pattern: see `crates/nomai-engine/tests/milestone_week4_5.rs`
- Must validate: snapshot determinism, replay verification, headless vs windowed toggle
- Acceptance criteria from #40:
  1. Save snapshot at tick 100, restore, run 200 more ticks: hash matches running straight through
  2. Record breakout session, replay, all checkpoint hashes match
  3. Headless mode: same simulation runs without window

**Step 1: Write milestone test**

```rust
// crates/nomai-engine/tests/milestone_week6_7.rs

//! Week 6-7 Milestone Test: Snapshot/Restore/Replay + Debug Renderer
//!
//! Validates:
//! 1. Snapshot at tick 100, restore, run to 300 == straight run to 300 (hash match)
//! 2. Record + replay with checkpoint verification
//! 3. Snapshot branching: fork, diverge, compare
//! 4. Headless simulation runs without GPU

use nomai_engine::prelude::*;
use nomai_engine::snapshot::EngineSnapshot;
use nomai_engine::replay::{ReplayRecorder, replay, ReplayLog};

fn setup_breakout_world() -> TickLoop {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<PhysicsBody>("physics_body");
    // ... register all breakout components
    // Spawn paddle, ball, walls, bricks
    // Attach physics
    let config = TickConfig { fixed_dt: 1.0 / 60.0, headless: true };
    let mut tick_loop = TickLoop::new(world, config);
    // Setup physics, register entities
    tick_loop
}

#[test]
fn milestone_snapshot_determinism() {
    // Run straight to tick 300, record hash
    let mut tl_a = setup_breakout_world();
    tl_a.run_ticks(300);
    let hash_straight = tl_a.state_hash();

    // Snapshot at tick 100, restore, run 200 more
    let mut tl_b = setup_breakout_world();
    tl_b.run_ticks(100);
    let snapshot = tl_b.capture_snapshot();
    tl_b.run_ticks(200); // continue to 300
    let hash_continued = tl_b.state_hash();

    // Run from scratch to 100, restore snapshot, run 200
    let mut tl_c = setup_breakout_world();
    tl_c.run_ticks(50); // run some ticks to dirty state
    tl_c.restore_from_snapshot(&snapshot).unwrap();
    tl_c.run_ticks(200);
    let hash_restored = tl_c.state_hash();

    assert_eq!(hash_straight, hash_continued, "straight vs continued mismatch");
    assert_eq!(hash_continued, hash_restored, "continued vs restored mismatch");
}

#[test]
fn milestone_replay_determinism() {
    let mut tl = setup_breakout_world();

    // Record 200 ticks with inputs
    let snapshot = tl.capture_snapshot();
    let mut recorder = ReplayRecorder::new(snapshot, 10);
    for tick in 0..200 {
        let input = if tick % 3 == 0 {
            InputFrame::from([("paddle_dx".to_string(), serde_json::json!(1.0))])
        } else {
            InputFrame::default()
        };
        tl.set_input(input.clone());
        let hash = tl.state_hash();
        recorder.record_tick(tick, &input, Some(hash));
        tl.tick();
    }
    let log = recorder.finish();

    // Replay and verify
    let result = replay(&mut tl, &log).unwrap();
    assert!(result.completed, "replay failed: {:?}", result.first_divergence);
}

#[test]
fn milestone_snapshot_branching() {
    let mut tl = setup_breakout_world();
    tl.run_ticks(50);
    let snapshot = tl.capture_snapshot();

    // Branch A: ball goes fast
    tl.run_ticks(100);
    let hash_a = tl.state_hash();

    // Branch B: restore and modify
    tl.restore_from_snapshot(&snapshot).unwrap();
    // Modify a component to diverge
    // ... set different velocity on ball
    tl.run_ticks(100);
    let hash_b = tl.state_hash();

    assert_ne!(hash_a, hash_b, "branches should diverge");
}

#[test]
fn milestone_headless_runs_without_gpu() {
    let config = TickConfig { headless: true, ..Default::default() };
    let world = World::new();
    let mut tl = TickLoop::new(world, config);
    tl.run_ticks(100);
    assert_eq!(tl.tick_count(), 100);
    assert!(tl.is_headless());
}
```

**Step 2: Run milestone test**

Run: `cargo test -p nomai-engine --test milestone_week6_7 -- --nocapture`
Expected: All 4 tests PASS

**Step 3: Commit**

```bash
git add crates/nomai-engine/tests/milestone_week6_7.rs
git commit -m "feat: week 6-7 milestone test -- snapshot/replay/renderer (#40)"
```

---

## Task Dependencies

```
Task 1 (ECS snapshot) ──► Task 2 (Engine snapshot + BLAKE3)
                               │
                               ├──► Task 3 (Replay)
                               │       │
                               │       ├──► Task 4 (Python bindings)
                               │       │
                               │       └──► Task 9 (Milestone test)
                               │
                               └──► Task 8 (Physics reconstruction)
                                       │
                                       └──► Task 9 (Milestone test)

Task 5 (Renderer setup) ──► Task 6 (Text + annotation) ──► Task 7 (Headless toggle)
                                                                   │
                                                                   └──► Task 9 (Milestone test)
```

**Parallelizable:**
- Tasks 1-3 (snapshot/replay) can run in parallel with Tasks 5-7 (renderer)
- Task 4 (Python bindings) depends on Tasks 2-3
- Task 8 (physics reconstruction) depends on Task 2
- Task 9 (milestone) depends on Tasks 3, 7, 8

**Agent assignment:**
- `rust-engine`: Tasks 1, 2, 3, 8, 9
- `renderer`: Tasks 5, 6, 7
- `python-verification`: Task 4

---

## Verification Checklist

Before marking Week 6-7 complete:

- [ ] `cargo test -p nomai-ecs --test snapshot_tests` passes
- [ ] `cargo test -p nomai-engine --test snapshot_engine_tests` passes
- [ ] `cargo test -p nomai-engine --test replay_tests` passes
- [ ] `cargo test -p nomai-engine --test snapshot_physics_tests` passes
- [ ] `cargo test -p nomai-engine --features renderer --test renderer_tests` passes
- [ ] `cargo test -p nomai-engine --features renderer --test annotation_tests` passes
- [ ] `cargo test -p nomai-engine --test headless_toggle_tests` passes
- [ ] `cargo test -p nomai-engine --test milestone_week6_7` passes
- [ ] `cd python/nomai-sdk && python -m pytest tests/test_snapshot_replay.py` passes
- [ ] `cargo clippy --workspace --all-features` is clean
- [ ] `cargo fmt --all -- --check` passes
- [ ] All milestone acceptance criteria from #40 are met
