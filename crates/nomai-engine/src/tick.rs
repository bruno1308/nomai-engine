//! Fixed-timestep tick loop for deterministic simulation.
//!
//! The [`TickLoop`] drives the Nomai Engine simulation forward. Each tick:
//!
//! 1. All registered systems run in declaration order, each receiving a shared
//!    reference to the [`World`] and a mutable reference to the [`CommandBuffer`].
//! 2. The command buffer is applied to the world (FIFO, deterministic).
//! 3. The tick counter and simulation time advance.
//!
//! Because system ordering is fixed, the command buffer is FIFO, and there is no
//! non-deterministic input (randomness must use a seeded RNG), the tick loop is
//! fully deterministic: same initial state + same systems + same inputs = same
//! final state.
//!
//! # Example
//!
//! ```
//! use nomai_engine::tick::{TickConfig, TickLoop};
//! use nomai_ecs::prelude::*;
//!
//! let world = World::new();
//! let config = TickConfig { fixed_dt: 1.0 / 60.0, ..Default::default() };
//! let mut tick_loop = TickLoop::new(world, config);
//!
//! // Register systems.
//! tick_loop.add_system("physics", |_world, _cmds| {
//!     // physics logic here
//! });
//!
//! // Run 10 ticks.
//! for _ in 0..10 {
//!     tick_loop.tick();
//! }
//!
//! assert_eq!(tick_loop.tick_count(), 10);
//! ```

use std::time::{Duration, Instant};

use nomai_ecs::command::CommandBuffer;
use nomai_ecs::world::World;

// ---------------------------------------------------------------------------
// TickConfig
// ---------------------------------------------------------------------------

/// Configuration for the fixed-timestep tick loop.
///
/// The `fixed_dt` is the duration in seconds of each simulation tick. A value
/// of `1.0 / 60.0` gives 60 ticks per second.
#[derive(Debug, Clone)]
pub struct TickConfig {
    /// Fixed time step in seconds per tick. Must be positive and finite.
    pub fixed_dt: f64,
    /// Headless mode: no rendering, tick as fast as possible.
    pub headless: bool,
}

impl Default for TickConfig {
    /// Defaults to 60 Hz (1/60 second per tick), headless off.
    fn default() -> Self {
        Self {
            fixed_dt: 1.0 / 60.0,
            headless: false,
        }
    }
}

// ---------------------------------------------------------------------------
// TickDiagnostics
// ---------------------------------------------------------------------------

/// Timing diagnostics for the last tick.
#[derive(Debug, Clone, Default)]
pub struct TickDiagnostics {
    /// Wall-clock time per system (in order of execution).
    pub system_times: Vec<(String, Duration)>,
    /// Total time for the tick (systems + command apply).
    pub total_time: Duration,
    /// Time spent applying commands.
    pub command_apply_time: Duration,
}

// ---------------------------------------------------------------------------
// InputFrame
// ---------------------------------------------------------------------------

/// A single frame of recorded input for replay.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct InputFrame {
    /// Arbitrary key-value pairs representing inputs for this tick.
    pub inputs: std::collections::HashMap<String, serde_json::Value>,
}

// ---------------------------------------------------------------------------
// SystemFn
// ---------------------------------------------------------------------------

/// A system function that operates on the world each tick.
///
/// Systems receive a shared reference to the [`World`] (read-only) and a
/// mutable reference to the [`CommandBuffer`] where they queue mutations.
/// All mutations happen via the command buffer to ensure determinism and
/// manifest traceability.
pub type SystemFn = fn(&World, &mut CommandBuffer);

// ---------------------------------------------------------------------------
// RegisteredSystem
// ---------------------------------------------------------------------------

/// A named system in the registry.
///
/// The name is used for debugging, logging, and manifest integration.
/// The `after` field lists the names of systems that must execute before
/// this system.
#[derive(Debug)]
struct RegisteredSystem {
    /// Human-readable name for this system (e.g., `"physics"`, `"movement"`).
    name: String,
    /// The system function to invoke each tick.
    func: SystemFn,
    /// Names of systems that must execute before this one.
    after: Vec<String>,
}

// ---------------------------------------------------------------------------
// TickLoop
// ---------------------------------------------------------------------------

