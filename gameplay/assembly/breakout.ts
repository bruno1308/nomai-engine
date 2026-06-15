// Breakout game WASM module
// Handles collision responses: logs collisions for Python to handle

import {
  log_msg,
} from "./host";

/**
 * Called once per tick after physics simulation.
 * Currently unused for breakout, but required as an export.
 */
export function tick(): void {
  // Per-tick game logic would go here (e.g., scoring, state changes)
  // For breakout, all logic happens in collision handlers
}

/**
 * Called automatically for each physics collision pair detected by rapier2d.
 *
 * For breakout, we just log collisions. Python handles brick destruction.
 */
export function on_collision(entityA: i64, entityB: i64): void {
  // Log the collision for debugging
  log_msg(2, "Collision: " + entityA.toString() + " <-> " + entityB.toString());
}
