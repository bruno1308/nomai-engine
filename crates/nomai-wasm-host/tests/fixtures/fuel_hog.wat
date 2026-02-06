(module
  ;; tick() runs an infinite loop to exhaust fuel.
  (func $tick (export "tick")
    (loop $forever
      br $forever
    )
  )
)