/// The deterministic fixed-timestep tick loop.
///
/// Drives the simulation forward by running systems in a fixed order each tick,
/// applying the resulting command buffer, and advancing the tick counter and
/// simulation time.
///
/// # Determinism Guarantee
///
/// Given the same initial [`World`] state, the same registered systems (in the
/// same order), and the same external inputs, the tick loop will produce
/// identical results across runs and platforms. This is guaranteed by:
///
/// - Fixed system execution order (declaration order).
/// - Deterministic command buffer application (FIFO).
/// - No floating-point non-determinism in the tick counter (uses `f64`
///   multiplication by tick count, not accumulation).
pub struct TickLoop {
    /// The ECS world containing all entities and components.
    world: World,
    /// The shared command buffer used each tick.
    command_buffer: CommandBuffer,
    /// Ordered list of systems to run each tick.
    systems: Vec<RegisteredSystem>,
    /// Number of ticks executed so far.
    tick_counter: u64,
    /// Fixed time step in seconds per tick.
    fixed_dt: f64,
    /// Configuration used to create this tick loop.
    config: TickConfig,
    /// Diagnostics from the last tick.
    last_diagnostics: TickDiagnostics,
    /// Current tick's input frame (set before tick() for replay).
    current_input: InputFrame,
}

impl TickLoop {
    /// Create a new tick loop with the given world and configuration.
    ///
    /// The tick counter starts at 0 and simulation time at 0.0.
    pub fn new(world: World, config: TickConfig) -> Self {
        assert!(
            config.fixed_dt > 0.0 && config.fixed_dt.is_finite(),
            "fixed_dt must be positive and finite, got {}",
            config.fixed_dt
        );
        Self {
            world,
            command_buffer: CommandBuffer::new(),
            systems: Vec::new(),
            tick_counter: 0,
            fixed_dt: config.fixed_dt,
            config,
            last_diagnostics: TickDiagnostics::default(),
            current_input: InputFrame::default(),
        }
    }

    /// Register a system to be run each tick.
    ///
    /// Systems are executed in the order they are registered. The `name` is
    /// used for debugging and manifest integration.
    ///
    /// # Panics
    ///
    /// Panics if a system with the same name is already registered.
    pub fn add_system(&mut self, name: &str, func: SystemFn) {
        self.add_system_after(name, &[], func);
    }

    /// Register a system with explicit execution dependencies.
    ///
    /// `after` lists system names that must execute before this system.
    /// Validates that all referenced systems exist and that no cycles would
    /// be created.
    ///
    /// # Panics
    ///
    /// - If any system in `after` is not already registered.
    /// - If a system with this name already exists.
    /// - If adding this system would create a dependency cycle.
    pub fn add_system_after(&mut self, name: &str, after: &[&str], func: SystemFn) {
        // Validate all "after" systems exist.
        for dep in after {
            assert!(
                self.systems.iter().any(|s| s.name == *dep),
                "system '{name}' declares dependency on '{dep}', but '{dep}' is not registered"
            );
        }

        assert!(
            !self.systems.iter().any(|s| s.name == name),
            "duplicate system name: {name:?}"
        );

        self.systems.push(RegisteredSystem {
            name: name.to_owned(),
            func,
            after: after.iter().map(|s| s.to_string()).collect(),
        });

        // Validate no cycles after insertion.
        self.validate_system_order();
    }

    /// Validate that there are no cycles in the system dependency graph.
    ///
    /// Uses depth-first search with a recursion stack to detect back edges.
    ///
    /// # Panics
    ///
    /// Panics if a cycle is detected in the dependency graph.
    fn validate_system_order(&self) {
        let mut visited = vec![false; self.systems.len()];
        let mut in_stack = vec![false; self.systems.len()];

        fn dfs(
            systems: &[RegisteredSystem],
            idx: usize,
            visited: &mut [bool],
            in_stack: &mut [bool],
        ) -> bool {
            if in_stack[idx] {
                return false; // cycle detected
            }
            if visited[idx] {
                return true;
            }
            visited[idx] = true;
            in_stack[idx] = true;
            for dep_name in &systems[idx].after {
                if let Some(dep_idx) = systems.iter().position(|s| s.name == *dep_name) {
                    if !dfs(systems, dep_idx, visited, in_stack) {
                        return false;
                    }
                }
            }
            in_stack[idx] = false;
            true
        }

        for i in 0..self.systems.len() {
            assert!(
                dfs(&self.systems, i, &mut visited, &mut in_stack),
                "cycle detected in system dependencies"
            );
        }
    }

