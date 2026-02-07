//! Nomai Engine -- Game engine core with deterministic tick loop and command buffer.
//!
//! This crate builds on [`nomai_ecs`] to provide the simulation driver: a
//! fixed-timestep tick loop that runs systems in a deterministic order, applies
//! commands via the [`CommandBuffer`](nomai_ecs::command::CommandBuffer), and
//! advances simulation time.
//!
//! # Quick Start
//!
//! ```
//! use nomai_engine::prelude::*;
//!
//! let mut world = World::new();
//! world.register_component::<u32>("score");
//!
//! let config = TickConfig { fixed_dt: 1.0 / 60.0, ..Default::default() };
//! let mut tick_loop = TickLoop::new(world, config);
//!
//! tick_loop.add_system("example", |_world, _cmds| {
//!     // game logic here
//! });
//!
//! tick_loop.run_ticks(100);
//! assert_eq!(tick_loop.tick_count(), 100);
//! ```

#![deny(unsafe_code)]

pub mod physics;
pub mod render;
pub mod replay;
pub mod snapshot;
pub mod tick;

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

/// Re-export the ECS crate for convenience.
pub use nomai_ecs;

/// Re-export the manifest crate for convenience.
pub use nomai_manifest;

/// Re-export the WASM host crate for convenience.
pub use nomai_wasm_host;

// ---------------------------------------------------------------------------
// Prelude
// ---------------------------------------------------------------------------

/// Convenience re-exports for common engine usage.
pub mod prelude {
    // Re-export everything from the ECS prelude.
    pub use nomai_ecs::prelude::*;

    // Engine-specific exports.
    pub use crate::replay::{
        replay, ReplayDivergence, ReplayEntry, ReplayLog, ReplayRecorder, ReplayResult,
    };
    pub use crate::snapshot::EngineSnapshot;
    pub use crate::tick::{InputFrame, SystemFn, TickConfig, TickDiagnostics, TickLoop};

    // Physics types.
    pub use crate::physics::{
        ColliderShape, CollisionPair, PhysicsBody, PhysicsBodyType, PhysicsWorld, Position,
        Velocity, PHYSICS_SYSTEM_NAME,
    };

    // Manifest types for convenient access.
    pub use nomai_manifest::journal::{ChangeJournal, ComponentChange};
    pub use nomai_manifest::manifest::{
        Aggregates, CausalChain, CausalStep, EntityEntry, GameEvent, ManifestPipeline,
        TickManifest,
    };
}
