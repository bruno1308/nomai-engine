# Scene Snapshot (Text Scene Representation) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a `SceneSnapshot` — a structured text representation of the current game scene — as a first-class engine output alongside the framebuffer, so AI can reason about game state without pixel-peeking.

**Architecture:** Reuse the renderer's entity-scanning logic (position, size, identity, visibility from both native physics and dynamic JSON components) to build a `SceneSnapshot` struct in Rust. Expose it to Python via PyO3 using the existing JSON round-trip pattern. The snapshot is a query of current state (not a diff), callable after any tick via `engine.scene_snapshot()`.

**Tech Stack:** Rust (nomai-manifest, nomai-engine, nomai-python), Python (nomai-sdk), serde_json, PyO3

---

## Schema

```rust
// nomai-manifest/src/scene.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneEntity {
    pub entity_id: u64,
    pub entity_type: String,
    pub role: String,
    pub tier: String,
    pub position: Option<[f64; 2]>,       // [x, y]
    pub size: Option<[f64; 2]>,           // [w, h]
    pub velocity: Option<[f64; 2]>,       // [dx, dy]
    pub visible: bool,
    pub z_index: f64,
    pub components: HashMap<String, serde_json::Value>,  // all dynamic components
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSnapshot {
    pub schema_version: u32,              // Start at 1
    pub tick: u64,
    pub sim_time: f64,
    pub entities: Vec<SceneEntity>,       // Sorted by z_index, then entity_id
    pub bounds: SceneBounds,
    pub entity_count: usize,
}
```

Python side mirrors this with frozen dataclasses following existing `manifest.py` patterns.

---

### Task 1: Add SceneSnapshot and SceneEntity structs to nomai-manifest

**Files:**
- Create: `crates/nomai-manifest/src/scene.rs`
- Modify: `crates/nomai-manifest/src/lib.rs` (add `pub mod scene;` and re-export)

**Step 1: Create `scene.rs` with the structs**

```rust
//! Scene snapshot — a full-state text representation of the game scene.
//!
//! A [`SceneSnapshot`] captures every visible entity's spatial data,
//! identity, and components at a single point in time. This is the
//! text equivalent of a rendered frame.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A single entity's state in the scene snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneEntity {
    /// Raw entity ID.
    pub entity_id: u64,
    /// Entity type (e.g., "character", "projectile", "destructible").
    pub entity_type: String,
    /// Entity role (e.g., "paddle", "ball", "brick").
    pub role: String,
    /// Identity tier ("Semantic", "Pooled", or "Unknown").
    pub tier: String,
    /// World-space position [x, y], if the entity has spatial data.
    pub position: Option<[f64; 2]>,
    /// Size [width, height], if the entity has size data.
    pub size: Option<[f64; 2]>,
    /// Velocity [dx, dy], if the entity has velocity data.
    pub velocity: Option<[f64; 2]>,
    /// Whether the entity is currently visible (rendered).
    pub visible: bool,
    /// Z-ordering index (lower = behind, higher = in front).
    pub z_index: f64,
    /// All dynamic components as raw JSON values.
    pub components: HashMap<String, serde_json::Value>,
}

/// Axis-aligned bounding box of the entire scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// A complete snapshot of the game scene at a single tick.
///
/// This is the text equivalent of a rendered frame. It captures every
/// entity's spatial state, identity, and components. Combined with
/// [`TickManifest`](crate::manifest::TickManifest) (per-tick diffs),
/// this gives AI full observability without pixel-peeking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSnapshot {
    /// Schema version for forward compatibility.
    pub schema_version: u32,
    /// Tick number when this snapshot was taken.
    pub tick: u64,
    /// Simulation time in seconds.
    pub sim_time: f64,
    /// All entities in the scene, sorted by (z_index, entity_id).
    pub entities: Vec<SceneEntity>,
    /// Bounding box of all entities with spatial data.
    pub bounds: SceneBounds,
    /// Total entity count (including non-spatial entities).
    pub entity_count: usize,
}
```

**Step 2: Add module to lib.rs**

In `crates/nomai-manifest/src/lib.rs`, add:
```rust
pub mod scene;
pub use scene::{SceneBounds, SceneEntity, SceneSnapshot};
```

**Step 3: Verify it compiles**

Run: `cd crates/nomai-manifest && cargo check`
Expected: compiles clean

