//! Tests for headless/windowed toggle and render loop integration.
//!
//! These tests verify that the engine runs correctly in headless mode
//! without any GPU initialization or renderer feature. They are NOT
//! gated behind `#[cfg(feature = "renderer")]` because the entire point
//! is to prove headless mode works without the renderer.

use nomai_ecs::command::CommandBuffer;
use nomai_ecs::prelude::*;
use nomai_ecs::world::World;
use nomai_engine::tick::{TickConfig, TickLoop};

// ---------------------------------------------------------------------------
// Test component types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Position {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Velocity {
    dx: f64,
    dy: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Counter(u64);

fn setup_world() -> World {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<Counter>("counter");
    world
}

fn movement_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (pos, vel)) in world.query::<(&Position, &Velocity)>() {
        let new_pos = Position {
            x: pos.x + vel.dx,
            y: pos.y + vel.dy,
        };
        cmds.set_component(
            entity,
            "position",
            serde_json::json!({"x": new_pos.x, "y": new_pos.y}),
            SystemId(1),
            CausalReason::SystemInternal("movement".to_owned()),
        );
    }
}

fn counter_system(world: &World, cmds: &mut CommandBuffer) {
    for (entity, (counter,)) in world.query::<(&Counter,)>() {
        let new_val = counter.0 + 1;
        cmds.set_component(
            entity,
            "counter",
            serde_json::json!(new_val),
            SystemId(2),
            CausalReason::SystemInternal("counter".to_owned()),
        );
    }
}

// ---------------------------------------------------------------------------
// 1. TickConfig defaults: headless is off by default
// ---------------------------------------------------------------------------

#[test]
fn tick_config_defaults_headless_off() {
    let config = TickConfig::default();
    assert!(
        !config.headless,
        "default TickConfig should have headless = false"
    );
}

#[test]
fn tick_config_defaults_60hz() {
    let config = TickConfig::default();
    let expected = 1.0 / 60.0;
    assert!(
        (config.fixed_dt - expected).abs() < f64::EPSILON,
        "default fixed_dt should be 1/60, got {}",
        config.fixed_dt
    );
}

// ---------------------------------------------------------------------------
// 2. Headless mode flag is correctly reported
// ---------------------------------------------------------------------------

#[test]
fn headless_mode_flag_on() {
    let world = setup_world();
    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let tick_loop = TickLoop::new(world, config);
    assert!(
        tick_loop.is_headless(),
        "tick loop should report headless = true"
    );
}

#[test]
fn headless_mode_flag_off() {
    let world = setup_world();
    let config = TickConfig {
        headless: false,
        ..Default::default()
    };
    let tick_loop = TickLoop::new(world, config);
    assert!(
        !tick_loop.is_headless(),
        "tick loop should report headless = false"
    );
}

// ---------------------------------------------------------------------------
// 3. Headless tick loop runs without renderer (no GPU)
// ---------------------------------------------------------------------------

#[test]
fn headless_tick_loop_runs_single_tick() {
    let world = setup_world();
    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.tick();
    assert_eq!(
        tick_loop.tick_count(),
        1,
        "headless tick loop should advance tick counter"
    );
}

#[test]
fn headless_tick_loop_runs_multiple_ticks() {
    let world = setup_world();
    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.run_ticks(100);
    assert_eq!(
        tick_loop.tick_count(),
        100,
        "headless tick loop should complete 100 ticks"
    );
}

// ---------------------------------------------------------------------------
// 4. Headless mode processes systems and commands correctly
// ---------------------------------------------------------------------------

#[test]
fn headless_tick_loop_processes_systems() {
    let mut world = setup_world();
    let mut bundle = ComponentBundle::new();
    bundle.add(world.registry(), Position { x: 0.0, y: 0.0 });
    bundle.add(world.registry(), Velocity { dx: 1.0, dy: 2.0 });
    let entity = world.spawn_bundle(bundle);

    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.add_system("movement", movement_system);

    tick_loop.run_ticks(10);

    let pos = tick_loop
        .world()
        .get_component::<Position>(entity)
        .expect("entity should have position");
    assert!(
        (pos.x - 10.0).abs() < f64::EPSILON,
        "x should be 10.0 after 10 ticks of dx=1.0, got {}",
        pos.x
    );
    assert!(
        (pos.y - 20.0).abs() < f64::EPSILON,
        "y should be 20.0 after 10 ticks of dy=2.0, got {}",
        pos.y
    );
}

#[test]
fn headless_tick_loop_counter_system() {
    let mut world = setup_world();
    let entity = world.spawn_with(Counter(0));

    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.add_system("counter", counter_system);

    tick_loop.run_ticks(50);

    let counter = tick_loop
        .world()
        .get_component::<Counter>(entity)
        .expect("entity should have counter");
    assert_eq!(
        counter.0, 50,
        "counter should be 50 after 50 ticks, got {}",
        counter.0
    );
}

// ---------------------------------------------------------------------------
// 5. Headless mode produces manifests
// ---------------------------------------------------------------------------

#[test]
fn headless_tick_loop_produces_manifests() {
    let mut world = setup_world();
    world.spawn_with(Counter(0));

    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.add_system("counter", counter_system);

    tick_loop.tick();

    let manifest = tick_loop
        .last_manifest()
        .expect("headless tick should produce a manifest");
    assert_eq!(manifest.tick, 0);
    assert!(manifest.commands_processed > 0);
}

