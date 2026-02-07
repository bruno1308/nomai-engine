//! Engine-level snapshot and restore with BLAKE3 hashing.
//!
//! Provides [`EngineSnapshot`] -- a serializable representation of the full
//! engine simulation state (ECS world, tick counter, fixed dt, input frame)
//! with a BLAKE3 content hash for integrity verification and determinism
//! testing.
//!
//! # Usage
//!
//! ```
//! use nomai_engine::prelude::*;
//!
//! let mut world = World::new();
//! world.register_component::<u32>("score");
//!
//! let config = TickConfig { fixed_dt: 1.0 / 60.0, ..Default::default() };
//! let mut tick_loop = TickLoop::new(world, config);
//! tick_loop.run_ticks(10);
//!
//! // Capture a snapshot.
//! let snapshot = tick_loop.capture_snapshot();
//! assert_eq!(snapshot.tick_counter, 10);
//! assert_eq!(snapshot.hash.len(), 64); // BLAKE3 hex digest
//!
//! // Run more ticks, then restore.
//! tick_loop.run_ticks(10);
//! assert_eq!(tick_loop.tick_count(), 20);
//!
//! tick_loop.restore_from_snapshot(&snapshot).unwrap();
//! assert_eq!(tick_loop.tick_count(), 10);
//! ```
//!
//! # Branching
//!
//! Use [`TickLoop::fork_snapshot`] to capture a branch point. Restore the
//! fork on separate `TickLoop` instances (or the same instance) to explore
//! divergent simulation paths:
//!
//! ```
//! use nomai_engine::prelude::*;
//!
//! let mut world = World::new();
//! world.register_component::<u32>("score");
//! let config = TickConfig { fixed_dt: 1.0 / 60.0, ..Default::default() };
//! let mut tick_loop = TickLoop::new(world, config);
//! tick_loop.run_ticks(50);
//!
//! let fork = tick_loop.fork_snapshot();
//!
//! // Branch A: continue running.
//! tick_loop.run_ticks(50);
//! let hash_a = tick_loop.state_hash();
//!
//! // Branch B: restore fork, modify state, run again.
//! tick_loop.restore_from_snapshot(&fork).unwrap();
//! tick_loop.run_ticks(50);
//! let hash_b = tick_loop.state_hash();
//!
//! // Same initial state + same systems + same inputs = same hash.
//! assert_eq!(hash_a, hash_b);
//! ```
//!
//! # What Is NOT Serialized
//!
//! - **Systems** (closures/fn pointers) -- the caller must re-register
//!   systems on a fresh `TickLoop` if needed. When restoring on the *same*
//!   `TickLoop` instance, registered systems are retained.
//! - **Physics world** (`rapier2d` state) -- not serializable. Reconstructed
//!   automatically from ECS component data during restore.
//! - **WASM module** (`wasmtime` state) -- not serializable. The caller
//!   must re-attach the WASM module after restore.
//! - **Manifest pipeline** -- reset to a fresh `ManifestPipeline::new()`
//!   on restore (history from before the snapshot is not preserved).
//! - **Diagnostics** -- per-tick timing is transient and not snapshotted.

use nomai_ecs::snapshot::WorldSnapshot;
use serde::{Deserialize, Serialize};

use crate::tick::{InputFrame, TickLoop};

// ---------------------------------------------------------------------------
// EngineSnapshot
// ---------------------------------------------------------------------------

/// A serializable snapshot of the full engine simulation state.
///
/// Contains the ECS world snapshot, tick metadata, the current input frame,
/// and a BLAKE3 hex digest of the serialized state for integrity checking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineSnapshot {
    /// Complete ECS world state (entities, components, allocator).
    pub world: WorldSnapshot,
    /// Number of ticks executed at the time of capture.
    pub tick_counter: u64,
    /// Fixed time step in seconds per tick.
    pub fixed_dt: f64,
    /// Input frame at the time of capture.
    pub current_input: InputFrame,
    /// BLAKE3 hex digest (64 lowercase hex chars) of the serialized
    /// world + tick state. Used for determinism verification.
    pub hash: String,
}

// ---------------------------------------------------------------------------
// Hashing helpers
// ---------------------------------------------------------------------------

/// Compute the BLAKE3 hex digest of the hashable engine state.
///
/// The hash covers the serialized world snapshot, tick counter, fixed dt,
/// and input frame -- i.e., everything that affects simulation determinism.
/// The hash field itself is NOT included (it is derived).
fn compute_hash(
    world: &WorldSnapshot,
    tick_counter: u64,
    fixed_dt: f64,
    input: &InputFrame,
) -> String {
    // Serialize the hashable parts to a canonical JSON byte stream.
    // We use a deterministic struct wrapper so the hash is stable.
    #[derive(Serialize)]
    struct HashableState<'a> {
        world: &'a WorldSnapshot,
        tick_counter: u64,
        fixed_dt: f64,
        current_input: &'a InputFrame,
    }

    let hashable = HashableState {
        world,
        tick_counter,
        fixed_dt,
        current_input: input,
    };

    let json_bytes = serde_json::to_vec(&hashable)
        .expect("EngineSnapshot state should always be JSON-serializable");

    blake3::hash(&json_bytes).to_hex().to_string()
}

