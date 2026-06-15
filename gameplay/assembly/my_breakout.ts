// Breakout gameplay logic for the Nomai engine — Iteration 2.
//
// Strategy: Python registers brick entity IDs with add_brick_id() before
// running ticks. The on_collision handler checks whether either collider is
// a known brick and only despawns that entity. The ball/paddle/walls are
// never despawned. Score and game state are tracked on a game-manager entity
// whose ID Python passes via set_game_entity().

import {
  despawn_entity,
  set_component,
  emit_event,
  log_msg,
  tick_number,
} from "./host";

// ---------------------------------------------------------------------------
// Module-local state
// ---------------------------------------------------------------------------

const MAX_BRICKS: i32 = 256;
let brickIds: i64[] = new Array<i64>(MAX_BRICKS);
let brickCount: i32 = 0;

let score: i32 = 0;
let gameEntityId: i64 = -1;

// ---------------------------------------------------------------------------
// Python-callable exports (called once before simulation)
// ---------------------------------------------------------------------------

/** Register a brick entity ID so we can identify it in on_collision. */
export function add_brick_id(brickId: i64): void {
  if (brickCount < MAX_BRICKS) {
    brickIds[brickCount] = brickId;
    brickCount++;
  }
}

/** Tell the WASM module which entity holds "score" / "state" components. */
export function set_game_entity(entityId: i64): void {
  gameEntityId = entityId;
}

/** Read the current score (useful for Python verification). */
export function get_score(): i32 {
  return score;
}

/** Read the current remaining brick count. */
export function get_brick_count(): i32 {
  return brickCount;
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

function isBrick(entityId: i64): bool {
  for (let i: i32 = 0; i < brickCount; i++) {
    if (brickIds[i] == entityId) {
      return true;
    }
  }
  return false;
}

/** Swap-remove the brick ID from the tracking array. */
function removeBrick(entityId: i64): void {
  for (let i: i32 = 0; i < brickCount; i++) {
    if (brickIds[i] == entityId) {
      brickIds[i] = brickIds[brickCount - 1];
      brickCount--;
      return;
    }
  }
}

/** Despawn a brick, update score/state, emit event. */
function destroyBrick(brickId: i64): void {
  despawn_entity(brickId, "brick_destroyed_by_ball");
  removeBrick(brickId);
  score += 100;

  log_msg(2, "brick " + brickId.toString() + " destroyed score=" + score.toString());

  // Update the game-manager entity's score component.
  if (gameEntityId >= 0) {
    set_component(gameEntityId, "score", score.toString(), "brick_hit");

    // Transition to "won" when all bricks are gone.
    if (brickCount == 0) {
      set_component(gameEntityId, "state", '"won"', "all_bricks_destroyed");
      log_msg(2, "GAME WON — all bricks destroyed!");
    }
  }

  // Emit a lightweight event (no external entity references to avoid traps).
  emit_event(
    '{"event_type":"brick_destroyed","tick":' + tick_number().toString() + '}'
  );
}

// ---------------------------------------------------------------------------
// Engine-called exports
// ---------------------------------------------------------------------------

/** Called once per tick after physics runs. */
export function tick(): void {
  // Physics drives movement and bouncing automatically.
  // All game logic lives in on_collision below.
}

/**
 * Called by the engine for every physics collision pair.
 *
 * We only despawn entities that are in our tracked brick list.
 * Ball, paddle, and wall entities are left untouched.
 */
export function on_collision(entityA: i64, entityB: i64): void {
  if (isBrick(entityA)) {
    destroyBrick(entityA);
  }
  if (isBrick(entityB)) {
    destroyBrick(entityB);
  }
}
