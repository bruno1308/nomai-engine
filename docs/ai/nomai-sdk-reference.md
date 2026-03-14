# Nomai SDK Reference (for AI agents)

> This document helps you build games with the Nomai engine.
> The engine has two layers: **Python** for setup and verification,
> **WASM (AssemblyScript)** for runtime game logic.

## Architecture — What Goes Where

| Layer | Language | Responsibility |
|-------|----------|----------------|
| **Python** | Python 3.12+ | Engine setup, entity spawning, physics config, verification, snapshots |
| **WASM** | AssemblyScript | Runtime game logic: collision responses, scoring, state machines |

**Why two layers?** The engine runs a tick loop that drives physics and renders
every frame. Python can't inject code into this loop — only WASM runs inside
it. Game logic written in Python only runs during headless simulation
(`engine.tick()`). Game logic written in WASM runs in both headless and visual
mode (`engine.run()`). If you want your game to work visually, put runtime
logic in WASM.

## Quick Start

```python
from nomai.engine import NomaiEngine

engine = NomaiEngine(headless=True, fixed_dt=1.0/60.0)
```

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
- **Coordinate System**: Y-up (math convention). Y=0 is the bottom of the screen,
  Y=600 is the top. Place bricks at high Y, paddle at low Y.
- **Collision Routing**: When physics detects a collision, the engine automatically
  calls the WASM export `on_collision(entity_a: i64, entity_b: i64)` if it exists.
  Your WASM module decides what to do (despawn, score, etc.).

## Python API Reference

### Engine Setup

| Method | Signature |
|--------|-----------|
| Create engine | `NomaiEngine(headless=True, fixed_dt=1.0/60.0)` |
| Register component | `engine.register_component(name: str)` |
| Init physics | `engine.init_physics()` |

### Entity Management

| Method | Signature |
|--------|-----------|
| Spawn entity | `engine.spawn_entity(entity_type: str, role: str, components: dict = None)` |
| Despawn entity | `engine.despawn_entity(entity_id: int)` |
| Set component | `engine.set_component(entity_id: int, component: str, value: Any)` |
| Get entity | `engine.get_entity(entity_id: int) -> EntityEntry` |
| List entities | `engine.entity_index() -> list[EntityEntry]` |

### Physics Bodies

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

| Method | Signature |
|--------|-----------|
| Single tick | `engine.tick() -> TickManifest` |
| Multiple ticks | `engine.run_ticks(n: int) -> list[TickManifest]` |
| Visual window | `engine.run(title="Game", width=800, height=600)` |

### Observation

| Method | Signature |
|--------|-----------|
| Scene snapshot | `engine.scene_snapshot() -> SceneSnapshot` |
| Snapshot text | `snapshot.summary() -> str` |
| State hash | `engine.state_hash() -> str` |

### WASM

| Method | Signature |
|--------|-----------|
| Load WASM | `engine.load_gameplay_wasm(wasm_bytes: bytes)` |
| Call WASM export | `engine.call_wasm_export(name: str, *args: int)` |

## WASM Gameplay (AssemblyScript)

### How Collision Routing Works

The engine tick loop is:
1. Run physics → detect collisions
2. Call WASM `tick()` export
3. For each collision pair, call WASM `on_collision(entity_a, entity_b)` if exported
4. Drain all WASM commands → apply to world → update manifest

Your WASM module receives collision pairs automatically. You decide what to do
with them (check entity types, despawn, update score, etc.).

### Host API (available from WASM)

These functions are imported from the `"nomai"` namespace in your AssemblyScript:

**Read functions:**
- `get_entity_count(): i32` — number of alive entities
- `sim_time(): f64` — current simulation time in seconds
- `tick_number(): i64` — current tick count

**Write functions (deferred — applied after WASM finishes):**
- `set_component(entityId, name, valueJson, reason)` — update a component (JSON value)
- `despawn_entity(entityId, reason)` — remove an entity
- `spawn_semantic(identityJson, componentsJson, reason)` — spawn a tracked entity
- `emit_event(eventJson)` — emit a game event for the manifest
- `log_msg(level, message)` — log (0=trace, 1=debug, 2=info, 3=warn, 4=error)