**Step 4: Commit**

```bash
git add crates/nomai-manifest/src/scene.rs crates/nomai-manifest/src/lib.rs
git commit -m "feat: add SceneSnapshot types to nomai-manifest"
```

---

### Task 2: Implement scene extraction in nomai-engine

**Files:**
- Create: `crates/nomai-engine/src/scene.rs`
- Modify: `crates/nomai-engine/src/lib.rs` (add `pub mod scene;`)
- Modify: `crates/nomai-engine/src/tick.rs` (add `scene_snapshot()` method to TickLoop)

**Step 1: Create `scene.rs` with extraction logic**

This reuses the same entity-scanning approach as `renderer.rs::extract_draw_commands`
but outputs `SceneSnapshot` instead of `DrawCommand`s. Key difference: includes
velocity, all dynamic components, and entity identity.

```rust
//! Scene snapshot extraction from ECS world state.
//!
//! Mirrors the renderer's entity-scanning logic but outputs structured
//! data instead of GPU draw commands.

use std::collections::HashMap;
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::Identity;
use nomai_ecs::world::World;
use nomai_manifest::scene::{SceneBounds, SceneEntity, SceneSnapshot};

use crate::physics::{PhysicsBody, Position, Velocity, ColliderShape};

/// Extract a full scene snapshot from the current world state.
///
/// Scans all entities for spatial data from two sources:
/// 1. Native physics: entities with Position + PhysicsBody components
/// 2. Dynamic JSON: entities with "position" + "size" dynamic components
///
/// This is the text equivalent of a rendered frame.
pub fn extract_scene_snapshot(
    world: &World,
    tick: u64,
    sim_time: f64,
) -> SceneSnapshot {
    let mut entities = Vec::new();
    let mut rendered = std::collections::HashSet::<EntityId>::new();

    // Phase 1: Native physics entities
    for (entity_id, (pos, body)) in world.query::<(&Position, &PhysicsBody)>() {
        rendered.insert(entity_id);
        let identity = world.get_component::<Identity>(entity_id);
        let vel = world.get_component::<Velocity>(entity_id);
        let visible = is_visible(world, entity_id);
        let z_index = read_z_index(world, entity_id);

        let (w, h) = match &body.collider {
            ColliderShape::Box { half_width, half_height } => {
                (*half_width * 2.0, *half_height * 2.0)
            }
            ColliderShape::Circle { radius } => {
                let d = *radius * 2.0;
                (d, d)
            }
        };

        let (entity_type, role, tier) = extract_identity(identity);
        let components = collect_dynamic_components(world, entity_id);

        entities.push(SceneEntity {
            entity_id: entity_id.into(),
            entity_type,
            role,
            tier,
            position: Some([pos.x, pos.y]),
            size: Some([w, h]),
            velocity: vel.map(|v| [v.dx, v.dy]),
            visible,
            z_index,
            components,
        });
    }

    // Phase 2: Dynamic JSON component entities
    for entity_id in world.all_entity_ids() {
        if rendered.contains(&entity_id) {
            continue;
        }

        let identity = world.get_component::<Identity>(entity_id);
        let visible = is_visible(world, entity_id);
        let z_index = read_z_index(world, entity_id);

        let pos = world.get_component_by_name(entity_id, "position")
            .and_then(|v| {
                let x = v.get("x")?.as_f64()?;
                let y = v.get("y")?.as_f64()?;
                Some([x, y])
            });

        let size = world.get_component_by_name(entity_id, "size")
            .and_then(|v| {
                let w = v.get("w")?.as_f64()?;
                let h = v.get("h")?.as_f64()?;
                Some([w, h])
            });

        let vel = world.get_component_by_name(entity_id, "velocity")
            .and_then(|v| {
                let dx = v.get("dx")?.as_f64()?;
                let dy = v.get("dy")?.as_f64()?;
                Some([dx, dy])
            });

        let (entity_type, role, tier) = extract_identity(identity);
        let components = collect_dynamic_components(world, entity_id);

        entities.push(SceneEntity {
            entity_id: entity_id.into(),
            entity_type,
            role,
            tier,
            position: pos,
            size,
            velocity: vel,
            visible,
            z_index,
            components,
        });
    }

    // Sort by (z_index, entity_id) for deterministic ordering
    entities.sort_by(|a, b| {
        a.z_index.partial_cmp(&b.z_index)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.entity_id.cmp(&b.entity_id))
    });

    // Compute bounds
    let bounds = compute_bounds(&entities);

    SceneSnapshot {
        schema_version: 1,
        tick,
        sim_time,
        entity_count: entities.len(),
        entities,
        bounds,
    }
}

fn is_visible(world: &World, entity_id: EntityId) -> bool {
    world.get_component_by_name(entity_id, "visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn read_z_index(world: &World, entity_id: EntityId) -> f64 {
    world.get_component_by_name(entity_id, "z_index")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

fn extract_identity(identity: Option<&Identity>) -> (String, String, String) {
    match identity {
        Some(id) => (
            id.type_name().to_owned(),
            id.role().unwrap_or("").to_owned(),
            id.tier_name().to_owned(),
        ),
        None => (
            "unknown".to_owned(),
            "unknown".to_owned(),
            "Unknown".to_owned(),
        ),
    }
}

fn collect_dynamic_components(
    world: &World,
    entity_id: EntityId,
) -> HashMap<String, serde_json::Value> {
    let mut components = HashMap::new();
    for name in world.registry().registered_names() {
        if let Some(val) = world.get_component_by_name(entity_id, name) {
            components.insert(name.to_owned(), val);
        }
    }
    components
}

fn compute_bounds(entities: &[SceneEntity]) -> SceneBounds {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    for e in entities {
        if let Some([x, y]) = e.position {
            let half_w = e.size.map(|s| s[0] / 2.0).unwrap_or(0.0);
            let half_h = e.size.map(|s| s[1] / 2.0).unwrap_or(0.0);
            min_x = min_x.min(x - half_w);
            min_y = min_y.min(y - half_h);
            max_x = max_x.max(x + half_w);
            max_y = max_y.max(y + half_h);
        }
    }

    if min_x > max_x {
        // No spatial entities
        return SceneBounds { min_x: 0.0, min_y: 0.0, max_x: 0.0, max_y: 0.0 };
    }

    SceneBounds { min_x, min_y, max_x, max_y }
}
```

