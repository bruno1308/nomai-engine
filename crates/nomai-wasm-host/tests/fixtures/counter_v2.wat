(module
  ;; V2: counts by 10 instead of 1.
  (global $count (mut i32) (i32.const 0))

  ;; tick() increments the counter by 10.
  (func $tick (export "tick")
    global.get $count
    i32.const 10
    i32.add
    global.set $count
  )

  ;; get_count() returns the current counter value.
  (func $get_count (export "get_count") (result i32)
    global.get $count
  )
)