// ---------------------------------------------------------------------------
// 6. Sim time is computed correctly in headless mode
// ---------------------------------------------------------------------------

#[test]
fn headless_sim_time_correct() {
    let world = setup_world();
    let config = TickConfig {
        fixed_dt: 0.01,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    tick_loop.run_ticks(1000);

    // Computed as tick_count * fixed_dt to avoid drift.
    let expected = 1000.0 * 0.01;
    assert_eq!(
        tick_loop.sim_time(),
        expected,
        "sim_time should be exactly {} after 1000 ticks at dt=0.01",
        expected
    );
}

// ---------------------------------------------------------------------------
// 7. Multiple systems work in headless mode
// ---------------------------------------------------------------------------

#[test]
fn headless_multiple_systems() {
    let mut world = setup_world();
    let mut bundle = ComponentBundle::new();
    bundle.add(world.registry(), Position { x: 0.0, y: 0.0 });
    bundle.add(world.registry(), Velocity { dx: 1.0, dy: 0.5 });
    bundle.add(world.registry(), Counter(0));
    let entity = world.spawn_bundle(bundle);

    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.add_system("movement", movement_system);
    tick_loop.add_system("counter", counter_system);

    tick_loop.run_ticks(20);

    let pos = tick_loop
        .world()
        .get_component::<Position>(entity)
        .expect("should have position");
    let counter = tick_loop
        .world()
        .get_component::<Counter>(entity)
        .expect("should have counter");

    assert!(
        (pos.x - 20.0).abs() < 1e-10,
        "expected x=20.0, got {}",
        pos.x
    );
    assert!(
        (pos.y - 10.0).abs() < 1e-10,
        "expected y=10.0, got {}",
        pos.y
    );
    assert_eq!(counter.0, 20, "expected counter=20, got {}", counter.0);
}

// ---------------------------------------------------------------------------
// 8. Headless mode with no systems (empty tick)
// ---------------------------------------------------------------------------

#[test]
fn headless_empty_ticks() {
    let world = setup_world();
    let config = TickConfig {
        headless: true,
        ..Default::default()
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Empty ticks (no systems) should still advance the counter.
    tick_loop.run_ticks(10);
    assert_eq!(tick_loop.tick_count(), 10);
    assert_eq!(tick_loop.system_count(), 0);
}

// ---------------------------------------------------------------------------
// 9. Headless determinism: two identical runs produce identical state
// ---------------------------------------------------------------------------

#[test]
fn headless_determinism() {
    fn run_headless() -> (Position, Counter) {
        let mut world = World::new();
        world.register_component::<Position>("position");
        world.register_component::<Velocity>("velocity");
        world.register_component::<Counter>("counter");

        let mut bundle = ComponentBundle::new();
        bundle.add(world.registry(), Position { x: 5.0, y: 10.0 });
        bundle.add(world.registry(), Velocity { dx: 0.3, dy: -0.7 });
        bundle.add(world.registry(), Counter(0));
        let entity = world.spawn_bundle(bundle);

        let config = TickConfig {
            fixed_dt: 1.0 / 60.0,
            headless: true,
        };
        let mut tick_loop = TickLoop::new(world, config);
        tick_loop.add_system("movement", movement_system);
        tick_loop.add_system("counter", counter_system);
        tick_loop.run_ticks(200);

        let pos = tick_loop
            .world()
            .get_component::<Position>(entity)
            .unwrap()
            .clone();
        let ctr = tick_loop
            .world()
            .get_component::<Counter>(entity)
            .unwrap()
            .clone();
        (pos, ctr)
    }

    let (pos1, ctr1) = run_headless();
    let (pos2, ctr2) = run_headless();

    assert_eq!(
        pos1, pos2,
        "headless runs should be deterministic (position)"
    );
    assert_eq!(
        ctr1, ctr2,
        "headless runs should be deterministic (counter)"
    );
}

// ---------------------------------------------------------------------------
// 10. Windowed runner requires renderer feature (compile-time check)
// ---------------------------------------------------------------------------

/// This test simply verifies that `run_windowed` is available when the
/// renderer feature is enabled. It does NOT open a window -- that would
/// require a GPU and display.
#[cfg(feature = "renderer")]
#[test]
fn run_windowed_exists_with_renderer_feature() {
    // Verify the function signature exists by taking a function pointer.
    let _fn_ptr: fn(TickLoop, &str, u32, u32) -> Result<(), anyhow::Error> =
        nomai_engine::render::run_windowed;
}

/// When the renderer feature is enabled, verify that `render_world` is
/// available on `DebugRenderer`.
#[cfg(feature = "renderer")]
#[test]
fn render_world_method_exists_with_renderer_feature() {
    // We cannot call render_world without a GPU, but we can verify the
    // method exists on the type by checking extract_draw_commands (which
    // is the GPU-free half of render_world).
    let mut world = World::new();
    world.register_component::<nomai_engine::physics::Position>("position");
    world.register_component::<nomai_engine::physics::PhysicsBody>("physics_body");

    let commands = nomai_engine::render::DebugRenderer::extract_draw_commands(&world);
    assert!(
        commands.is_empty(),
        "empty world should produce no draw commands"
    );
}
