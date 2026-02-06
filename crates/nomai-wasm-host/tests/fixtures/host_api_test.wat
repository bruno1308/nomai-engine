(module
  ;; Import host functions from the "nomai" namespace.
  (import "nomai" "get_entity_count" (func $get_entity_count (result i32)))
  (import "nomai" "tick_number" (func $tick_number (result i64)))
  (import "nomai" "sim_time" (func $sim_time (result f64)))
  (import "nomai" "set_component"
    (func $set_component (param i64 i32 i32 i32 i32 i32 i32)))
  (import "nomai" "log" (func $log (param i32 i32 i32)))

  ;; Linear memory (1 page = 64KiB), exported so the host can read strings.
  (memory (export "memory") 1)

  ;; Globals to store results for verification via exported getters.
  (global $last_entity_count (mut i32) (i32.const -1))
  (global $last_tick_number (mut i64) (i64.const -1))

  ;; -- Data segments for hardcoded strings --
  ;; "health" at offset 0, length 6
  (data (i32.const 0) "health")
  ;; "42" (JSON value) at offset 16, length 2
  (data (i32.const 16) "42")
  ;; "damage_from_wasm" at offset 32, length 16
  (data (i32.const 32) "damage_from_wasm")
  ;; "hello from wasm" at offset 64, length 15
  (data (i32.const 64) "hello from wasm")

  ;; tick() -- called each frame. Reads entity count and tick number,
  ;; stores them for later verification, then calls set_component with
  ;; hardcoded strings.
  (func $tick (export "tick")
    ;; Read and store entity count.
    (global.set $last_entity_count (call $get_entity_count))

    ;; Read and store tick number.
    (global.set $last_tick_number (call $tick_number))

    ;; Call set_component:
    ;;   entity_id = 42 (i64)
    ;;   name = "health" (ptr=0, len=6)
    ;;   value = "42" (ptr=16, len=2)
    ;;   reason = "damage_from_wasm" (ptr=32, len=16)
    (call $set_component
      (i64.const 42)     ;; entity_id
      (i32.const 0)      ;; name_ptr
      (i32.const 6)      ;; name_len
      (i32.const 16)     ;; value_ptr
      (i32.const 2)      ;; value_len
      (i32.const 32)     ;; reason_ptr
      (i32.const 16)     ;; reason_len
    )

    ;; Call log (level=2=info, msg="hello from wasm", len=15)
    (call $log
      (i32.const 2)      ;; level = info
      (i32.const 64)     ;; msg_ptr
      (i32.const 15)     ;; msg_len
    )
  )

  ;; Getter: returns the last entity count read by tick().
  (func (export "get_last_entity_count") (result i32)
    global.get $last_entity_count
  )

  ;; Getter: returns the last tick number read by tick().
  (func (export "get_last_tick_number") (result i64)
    global.get $last_tick_number
  )
)