// ---------------------------------------------------------------------------
// TickLoop snapshot/restore methods
// ---------------------------------------------------------------------------

impl TickLoop {
    /// Capture a complete snapshot of the engine simulation state.
    ///
    /// Serializes the ECS world, tick counter, fixed dt, and current input
    /// frame, then computes a BLAKE3 hash of the serialized state.
    ///
    /// The resulting [`EngineSnapshot`] can be serialized to JSON for
    /// storage, transmitted to another process, or used to restore this
    /// (or another) `TickLoop` to the captured state.
    ///
    /// # Note
    ///
    /// Systems, physics world, WASM module, manifest pipeline, and
    /// diagnostics are NOT included in the snapshot. See module-level
    /// documentation for details.
    pub fn capture_snapshot(&self) -> EngineSnapshot {
        let world_snapshot = self.world().capture_snapshot();
        let tick_counter = self.tick_count();
        let fixed_dt = self.fixed_dt();
        let current_input = self.current_input().clone();

        let hash = compute_hash(&world_snapshot, tick_counter, fixed_dt, &current_input);

        EngineSnapshot {
            world: world_snapshot,
            tick_counter,
            fixed_dt,
            current_input,
            hash,
        }
    }

    /// Restore the engine simulation state from a previously captured snapshot.
    ///
    /// Before restoring, the snapshot's BLAKE3 hash is verified by recomputing
    /// it from the snapshot's data. If the hash does not match, restore is
    /// aborted with an error (the `TickLoop` state is not modified).
    ///
    /// This restores:
    /// - The ECS world (all entities, components, allocator state)
    /// - The tick counter
    /// - The fixed time step (`fixed_dt`)
    /// - The current input frame
    ///
    /// This resets:
    /// - The manifest pipeline (to a fresh `ManifestPipeline::new()`)
    /// - The command buffer (cleared)
    ///
    /// This also handles:
    /// - Physics world (automatically reconstructed from ECS state)
    ///
    /// This does NOT touch:
    /// - Registered systems (retained on the same `TickLoop` instance)
    /// - WASM module (caller must re-attach)
    /// - Diagnostics (reset implicitly on next tick)
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot hash is invalid (corruption/tampering)
    /// or if the world restore fails (e.g., snapshot references a component
    /// type not registered in the current world).
    pub fn restore_from_snapshot(
        &mut self,
        snapshot: &EngineSnapshot,
    ) -> Result<(), anyhow::Error> {
        // Pre-validate fixed_dt before any state mutation to avoid panics
        // from set_fixed_dt after partial restore.
        if !(snapshot.fixed_dt > 0.0 && snapshot.fixed_dt.is_finite()) {
            return Err(anyhow::anyhow!(
                "snapshot has invalid fixed_dt: {}. Must be positive and finite.",
                snapshot.fixed_dt
            ));
        }

        // Verify snapshot integrity by recomputing the hash from its data.
        let expected_hash = compute_hash(
            &snapshot.world,
            snapshot.tick_counter,
            snapshot.fixed_dt,
            &snapshot.current_input,
        );
        if expected_hash != snapshot.hash {
            return Err(anyhow::anyhow!(
                "snapshot hash mismatch: recorded {} but recomputed {}. \
                 The snapshot may be corrupted or tampered with.",
                snapshot.hash,
                expected_hash
            ));
        }

        // Restore the ECS world state.
        self.world_mut()
            .restore_from_snapshot(&snapshot.world)
            .map_err(|e| anyhow::anyhow!("failed to restore world from snapshot: {e}"))?;

        // Restore tick counter. We use the internal field via a helper.
        self.set_tick_counter(snapshot.tick_counter);

        // Restore the fixed time step so the simulation runs at the same
        // rate as when the snapshot was captured.
        self.set_fixed_dt(snapshot.fixed_dt);

        // Restore the input frame.
        self.set_input(snapshot.current_input.clone());

        // Reset the manifest pipeline -- history from before the snapshot
        // is not meaningful after restore.
        self.reset_manifest();

        // Clear the command buffer to prevent stale commands from leaking
        // into the next tick.
        self.clear_command_buffer();

        // Reconstruct the rapier2d physics world from restored ECS state.
        // rapier types are not serializable, so we rebuild from Position,
        // Velocity, and PhysicsBody components that were restored above.
        self.reconstruct_physics();

        Ok(())
    }

    /// Compute and return the BLAKE3 state hash without allocating a full snapshot.
    ///
    /// This is a convenience method equivalent to `capture_snapshot().hash`
    /// but avoids storing the full snapshot if only the hash is needed.
    /// Note: internally this still serializes the world to compute the hash.
    pub fn state_hash(&self) -> String {
        let world_snapshot = self.world().capture_snapshot();
        compute_hash(
            &world_snapshot,
            self.tick_count(),
            self.fixed_dt(),
            self.current_input(),
        )
    }

    /// Fork the current simulation state for branching scenarios.
    ///
    /// This is semantically identical to [`capture_snapshot`](Self::capture_snapshot)
    /// but named for clarity in branching workflows where the snapshot
    /// represents a divergence point rather than a save point.
    pub fn fork_snapshot(&self) -> EngineSnapshot {
        self.capture_snapshot()
    }
}
