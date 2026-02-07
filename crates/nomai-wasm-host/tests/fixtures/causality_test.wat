(module
  ;; Import host functions from the "nomai" namespace.
  (import "nomai" "get_entity_count" (func $get_entity_count (result i32)))
  (import "nomai" "tick_number" (func $tick_number (result i64)))
  (import "nomai" "set_component"
    (func $set_component (param i64 i32 i32 i32 i32 i32 i32)))

  ;; Linear memory (1 page = 64KiB), exported so the host can read strings.
  (memory (export "memory") 1)

  ;; -- Data segments for hardcoded strings --
  ;; "health" at offset 0 (6 bytes)
  (data (i32.const 0) "health")
  ;; "42" (JSON number) at offset 16 (2 bytes)
  (data (i32.const 16) "42")
  ;; "wasm_causality_test" at offset 32 (19 bytes)
  (data (i32.const 32) "wasm_causality_test")

  ;; tick() -- calls set_component with a specific causality reason string.
  ;; Targets entity_id=0 (the first entity spawned in the ECS allocator).
  (func $tick (export "tick")
    ;; Read entity count (exercises the read path).
    (drop (call $get_entity_count))

    ;; Read tick number (exercises the read path).
    (drop (call $tick_number))

    ;; Call set_component:
    ;;   entity_id = 0 (i64)
    ;;   name = "health" (ptr=0, len=6)
    ;;   value = "42" (ptr=16, len=2)
    ;;   reason = "wasm_causality_test" (ptr=32, len=19)
    (call $set_component
      (i64.const 0)     ;; entity_id (first entity spawned = index 0, gen 0)
      (i32.const 0)     ;; name_ptr
      (i32.const 6)     ;; name_len ("health")
      (i32.const 16)    ;; value_ptr
      (i32.const 2)     ;; value_len ("42")
      (i32.const 32)    ;; reason_ptr
      (i32.const 19)    ;; reason_len ("wasm_causality_test")
    )
  )
)
