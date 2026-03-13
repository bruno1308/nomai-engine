# Task: Breakout Game

## Game Description
A classic breakout game. A paddle at the bottom of the screen, a ball
that bounces, and a grid of bricks at the top. The ball destroys bricks
on contact.

## Required Entities
- 1 paddle (type: "character", role: "paddle")
  - Position: bottom center (x=400, y=560)
  - Size: width=100, height=15
- 1 ball (type: "projectile", role: "ball")
  - Position: center area (x=400, y=300)
  - Size: 8x8
  - Initial velocity: dx=200, dy=-300
- 20 bricks (type: "destructible", role: "brick")
  - Layout: 4 rows x 5 columns
  - Each brick: width=60, height=20, spacing=10
  - Starting Y around 60, centered horizontally
- 3 boundary walls (type: "boundary")
  - wall_top (role: "wall_top"): top edge
  - wall_left (role: "wall_left"): left edge
  - wall_right (role: "wall_right"): right edge

## Game Area
- Width: 800, Height: 600

## Required Behaviors
- Ball bounces off paddle, walls, and bricks (restitution=1.0)
- Ball destroys brick on contact (despawn the brick entity)
- Ball velocity is preserved through bounces
- Paddle is kinematic (does not move from physics)
- Walls and bricks are static

## Physics Setup
- Use `init_physics()` before registering physics bodies
- Ball: dynamic body, circle collider (radius=8)
- Paddle: kinematic body, box collider
- Bricks: static body, box collider
- Walls: static body, box collider

## WASM Gameplay
- Load from: `gameplay/build/gameplay.wasm`

## Simulation
- Run for 300 ticks with `fixed_dt=1.0/60.0`
- During simulation, check each tick's manifest for collision events
- When ball hits a brick (collision event involving both), despawn the brick

## Success Criteria
After 300 ticks:
- All entity types exist with correct roles
- Ball has moved from starting position
- Physics collisions are working (ball bounces off walls/paddle)
- At least some bricks have been destroyed

## Output
Your script must be a single Python file at the path specified by the harness.
The script must:
1. Create the engine with `NomaiEngine(headless=True, fixed_dt=1.0/60.0)`
2. Register components, init physics, spawn all entities
3. Register physics bodies for all entities
4. Load the WASM gameplay
5. Run for 300 ticks with collision-based brick despawning
6. Print the final `engine.scene_snapshot().summary()` to stdout
7. Print `ENTITY_COUNT: <N>` as the last line of stdout

## Reference
- See `docs/ai/nomai-sdk-reference.md` for the full API reference
- See `run_eval_baseline.py` for a complete working example
