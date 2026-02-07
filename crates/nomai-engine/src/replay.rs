//! Deterministic replay with input recording and checkpoint verification.
//!
//! The replay system records [`InputFrame`]s and periodic state hash checkpoints
//! during simulation, producing a [`ReplayLog`]. The log can then be replayed
//! against a [`TickLoop`] to verify determinism: the replay function restores
//! the initial snapshot, feeds the recorded inputs tick-by-tick, and compares
//! state hashes at each checkpoint.
//!
//! # Recording
//!
//! ```no_run
//! use nomai_engine::prelude::*;
//! use nomai_engine::replay::ReplayRecorder;
//!
//! let world = World::new();
//! let config = TickConfig { fixed_dt: 1.0 / 60.0, ..Default::default() };
//! let mut tick_loop = TickLoop::new(world, config);
//!
//! let snapshot = tick_loop.capture_snapshot();
//! let mut recorder = ReplayRecorder::new(snapshot, 10); // checkpoint every 10 ticks
//!
//! for _ in 0..100 {
//!     let input = tick_loop.current_input().clone();
//!     let hash = tick_loop.state_hash();
//!     let tick = tick_loop.tick_count();
//!     recorder.record_tick(tick, &input, Some(hash));
//!     tick_loop.tick();
//! }
//!
//! let log = recorder.finish();
//! ```
//!
//! # Replaying
//!
//! ```no_run
//! use nomai_engine::prelude::*;
//! use nomai_engine::replay::{replay, ReplayLog};
//!
//! # let log = todo!();
//! let world = World::new();
//! let config = TickConfig { fixed_dt: 1.0 / 60.0, ..Default::default() };
//! let mut tick_loop = TickLoop::new(world, config);
//!
//! let result = replay(&mut tick_loop, &log).expect("replay should succeed");
//! assert!(result.completed);
//! assert!(result.first_divergence.is_none());
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::snapshot::EngineSnapshot;
use crate::tick::{InputFrame, TickLoop};

// ---------------------------------------------------------------------------
// ReplayLog
// ---------------------------------------------------------------------------

/// A complete replay log: initial snapshot + ordered sequence of inputs and
/// checkpoints.
///
/// The log is fully serializable to JSON for storage, transmission, or
/// regression test fixtures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayLog {
    /// The engine snapshot captured at the start of recording.
    /// Replay begins by restoring this snapshot.
    pub initial_snapshot: EngineSnapshot,

    /// Optional BLAKE3 hex digest of the WASM gameplay module binary that was
    /// active during recording. Used for informational purposes; the replay
    /// function does not enforce this (the caller is responsible for loading
    /// the correct module).
    pub gameplay_module_hash: Option<String>,

    /// Total number of ticks that were recorded. Replay will execute exactly
    /// this many ticks from the initial snapshot, regardless of how many
    /// entries (inputs/checkpoints) exist.
    pub total_ticks: u64,

    /// Ordered sequence of replay entries (inputs and checkpoints).
    pub entries: Vec<ReplayEntry>,
}

// ---------------------------------------------------------------------------
// ReplayEntry
// ---------------------------------------------------------------------------

/// A single entry in a [`ReplayLog`]: either an input frame or a state hash
/// checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplayEntry {
    /// An input frame recorded at the given tick.
    Input {
        /// The tick number at which this input was active.
        tick: u64,
        /// The input frame for this tick.
        input: InputFrame,
    },
    /// A state hash checkpoint recorded at the given tick.
    Checkpoint {
        /// The tick number at which this checkpoint was taken (before the tick
        /// was executed).
        tick: u64,
        /// The BLAKE3 hex digest of the engine state at this tick.
        state_hash: String,
    },
}

// ---------------------------------------------------------------------------
// ReplayResult
// ---------------------------------------------------------------------------

/// The outcome of replaying a [`ReplayLog`] against a [`TickLoop`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplayResult {
    /// Whether the replay ran to completion without errors.
    pub completed: bool,
    /// The total number of ticks replayed.
    pub ticks_replayed: u64,
    /// The first checkpoint where the replayed state hash did not match the
    /// recorded hash. `None` if all checkpoints matched (deterministic).
    pub first_divergence: Option<ReplayDivergence>,
}

// ---------------------------------------------------------------------------
// ReplayDivergence
// ---------------------------------------------------------------------------

/// Details about a determinism failure detected during replay.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplayDivergence {
    /// The tick at which the divergence was detected.
    pub tick: u64,
    /// The state hash recorded in the replay log at this tick.
    pub expected_hash: String,
    /// The state hash computed during replay at this tick.
    pub actual_hash: String,
}