    /// Execute one simulation tick.
    ///
    /// 1. Runs all registered systems in order. Each system reads from the
    ///    world and writes commands to the shared command buffer.
    /// 2. Applies the command buffer to the world (deterministic FIFO order).
    /// 3. Advances the tick counter and simulation time.
    ///
    /// Returns the list of processed commands from this tick (useful for the
    /// change journal / manifest pipeline). Check each command's
    /// `applied_successfully` field to distinguish real mutations from failed
    /// attempts.
    pub fn tick(&mut self) -> Vec<nomai_ecs::command::Command> {
        let tick_start = Instant::now();
        let mut system_times = Vec::with_capacity(self.systems.len());

        // Phase 1: Run all systems in registered order with timing.
        for system in &self.systems {
            let sys_start = Instant::now();
            (system.func)(&self.world, &mut self.command_buffer);
            system_times.push((system.name.clone(), sys_start.elapsed()));
        }

        // Phase 2: Apply command buffer to the world with timing.
        let apply_start = Instant::now();
        let applied = self.command_buffer.apply(&mut self.world);
        let command_apply_time = apply_start.elapsed();

        // Phase 3: Advance tick counter.
        self.tick_counter += 1;

        self.last_diagnostics = TickDiagnostics {
            system_times,
            total_time: tick_start.elapsed(),
            command_apply_time,
        };

        applied
    }

    /// Run multiple ticks in sequence.
    ///
    /// Equivalent to calling [`tick`](Self::tick) `count` times. Returns the
    /// total number of commands processed (both successful and failed) across
    /// all ticks. Use [`Command::applied_successfully`](nomai_ecs::command::Command::applied_successfully)
    /// to distinguish successful mutations from failed attempts.
    pub fn run_ticks(&mut self, count: u64) -> u64 {
        let mut total_commands = 0u64;
        for _ in 0..count {
            let applied = self.tick();
            total_commands += applied.len() as u64;
        }
        total_commands
    }

    // -- accessors ----------------------------------------------------------

    /// The number of ticks executed so far.
    pub fn tick_count(&self) -> u64 {
        self.tick_counter
    }

    /// The current simulation time in seconds.
    ///
    /// Computed as `tick_count * fixed_dt` to avoid floating-point drift from
    /// repeated addition.
    pub fn sim_time(&self) -> f64 {
        self.tick_counter as f64 * self.fixed_dt
    }

    /// The fixed time step in seconds per tick.
    pub fn fixed_dt(&self) -> f64 {
        self.fixed_dt
    }

    /// Read-only access to the ECS world.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Mutable access to the ECS world.
    ///
    /// Use sparingly -- prefer the command buffer for mutations during
    /// simulation. Direct world access is appropriate for initial setup
    /// and testing.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// The number of registered systems.
    pub fn system_count(&self) -> usize {
        self.systems.len()
    }

    /// The names of all registered systems, in execution order.
    pub fn system_names(&self) -> Vec<&str> {
        self.systems.iter().map(|s| s.name.as_str()).collect()
    }

    /// Diagnostics from the last tick (timing per system).
    pub fn last_diagnostics(&self) -> &TickDiagnostics {
        &self.last_diagnostics
    }

    /// Set the input frame for the next tick (used for replay).
    pub fn set_input(&mut self, input: InputFrame) {
        self.current_input = input;
    }

    /// Read the current tick's input frame.
    pub fn current_input(&self) -> &InputFrame {
        &self.current_input
    }

