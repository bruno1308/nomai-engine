// Nomai Breakout -- gameplay entry point.
//
// Re-exports the tick function and test helpers from the breakout module.
// The engine calls `tick()` once per simulation step after physics.
// The engine calls `handleBrickHit()` when a ball-brick collision occurs.

export {
  tick,
  handleBrickHit,
  get_score,
  get_bricks_destroyed,
} from "./breakout";
