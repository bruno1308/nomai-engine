(module
  ;; Import host functions from the "nomai" namespace.
  (import "nomai" "tick_number" (func $tick_number (result i64)))
  (import "nomai" "set_component"
    (func $set_component (param i64 i32 i32 i32 i32 i32 i32)))

  ;; Linear memory (1 page = 64KiB), exported so the host can read strings.
  (memory (export "memory") 1)

  ;; -- Data segments for hardcoded strings --
  ;; "position" at offset 0 (8 bytes)
  (data (i32.const 0) "position")
  ;; reason: "move_toward_target" at offset 16 (18 bytes)
  (data (i32.const 16) "move_toward_target")
  ;; value: {"x":1.0,"y":0.0} at offset 48 (17 bytes)
  (data (i32.const 48) "{\"x\":1.0,\"y\":0.0}")

  ;; tick() -- sets position.x to 1.0 each tick (correct: moving right toward target)
  (func $tick (export "tick")
    ;; Read tick number (exercises read path).
    (drop (call $tick_number))

    ;; Call set_component:
    ;;   entity_id = 0 (i64)
    ;;   name = "position" (ptr=0, len=8)
    ;;   value = {"x":1.0,"y":0.0} (ptr=48, len=17)
    ;;   reason = "move_toward_target" (ptr=16, len=18)
    (call $set_component
      (i64.const 0)     ;; entity_id = 0
      (i32.const 0)     ;; name_ptr ("position")
      (i32.const 8)     ;; name_len
      (i32.const 48)    ;; value_ptr
      (i32.const 17)    ;; value_len
      (i32.const 16)    ;; reason_ptr
      (i32.const 18)    ;; reason_len
    )
  )
)