    /// Whether headless mode is enabled.
    pub fn is_headless(&self) -> bool {
        self.config.headless
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nomai_ecs::prelude::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    // -- test component types -----------------------------------------------

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
    struct Health(u32);

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Counter(u64);

    fn setup_world() -> World {
        let mut world = World::new();
        world.register_component::<Position>("position");
        world.register_component::<Velocity>("velocity");
        world.register_component::<Health>("health");
        world.register_component::<Counter>("counter");
        world
    }

    // -- 1. Basic construction and defaults ---------------------------------

    #[test]
    fn new_tick_loop_starts_at_zero() {
        let world = setup_world();
        let tick_loop = TickLoop::new(world, TickConfig::default());

        assert_eq!(tick_loop.tick_count(), 0);
        assert_eq!(tick_loop.sim_time(), 0.0);
        assert_eq!(tick_loop.system_count(), 0);
    }

    #[test]
    fn default_config_is_60hz() {
        let config = TickConfig::default();
        let expected = 1.0 / 60.0;
        assert!((config.fixed_dt - expected).abs() < f64::EPSILON);
    }

    #[test]
    #[should_panic(expected = "fixed_dt must be positive")]
    fn zero_dt_panics() {
        let world = World::new();
        let _tick_loop = TickLoop::new(
            world,
            TickConfig {
                fixed_dt: 0.0,
                ..Default::default()
            },
        );
    }

    #[test]
    #[should_panic(expected = "fixed_dt must be positive")]
    fn negative_dt_panics() {
        let world = World::new();
        let _tick_loop = TickLoop::new(
            world,
            TickConfig {
                fixed_dt: -1.0,
                ..Default::default()
            },
        );
    }

    #[test]
    #[should_panic(expected = "fixed_dt must be positive")]
    fn infinity_dt_panics() {
        let world = World::new();
        let _tick_loop = TickLoop::new(
            world,
            TickConfig {
                fixed_dt: f64::INFINITY,
                ..Default::default()
            },
        );
    }

    // -- 2. System registration ---------------------------------------------

    #[test]
    fn add_systems_in_order() {
        let world = setup_world();
        let mut tick_loop = TickLoop::new(world, TickConfig::default());

        tick_loop.add_system("alpha", |_w, _c| {});
        tick_loop.add_system("beta", |_w, _c| {});
        tick_loop.add_system("gamma", |_w, _c| {});

        assert_eq!(tick_loop.system_count(), 3);
        assert_eq!(tick_loop.system_names(), vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    #[should_panic(expected = "duplicate system name")]
    fn duplicate_system_name_panics() {
        let world = setup_world();
        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("physics", |_w, _c| {});
        tick_loop.add_system("physics", |_w, _c| {});
    }

    // -- 3. Empty tick (no systems) -----------------------------------------

    #[test]
    fn empty_tick_advances_counter_and_time() {
        let world = setup_world();
        let config = TickConfig {
            fixed_dt: 0.01,
            ..Default::default()
        };
        let mut tick_loop = TickLoop::new(world, config);

        tick_loop.tick();
        assert_eq!(tick_loop.tick_count(), 1);
        assert!((tick_loop.sim_time() - 0.01).abs() < f64::EPSILON);

        tick_loop.tick();
        assert_eq!(tick_loop.tick_count(), 2);
        assert!((tick_loop.sim_time() - 0.02).abs() < f64::EPSILON);
    }

    // -- 4. System execution order ------------------------------------------

    /// We use atomics to record the order systems execute in. Each system
    /// increments a shared counter and records its own invocation index.
    static ORDER_COUNTER: AtomicU64 = AtomicU64::new(0);
    static SYSTEM_A_ORDER: AtomicU64 = AtomicU64::new(u64::MAX);
    static SYSTEM_B_ORDER: AtomicU64 = AtomicU64::new(u64::MAX);
    static SYSTEM_C_ORDER: AtomicU64 = AtomicU64::new(u64::MAX);

    fn system_a(_world: &World, _cmds: &mut CommandBuffer) {
        SYSTEM_A_ORDER.store(
            ORDER_COUNTER.fetch_add(1, Ordering::SeqCst),
            Ordering::SeqCst,
        );
    }

    fn system_b(_world: &World, _cmds: &mut CommandBuffer) {
        SYSTEM_B_ORDER.store(
            ORDER_COUNTER.fetch_add(1, Ordering::SeqCst),
            Ordering::SeqCst,
        );
    }

    fn system_c(_world: &World, _cmds: &mut CommandBuffer) {
        SYSTEM_C_ORDER.store(
            ORDER_COUNTER.fetch_add(1, Ordering::SeqCst),
            Ordering::SeqCst,
        );
    }

    #[test]
    fn systems_execute_in_registration_order() {
        ORDER_COUNTER.store(0, Ordering::SeqCst);
        SYSTEM_A_ORDER.store(u64::MAX, Ordering::SeqCst);
        SYSTEM_B_ORDER.store(u64::MAX, Ordering::SeqCst);
        SYSTEM_C_ORDER.store(u64::MAX, Ordering::SeqCst);

        let world = setup_world();
        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("A", system_a);
        tick_loop.add_system("B", system_b);
        tick_loop.add_system("C", system_c);

        tick_loop.tick();

        let a = SYSTEM_A_ORDER.load(Ordering::SeqCst);
        let b = SYSTEM_B_ORDER.load(Ordering::SeqCst);
        let c = SYSTEM_C_ORDER.load(Ordering::SeqCst);

        assert!(a < b, "system A ({a}) should run before system B ({b})");
        assert!(b < c, "system B ({b}) should run before system C ({c})");
    }

    // -- 5. Systems produce commands that modify world ----------------------

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

    #[test]
    fn system_commands_modify_world() {
        let mut world = setup_world();
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 0.0, y: 0.0 });
        b.add(world.registry(), Velocity { dx: 1.0, dy: 2.0 });
        let entity = world.spawn_bundle(b);

        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("movement", movement_system);

        tick_loop.tick();

        let pos = tick_loop.world().get_component::<Position>(entity).unwrap();
        assert!((pos.x - 1.0).abs() < f64::EPSILON);
        assert!((pos.y - 2.0).abs() < f64::EPSILON);
    }

    // -- 6. Multiple ticks accumulate changes --------------------------------

    #[test]
    fn multiple_ticks_accumulate_movement() {
        let mut world = setup_world();
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 0.0, y: 0.0 });
        b.add(world.registry(), Velocity { dx: 3.0, dy: -1.0 });
        let entity = world.spawn_bundle(b);

        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("movement", movement_system);