// ---------------------------------------------------------------------------
// ReplayRecorder
// ---------------------------------------------------------------------------

/// Records a simulation run into a [`ReplayLog`].
///
/// Create a recorder with an initial snapshot and a checkpoint interval.
/// Call [`record_tick`](Self::record_tick) before each tick to capture
/// inputs and periodic state hash checkpoints. When done, call
/// [`finish`](Self::finish) to consume the recorder and produce the log.
///
/// The recorder enforces monotonically increasing tick numbers: each call
/// to [`record_tick`](Self::record_tick) must supply a tick strictly greater
/// than the previous call (or any tick on the first call).
pub struct ReplayRecorder {
    /// The replay log being built.
    log: ReplayLog,
    /// How often (in ticks) to record a state hash checkpoint.
    checkpoint_interval: u64,
    /// Number of ticks recorded so far.
    ticks_recorded: u64,
    /// The tick number of the last `record_tick` call, used to enforce
    /// monotonic ordering. `None` before the first call.
    last_tick: Option<u64>,
}

impl ReplayRecorder {
    /// Create a new replay recorder.
    ///
    /// # Arguments
    ///
    /// * `snapshot` -- The engine snapshot captured at the start of recording.
    /// * `checkpoint_interval` -- How often (in ticks) to record a state hash
    ///   checkpoint. A value of 10 means checkpoints at ticks 0, 10, 20, etc.
    ///   A value of 0 means no automatic checkpoints (only explicit ones via
    ///   the `state_hash` parameter of [`record_tick`](Self::record_tick)).
    pub fn new(snapshot: EngineSnapshot, checkpoint_interval: u64) -> Self {
        Self {
            log: ReplayLog {
                initial_snapshot: snapshot,
                gameplay_module_hash: None,
                total_ticks: 0,
                entries: Vec::new(),
            },
            checkpoint_interval,
            ticks_recorded: 0,
            last_tick: None,
        }
    }

    /// Set the optional BLAKE3 hex digest of the WASM gameplay module binary.
    pub fn set_gameplay_module_hash(&mut self, hash: String) {
        self.log.gameplay_module_hash = Some(hash);
    }

    /// Record a single tick.
    ///
    /// Call this **before** executing the tick. The recorder will:
    ///
    /// 1. Record the input frame if it is non-empty.
    /// 2. Record a checkpoint if `state_hash` is `Some` and the tick falls on
    ///    the checkpoint interval (or `checkpoint_interval` is 0, in which case
    ///    every tick with a provided hash gets a checkpoint).
    ///
    /// # Arguments
    ///
    /// * `tick` -- The current tick number (before execution).
    /// * `input` -- The input frame for this tick.
    /// * `state_hash` -- Optional BLAKE3 hex digest of the current engine
    ///   state. If `None`, no checkpoint is recorded even if the interval
    ///   matches.
    ///
    /// # Panics
    ///
    /// Panics if `tick` is not strictly greater than the tick supplied in the
    /// previous call (i.e., ticks must be recorded in monotonically increasing
    /// order).
    pub fn record_tick(&mut self, tick: u64, input: &InputFrame, state_hash: Option<String>) {
        if let Some(prev) = self.last_tick {
            assert!(
                tick > prev,
                "ReplayRecorder::record_tick: tick {tick} is not strictly greater than previous tick {prev}. \
                 Ticks must be recorded in monotonically increasing order."
            );
        }
        self.last_tick = Some(tick);
        self.ticks_recorded += 1;

        // Record non-empty inputs.
        if !input.is_empty() {
            self.log.entries.push(ReplayEntry::Input {
                tick,
                input: input.clone(),
            });
        }

        // Record checkpoint if at the right interval and hash is provided.
        if let Some(hash) = state_hash {
            let should_checkpoint = if self.checkpoint_interval == 0 {
                // interval 0 means "checkpoint whenever a hash is provided"
                true
            } else {
                tick % self.checkpoint_interval == 0
            };

            if should_checkpoint {
                self.log.entries.push(ReplayEntry::Checkpoint {
                    tick,
                    state_hash: hash,
                });
            }
        }
    }

    /// Finish recording and return the completed [`ReplayLog`].
    pub fn finish(mut self) -> ReplayLog {
        self.log.total_ticks = self.ticks_recorded;
        self.log
    }
}

// ---------------------------------------------------------------------------
// replay()
// ---------------------------------------------------------------------------

