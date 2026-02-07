(module
  ;; Module whose tick() always traps via unreachable instruction.
  ;; Used to test trap recovery and consecutive trap counting.
  (func $tick (export "tick")
    unreachable
  )
)
