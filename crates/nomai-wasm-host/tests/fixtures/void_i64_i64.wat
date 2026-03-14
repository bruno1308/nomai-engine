(module
  ;; Mutable globals to store the last values passed to on_collision.
  (global $last_p1 (mut i64) (i64.const 0))
  (global $last_p2 (mut i64) (i64.const 0))

  ;; tick() -- required export, does nothing.
  (func $tick (export "tick")
    nop
  )

  ;; on_collision(entity_a: i64, entity_b: i64) -- stores both parameters.
  (func $on_collision (export "on_collision") (param $entity_a i64) (param $entity_b i64)
    local.get $entity_a
    global.set $last_p1
    local.get $entity_b
    global.set $last_p2
  )

  ;; get_last_p1() -- returns the first stored parameter.
  (func $get_last_p1 (export "get_last_p1") (result i64)
    global.get $last_p1
  )

  ;; get_last_p2() -- returns the second stored parameter.
  (func $get_last_p2 (export "get_last_p2") (result i64)
    global.get $last_p2
  )
)
