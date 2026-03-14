# Task: Breakout Game

## Game Description
A classic breakout game. A paddle at the bottom of the screen, a ball
that bounces, and a grid of bricks at the top. The ball destroys bricks
on contact.

## Coordinate System
The engine uses Y-up coordinates: Y=0 is the bottom of the screen,
Y=600 is the top. Place bricks at high Y values, paddle at low Y.

## Required Entities
- 1 paddle (type: "character", role: "paddle")
  - Position: bottom center (x=400, y=40)
  - Size: width=100, height=15
  - Body type: kinematic
- 1 ball (type: "projectile", role: "ball")
  - Position: center area (x=400, y=300)
  - Size: 8x8
  - Initial velocity: dx=200, dy=300 (positive dy = upward toward bricks)
  - Body type: dynamic, circle collider (radius=8)
- 20 bricks (type: "destructible", role: "brick")
  - Layout: 4 rows x 5 columns
  - Each brick: width=60, height=20, spacing=10
  - Starting Y around 450, centered horizontally
  - Body type: static
- 4 boundary walls (type: "boundary")
  - wall_top: top edge (y = GAME_HEIGHT + offset)
  - wall_bottom: bottom edge (y = -offset)
  - wall_left: left edge (x = -offset)
  - wall_right: right edge (x = GAME_WIDTH + offset)
  - Body type: static

## Game Area
- Width: 800, Height: 600

## Required Behaviors
- Ball bounces off paddle, walls, and bricks (restitution=1.0)
- Ball destroys brick on contact (despawn the brick entity)
- Score increases when bricks are destroyed
- All game logic (collision responses) must be in WASM, not Python

## Architecture — Two Files Required

### 1. WASM Gameplay Module (AssemblyScript)
Write an AssemblyScript file that handles collision responses. The engine
automatically calls `on_collision(entity_a: i64, entity_b: i64)` for each
physics collision pair. Your handler should despawn bricks when hit.

See `gameplay/assembly/host.ts` for available host functions:
- `despawn_entity(entityId, reason)` — remove an entity
- `set_component(entityId, name, valueJson, reason)` — update a component
- `emit_event(eventJson)` — emit a game event for the manifest
- `log_msg(level, message)` — log a message

Example collision handler:
```typescript
import { despawn_entity, log_msg } from "./host";

export function tick(): void { }

export function on_collision(entityA: i64, entityB: i64): void {
  // Despawn both — the engine ignores despawn for non-destructible entities
  despawn_entity(entityA, "collision_destroy");
  despawn_entity(entityB, "collision_destroy");
}
```

Compile with:
```bash
cd gameplay && npx asc assembly/my_breakout.ts --outFile build/my_breakout.wasm --optimize --exportRuntime
```

### 2. Python Setup Script (game.py)
The Python script handles:
- Engine creation and setup
- Entity spawning and physics registration
- Loading the compiled WASM module
- Running the simulation
- Verification via snapshots

## Physics Setup
- Use `init_physics()` before registering physics bodies
- Ball: dynamic body, circle collider (radius=8), restitution=1.0
- Paddle: kinematic body, box collider, restitution=1.0
- Bricks: static body, box collider, restitution=1.0
- Walls: static body, box collider, restitution=1.0

## Simulation
- Run for 300 ticks with `fixed_dt=1.0/60.0`
- The WASM `on_collision` handler runs automatically each tick — you do NOT
  need to handle collisions in Python
- After simulation, verify with `engine.scene_snapshot().summary()`

## Success Criteria
After 300 ticks:
- All entity types exist with correct roles
- Ball has moved from starting position
- Ball is within game bounds (0-800, 0-600)
- Physics collisions are working (ball bounces off walls/paddle)
- At least some bricks have been destroyed (despawned by WASM handler)

## Output
Your output consists of TWO files:

1. **AssemblyScript file** at `gameplay/assembly/my_breakout.ts`
   - Must export `tick()` and `on_collision(entity_a: i64, entity_b: i64)`
   - Import host functions from `"./host"`
   - Compile to WASM before loading

2. **Python script** at `game.py` in the working directory
   - Create the engine with `NomaiEngine(headless=True, fixed_dt=1.0/60.0)`
   - Register components, init physics, spawn all entities
   - Register physics bodies for all entities
   - Compile the AssemblyScript: run `npx asc` via subprocess
   - Load the compiled WASM: `engine.load_gameplay_wasm(wasm_bytes)`
   - Run for 300 ticks (collision handling is automatic via WASM)
   - Print the final `engine.scene_snapshot().summary()` to stdout
   - Print `ENTITY_COUNT: <N>` as the last line of stdout
   - Save the final snapshot as `snapshot.json`

## Reference
- See `docs/ai/nomai-sdk-reference.md` for the full API reference
- See `gameplay/assembly/host.ts` for WASM host function bindings
- See `gameplay/assembly/breakout.ts` for an example WASM gameplay module