**Step 2: Add module to engine lib.rs**

In `crates/nomai-engine/src/lib.rs`, add:
```rust
pub mod scene;
```

**Step 3: Add `scene_snapshot()` to TickLoop**

In `crates/nomai-engine/src/tick.rs`, add a method to the TickLoop impl block:

```rust
use crate::scene::extract_scene_snapshot;

impl TickLoop {
    /// Capture a scene snapshot of the current world state.
    ///
    /// Returns a structured text representation of every entity's
    /// spatial data, identity, and components. This is the text
    /// equivalent of a rendered frame.
    pub fn scene_snapshot(&self) -> SceneSnapshot {
        extract_scene_snapshot(
            &self.world,
            self.tick_counter,
            self.sim_time(),
        )
    }
}
```

**Step 4: Verify it compiles**

Run: `cd crates/nomai-engine && cargo check`
Expected: compiles clean

**Step 5: Commit**

```bash
git add crates/nomai-engine/src/scene.rs crates/nomai-engine/src/lib.rs crates/nomai-engine/src/tick.rs
git commit -m "feat: implement scene snapshot extraction from ECS world"
```

---

### Task 3: Expose scene_snapshot() to Python via PyO3

**Files:**
- Modify: `crates/nomai-python/src/engine.rs` (add scene_snapshot method)

**Step 1: Add the Python method**

In `crates/nomai-python/src/engine.rs`, add to the `#[pymethods] impl PyNomaiEngine` block:

```rust
/// Capture a scene snapshot of the current world state.
///
/// Returns a dict containing every entity's spatial data, identity,
/// and components. This is the text equivalent of a rendered frame.
fn scene_snapshot(&self, py: Python<'_>) -> PyResult<PyObject> {
    let snapshot = self.loop_ref()?.scene_snapshot();
    let json_str = serde_json::to_string(&snapshot).map_err(|e| {
        pyo3::exceptions::PyRuntimeError::new_err(format!(
            "failed to serialize SceneSnapshot to JSON: {e}"
        ))
    })?;
    let json_mod = py.import("json")?;
    let dict = json_mod.call_method1("loads", (json_str,))?;
    Ok(dict.unbind())
}
```