Every write carries a `reason` string that feeds into the manifest's causal chain.

### Writing a WASM Gameplay Module

Create an AssemblyScript file (e.g. `gameplay/assembly/my_game.ts`):

```typescript
import {
  get_entity_count, tick_number,
  set_component, despawn_entity, emit_event, log_msg,
} from "./host";

// Module-local state (resets on hot-swap — persistent state lives in ECS)
let score: i32 = 0;

// Called once per tick after physics
export function tick(): void {
  // Per-tick logic here (e.g. push score to ECS)
}

// Called automatically for each physics collision pair
export function on_collision(entityA: i64, entityB: i64): void {
  // Check entity types and respond
  // e.g. despawn a brick, update score, emit event
  despawn_entity(entityA, "destroyed_by_collision");
  score += 100;
}
```

### Compiling WASM

```bash
cd gameplay
npm install                    # first time only
npx asc assembly/my_game.ts --outFile build/my_game.wasm --optimize --exportRuntime
```

### Loading WASM in Python

```python
from pathlib import Path

wasm_path = Path("gameplay/build/my_game.wasm")
engine.load_gameplay_wasm(wasm_path.read_bytes())
```

### Example: Breakout Collision Handler

See `gameplay/assembly/breakout.ts` for a complete example. The key pattern:

```typescript
export function on_collision(entityA: i64, entityB: i64): void {
  // The engine calls this for every collision pair.
  // You need to figure out which entity is which and respond.
  // For breakout: if one entity is a brick, despawn it.
  despawn_entity(entityA, "brick_destroyed_by_ball");
  despawn_entity(entityB, "brick_destroyed_by_ball");
  // The engine ignores despawn commands for already-dead entities,
  // so it's safe to despawn both — only the brick will actually die.
}
```

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
    "velocity": {"dx": 200, "dy": 300},  # positive dy = upward
})

# 5. Tick once to apply spawns
engine.tick()

# 6. Get entity IDs from index
index = engine.entity_index()
ball_id = next(e.entity_id for e in index if e.role == "ball")

# 7. Register physics bodies using entity IDs
engine.register_physics_entity(
    ball_id, 400, 300, 200, 300,
    "dynamic", "circle",
    collider_radius=8.0,
    restitution=1.0,
)

# 8. Compile and load gameplay WASM (game logic lives here)
# See "Compiling WASM" section above
wasm_bytes = Path("gameplay/build/my_game.wasm").read_bytes()
engine.load_gameplay_wasm(wasm_bytes)
```

### Verifying Your Work

```python
# After running ticks, check the scene
snapshot = engine.scene_snapshot()
print(snapshot.summary())  # Human-readable game state
```

### Visual Mode

```python
# Opens a window — WASM game logic runs every frame
engine.run(title="My Game", width=800, height=600)
```

The visual output and the headless simulation produce identical results
because both use the same tick loop (physics → WASM → commands → manifest).

## File Locations

| What | Path |
|------|------|
| Engine class | `python/nomai-sdk/nomai/engine.py` |
| Scene classes | `python/nomai-sdk/nomai/scene.py` |
| Manifest types | `python/nomai-sdk/nomai/manifest.py` |
| Host API bindings | `gameplay/assembly/host.ts` |
| Example: breakout | `gameplay/assembly/breakout.ts` |
| WASM build output | `gameplay/build/` |

## Debugging Tips

- If entities aren't appearing: did you call `engine.tick()` after spawning?
- If physics aren't working: did you call `init_physics()` before `register_physics_entity()`?
- If ball passes through walls: check `restitution=1.0` and collider dimensions
- If collisions don't trigger game logic: does your WASM export `on_collision`?
- To see what changed: inspect the `TickManifest` returned by `tick()`
- To see full game state: call `engine.scene_snapshot().summary()`
- Coordinate system is Y-up: paddle at low Y (bottom), bricks at high Y (top)
