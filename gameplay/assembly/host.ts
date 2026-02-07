// Host function bindings for the Nomai engine.
//
// These are provided by the Rust WASM host at runtime under the "nomai"
// namespace. Raw declarations use (ptr, len) pairs for string arguments.
// Convenience wrappers below handle UTF-8 encoding automatically.
//
// Design rules:
// - Reads are immediate (consistent snapshot of world state).
// - Writes are deferred (applied after all scripts finish).
// - Every write carries a `reason` string for manifest causality.

// ---------------------------------------------------------------------------
// Raw host function declarations (nomai namespace)
// ---------------------------------------------------------------------------

// Read functions

@external("nomai", "get_entity_count")
export declare function get_entity_count(): i32;

@external("nomai", "sim_time")
export declare function sim_time(): f64;

@external("nomai", "tick_number")
export declare function tick_number(): i64;

@external("nomai", "get_component")
export declare function _get_component(
  entity_id: i64,
  name_ptr: i32,
  name_len: i32
): i64;

// Write functions

@external("nomai", "set_component")
export declare function _set_component(
  entity_id: i64,
  name_ptr: i32,
  name_len: i32,
  value_ptr: i32,
  value_len: i32,
  reason_ptr: i32,
  reason_len: i32
): void;

@external("nomai", "spawn_semantic")
export declare function _spawn_semantic(
  identity_ptr: i32,
  identity_len: i32,
  components_ptr: i32,
  components_len: i32,
  reason_ptr: i32,
  reason_len: i32
): i64;

@external("nomai", "spawn_pooled")
export declare function _spawn_pooled(
  identity_ptr: i32,
  identity_len: i32,
  components_ptr: i32,
  components_len: i32,
  reason_ptr: i32,
  reason_len: i32
): i64;

@external("nomai", "despawn")
export declare function _despawn(
  entity_id: i64,
  reason_ptr: i32,
  reason_len: i32
): void;

@external("nomai", "emit_event")
export declare function _emit_event(
  event_ptr: i32,
  event_len: i32
): void;

@external("nomai", "log")
export declare function _log(
  level: i32,
  msg_ptr: i32,
  msg_len: i32
): void;

// ---------------------------------------------------------------------------
// Internal helper: encode a string to UTF-8 and return the ArrayBuffer
// ---------------------------------------------------------------------------

function encodeString(s: string): ArrayBuffer {
  return String.UTF8.encode(s);
}

// ---------------------------------------------------------------------------
// Convenience wrappers
// ---------------------------------------------------------------------------

/**
 * Check whether an entity has a component.
 *
 * Returns 0 if the component exists on the entity, -1 otherwise.
 * The component name is automatically UTF-8 encoded and passed via
 * linear memory.
 */
export function get_component(entityId: i64, name: string): i64 {
  const nameBuf = encodeString(name);
  return _get_component(
    entityId,
    changetype<i32>(nameBuf),
    nameBuf.byteLength
  );
}

/**
 * Set a component value on an entity.
 *
 * The value must be a JSON-encoded string. The reason string feeds into
 * the manifest's causality chain -- it should explain *why* this change
 * is happening (e.g. "ball_bounced_off_wall").
 */
export function set_component(
  entityId: i64,
  name: string,
  valueJson: string,
  reason: string
): void {
  const nameBuf = encodeString(name);
  const valueBuf = encodeString(valueJson);
  const reasonBuf = encodeString(reason);
  _set_component(
    entityId,
    changetype<i32>(nameBuf),
    nameBuf.byteLength,
    changetype<i32>(valueBuf),
    valueBuf.byteLength,
    changetype<i32>(reasonBuf),
    reasonBuf.byteLength
  );
}

/**
 * Spawn a semantic entity (full manifest presence, full causality).
 *
 * The identity and components arguments are JSON-encoded strings.
 * Returns a placeholder entity ID (the real ID is assigned when the
 * command buffer is applied by the engine).
 */
export function spawn_semantic(
  identityJson: string,
  componentsJson: string,
  reason: string
): i64 {
  const identityBuf = encodeString(identityJson);
  const componentsBuf = encodeString(componentsJson);
  const reasonBuf = encodeString(reason);
  return _spawn_semantic(
    changetype<i32>(identityBuf),
    identityBuf.byteLength,
    changetype<i32>(componentsBuf),
    componentsBuf.byteLength,
    changetype<i32>(reasonBuf),
    reasonBuf.byteLength
  );
}

/**
 * Spawn a pooled entity (type-level aggregation in manifest).
 *
 * The identity and components arguments are JSON-encoded strings.
 * Returns a placeholder entity ID (the real ID is assigned when the
 * command buffer is applied by the engine).
 */
export function spawn_pooled(
  identityJson: string,
  componentsJson: string,
  reason: string
): i64 {
  const identityBuf = encodeString(identityJson);
  const componentsBuf = encodeString(componentsJson);
  const reasonBuf = encodeString(reason);
  return _spawn_pooled(
    changetype<i32>(identityBuf),
    identityBuf.byteLength,
    changetype<i32>(componentsBuf),
    componentsBuf.byteLength,
    changetype<i32>(reasonBuf),
    reasonBuf.byteLength
  );
}

/**
 * Despawn an entity.
 *
 * The reason string feeds into the manifest's causality chain.
 */
export function despawn_entity(entityId: i64, reason: string): void {
  const reasonBuf = encodeString(reason);
  _despawn(
    entityId,
    changetype<i32>(reasonBuf),
    reasonBuf.byteLength
  );
}

/**
 * Emit a game event.
 *
 * The event must be a JSON-encoded string matching the GameEvent schema
 * expected by the engine (with event_type, data, etc.).
 */
export function emit_event(eventJson: string): void {
  const eventBuf = encodeString(eventJson);
  _emit_event(
    changetype<i32>(eventBuf),
    eventBuf.byteLength
  );
}

/**
 * Log a message from WASM.
 *
 * Levels: 0=trace, 1=debug, 2=info, 3=warn, 4=error
 */
export function log_msg(level: i32, msg: string): void {
  const msgBuf = encodeString(msg);
  _log(
    level,
    changetype<i32>(msgBuf),
    msgBuf.byteLength
  );
}
