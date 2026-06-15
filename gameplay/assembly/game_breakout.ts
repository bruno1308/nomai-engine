// Improved Breakout gameplay logic for the Nomai engine.
//
// This version maintains a set of brick entity IDs so we can identify
// and destroy them on collision.
//
// Design:
// - `initialize_bricks(ids, count)` is called from Python with brick IDs
// - `on_collision(entityA, entityB)` checks if either is a brick and despawns it
// - `tick()` is called once per engine tick for any per-frame logic

import {
  despawn_entity,
  log_msg,
} from "./host";

// Maximum number of bricks we'll track
const MAX_BRICKS: i32 = 256;

// Array of brick entity IDs
let brick_ids: i64[] = new Array(MAX_BRICKS);
let brick_count: i32 = 0;

// ---------------------------------------------------------------------------
// Exported functions called from Python
// ---------------------------------------------------------------------------

/**
 * Initialize the set of brick entity IDs.
 * Called once from Python after spawning all bricks but before running the simulation.
 */
export function initialize_bricks(ids_ptr: i32, ids_len: i32): void {
  // The ids_ptr points to a buffer of i64 values in linear memory
  // We need to copy them into our brick_ids array
  // For now, this is a placeholder - we'll use an alternative approach
  log_msg(2, "initialize_bricks called with " + ids_len.toString() + " bricks");
}

/**
 * Add a brick ID to track
 */
export function add_brick_id(brick_id: i64): void {
  if (brick_count < MAX_BRICKS) {
    brick_ids[brick_count] = brick_id;
    brick_count += 1;
  }
}

/**
 * Check if an entity ID is a brick
 */
function is_brick(entity_id: i64): bool {
  for (let i: i32 = 0; i < brick_count; i++) {
    if (brick_ids[i] == entity_id) {
      return true;
    }
  }
  return false;
}

// ---------------------------------------------------------------------------
// Exported functions
// ---------------------------------------------------------------------------

/**
 * Called once per engine tick after physics has run.
 */
export function tick(): void {
  // No per-tick logic needed for basic breakout
  // Physics handles ball movement and bouncing
  // Collision handler (on_collision) handles brick destruction
}

/**
 * Called by the engine for each physics collision pair.
 *
 * If either entity is a brick, despawn it.
 *
 * @param entityA - First entity in collision pair
 * @param entityB - Second entity in collision pair
 */
export function on_collision(entityA: i64, entityB: i64): void {
  // Check if entityA is a brick
  if (is_brick(entityA)) {
    despawn_entity(entityA, "collision_destroy");
    log_msg(2, "Destroyed brick " + entityA.toString());
  }

  // Check if entityB is a brick
  if (is_brick(entityB)) {
    despawn_entity(entityB, "collision_destroy");
    log_msg(2, "Destroyed brick " + entityB.toString());
  }
}