/// Replay a [`ReplayLog`] against a [`TickLoop`], verifying determinism at
/// each checkpoint.
///
/// The function:
///
/// 1. Validates the replay log (no duplicate entries, no tick overflow).
/// 2. Restores the initial snapshot from the log.
/// 3. Iterates from the snapshot's starting tick for `total_ticks` ticks.
/// 4. For each tick, checks any checkpoint (before execution), sets the
///    recorded input (or a default empty input), then executes the tick.
/// 5. At each checkpoint tick, computes the state hash and compares it to the
///    recorded hash.
///
/// Replay stops at the first divergence but still reports a successful
/// completion of ticks up to that point.
///
/// # Arguments
///
/// * `tick_loop` -- The tick loop to replay on. Systems must already be
///   registered. The tick loop's state will be overwritten by the snapshot
///   restore.
/// * `log` -- The replay log to replay.
///
/// # Errors
///
/// Returns an error if the replay log is malformed (duplicate entries,
/// tick range overflow) or if the initial snapshot restore fails. All
/// validation is performed *before* mutating the `TickLoop`, so on error
/// the tick loop state is guaranteed to be unmodified.
pub fn replay(tick_loop: &mut TickLoop, log: &ReplayLog) -> Result<ReplayResult, anyhow::Error> {
    // Step 1: Validate the replay log BEFORE mutating the TickLoop.
    // This ensures that on any validation error the caller's state is untouched.

    // 1a: Build lookup maps from entries, rejecting duplicates.
    let mut input_map: BTreeMap<u64, InputFrame> = BTreeMap::new();
    let mut checkpoint_map: BTreeMap<u64, String> = BTreeMap::new();

    for entry in &log.entries {
        match entry {
            ReplayEntry::Input { tick, input } => {
                if input_map.contains_key(tick) {
                    return Err(anyhow::anyhow!(
                        "replay log contains duplicate Input entry at tick {tick}"
                    ));
                }
                input_map.insert(*tick, input.clone());
            }
            ReplayEntry::Checkpoint { tick, state_hash } => {
                if checkpoint_map.contains_key(tick) {
                    return Err(anyhow::anyhow!(
                        "replay log contains duplicate Checkpoint entry at tick {tick}"
                    ));
                }
                checkpoint_map.insert(*tick, state_hash.clone());
            }
        }
    }

    // 1b: Determine the tick range and validate for overflow.
    let start_tick = log.initial_snapshot.tick_counter;
    let total_ticks = log.total_ticks;

    // If there are no ticks to replay, we are trivially complete.
    if total_ticks == 0 {
        return Ok(ReplayResult {
            completed: true,
            ticks_replayed: 0,
            first_divergence: None,
        });
    }

    // The end tick is exclusive: we replay ticks [start_tick, start_tick + total_ticks).
    let end_tick = start_tick.checked_add(total_ticks).ok_or_else(|| {
        anyhow::anyhow!(
            "tick range overflow: start_tick ({start_tick}) + total_ticks ({total_ticks}) exceeds u64::MAX"
        )
    })?;

    // Step 2: Restore the initial snapshot (safe to mutate now -- log is valid).
    tick_loop
        .restore_from_snapshot(&log.initial_snapshot)
        .map_err(|e| anyhow::anyhow!("failed to restore initial snapshot for replay: {e}"))?;

    // Step 4: Iterate through ticks.
    let mut ticks_replayed: u64 = 0;

    for tick in start_tick..end_tick {
        // 4a: Set input for this tick BEFORE checking the checkpoint, because
        // during recording the state hash was computed after set_input but
        // before tick execution. The hash includes the current_input field.
        let input = input_map
            .get(&tick)
            .cloned()
            .unwrap_or_default();
        tick_loop.set_input(input);

        // 4b: Check checkpoint BEFORE executing the tick (checkpoints are
        // recorded before tick execution, after input is set).
        if let Some(expected_hash) = checkpoint_map.get(&tick) {
            let actual_hash = tick_loop.state_hash();
            if &actual_hash != expected_hash {
                return Ok(ReplayResult {
                    completed: false,
                    ticks_replayed,
                    first_divergence: Some(ReplayDivergence {
                        tick,
                        expected_hash: expected_hash.clone(),
                        actual_hash,
                    }),
                });
            }
        }

        // 4c: Execute the tick.
        tick_loop.tick();
        ticks_replayed += 1;
    }

    Ok(ReplayResult {
        completed: true,
        ticks_replayed,
        first_divergence: None,
    })
}