        tick_loop.run_ticks(10);

        let pos = tick_loop.world().get_component::<Position>(entity).unwrap();
        assert!((pos.x - 30.0).abs() < f64::EPSILON);
        assert!((pos.y - (-10.0)).abs() < f64::EPSILON);
        assert_eq!(tick_loop.tick_count(), 10);
    }

    // -- 7. run_ticks helper ------------------------------------------------

    #[test]
    fn run_ticks_returns_total_commands() {
        let mut world = setup_world();
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 0.0, y: 0.0 });
        b.add(world.registry(), Velocity { dx: 1.0, dy: 0.0 });
        let _entity = world.spawn_bundle(b);

        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("movement", movement_system);

        // Each tick should produce 1 command (1 entity with pos+vel).
        let total = tick_loop.run_ticks(5);
        assert_eq!(total, 5);
    }

    // -- 8. Sim time is computed, not accumulated ---------------------------

    #[test]
    fn sim_time_computed_not_accumulated() {
        let world = setup_world();
        let config = TickConfig {
            fixed_dt: 0.1,
            ..Default::default()
        };
        let mut tick_loop = TickLoop::new(world, config);

        tick_loop.run_ticks(1000);

        // If we accumulated 0.1 a thousand times, floating-point drift would
        // give us something slightly off 100.0. But since we compute
        // tick_count * fixed_dt, it should be exact.
        assert_eq!(tick_loop.sim_time(), 1000.0 * 0.1);
        assert_eq!(tick_loop.tick_count(), 1000);
    }

    // -- 9. Command buffer is cleared between ticks -------------------------

    fn counter_system(world: &World, cmds: &mut CommandBuffer) {
        for (entity, (counter,)) in world.query::<(&Counter,)>() {
            let new_val = counter.0 + 1;
            cmds.set_component(
                entity,
                "counter",
                serde_json::json!(new_val),
                SystemId(0),
                CausalReason::SystemInternal("increment".to_owned()),
            );
        }
    }

    #[test]
    fn command_buffer_cleared_between_ticks() {
        let mut world = setup_world();
        let entity = world.spawn_with(Counter(0));

        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("counter", counter_system);

        // Each tick should produce exactly 1 command, not accumulate from
        // previous ticks.
        let cmds_tick_1 = tick_loop.tick();
        assert_eq!(cmds_tick_1.len(), 1);

        let cmds_tick_2 = tick_loop.tick();
        assert_eq!(cmds_tick_2.len(), 1);

        let cmds_tick_3 = tick_loop.tick();
        assert_eq!(cmds_tick_3.len(), 1);

        let counter = tick_loop.world().get_component::<Counter>(entity).unwrap();
        assert_eq!(counter.0, 3);
    }

    // -- 10. DETERMINISM: 100-tick test -------------------------------------

    /// Helper: build a deterministic world with known initial state.
    fn build_deterministic_world() -> (World, EntityId, EntityId) {
        let mut world = World::new();
        world.register_component::<Position>("position");
        world.register_component::<Velocity>("velocity");
        world.register_component::<Health>("health");
        world.register_component::<Counter>("counter");

        // Entity 1: moving entity.
        let mut b1 = ComponentBundle::new();
        b1.add(world.registry(), Position { x: 0.0, y: 0.0 });
        b1.add(world.registry(), Velocity { dx: 1.5, dy: -0.5 });
        b1.add(world.registry(), Counter(0));
        let e1 = world.spawn_bundle(b1);

        // Entity 2: stationary entity with counter.
        let mut b2 = ComponentBundle::new();
        b2.add(world.registry(), Position { x: 100.0, y: 200.0 });
        b2.add(world.registry(), Counter(0));
        let e2 = world.spawn_bundle(b2);

        (world, e1, e2)
    }

    /// System: move entities with Position + Velocity.
    fn deterministic_movement(world: &World, cmds: &mut CommandBuffer) {
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

    /// System: increment Counter on all entities that have one.
    fn deterministic_counter(world: &World, cmds: &mut CommandBuffer) {
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

    /// Run 100 ticks on a world with the given systems. Returns the final
    /// state of all tracked values.
    fn run_100_ticks() -> (Position, Counter, Position, Counter, u64, f64) {
        let (world, e1, e2) = build_deterministic_world();
        let config = TickConfig {
            fixed_dt: 1.0 / 60.0,
            ..Default::default()
        };
        let mut tick_loop = TickLoop::new(world, config);

        tick_loop.add_system("movement", deterministic_movement);
        tick_loop.add_system("counter", deterministic_counter);

        let total_cmds = tick_loop.run_ticks(100);

        let pos1 = tick_loop
            .world()
            .get_component::<Position>(e1)
            .unwrap()
            .clone();
        let counter1 = tick_loop
            .world()
            .get_component::<Counter>(e1)
            .unwrap()
            .clone();
        let pos2 = tick_loop
            .world()
            .get_component::<Position>(e2)
            .unwrap()
            .clone();
        let counter2 = tick_loop
            .world()
            .get_component::<Counter>(e2)
            .unwrap()
            .clone();

        (
            pos1,
            counter1,
            pos2,
            counter2,
            total_cmds,
            tick_loop.sim_time(),
        )
    }

    #[test]
    fn determinism_100_ticks_identical_runs() {
        // Run the same simulation twice and verify identical results.
        let run1 = run_100_ticks();
        let run2 = run_100_ticks();

        // Position of entity 1.
        assert_eq!(run1.0, run2.0, "entity 1 position diverged");

        // Counter of entity 1.
        assert_eq!(run1.1, run2.1, "entity 1 counter diverged");

        // Position of entity 2.
        assert_eq!(run1.2, run2.2, "entity 2 position diverged");

        // Counter of entity 2.
        assert_eq!(run1.3, run2.3, "entity 2 counter diverged");

        // Total commands applied.
        assert_eq!(run1.4, run2.4, "total command count diverged");

        // Simulation time.
        assert_eq!(run1.5, run2.5, "simulation time diverged");
    }

    #[test]
    fn determinism_100_ticks_expected_values() {
        let (pos1, counter1, pos2, counter2, total_cmds, sim_time) = run_100_ticks();

        // Entity 1: moved 100 ticks * (1.5, -0.5) from (0, 0).
        assert!(
            (pos1.x - 150.0).abs() < 1e-10,
            "entity 1 x: expected 150.0, got {}",
            pos1.x
        );
        assert!(
            (pos1.y - (-50.0)).abs() < 1e-10,
            "entity 1 y: expected -50.0, got {}",
            pos1.y
        );

        // Entity 1 counter: incremented each tick.
        assert_eq!(counter1.0, 100);

        // Entity 2: stationary (no velocity component).
        assert!(
            (pos2.x - 100.0).abs() < 1e-10,
            "entity 2 x: expected 100.0, got {}",
            pos2.x
        );
        assert!(
            (pos2.y - 200.0).abs() < 1e-10,
            "entity 2 y: expected 200.0, got {}",
            pos2.y
        );

        // Entity 2 counter: also incremented each tick.
        assert_eq!(counter2.0, 100);

        // Total commands: entity 1 produces 2 cmds/tick (movement + counter),
        // entity 2 produces 1 cmd/tick (counter only). 3 * 100 = 300.
        assert_eq!(total_cmds, 300);

        // Sim time: 100 * (1/60).
        let expected_time = 100.0 * (1.0 / 60.0);
        assert!(
            (sim_time - expected_time).abs() < 1e-10,
            "sim_time: expected {expected_time}, got {sim_time}"
        );
    }

    // -- 11. Multiple entities with same components -------------------------

    #[test]
    fn multiple_entities_all_updated() {
        let mut world = setup_world();

        let mut entities = Vec::new();
        for i in 0..10 {
            let mut b = ComponentBundle::new();
            b.add(
                world.registry(),
                Position {
                    x: i as f64,
                    y: 0.0,
                },
            );
            b.add(world.registry(), Velocity { dx: 1.0, dy: 1.0 });
            entities.push(world.spawn_bundle(b));
        }

        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("movement", movement_system);
        tick_loop.run_ticks(5);

        for (i, &entity) in entities.iter().enumerate() {
            let pos = tick_loop.world().get_component::<Position>(entity).unwrap();
            let expected_x = i as f64 + 5.0;
            let expected_y = 5.0;
            assert!(
                (pos.x - expected_x).abs() < 1e-10,
                "entity {i} x: expected {expected_x}, got {}",
                pos.x
            );
            assert!(
                (pos.y - expected_y).abs() < 1e-10,
                "entity {i} y: expected {expected_y}, got {}",
                pos.y
            );
        }
    }

    // -- 12. Despawn via system command --------------------------------------

    fn despawn_low_health(world: &World, cmds: &mut CommandBuffer) {
        for (entity, (health,)) in world.query::<(&Health,)>() {
            if health.0 == 0 {
                cmds.despawn(
                    entity,
                    SystemId(10),
                    CausalReason::GameRule("health_depleted".to_owned()),
                );
            }
        }
    }

    #[test]
    fn system_can_despawn_entities() {
        let mut world = setup_world();
        let alive = world.spawn_with(Health(100));
        let doomed = world.spawn_with(Health(0));

        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("reaper", despawn_low_health);

        tick_loop.tick();

        assert!(tick_loop.world().is_alive(alive));
        assert!(!tick_loop.world().is_alive(doomed));
        assert_eq!(tick_loop.world().entity_count(), 1);
    }

    // -- 13. Tick returns applied commands ----------------------------------

    #[test]
    fn tick_returns_applied_commands() {
        let mut world = setup_world();
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Position { x: 0.0, y: 0.0 });
        b.add(world.registry(), Velocity { dx: 1.0, dy: 0.0 });
        let _entity = world.spawn_bundle(b);

        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("movement", movement_system);

        let cmds = tick_loop.tick();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(
            cmds[0].kind,
            nomai_ecs::command::CommandKind::SetComponent { .. }
        ));
    }

    // -- 14. World access after simulation ----------------------------------

    #[test]
    fn world_mut_allows_setup_after_construction() {
        let world = setup_world();
        let mut tick_loop = TickLoop::new(world, TickConfig::default());

        let entity = tick_loop.world_mut().spawn_with(Counter(0));
        tick_loop.add_system("counter", counter_system);

        tick_loop.run_ticks(50);

        let counter = tick_loop.world().get_component::<Counter>(entity).unwrap();
        assert_eq!(counter.0, 50);
    }

    // -- 15. Determinism with multiple entity types -------------------------

    #[test]
    fn determinism_multiple_entity_types() {
        // Create a complex scenario with mixed entity archetypes.
        fn build_complex_world() -> (World, Vec<EntityId>) {
            let mut world = World::new();
            world.register_component::<Position>("position");
            world.register_component::<Velocity>("velocity");
            world.register_component::<Counter>("counter");

            let mut entities = Vec::new();

            // Type A: Position + Velocity + Counter.
            for i in 0..5 {
                let mut b = ComponentBundle::new();
                b.add(
                    world.registry(),
                    Position {
                        x: i as f64 * 10.0,
                        y: 0.0,
                    },
                );
                b.add(world.registry(), Velocity { dx: 1.0, dy: 0.5 });
                b.add(world.registry(), Counter(0));
                entities.push(world.spawn_bundle(b));
            }

            // Type B: Position + Counter (no velocity).
            for i in 0..3 {
                let mut b = ComponentBundle::new();
                b.add(
                    world.registry(),
                    Position {
                        x: 100.0 + i as f64,
                        y: 100.0,
                    },
                );
                b.add(world.registry(), Counter(0));
                entities.push(world.spawn_bundle(b));
            }

            // Type C: Counter only.
            for _ in 0..2 {
                entities.push(world.spawn_with(Counter(0)));
            }

            (world, entities)
        }

        fn run_complex() -> Vec<(Option<Position>, Option<Counter>)> {
            let (world, entities) = build_complex_world();
            let config = TickConfig {
                fixed_dt: 1.0 / 60.0,
                ..Default::default()
            };
            let mut tick_loop = TickLoop::new(world, config);
            tick_loop.add_system("movement", deterministic_movement);
            tick_loop.add_system("counter", deterministic_counter);
            tick_loop.run_ticks(100);

            entities
                .iter()
                .map(|&e| {
                    let pos = tick_loop.world().get_component::<Position>(e).cloned();
                    let ctr = tick_loop.world().get_component::<Counter>(e).cloned();
                    (pos, ctr)
                })
                .collect()
        }

        let run1 = run_complex();
        let run2 = run_complex();

        assert_eq!(run1.len(), run2.len());
        for (i, (a, b)) in run1.iter().zip(run2.iter()).enumerate() {
            assert_eq!(a, b, "entity {i} diverged between runs");
        }
    }

    // -- 16. System dependency ordering ------------------------------------

    #[test]
    fn system_dependency_ordering() {
        let world = setup_world();
        let mut tick_loop = TickLoop::new(world, TickConfig::default());

        tick_loop.add_system("alpha", |_w, _c| {});
        tick_loop.add_system_after("beta", &["alpha"], |_w, _c| {});
        tick_loop.add_system_after("gamma", &["beta"], |_w, _c| {});

        assert_eq!(tick_loop.system_names(), vec!["alpha", "beta", "gamma"]);
    }

    // -- 17. Missing dependency panics ------------------------------------

    #[test]
    #[should_panic(expected = "not registered")]
    fn system_dependency_on_missing_panics() {
        let world = setup_world();
        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system_after("beta", &["alpha"], |_w, _c| {});
    }

    // -- 18. Tick diagnostics records timing ------------------------------

    #[test]
    fn tick_diagnostics_records_timing() {
        let mut world = setup_world();
        world.spawn_with(Counter(0));
        let mut tick_loop = TickLoop::new(world, TickConfig::default());
        tick_loop.add_system("counter", counter_system);

        tick_loop.tick();

        let diag = tick_loop.last_diagnostics();
        assert_eq!(diag.system_times.len(), 1);
        assert_eq!(diag.system_times[0].0, "counter");
        assert!(diag.total_time > Duration::ZERO);
    }

    // -- 19. Headless config -----------------------------------------------

    #[test]
    fn headless_config() {
        let world = setup_world();
        let config = TickConfig {
            fixed_dt: 1.0 / 60.0,
            headless: true,
        };
        let tick_loop = TickLoop::new(world, config);
        assert!(tick_loop.is_headless());
    }

    #[test]
    fn default_not_headless() {
        let world = setup_world();
        let tick_loop = TickLoop::new(world, TickConfig::default());
        assert!(!tick_loop.is_headless());
    }

    // -- 20. Input frame injection ----------------------------------------

    #[test]
    fn input_frame_injection() {
        let world = setup_world();
        let mut tick_loop = TickLoop::new(world, TickConfig::default());

        let mut input = InputFrame::default();
        input
            .inputs
            .insert("move_x".to_string(), serde_json::json!(1.0));

        tick_loop.set_input(input);
        assert_eq!(
            tick_loop.current_input().inputs.get("move_x"),
            Some(&serde_json::json!(1.0))
        );
    }
}