**Step 2: Build the Python extension**

Run: `cd crates/nomai-python && maturin develop --release`
Expected: builds successfully

**Step 3: Quick smoke test**

```python
python -c "
from nomai.engine import NomaiEngine
e = NomaiEngine(headless=True)
e.register_component('position')
e.register_component('size')
e.spawn_entity('character', 'paddle', {'position': {'x': 400, 'y': 500}, 'size': {'w': 100, 'h': 15}})
e.tick()
snap = e._engine.scene_snapshot()
print(f'entities: {len(snap[\"entities\"])}, tick: {snap[\"tick\"]}')
for ent in snap['entities']:
    print(f'  {ent[\"role\"]}: pos={ent.get(\"position\")}, size={ent.get(\"size\")}')
"
```

**Step 4: Commit**

```bash
git add crates/nomai-python/src/engine.rs
git commit -m "feat: expose scene_snapshot() to Python via PyO3"
```

---

### Task 4: Add SceneSnapshot Python dataclasses to nomai-sdk

**Files:**
- Create: `python/nomai-sdk/nomai/scene.py`
- Modify: `python/nomai-sdk/nomai/engine.py` (add scene_snapshot method)
- Modify: `python/nomai-sdk/nomai/__init__.py` (export new types)

**Step 1: Create scene.py with dataclasses**

```python
"""Scene snapshot types — the text equivalent of a rendered frame.

A ``SceneSnapshot`` captures every entity's spatial data, identity,
and components at a single point in time. Combined with
``TickManifest`` (per-tick diffs), this gives AI full observability
without pixel-peeking.
"""

from __future__ import annotations

import json
import logging
from dataclasses import dataclass, field
from typing import Self

logger = logging.getLogger(__name__)


@dataclass(frozen=True)
class SceneEntity:
    """A single entity's state in the scene snapshot."""

    entity_id: int
    entity_type: str
    role: str
    tier: str
    position: tuple[float, float] | None
    size: tuple[float, float] | None
    velocity: tuple[float, float] | None
    visible: bool
    z_index: float
    components: dict[str, object] = field(default_factory=dict)

    def to_dict(self) -> dict[str, object]:
        return {
            "entity_id": self.entity_id,
            "entity_type": self.entity_type,
            "role": self.role,
            "tier": self.tier,
            "position": list(self.position) if self.position else None,
            "size": list(self.size) if self.size else None,
            "velocity": list(self.velocity) if self.velocity else None,
            "visible": self.visible,
            "z_index": self.z_index,
            "components": dict(self.components),
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        raw_pos = data.get("position")
        raw_size = data.get("size")
        raw_vel = data.get("velocity")
        return cls(
            entity_id=int(data["entity_id"]),
            entity_type=str(data.get("entity_type", "unknown")),
            role=str(data.get("role", "unknown")),
            tier=str(data.get("tier", "Unknown")),
            position=tuple(raw_pos) if raw_pos else None,
            size=tuple(raw_size) if raw_size else None,
            velocity=tuple(raw_vel) if raw_vel else None,
            visible=bool(data.get("visible", True)),
            z_index=float(data.get("z_index", 0.0)),
            components=dict(data.get("components", {})),
        )


@dataclass(frozen=True)
class SceneBounds:
    """Axis-aligned bounding box of the scene."""

    min_x: float
    min_y: float
    max_x: float
    max_y: float

    def to_dict(self) -> dict[str, float]:
        return {
            "min_x": self.min_x,
            "min_y": self.min_y,
            "max_x": self.max_x,
            "max_y": self.max_y,
        }

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        return cls(
            min_x=float(data.get("min_x", 0.0)),
            min_y=float(data.get("min_y", 0.0)),
            max_x=float(data.get("max_x", 0.0)),
            max_y=float(data.get("max_y", 0.0)),
        )


@dataclass(frozen=True)
class SceneSnapshot:
    """A complete snapshot of the game scene at a single tick.

    This is the text equivalent of a rendered frame. Combined with
    ``TickManifest`` (per-tick diffs), this gives AI full observability.
    """

    schema_version: int
    tick: int
    sim_time: float
    entities: list[SceneEntity]
    bounds: SceneBounds
    entity_count: int

    def to_dict(self) -> dict[str, object]:
        return {
            "schema_version": self.schema_version,
            "tick": self.tick,
            "sim_time": self.sim_time,
            "entities": [e.to_dict() for e in self.entities],
            "bounds": self.bounds.to_dict(),
            "entity_count": self.entity_count,
        }

    def to_json(self, indent: int | None = 2) -> str:
        return json.dumps(self.to_dict(), indent=indent)

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> Self:
        raw_entities = data.get("entities", [])
        entities = [SceneEntity.from_dict(e) for e in raw_entities]
        raw_bounds = data.get("bounds", {})
        bounds = SceneBounds.from_dict(raw_bounds)
        return cls(
            schema_version=int(data.get("schema_version", 1)),
            tick=int(data.get("tick", 0)),
            sim_time=float(data.get("sim_time", 0.0)),
            entities=entities,
            bounds=bounds,
            entity_count=int(data.get("entity_count", len(entities))),
        )

    def entity_by_role(self, role: str) -> SceneEntity | None:
        """Find the first entity with the given role."""
        for e in self.entities:
            if e.role == role:
                return e
        return None

    def entities_by_role(self, role: str) -> list[SceneEntity]:
        """Find all entities with the given role."""
        return [e for e in self.entities if e.role == role]

    def entities_by_type(self, entity_type: str) -> list[SceneEntity]:
        """Find all entities with the given type."""
        return [e for e in self.entities if e.entity_type == entity_type]

    def summary(self) -> str:
        """Human-readable summary of the scene."""
        lines = [
            f"Scene @ tick {self.tick} (t={self.sim_time:.3f}s)",
            f"  Entities: {self.entity_count}",
            f"  Bounds: ({self.bounds.min_x:.0f},{self.bounds.min_y:.0f}) to ({self.bounds.max_x:.0f},{self.bounds.max_y:.0f})",
        ]
        for e in self.entities:
            pos_str = f"({e.position[0]:.1f},{e.position[1]:.1f})" if e.position else "none"
            size_str = f"{e.size[0]:.0f}x{e.size[1]:.0f}" if e.size else "none"
            vel_str = f"v=({e.velocity[0]:.1f},{e.velocity[1]:.1f})" if e.velocity else ""
            vis = "" if e.visible else " [hidden]"
            lines.append(f"  [{e.entity_id}] {e.role} ({e.entity_type}) @ {pos_str} {size_str} {vel_str}{vis}")
        return "\n".join(lines)
```

