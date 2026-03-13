# Nomai SDK Reference (for AI agents)

> This document helps you build games with the Nomai engine.
> For a complete working example, read `run_eval_baseline.py`.

## Quick Start

```python
from nomai.engine import NomaiEngine
from nomai.scene import SceneSnapshot

engine = NomaiEngine(headless=True, fixed_dt=1.0/60.0)
```

See working example: `run_eval_baseline.py` (lines 73-156 for full setup)

## Core Concepts

- **ECS Architecture**: Entities have numeric IDs. Components are key-value dicts
  attached by name. No inheritance — entity behavior comes from which components
  it has and its physics configuration.
- **Entity Types & Roles**: Each entity has a `type` (category like "character",
  "projectile", "destructible", "boundary") and a `role` (specific name like
  "paddle", "ball", "brick", "wall_top").
- **Manifests**: Every `tick()` returns a `TickManifest` recording all state changes
  that happened during that tick. This is your primary debugging tool.
- **Snapshots**: `engine.scene_snapshot()` returns a `SceneSnapshot`. Call
  `.summary()` on it for a human-readable text description of the full game state.
  Use this to verify your work.
- **Physics**: Rapier2D-based. You must `init_physics()` before registering physics
  bodies. Bodies can be `"dynamic"` (moves), `"kinematic"` (player-controlled),
  or `"static"` (fixed).

## API Reference

### Engine Setup

| Step | Method | Signature | Example |
|------|--------|-----------|---------|
| Create engine | `NomaiEngine()` | `NomaiEngine(headless=True, fixed_dt=1.0/60.0)` | `run_eval_baseline.py:75` |
| Register component | `register_component()` | `engine.register_component(name: str)` | `:76-81` |
| Init physics | `init_physics()` | `engine.init_physics()` | `:82` |

### Entity Management

| Step | Method | Signature | Example |
|------|--------|-----------|---------|
| Spawn entity | `spawn_entity()` | `engine.spawn_entity(entity_type: str, role: str, components: dict = None)` | `:84-106` |
| Despawn entity | `despawn_entity()` | `engine.despawn_entity(entity_id: int)` | N/A |
| Set component | `set_component()` | `engine.set_component(entity_id: int, component: str, value: Any)` | N/A |
| Get entity | `get_entity()` | `engine.get_entity(entity_id: int) -> EntityEntry` | N/A |
| List entities | `entity_index()` | `engine.entity_index() -> list[EntityEntry]` | `:110` |

### Physics Bodies

| Step | Method | Signature | Example |
|------|--------|-----------|---------|
| Register body | `register_physics_entity()` | See below | `:115-151` |

Full signature:
```python
engine.register_physics_entity(
    entity_id: int,
    x: float, y: float,          # position
    dx: float, dy: float,        # velocity
    body_type: str,               # "dynamic", "kinematic", or "static"
    collider_type: str,           # "circle" or "box"
    collider_radius: float = None,          # for "circle"
    collider_half_width: float = None,      # for "box"
    collider_half_height: float = None,     # for "box"
    restitution: float = 0.5,               # bounciness (1.0 = perfect)
    is_sensor: bool = False,                # trigger zone, no collision
)
```

### Simulation

| Step | Method | Signature | Example |
|------|--------|-----------|---------|
| Single tick | `tick()` | `engine.tick() -> TickManifest` | `:108` |
| Multiple ticks | `run_ticks()` | `engine.run_ticks(n: int) -> list[TickManifest]` | N/A |
| Conditional run | `run_until()` | `engine.run_until(condition, max_ticks=10000)` | N/A |

### Observation

| Step | Method | Signature | Example |
|------|--------|-----------|---------|
| Scene snapshot | `scene_snapshot()` | `engine.scene_snapshot() -> SceneSnapshot` | N/A |
| Snapshot text | `summary()` | `snapshot.summary() -> str` | N/A |
| State hash | `state_hash()` | `engine.state_hash() -> str` | N/A |
| Manifest history | `manifest_history()` | `engine.manifest_history() -> list[TickManifest]` | N/A |

### WASM Gameplay

| Step | Method | Signature | Example |
|------|--------|-----------|---------|
| Load WASM | `load_gameplay_wasm()` | `engine.load_gameplay_wasm(wasm_bytes: bytes)` | `:153-154` |

## Common Patterns

### Setup Order (critical)

```python
# 1. Create engine
engine = NomaiEngine(headless=True, fixed_dt=1.0/60.0)

# 2. Register ALL components BEFORE spawning entities
engine.register_component("position")
engine.register_component("velocity")
engine.register_component("size")

# 3. Init physics BEFORE registering physics bodies
engine.init_physics()

# 4. Spawn entities with initial components
engine.spawn_entity("projectile", "ball", {
    "position": {"x": 400, "y": 300},
    "velocity": {"dx": 200, "dy": -150},
})

# 5. Tick once to apply spawns
engine.tick()

# 6. Get entity IDs from index
index = engine.entity_index()
ball_id = next(e.entity_id for e in index if e.role == "ball")

# 7. Register physics bodies using entity IDs
engine.register_physics_entity(
    ball_id, 400, 300, 200, -150,
    "dynamic", "circle",
    collider_radius=8.0,
    restitution=1.0,
)

# 8. Load gameplay WASM
wasm_bytes = Path("gameplay/build/gameplay.wasm").read_bytes()
engine.load_gameplay_wasm(wasm_bytes)
```

### Verifying Your Work

```python
# After running ticks, check the scene
snapshot = engine.scene_snapshot()
print(snapshot.summary())  # Human-readable game state

# Check specific entities
ball = snapshot.entity_by_role("ball")
bricks = snapshot.entities_by_role("brick")
print(f"Ball position: {ball}")
print(f"Remaining bricks: {len(bricks)}")
```

### Handling Collisions

The baseline handles brick destruction by checking manifests for collision events.
See `run_eval_baseline.py:618-645` for the simulation loop that despawns bricks
on ball-brick collision.

## File Locations

| What | Path |
|------|------|
| Engine class | `python/nomai-sdk/nomai/engine.py` |
| Scene classes | `python/nomai-sdk/nomai/scene.py` |
| Manifest types | `python/nomai-sdk/nomai/manifest.py` |
| WASM gameplay | `gameplay/build/gameplay.wasm` |
| Working example | `run_eval_baseline.py` |

## Debugging Tips

- If entities aren't appearing: did you call `engine.tick()` after spawning?
- If physics aren't working: did you call `init_physics()` before `register_physics_entity()`?
- If ball passes through walls: check `restitution=1.0` and collider dimensions
- To see what changed: inspect the `TickManifest` returned by `tick()`
- To see full game state: call `engine.scene_snapshot().summary()`
