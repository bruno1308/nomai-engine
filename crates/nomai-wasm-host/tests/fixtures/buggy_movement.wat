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
  ;; reason: "move_away_buggy" at offset 16 (15 bytes)
  (data (i32.const 16) "move_away_buggy")
  ;; BUGGY value: {"x":-1.0,"y":0.0} at offset 48 (18 bytes)
  (data (i32.const 48) "{\"x\":-1.0,\"y\":0.0}")

  ;; tick() -- sets position.x to -1.0 each tick (BUGGY: moving away from target)
  (func $tick (export "tick")
    ;; Read tick number (exercises read path).
    (drop (call $tick_number))

    ;; Call set_component:
    ;;   entity_id = 0 (i64)
    ;;   name = "position" (ptr=0, len=8)
    ;;   value = {"x":-1.0,"y":0.0} (ptr=48, len=18)
    ;;   reason = "move_away_buggy" (ptr=16, len=15)
    (call $set_component
      (i64.const 0)     ;; entity_id = 0
      (i32.const 0)     ;; name_ptr ("position")
      (i32.const 8)     ;; name_len
      (i32.const 48)    ;; value_ptr
      (i32.const 18)    ;; value_len
      (i32.const 16)    ;; reason_ptr
      (i32.const 15)    ;; reason_len
    )
  )
)