**Step 2: Add scene_snapshot() to NomaiEngine**

In `python/nomai-sdk/nomai/engine.py`, add:

```python
from nomai.scene import SceneSnapshot

class NomaiEngine:
    def scene_snapshot(self) -> SceneSnapshot:
        """Capture a scene snapshot of the current world state.

        Returns a structured text representation of every entity's
        spatial data, identity, and components. This is the text
        equivalent of a rendered frame.
        """
        raw = self._engine.scene_snapshot()
        return SceneSnapshot.from_dict(raw)
```

**Step 3: Commit**

```bash
git add python/nomai-sdk/nomai/scene.py python/nomai-sdk/nomai/engine.py
git commit -m "feat: add SceneSnapshot Python types and engine method"
```

---

### Task 5: Write tests

**Files:**
- Create: `python/nomai-sdk/tests/test_scene.py`
- Rust test: `crates/nomai-manifest/tests/scene_tests.rs` (serde round-trip)

**Step 1: Write Python tests**

```python
"""Tests for SceneSnapshot types and serialization."""

from nomai.scene import SceneBounds, SceneEntity, SceneSnapshot


class TestSceneEntity:
    def test_creation(self):
        e = SceneEntity(
            entity_id=1, entity_type="character", role="paddle",
            tier="Semantic", position=(400.0, 500.0), size=(100.0, 15.0),
            velocity=None, visible=True, z_index=0.0,
        )
        assert e.role == "paddle"
        assert e.position == (400.0, 500.0)

    def test_serialization_round_trip(self):
        e = SceneEntity(
            entity_id=1, entity_type="projectile", role="ball",
            tier="Semantic", position=(200.0, 300.0), size=(16.0, 16.0),
            velocity=(100.0, -150.0), visible=True, z_index=1.0,
            components={"score": 10},
        )
        d = e.to_dict()
        e2 = SceneEntity.from_dict(d)
        assert e2.entity_id == e.entity_id
        assert e2.velocity == (100.0, -150.0)
        assert e2.components == {"score": 10}


class TestSceneBounds:
    def test_round_trip(self):
        b = SceneBounds(min_x=-10.0, min_y=0.0, max_x=810.0, max_y=600.0)
        b2 = SceneBounds.from_dict(b.to_dict())
        assert b2 == b


class TestSceneSnapshot:
    def test_creation_and_summary(self):
        snap = SceneSnapshot(
            schema_version=1, tick=42, sim_time=0.7,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0),
                    size=(100.0, 15.0), velocity=None,
                    visible=True, z_index=0.0,
                ),
                SceneEntity(
                    entity_id=2, entity_type="projectile", role="ball",
                    tier="Semantic", position=(200.0, 300.0),
                    size=(16.0, 16.0), velocity=(200.0, -300.0),
                    visible=True, z_index=1.0,
                ),
            ],
            bounds=SceneBounds(min_x=150.0, min_y=292.0, max_x=450.0, max_y=567.5),
            entity_count=2,
        )
        assert snap.entity_count == 2
        assert snap.entity_by_role("paddle") is not None
        assert snap.entity_by_role("ball").velocity == (200.0, -300.0)
        assert len(snap.entities_by_type("character")) == 1
        summary = snap.summary()
        assert "tick 42" in summary
        assert "paddle" in summary

    def test_json_round_trip(self):
        snap = SceneSnapshot(
            schema_version=1, tick=10, sim_time=0.166,
            entities=[
                SceneEntity(
                    entity_id=1, entity_type="character", role="paddle",
                    tier="Semantic", position=(400.0, 560.0),
                    size=(100.0, 15.0), velocity=None,
                    visible=True, z_index=0.0,
                ),
            ],
            bounds=SceneBounds(min_x=350.0, min_y=552.5, max_x=450.0, max_y=567.5),
            entity_count=1,
        )
        json_str = snap.to_json()
        import json
        data = json.loads(json_str)
        snap2 = SceneSnapshot.from_dict(data)
        assert snap2.tick == snap.tick
        assert snap2.entities[0].role == "paddle"
        assert snap2.schema_version == 1
```

