// Nomai Prism Pop -- gameplay entry point.
//
// Re-exports the tick function and helpers from the prism_pop module.
// The engine calls `tick()` once per simulation step.
// Python calls `handleSwap()` to inject player swap inputs.

export {
  tick,
  handleSwap,
  get_score,
  get_phase,
  get_cascade_depth,
  get_tile_at,
  get_valid_move_count,
} from "./prism_pop";
