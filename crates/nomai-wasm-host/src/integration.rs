//! Integration orchestrator: connects WASM module execution to the full
//! ECS tick loop and manifest pipeline.
//!
//! The [`run_wasm_tick`] function executes a complete tick with WASM gameplay:
//! prepare host state -> execute WASM -> drain commands -> apply to world ->
//! process through manifest pipeline.
//!
//! # Causal Chain Preservation
//!
//! The critical property this module guarantees is that causal chains are
//! **unbroken** across the WASM boundary. Every command emitted by WASM
//! gameplay code carries:
//!
//! - [`SystemId::WASM_GAMEPLAY`] as the issuing system
//! - [`CausalReason::GameRule`] with the reason string provided by the WASM module
//!
//! These flow through [`CommandBuffer::apply`] into the world, then through
//! [`ManifestPipeline::process_commands`] into the manifest's component changes
//! and causal chains. At no point is the causality metadata lost or transformed.

use nomai_ecs::world::World;
use nomai_manifest::manifest::{ManifestPipeline, TickManifest};

use crate::module::WasmModule;
use crate::WasmError;

/// Run a full tick with WASM gameplay execution and manifest generation.
///
/// This function connects the WASM module to the full engine pipeline:
///
/// 1. Prepare host state with tick metadata and world snapshot
/// 2. Begin manifest tick
/// 3. Execute WASM module's `tick()` function
/// 4. Drain commands and events from WASM host state
/// 5. Apply commands to the ECS world
/// 6. Process applied commands through the manifest pipeline
/// 7. Record events in the manifest
/// 8. Finalize manifest and return it
///
/// # Returns
///
/// A tuple of `(TickManifest, usize)` where the second element is the
/// number of commands that were applied to the world.
///
/// # Errors
///
/// Returns [`WasmError`] if the WASM module's `tick()` function traps
/// or runs out of fuel.
pub fn run_wasm_tick(
    module: &mut WasmModule,
    world: &mut World,
    manifest: &mut ManifestPipeline,
    tick: u64,
    sim_time: f64,
) -> Result<(TickManifest, usize), WasmError> {
    // 1. Prepare host state with tick metadata.
    module.host_state_mut().begin_tick(tick, sim_time);

    // 2. Build a world snapshot for WASM to read.
    //    Populate entity_count so that get_entity_count() returns the
    //    correct value inside the WASM module.
    let entity_count = world.entity_count();
    module.host_state_mut().entity_count = entity_count;

    // 3. Begin manifest tick (clear per-tick state in the pipeline).
    manifest.begin_tick();

    // 4. Execute WASM tick.
    module.call_tick()?;

    // 5. Drain commands and events from host state.
    let mut cmd_buf = module.drain_commands();
    let events = module.drain_events();

    // 6. Apply commands to the world.
    let applied = cmd_buf.apply(world);
    let cmd_count = applied.len();

    // 7. Process applied commands through manifest pipeline.
    manifest.process_commands(&applied, tick, world);

    // 8. Record events.
    for event in events {
        manifest.record_event(event);
    }

    // 9. Finalize and return manifest.
    let tick_manifest = manifest.end_tick(
        tick,
        sim_time,
        vec!["wasm_gameplay".to_owned()],
        world,
    );

    Ok((tick_manifest, cmd_count))
}
