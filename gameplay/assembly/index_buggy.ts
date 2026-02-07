// Nomai Breakout -- BUGGY gameplay entry point.
//
// Re-exports from the deliberately-buggy breakout module.
// Used by demo_breakout.py to demonstrate verification failure detection.

export {
  tick,
  handleBrickHit,
  get_score,
  get_bricks_destroyed,
} from "./breakout_buggy";
