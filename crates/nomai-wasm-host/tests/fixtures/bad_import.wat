(module
  ;; Module that imports from an unauthorized namespace.
  ;; Should be rejected by import validation with WasmError::InvalidImport.
  (import "unknown_module" "unknown_func" (func))

  (func $tick (export "tick")
    nop
  )
)
