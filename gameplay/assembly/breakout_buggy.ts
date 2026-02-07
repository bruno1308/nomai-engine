// Buggy breakout gameplay module -- deliberate bugs for verification demo.
//
// This module is intentionally broken to demonstrate the verification loop.
// The fixed version is in breakout.ts.
//
// BUGS:
// 1. handleBrickHit() does NOT despawn the brick
// 2. handleBrickHit() does NOT emit the brick_destroyed event
// The score still increments, creating an inconsistent state that the
// verification engine should detect.

import {
  get_entity_count,
  tick_number,
  set_component,
  log_msg,
} from "./host";

let score: i32 = 0;
let bricksDestroyed: i32 = 0;

/**
 * Called once per engine tick after physics has run.
 */
export function tick(): void {
  const currentTick: i64 = tick_number();
  const entities: i32 = get_entity_count();

  log_msg(
    0,
    "buggy breakout tick " +
      currentTick.toString() +
      ", entities: " +
      entities.toString()
  );

  if (bricksDestroyed > 0) {
    set_component(
      0,
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
 * BUG: This handler increments score but does NOT despawn the brick
 * and does NOT emit the brick_destroyed event.
 *
 * The verification engine should detect:
 * - brick_destroyed_on_hit FAIL: brick still alive after collision
 * - game_won_when_no_bricks FAIL: bricks never reach zero
 */
export function handleBrickHit(brickEntityId: i64): void {
  score += 100;
  bricksDestroyed += 1;

  // BUG: Missing despawn_entity() call -- brick stays alive
  // BUG: Missing emit_event() call -- no brick_destroyed event in manifest

  log_msg(
    2,
    "Buggy: Brick " +
      brickEntityId.toString() +
      " hit but NOT destroyed! Score: " +
      score.toString()
  );
}

export function get_score(): i32 {
  return score;
}

export function get_bricks_destroyed(): i32 {
  return bricksDestroyed;
}
