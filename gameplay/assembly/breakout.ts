// Breakout gameplay logic for the Nomai engine.
//
// This module reads collision events and entity state from the host,
// then applies game rules: brick destruction, scoring, and ball reset.
// Physics is handled natively by rapier2d -- this module only responds
// to collision events.
//
// Design:
// - `tick()` is called once per engine tick after physics has run.
// - `handleBrickHit(brickEntityId)` is called by the engine when a
//   ball-brick collision is detected.
// - All state mutations flow through the host API as deferred commands.
// - Every command carries a reason string for manifest causality.

import {
  get_entity_count,
  tick_number,
  set_component,
  despawn_entity,
  emit_event,
  log_msg,
} from "./host";

// ---------------------------------------------------------------------------
// Game state tracking
// ---------------------------------------------------------------------------
// These values reset on module swap (hot-swap). All persistent state
// lives in ECS components -- these are just per-module accumulators
// that get pushed to the ECS each tick.

let score: i32 = 0;
let bricksDestroyed: i32 = 0;

// ---------------------------------------------------------------------------
// Exported functions
// ---------------------------------------------------------------------------

/**
 * Called once per engine tick after physics has run.
 *
 * Reads world state and pushes the current score to the ECS if any
 * bricks have been destroyed. The engine applies these commands after
 * all scripts finish.
 */
export function tick(): void {
  const currentTick: i64 = tick_number();
  const entities: i32 = get_entity_count();

  log_msg(
    0,
    "breakout tick " +
      currentTick.toString() +
      ", entities: " +
      entities.toString()
  );

  // Push score to the well-known score entity (entity 0 by convention)
  // whenever we have destroyed at least one brick.
  if (bricksDestroyed > 0) {
    set_component(
      0, // score entity
      "score",
      '{"points":' +
        score.toString() +
        ',"bricks_destroyed":' +
        bricksDestroyed.toString() +
        "}",
      "score_updated_after_brick_destruction"
    );
  }
}

/**
 * Called by the engine when a ball-brick collision is detected.
 *
 * Applies the game rule: brick is destroyed, score increases by 100.
 * Emits a `brick_destroyed` event for the manifest causal chain.
 *
 * @param brickEntityId - The entity ID of the brick that was hit.
 */
export function handleBrickHit(brickEntityId: i64): void {
  score += 100;
  bricksDestroyed += 1;

  // Despawn the brick with a causal reason.
  despawn_entity(brickEntityId, "brick_destroyed_by_ball");

  // Emit a game event so the manifest records the full causal chain:
  //   collision detected (physics) -> brick_destroyed (game rule) -> score update
  // The JSON must match the Rust GameEvent struct exactly:
  //   { event_type, description, involved_entities, caused_by, reason, tick }
  const eventJson: string =
    '{"event_type":"brick_destroyed",' +
    '"description":"Ball hit brick ' +
    brickEntityId.toString() +
    '",' +
    '"involved_entities":[' +
    brickEntityId.toString() +
    "]," +
    '"caused_by":100,' +
    '"reason":{"GameRule":"brick_destroyed_by_ball"},' +
    '"tick":' +
    tick_number().toString() +
    "}";
  emit_event(eventJson);

  log_msg(
    2,
    "Brick " +
      brickEntityId.toString() +
      " destroyed! Score: " +
      score.toString()
  );
}

// ---------------------------------------------------------------------------
// Test helpers -- exported so the host can inspect module state
// ---------------------------------------------------------------------------

/** Get the current score. Exported for testing and verification. */
export function get_score(): i32 {
  return score;
}

/** Get the number of bricks destroyed. Exported for testing and verification. */
export function get_bricks_destroyed(): i32 {
  return bricksDestroyed;
}
