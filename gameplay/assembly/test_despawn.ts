// Simple test to see if despawn_entity works
import { despawn_entity, log_msg } from "./host";

export function tick(): void {
  log_msg(2, "test_despawn tick called");
}

export function on_collision(entityA: i64, entityB: i64): void {
  log_msg(2, "Collision detected");
  // Try to despawn the first entity
  log_msg(2, "Attempting despawn of entity " + entityA.toString());
  despawn_entity(entityA, "test_despawn_collision");
  log_msg(2, "Despawn command sent");
}
