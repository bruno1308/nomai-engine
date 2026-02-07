;; Benchmark fixture: 50 host calls per tick (25 reads + 25 writes).
;;
;; This module exercises a realistic WASM gameplay workload for Spike B
;; gate evaluation. The kill criterion is: 50 host calls >1ms -> spike FAILS.
;;
;; Read calls: alternating get_entity_count and tick_number (25 total)
;; Write calls: set_component on entity 0 with "health" = "42" (25 total)
(module
  ;; Import host functions from the "nomai" namespace.
  (import "nomai" "get_entity_count" (func $get_entity_count (result i32)))
  (import "nomai" "tick_number" (func $tick_number (result i64)))
  (import "nomai" "set_component"
    (func $set_component (param i64 i32 i32 i32 i32 i32 i32)))

  ;; Linear memory (1 page = 64KiB), exported so the host can read strings.
  (memory (export "memory") 1)

  ;; -- Data segments for set_component calls --
  ;; "health" at offset 0 (6 bytes)
  (data (i32.const 0) "health")
  ;; "42" (JSON number) at offset 16 (2 bytes)
  (data (i32.const 16) "42")
  ;; "bench_reason" at offset 32 (12 bytes)
  (data (i32.const 32) "bench_reason")

  ;; Globals to store intermediate results (prevents dead-code elimination).
  (global $sink_i32 (mut i32) (i32.const 0))
  (global $sink_i64 (mut i64) (i64.const 0))

  ;; tick() -- makes exactly 50 host calls: 25 reads + 25 writes.
  (func $tick (export "tick")
    (local $i i32)

    ;; -- 25 read calls: alternating get_entity_count and tick_number --
    (local.set $i (i32.const 0))
    (block $break_reads
      (loop $read_loop
        (br_if $break_reads (i32.ge_u (local.get $i) (i32.const 25)))

        ;; Alternate between the two read calls.
        (if (i32.rem_u (local.get $i) (i32.const 2))
          (then
            (global.set $sink_i64 (call $tick_number))
          )
          (else
            (global.set $sink_i32 (call $get_entity_count))
          )
        )

        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $read_loop)
      )
    )

    ;; -- 25 write calls: set_component on entity 0 --
    (local.set $i (i32.const 0))
    (block $break_writes
      (loop $write_loop
        (br_if $break_writes (i32.ge_u (local.get $i) (i32.const 25)))

        (call $set_component
          (i64.const 0)     ;; entity_id
          (i32.const 0)     ;; name_ptr ("health")
          (i32.const 6)     ;; name_len
          (i32.const 16)    ;; value_ptr ("42")
          (i32.const 2)     ;; value_len
          (i32.const 32)    ;; reason_ptr ("bench_reason")
          (i32.const 12)    ;; reason_len
        )

        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $write_loop)
      )
    )
  )
)
