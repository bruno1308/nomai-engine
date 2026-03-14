(module
  ;; Mutable global to store the last value passed to handle_hit.
  (global $last_param (mut i64) (i64.const 0))

  ;; tick() -- required export, does nothing.
  (func $tick (export "tick")
    nop
  )

  ;; handle_hit(entity_id: i64) -- stores the parameter.
  (func $handle_hit (export "handle_hit") (param $entity_id i64)
    local.get $entity_id
    global.set $last_param
  )

  ;; get_last_param() -- returns the stored parameter for verification.
  (func $get_last_param (export "get_last_param") (result i64)
    global.get $last_param
  )
)
