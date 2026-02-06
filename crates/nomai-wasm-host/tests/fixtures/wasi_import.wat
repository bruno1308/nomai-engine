(module
  ;; Module that imports a WASI function -- should fail to instantiate
  ;; because we do not provide WASI in our sandbox.
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32))
  )

  (func $tick (export "tick")
    nop
  )
)