**Step 2: Run tests**

Run: `cd python/nomai-sdk && python -m pytest tests/test_scene.py -v`
Expected: all pass (pure Python, no engine needed)

**Step 3: Write Rust serde test**

In `crates/nomai-manifest/tests/scene_tests.rs`:

```rust
use nomai_manifest::scene::{SceneBounds, SceneEntity, SceneSnapshot};
use std::collections::HashMap;

#[test]
fn scene_snapshot_serialization_roundtrip() {
    let snapshot = SceneSnapshot {
        schema_version: 1,
        tick: 42,
        sim_time: 0.7,
        entities: vec![
            SceneEntity {
                entity_id: 1,
                entity_type: "character".into(),
                role: "paddle".into(),
                tier: "Semantic".into(),
                position: Some([400.0, 560.0]),
                size: Some([100.0, 15.0]),
                velocity: None,
                visible: true,
                z_index: 0.0,
                components: HashMap::new(),
            },
        ],
        bounds: SceneBounds {
            min_x: 350.0, min_y: 552.5,
            max_x: 450.0, max_y: 567.5,
        },
        entity_count: 1,
    };

    let json = serde_json::to_string(&snapshot).unwrap();
    let deserialized: SceneSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.tick, 42);
    assert_eq!(deserialized.entities.len(), 1);
    assert_eq!(deserialized.entities[0].role, "paddle");
    assert_eq!(deserialized.schema_version, 1);
}

#[test]
fn scene_entity_with_all_fields() {
    let mut comps = HashMap::new();
    comps.insert("score".into(), serde_json::json!(10));

    let entity = SceneEntity {
        entity_id: 2,
        entity_type: "projectile".into(),
        role: "ball".into(),
        tier: "Semantic".into(),
        position: Some([200.0, 300.0]),
        size: Some([16.0, 16.0]),
        velocity: Some([100.0, -150.0]),
        visible: true,
        z_index: 1.0,
        components: comps,
    };

    let json = serde_json::to_string(&entity).unwrap();
    let de: SceneEntity = serde_json::from_str(&json).unwrap();
    assert_eq!(de.velocity, Some([100.0, -150.0]));
    assert_eq!(de.components["score"], 10);
}
```

