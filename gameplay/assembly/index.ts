// Example Nomai gameplay module.
//
// Demonstrates reading world state and emitting commands via the host API.
// This module moves entity 0 to the right each tick and logs tick info.

import {
  get_entity_count,
  tick_number,
  set_component,
  log_msg,
} from "./host";

/**
 * Called once per engine tick. This is the main entry point for gameplay
 * logic.
 *
 * The engine calls this function after populating the world snapshot. All
 * commands emitted here are deferred and applied after all scripts finish.
 */
export function tick(): void {
  const currentTick: i64 = tick_number();
  const entityCount: i32 = get_entity_count();

  // Log tick info at debug level.
  log_msg(
    1,
    "tick " + currentTick.toString() + ", entities: " + entityCount.toString()
  );

  // Move entity 0 to the right each tick.
  const entityId: i64 = 0;
  const x: f64 = f64(currentTick);
  const valueJson: string = '{"x":' + x.toString() + ',"y":0.0}';
  set_component(entityId, "position", valueJson, "move_right_each_tick");
}
