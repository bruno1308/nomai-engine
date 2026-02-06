(module
  ;; Module that does NOT export a tick() function.
  (func $other (export "other") (result i32)
    i32.const 42
  )
)