**Step 4: Run Rust tests**

Run: `cd crates/nomai-manifest && cargo test`
Expected: all pass

**Step 5: Commit**

```bash
git add python/nomai-sdk/tests/test_scene.py crates/nomai-manifest/tests/scene_tests.rs
git commit -m "test: scene snapshot serialization and Python types"
```

---

### Task 6: Integration test with live engine

**Files:**
- Create: `python/nomai-sdk/tests/test_scene_integration.py`

**Step 1: Write integration test**

```python
"""Integration test: scene snapshot from live engine."""

import pytest
from nomai.engine import NomaiEngine
from nomai.scene import SceneSnapshot


@pytest.fixture
def breakout_engine():
    """Create a minimal breakout engine with paddle and ball."""
    engine = NomaiEngine(headless=True, fixed_dt=1.0 / 60.0)
    engine.register_component("position")
    engine.register_component("velocity")
    engine.register_component("size")
    engine.init_physics()

    engine.spawn_entity("character", "paddle", {
        "position": {"x": 400.0, "y": 560.0},
        "size": {"w": 100.0, "h": 15.0},
    })
    engine.spawn_entity("projectile", "ball", {
        "position": {"x": 400.0, "y": 300.0},
        "velocity": {"dx": 200.0, "dy": -300.0},
    })
    engine.tick()  # apply spawns
    return engine


class TestSceneSnapshotIntegration:
    def test_snapshot_has_entities(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        assert isinstance(snap, SceneSnapshot)
        assert snap.entity_count >= 2
        assert snap.schema_version == 1

    def test_paddle_in_snapshot(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        paddle = snap.entity_by_role("paddle")
        assert paddle is not None
        assert paddle.entity_type == "character"
        assert paddle.position is not None
        assert abs(paddle.position[0] - 400.0) < 1.0

    def test_ball_has_velocity(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        ball = snap.entity_by_role("ball")
        assert ball is not None
        assert ball.velocity is not None

    def test_snapshot_advances_with_tick(self, breakout_engine):
        snap1 = breakout_engine.scene_snapshot()
        breakout_engine.tick()
        snap2 = breakout_engine.scene_snapshot()
        assert snap2.tick == snap1.tick + 1

    def test_snapshot_deterministic(self, breakout_engine):
        """Same state produces identical snapshots."""
        snap1 = breakout_engine.scene_snapshot()
        snap2 = breakout_engine.scene_snapshot()
        assert snap1.to_json() == snap2.to_json()

    def test_summary_readable(self, breakout_engine):
        snap = breakout_engine.scene_snapshot()
        summary = snap.summary()
        assert "paddle" in summary
        assert "ball" in summary
```

**Step 2: Run integration tests**

Run: `cd python/nomai-sdk && python -m pytest tests/test_scene_integration.py -v`
Expected: all pass

**Step 3: Commit**

```bash
git add python/nomai-sdk/tests/test_scene_integration.py
git commit -m "test: scene snapshot integration with live engine"
```

---

### Task 7: Build and verify end-to-end

**Step 1: Full Rust build**

Run: `cargo build --release`
Expected: clean build

**Step 2: Build Python extension**

Run: `cd crates/nomai-python && maturin develop --release`
Expected: builds and installs

**Step 3: Run all tests**

Run: `cd python/nomai-sdk && python -m pytest tests/ -v`
Expected: all tests pass including new scene tests

**Step 4: Run demo to verify**

```python
python -c "
from nomai.engine import NomaiEngine
e = NomaiEngine(headless=True, fixed_dt=1/60)
e.register_component('position')
e.register_component('velocity')
e.register_component('size')
e.init_physics()
e.spawn_entity('character', 'paddle', {'position': {'x': 400, 'y': 560}, 'size': {'w': 100, 'h': 15}})
e.spawn_entity('projectile', 'ball', {'position': {'x': 400, 'y': 300}, 'velocity': {'dx': 200, 'dy': -300}})
e.tick()
snap = e.scene_snapshot()
print(snap.summary())
print()
print('--- JSON (first 500 chars) ---')
print(snap.to_json()[:500])
"
```

**Step 5: Final commit**

```bash
git add -A
git commit -m "feat: complete scene snapshot -- text representation of rendered frame"
```
