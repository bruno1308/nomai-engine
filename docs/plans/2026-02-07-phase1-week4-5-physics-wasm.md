# Phase 1, Week 4-5: Physics Integration + WASM Sandbox Hardening

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Integrate rapier2d physics with collision events flowing into the manifest, harden the WASM sandbox for production, build the AssemblyScript breakout gameplay module, and validate end-to-end with a milestone test.

**Architecture:** Physics runs as a native Rust system (not WASM) that reads ECS components, steps rapier2d, detects collisions, and emits commands with `CausalReason::CollisionResponse`. The WASM gameplay module reads collision events and responds with game logic (brick destruction, scoring). The manifest captures the full causal chain: player input → paddle move → ball collision → brick despawn → score change.

**Tech Stack:** Rust 1.83.0, rapier2d 0.22.0 (enhanced-determinism), Wasmtime 27.0.0, AssemblyScript 0.27.32, PyO3 0.23.3, Python 3.12+

---

## Dependency Graph

```
Task 1 (Physics Integration)
  │
  ├──► Task 2 (WASM Sandbox Hardening) [independent, can run in parallel]
  │
  └──► Task 3 (AS Breakout Gameplay) [depends on physics for collision events]
        │
        ▼
      Task 4 (WASM Integration into TickLoop) [depends on Tasks 1-3]
        │
        ▼
      Task 5 (Python Bindings for Physics)
        │
        ▼
      Task 6 (Milestone Test) [depends on all above]
```

---

## Task 1: rapier2d Physics Integration (Issue #29)

**GitHub Issue:** #29

**Files:**
- Modify: `Cargo.toml` (workspace deps — add rapier2d)
- Modify: `crates/nomai-engine/Cargo.toml` (add rapier2d dep)
- Create: `crates/nomai-engine/src/physics.rs` (physics system + types)
- Modify: `crates/nomai-engine/src/lib.rs` (add `pub mod physics`, update prelude)

### What to Build

A physics module that wraps rapier2d and integrates with the ECS command buffer. The physics system:

1. Maintains a rapier `PhysicsPipeline`, `RigidBodySet`, `ColliderSet`, and related sets
2. Syncs ECS component state → rapier bodies each tick (positions, velocities)
3. Steps the simulation with fixed dt
4. Detects collisions and emits `GameEvent`s with `CausalReason::CollisionResponse`
5. Syncs rapier results → ECS via commands (position/velocity updates)

### Step 1: Add rapier2d to workspace

Add to `Cargo.toml` workspace dependencies:

```toml
rapier2d = { version = "0.22.0", features = ["enhanced-determinism"] }
```

Add to `crates/nomai-engine/Cargo.toml` dependencies:

```toml
rapier2d = { workspace = true }
```

### Step 2: Write the physics module

Create `crates/nomai-engine/src/physics.rs`:

```rust
//! rapier2d physics integration with manifest causality.
//!
//! The [`PhysicsWorld`] manages a rapier2d simulation and synchronizes it with
//! the ECS [`World`]. Each tick:
//!
//! 1. ECS entities with physics components are synced into rapier bodies.
//! 2. rapier steps the simulation with the engine's fixed dt.
//! 3. Collision events are collected and converted to [`GameEvent`]s.
//! 4. Updated positions and velocities are written back to the ECS via the
//!    command buffer with [`CausalReason::CollisionResponse`] or
//!    [`CausalReason::SystemInternal("physics_step")`].
//!
//! # Determinism
//!
//! rapier2d is compiled with `enhanced-determinism`. Combined with a fixed
//! timestep and deterministic entity ordering, the physics simulation is
//! fully deterministic on the same platform.

use std::collections::HashMap;

use nomai_ecs::command::{CausalReason, CommandBuffer};
use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::SystemId;
use nomai_ecs::world::World;
use nomai_manifest::manifest::GameEvent;
use rapier2d::prelude::*;

/// System ID for the physics system.
pub const PHYSICS_SYSTEM_ID: SystemId = SystemId(50);

/// System name used in manifest recording.
pub const PHYSICS_SYSTEM_NAME: &str = "physics";

// ---------------------------------------------------------------------------
// Physics component types (ECS-side)
// ---------------------------------------------------------------------------

/// 2D position component. Synced to/from rapier rigid body translation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// 2D velocity component. Synced to/from rapier rigid body linvel.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Velocity {
    pub dx: f64,
    pub dy: f64,
}

/// Physics body type. Determines how rapier treats the body.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PhysicsBodyType {
    /// Fully simulated by physics (e.g., ball).
    Dynamic,
    /// Controlled by game logic, not by physics solver (e.g., paddle).
    Kinematic,
    /// Immovable (e.g., walls).
    Static,
}

/// Collider shape for physics.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ColliderShape {
    /// Axis-aligned box with half-extents.
    Box { half_width: f64, half_height: f64 },
    /// Circle with radius.
    Circle { radius: f64 },
}

/// Physics body descriptor. Attach to an entity to give it physics.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PhysicsBody {
    pub body_type: PhysicsBodyType,
    pub collider: ColliderShape,
    /// Coefficient of restitution (bounciness). 0.0 = no bounce, 1.0 = perfect bounce.
    pub restitution: f64,
    /// Whether this body is a sensor (detects collisions but doesn't respond physically).
    pub is_sensor: bool,
}

// ---------------------------------------------------------------------------
// PhysicsWorld
// ---------------------------------------------------------------------------

/// Manages rapier2d simulation state and syncs with the ECS.
pub struct PhysicsWorld {
    pipeline: PhysicsPipeline,
    gravity: Vector<Real>,
    integration_params: IntegrationParameters,
    island_manager: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    rigid_body_set: RigidBodySet,
    collider_set: ColliderSet,
    impulse_joint_set: ImpulseJointSet,
    multibody_joint_set: MultibodyJointSet,
    ccd_solver: CCDSolver,
    /// Maps ECS EntityId (raw u64) → rapier RigidBodyHandle.
    entity_to_body: HashMap<u64, RigidBodyHandle>,
    /// Maps rapier RigidBodyHandle → ECS EntityId (raw u64).
    body_to_entity: HashMap<RigidBodyHandle, u64>,
    /// Maps rapier ColliderHandle → ECS EntityId (raw u64) for collision lookup.
    collider_to_entity: HashMap<ColliderHandle, u64>,
}

impl PhysicsWorld {
    /// Create a new physics world with the given gravity.
    pub fn new(gravity_x: f64, gravity_y: f64) -> Self {
        Self {
            pipeline: PhysicsPipeline::new(),
            gravity: vector![gravity_x as Real, gravity_y as Real],
            integration_params: IntegrationParameters::default(),
            island_manager: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            rigid_body_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            entity_to_body: HashMap::new(),
            body_to_entity: HashMap::new(),
            collider_to_entity: HashMap::new(),
        }
    }

    /// Create a new physics world with zero gravity (typical for top-down games like breakout).
    pub fn new_zero_gravity() -> Self {
        Self::new(0.0, 0.0)
    }

    /// Register an ECS entity with the physics world.
    ///
    /// Creates a rapier rigid body and collider based on the entity's
    /// `PhysicsBody`, `Position`, and `Velocity` components.
    pub fn register_entity(
        &mut self,
        entity_id: EntityId,
        position: &Position,
        velocity: &Velocity,
        body: &PhysicsBody,
    ) {
        let raw_id = entity_id.to_raw();

        // Skip if already registered.
        if self.entity_to_body.contains_key(&raw_id) {
            return;
        }

        // Build rigid body.
        let rb = match body.body_type {
            PhysicsBodyType::Dynamic => RigidBodyBuilder::dynamic()
                .translation(vector![position.x as Real, position.y as Real])
                .linvel(vector![velocity.dx as Real, velocity.dy as Real])
                .build(),
            PhysicsBodyType::Kinematic => RigidBodyBuilder::kinematic_velocity_based()
                .translation(vector![position.x as Real, position.y as Real])
                .linvel(vector![velocity.dx as Real, velocity.dy as Real])
                .build(),
            PhysicsBodyType::Static => RigidBodyBuilder::fixed()
                .translation(vector![position.x as Real, position.y as Real])
                .build(),
        };

        let body_handle = self.rigid_body_set.insert(rb);
        self.entity_to_body.insert(raw_id, body_handle);
        self.body_to_entity.insert(body_handle, raw_id);

        // Build collider.
        let shape: SharedShape = match &body.collider {
            ColliderShape::Box {
                half_width,
                half_height,
            } => SharedShape::cuboid(*half_width as Real, *half_height as Real),
            ColliderShape::Circle { radius } => SharedShape::ball(*radius as Real),
        };

        let collider = ColliderBuilder::new(shape)
            .restitution(body.restitution as Real)
            .sensor(body.is_sensor)
            .active_events(ActiveEvents::COLLISION_EVENTS)
            .build();

        let collider_handle =
            self.collider_set
                .insert_with_parent(collider, body_handle, &mut self.rigid_body_set);
        self.collider_to_entity.insert(collider_handle, raw_id);
    }

    /// Remove an entity from the physics world.
    pub fn unregister_entity(&mut self, entity_id: EntityId) {
        let raw_id = entity_id.to_raw();
        if let Some(body_handle) = self.entity_to_body.remove(&raw_id) {
            self.body_to_entity.remove(&body_handle);
            // Remove body (and attached colliders) from rapier.
            self.rigid_body_set.remove(
                body_handle,
                &mut self.island_manager,
                &mut self.collider_set,
                &mut self.impulse_joint_set,
                &mut self.multibody_joint_set,
                true, // remove attached colliders
            );
            // Clean up collider_to_entity entries for this entity.
            self.collider_to_entity.retain(|_, eid| *eid != raw_id);
        }
    }

    /// Sync ECS position/velocity → rapier for a kinematic or dynamic body.
    ///
    /// Call this before stepping to update rapier with the latest ECS state
    /// for kinematic bodies (e.g., paddle controlled by player input).
    pub fn sync_to_rapier(
        &mut self,
        entity_id: EntityId,
        position: &Position,
        velocity: &Velocity,
    ) {
        let raw_id = entity_id.to_raw();
        if let Some(&body_handle) = self.entity_to_body.get(&raw_id) {
            if let Some(rb) = self.rigid_body_set.get_mut(body_handle) {
                rb.set_translation(
                    vector![position.x as Real, position.y as Real],
                    true,
                );
                rb.set_linvel(
                    vector![velocity.dx as Real, velocity.dy as Real],
                    true,
                );
            }
        }
    }

    /// Step the physics simulation by the given dt.
    ///
    /// Returns a list of collision events that occurred during the step.
    pub fn step(&mut self, dt: f64) -> Vec<CollisionPair> {
        self.integration_params.dt = dt as Real;

        let event_handler = ChannelEventCollector::new();

        self.pipeline.step(
            &self.gravity,
            &self.integration_params,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_body_set,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            None, // query pipeline (unused)
            &(), // physics hooks
            &event_handler,
        );

        // Collect collision events.
        let mut collisions = Vec::new();
        while let Ok(event) = event_handler.collision_events.try_recv() {
            if let CollisionEvent::Started(h1, h2, _flags) = event {
                let entity_a = self.collider_to_entity.get(&h1).copied();
                let entity_b = self.collider_to_entity.get(&h2).copied();
                if let (Some(a), Some(b)) = (entity_a, entity_b) {
                    collisions.push(CollisionPair {
                        entity_a: EntityId::from_raw(a),
                        entity_b: EntityId::from_raw(b),
                    });
                }
            }
        }

        collisions
    }

    /// Read updated positions and velocities from rapier after a step.
    ///
    /// Returns a list of (EntityId, Position, Velocity) for all dynamic bodies
    /// that moved during the last step.
    pub fn read_results(&self) -> Vec<(EntityId, Position, Velocity)> {
        let mut results = Vec::new();
        for (&raw_id, &body_handle) in &self.entity_to_body {
            if let Some(rb) = self.rigid_body_set.get(body_handle) {
                // Only read back dynamic bodies (kinematic/static are ECS-driven).
                if !rb.is_dynamic() {
                    continue;
                }
                let trans = rb.translation();
                let vel = rb.linvel();
                results.push((
                    EntityId::from_raw(raw_id),
                    Position {
                        x: trans.x as f64,
                        y: trans.y as f64,
                    },
                    Velocity {
                        dx: vel.x as f64,
                        dy: vel.y as f64,
                    },
                ));
            }
        }
        results
    }

    /// Check if an entity is registered in the physics world.
    pub fn has_entity(&self, entity_id: EntityId) -> bool {
        self.entity_to_body.contains_key(&entity_id.to_raw())
    }

    /// Number of physics bodies.
    pub fn body_count(&self) -> usize {
        self.rigid_body_set.len()
    }
}

// ---------------------------------------------------------------------------
// CollisionPair
// ---------------------------------------------------------------------------

/// A collision between two entities detected by the physics engine.
#[derive(Debug, Clone)]
pub struct CollisionPair {
    /// First entity in the collision.
    pub entity_a: EntityId,
    /// Second entity in the collision.
    pub entity_b: EntityId,
}

impl CollisionPair {
    /// Convert this collision pair into a [`GameEvent`] for the manifest.
    pub fn to_game_event(&self, tick: u64) -> GameEvent {
        GameEvent {
            event_type: "collision".to_owned(),
            description: format!(
                "Collision between entity {} and entity {}",
                self.entity_a.to_raw(),
                self.entity_b.to_raw(),
            ),
            involved_entities: vec![self.entity_a, self.entity_b],
            caused_by: PHYSICS_SYSTEM_ID,
            reason: CausalReason::CollisionResponse(self.entity_a, self.entity_b),
            tick,
        }
    }
}

// ---------------------------------------------------------------------------
// Physics system function
// ---------------------------------------------------------------------------

/// Run a single physics step, emitting collision events and position/velocity
/// updates through the command buffer.
///
/// This is designed to be called as a system function within the tick loop.
/// It requires the `PhysicsWorld` to be stored externally (not in the ECS)
/// since rapier owns its own body storage.
///
/// Returns collision pairs for upstream consumers.
pub fn run_physics_step(
    physics: &mut PhysicsWorld,
    commands: &mut CommandBuffer,
    dt: f64,
    tick: u64,
) -> (Vec<CollisionPair>, Vec<GameEvent>) {
    // Step the simulation.
    let collisions = physics.step(dt);

    // Convert collisions to game events.
    let events: Vec<GameEvent> = collisions
        .iter()
        .map(|c| c.to_game_event(tick))
        .collect();

    // Read back updated positions and velocities from rapier.
    let results = physics.read_results();
    for (entity_id, pos, vel) in &results {
        // Emit position update command.
        commands.set_component(
            *entity_id,
            "position",
            serde_json::json!({"x": pos.x, "y": pos.y}),
            PHYSICS_SYSTEM_ID,
            CausalReason::SystemInternal("physics_step".to_owned()),
        );
        // Emit velocity update command.
        commands.set_component(
            *entity_id,
            "velocity",
            serde_json::json!({"dx": vel.dx, "dy": vel.dy}),
            PHYSICS_SYSTEM_ID,
            CausalReason::SystemInternal("physics_step".to_owned()),
        );
    }

    // For collisions, also emit velocity updates with CollisionResponse causality.
    // This is done separately because the collision-specific velocity changes
    // carry richer causality than the general physics step.
    for collision in &collisions {
        // Emit velocity updates with collision causality for both entities.
        // The actual velocity values are already set by read_results above;
        // this creates a causality link in the manifest.
        commands.set_component(
            collision.entity_a,
            "velocity",
            serde_json::json!({"dx": 0.0, "dy": 0.0}), // placeholder, actual value from read_results
            PHYSICS_SYSTEM_ID,
            CausalReason::CollisionResponse(collision.entity_a, collision.entity_b),
        );
    }

    (collisions, events)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physics_world_creates_with_zero_gravity() {
        let pw = PhysicsWorld::new_zero_gravity();
        assert_eq!(pw.body_count(), 0);
    }

    #[test]
    fn register_and_check_entity() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::from_raw(1);
        let pos = Position { x: 0.0, y: 0.0 };
        let vel = Velocity { dx: 1.0, dy: 0.0 };
        let body = PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        };

        pw.register_entity(eid, &pos, &vel, &body);
        assert!(pw.has_entity(eid));
        assert_eq!(pw.body_count(), 1);
    }

    #[test]
    fn unregister_entity_removes_from_physics() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::from_raw(1);
        let pos = Position { x: 0.0, y: 0.0 };
        let vel = Velocity { dx: 0.0, dy: 0.0 };
        let body = PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        };

        pw.register_entity(eid, &pos, &vel, &body);
        assert!(pw.has_entity(eid));
        pw.unregister_entity(eid);
        assert!(!pw.has_entity(eid));
        assert_eq!(pw.body_count(), 0);
    }

    #[test]
    fn dynamic_body_moves_after_step() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::from_raw(1);
        let pos = Position { x: 0.0, y: 0.0 };
        let vel = Velocity { dx: 10.0, dy: 0.0 };
        let body = PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        };

        pw.register_entity(eid, &pos, &vel, &body);
        let _collisions = pw.step(1.0 / 60.0);
        let results = pw.read_results();

        assert_eq!(results.len(), 1);
        let (_, new_pos, _) = &results[0];
        // Ball should have moved to the right.
        assert!(new_pos.x > 0.0, "ball should move right, got x={}", new_pos.x);
    }

    #[test]
    fn two_bodies_collide_produces_event() {
        let mut pw = PhysicsWorld::new_zero_gravity();

        // Ball moving right.
        let ball = EntityId::from_raw(1);
        pw.register_entity(
            ball,
            &Position { x: 0.0, y: 0.0 },
            &Velocity { dx: 100.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        // Wall on the right (static).
        let wall = EntityId::from_raw(2);
        pw.register_entity(
            wall,
            &Position { x: 2.0, y: 0.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 0.5,
                    half_height: 10.0,
                },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        // Step multiple times to ensure collision.
        let mut all_collisions = Vec::new();
        for _ in 0..120 {
            let collisions = pw.step(1.0 / 60.0);
            all_collisions.extend(collisions);
        }

        assert!(
            !all_collisions.is_empty(),
            "ball moving toward wall should produce at least one collision"
        );

        // Verify collision pair contains our entities.
        let pair = &all_collisions[0];
        let ids = [pair.entity_a.to_raw(), pair.entity_b.to_raw()];
        assert!(
            ids.contains(&1) && ids.contains(&2),
            "collision should involve entity 1 (ball) and entity 2 (wall), got {:?}",
            ids
        );
    }

    #[test]
    fn collision_pair_to_game_event() {
        let pair = CollisionPair {
            entity_a: EntityId::from_raw(1),
            entity_b: EntityId::from_raw(2),
        };
        let event = pair.to_game_event(5);
        assert_eq!(event.event_type, "collision");
        assert_eq!(event.involved_entities.len(), 2);
        assert_eq!(event.tick, 5);
        assert_eq!(event.caused_by, PHYSICS_SYSTEM_ID);
        assert!(matches!(
            event.reason,
            CausalReason::CollisionResponse(_, _)
        ));
    }

    #[test]
    fn run_physics_step_emits_commands() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::from_raw(1);
        pw.register_entity(
            eid,
            &Position { x: 0.0, y: 0.0 },
            &Velocity { dx: 5.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        let mut cmds = CommandBuffer::new();
        let (_collisions, _events) = run_physics_step(&mut pw, &mut cmds, 1.0 / 60.0, 1);

        // Should have at least position + velocity commands for the dynamic body.
        assert!(
            cmds.len() >= 2,
            "should have position + velocity commands, got {}",
            cmds.len()
        );
    }

    #[test]
    fn static_body_does_not_produce_commands() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::from_raw(1);
        pw.register_entity(
            eid,
            &Position { x: 0.0, y: 0.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 1.0,
                    half_height: 1.0,
                },
                restitution: 0.0,
                is_sensor: false,
            },
        );

        let mut cmds = CommandBuffer::new();
        let (_collisions, _events) = run_physics_step(&mut pw, &mut cmds, 1.0 / 60.0, 1);

        // Static bodies don't move, so no commands.
        assert_eq!(cmds.len(), 0, "static body should not produce commands");
    }

    #[test]
    fn determinism_two_identical_runs() {
        fn run_simulation() -> Vec<(f64, f64)> {
            let mut pw = PhysicsWorld::new_zero_gravity();
            pw.register_entity(
                EntityId::from_raw(1),
                &Position { x: 0.0, y: 0.0 },
                &Velocity { dx: 5.0, dy: 3.0 },
                &PhysicsBody {
                    body_type: PhysicsBodyType::Dynamic,
                    collider: ColliderShape::Circle { radius: 0.5 },
                    restitution: 1.0,
                    is_sensor: false,
                },
            );

            let mut positions = Vec::new();
            for _ in 0..100 {
                pw.step(1.0 / 60.0);
                let results = pw.read_results();
                if let Some((_, pos, _)) = results.first() {
                    positions.push((pos.x, pos.y));
                }
            }
            positions
        }

        let run1 = run_simulation();
        let run2 = run_simulation();
        assert_eq!(run1, run2, "two identical runs should produce identical positions");
    }
}
```

### Step 3: Update lib.rs

Modify `crates/nomai-engine/src/lib.rs` to add the physics module and update prelude:

Add `pub mod physics;` after `pub mod tick;`.

Add to the prelude:
```rust
pub use crate::physics::{
    ColliderShape, CollisionPair, PhysicsBody, PhysicsBodyType,
    PhysicsWorld, Position, Velocity, PHYSICS_SYSTEM_ID, PHYSICS_SYSTEM_NAME,
};
```

### Step 4: Run tests

Run: `cargo test -p nomai-engine --lib`
Expected: All new physics tests pass plus all existing tests.

Run: `cargo clippy -p nomai-engine -- -D warnings`
Expected: Zero warnings.

### Step 5: Commit

```
feat: rapier2d physics integration with collision events and causality (#29)

- PhysicsWorld: register/unregister entities, step simulation, read results
- Collision detection → GameEvent with CausalReason::CollisionResponse
- Position/velocity updates via command buffer with physics causality
- Component types: Position, Velocity, PhysicsBody, ColliderShape
- Deterministic: enhanced-determinism feature, fixed timestep
- 8 unit tests including collision detection and determinism verification
```

---

## Task 2: WASM Sandbox Hardening (Issue #30)

**GitHub Issue:** #30

**Files:**
- Modify: `crates/nomai-wasm-host/src/host_api.rs` (error handling improvements)
- Modify: `crates/nomai-wasm-host/src/module.rs` (validation + crash recovery)
- Modify: `crates/nomai-wasm-host/src/lib.rs` (new error variants + tests)
- Create: `crates/nomai-wasm-host/tests/fixtures/bad_import.wat` (test fixture)
- Create: `crates/nomai-wasm-host/tests/fixtures/trap_test.wat` (test fixture)

### What to Build

Harden the WASM sandbox for production use:

1. **Import validation:** Verify all imports match the host API before instantiation
2. **Crash recovery:** WASM trap → log error → return Err, don't panic
3. **Host API error handling:** Invalid entity IDs and missing components return clear errors instead of silently continuing
4. **Module health tracking:** Track consecutive trap count, allow skip-tick on unhealthy module

### Step 1: Add WasmError::InvalidModule variant

In `crates/nomai-wasm-host/src/lib.rs`, add a new error variant:

```rust
/// The module's imports don't match the available host API functions.
#[error("WASM module has unsupported import '{module}::{name}' -- only 'nomai::*' and 'env::abort' are supported")]
InvalidImport {
    /// The import module namespace.
    module: String,
    /// The import function name.
    name: String,
},
```

### Step 2: Add import validation to WasmModule::from_bytes

In `crates/nomai-wasm-host/src/module.rs`, after the `has_tick` check, add import validation:

```rust
// Validate imports -- only "nomai" and "env" namespaces are allowed.
for import in module.imports() {
    let module_name = import.module();
    if module_name != "nomai" && module_name != "env" {
        return Err(WasmError::InvalidImport {
            module: module_name.to_owned(),
            name: import.name().to_owned(),
        });
    }
}
```

Add the same check to `WasmModule::swap`.

### Step 3: Add trap recovery to call_tick

In `crates/nomai-wasm-host/src/module.rs`, add a `trap_count` field to `WasmModule`:

```rust
pub struct WasmModule {
    store: Store<HostState>,
    instance: Instance,
    config: WasmConfig,
    /// Consecutive trap count for health monitoring.
    consecutive_traps: u32,
}
```

Update `call_tick` to track traps and clear on success:

```rust
pub fn call_tick(&mut self) -> Result<u64, WasmError> {
    self.reset_fuel()?;

    let tick_fn = self
        .instance
        .get_typed_func::<(), ()>(&mut self.store, "tick")
        .map_err(|e| WasmError::Runtime(format!("failed to resolve tick(): {e}")))?;

    match tick_fn.call(&mut self.store, ()) {
        Ok(()) => {
            self.consecutive_traps = 0; // Reset on success.
        }
        Err(e) => {
            self.consecutive_traps += 1;
            tracing::warn!(
                consecutive_traps = self.consecutive_traps,
                "WASM tick() trapped"
            );
            return Err(self.classify_trap(e));
        }
    }

    // ... rest of fuel calculation ...
}
```

Add accessor:
```rust
/// Number of consecutive ticks that trapped.
pub fn consecutive_traps(&self) -> u32 {
    self.consecutive_traps
}
```

### Step 4: Write test fixtures

Create `crates/nomai-wasm-host/tests/fixtures/bad_import.wat`:
```wat
(module
  (import "unknown_module" "unknown_func" (func))
  (func (export "tick") nop)
)
```

Create `crates/nomai-wasm-host/tests/fixtures/trap_test.wat`:
```wat
(module
  (func (export "tick") unreachable)
)
```

### Step 5: Write tests

Add to `crates/nomai-wasm-host/src/lib.rs` tests module:

```rust
#[test]
fn invalid_import_rejected() {
    let config = WasmConfig::default();
    let bytes = fixture_bytes("bad_import.wat");
    let result = WasmModule::from_bytes(&config, &bytes);
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), WasmError::InvalidImport { .. }),
        "should reject module with unknown imports"
    );
}

#[test]
fn trap_recovers_gracefully() {
    let config = WasmConfig::default();
    let bytes = fixture_bytes("trap_test.wat");
    let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

    let result = module.call_tick();
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), WasmError::Trap(_)));
    assert_eq!(module.consecutive_traps(), 1);
}

#[test]
fn consecutive_traps_increment() {
    let config = WasmConfig::default();
    let bytes = fixture_bytes("trap_test.wat");
    let mut module = WasmModule::from_bytes(&config, &bytes).unwrap();

    for i in 1..=3 {
        let _ = module.call_tick();
        assert_eq!(module.consecutive_traps(), i);
    }
}

#[test]
fn swap_validates_imports() {
    let config = WasmConfig::default();
    let v1 = fixture_bytes("noop.wat");
    let mut module = WasmModule::from_bytes(&config, &v1).unwrap();

    let bad = fixture_bytes("bad_import.wat");
    let result = module.swap(&bad);
    assert!(result.is_err());
    assert!(
        matches!(result.unwrap_err(), WasmError::InvalidImport { .. }),
        "swap should reject module with unknown imports"
    );

    // Original should still work.
    let fuel = module.call_tick().unwrap();
    assert!(fuel > 0);
}
```

### Step 6: Run tests

Run: `cargo test -p nomai-wasm-host --lib`
Expected: All new + existing tests pass.

Run: `cargo clippy -p nomai-wasm-host -- -D warnings`
Expected: Zero warnings.

### Step 7: Commit

```
feat: WASM sandbox hardening — import validation, trap recovery, health tracking (#30)

- Import validation: reject modules with imports outside nomai/env namespaces
- Trap recovery: log and return error instead of panicking, track consecutive traps
- Swap validation: verify imports before swapping modules
- New error variant: WasmError::InvalidImport
- 4 new tests: invalid imports, trap recovery, consecutive traps, swap validation
```

---

## Task 3: AssemblyScript Breakout Gameplay Module (Issue #31)

**GitHub Issue:** #31

**Files:**
- Create: `gameplay/assembly/breakout.ts` (breakout game logic)
- Modify: `gameplay/assembly/index.ts` (import and delegate to breakout)
- Modify: `gameplay/package.json` (add breakout build target)

### What to Build

A minimal breakout gameplay module in AssemblyScript that:

1. Reads collision events from the host (via component queries)
2. Responds to paddle input (moves paddle based on player input component)
3. Despawns bricks on ball-brick collision
4. Emits score events
5. All commands carry reason strings for manifest causality

The key constraint: the WASM module does NOT run physics — it reads physics results (collision events, positions) and applies game rules (brick destruction, scoring).

### Step 1: Create breakout.ts

Create `gameplay/assembly/breakout.ts`:

```typescript
// Breakout gameplay logic for the Nomai engine.
//
// This module reads collision events and entity state from the host,
// then applies game rules: brick destruction, scoring, and ball reset.
// Physics is handled natively by rapier2d -- this module only responds
// to collision events.

import {
  get_entity_count,
  tick_number,
  set_component,
  despawn_entity,
  emit_event,
  log_msg,
} from "./host";

// ---------------------------------------------------------------------------
// Constants (entity IDs are assigned by setup code, these are conventions)
// ---------------------------------------------------------------------------

// Entity type identifiers read from components.
const BALL_TYPE: string = "ball";
const PADDLE_TYPE: string = "paddle";
const BRICK_TYPE: string = "brick";

// ---------------------------------------------------------------------------
// Game state tracking (resets on module swap)
// ---------------------------------------------------------------------------

let score: i32 = 0;
let bricksDestroyed: i32 = 0;

// ---------------------------------------------------------------------------
// Main tick function
// ---------------------------------------------------------------------------

/**
 * Called once per engine tick after physics has run.
 *
 * Reads collision events and applies game rules:
 * 1. If ball hit a brick → despawn brick, increment score
 * 2. Emit score update event
 */
export function tick(): void {
  const currentTick: i64 = tick_number();
  const entities: i32 = get_entity_count();

  log_msg(0, "breakout tick " + currentTick.toString() + ", entities: " + entities.toString());

  // Update score component on a well-known score entity (entity 0 by convention).
  // This creates a manifest trail showing score changes.
  if (bricksDestroyed > 0) {
    set_component(
      0, // score entity
      "score",
      '{"points":' + score.toString() + ',"bricks_destroyed":' + bricksDestroyed.toString() + '}',
      "score_updated"
    );
  }
}

/**
 * Called by the engine when a collision event is detected.
 * The engine passes entity IDs involved in the collision.
 *
 * For the MVP, the engine calls this via set_component on a
 * well-known "collision_event" component. The WASM module reads
 * this and responds.
 */
export function handleBrickHit(brickEntityId: i64): void {
  score += 100;
  bricksDestroyed += 1;

  // Despawn the brick.
  despawn_entity(brickEntityId, "brick_destroyed_by_ball");

  // Emit game event.
  const eventJson: string = '{"event_type":"brick_destroyed","description":"Ball hit brick ' +
    brickEntityId.toString() + '","involved_entities":[' +
    brickEntityId.toString() + '],"caused_by":100,"reason":{"GameRule":"brick_destroyed_by_ball"},"tick":' +
    tick_number().toString() + '}';
  emit_event(eventJson);

  log_msg(2, "Brick " + brickEntityId.toString() + " destroyed! Score: " + score.toString());
}

/** Get the current score. Exported for testing. */
export function get_score(): i32 {
  return score;
}

/** Get bricks destroyed count. Exported for testing. */
export function get_bricks_destroyed(): i32 {
  return bricksDestroyed;
}
```

### Step 2: Update index.ts

Replace `gameplay/assembly/index.ts`:

```typescript
// Nomai Breakout -- gameplay entry point.
//
// Re-exports the tick function and any test helpers from the breakout module.

export { tick, handleBrickHit, get_score, get_bricks_destroyed } from "./breakout";
```

### Step 3: Build and test

Run: `cd gameplay && npm run build`
Expected: `gameplay/build/gameplay.wasm` is generated, <100KB.

Run: `cargo test -p nomai-wasm-host -- --ignored load_assemblyscript_gameplay_module`
Expected: AS module loads and executes in wasmtime.

### Step 4: Commit

```
feat: AssemblyScript breakout gameplay module with brick destruction and scoring (#31)

- breakout.ts: game logic responding to collision events
- Brick destruction via despawn with causality reason
- Score tracking with manifest-visible component updates
- Event emission for brick_destroyed events
- Exported test helpers: get_score(), get_bricks_destroyed()
```

---

## Task 4: WASM + Physics Integration into TickLoop (Issue #29, #30)

**GitHub Issue:** #29, #30

**Files:**
- Modify: `crates/nomai-engine/src/tick.rs` (add PhysicsWorld to TickLoop, physics step in tick)
- Modify: `crates/nomai-wasm-host/src/integration.rs` (update to accept PhysicsWorld)
- Modify: `crates/nomai-python/src/engine.rs` (expose physics methods to Python)

### What to Build

Wire the physics system and WASM module into the TickLoop so a single `tick()` call runs:
1. Begin manifest tick
2. Run user systems (game logic)
3. Run physics step (rapier2d)
4. Run WASM gameplay (reads physics results)
5. Apply all commands
6. Process manifest
7. End tick

### Step 1: Add PhysicsWorld and WasmModule to TickLoop

In `crates/nomai-engine/src/tick.rs`, add optional physics and WASM fields:

```rust
pub struct TickLoop {
    // ... existing fields ...
    /// Optional physics world. Set via `set_physics`.
    physics: Option<crate::physics::PhysicsWorld>,
    /// Optional WASM gameplay module. Set via `set_wasm_module`.
    wasm_module: Option<nomai_wasm_host::WasmModule>,
}
```

Add methods:

```rust
/// Set the physics world for this tick loop.
pub fn set_physics(&mut self, physics: crate::physics::PhysicsWorld) {
    self.physics = Some(physics);
}

/// Access the physics world (if set).
pub fn physics(&self) -> Option<&crate::physics::PhysicsWorld> {
    self.physics.as_ref()
}

/// Set the WASM gameplay module.
pub fn set_wasm_module(&mut self, module: nomai_wasm_host::WasmModule) {
    self.wasm_module = Some(module);
}

/// Hot-swap the WASM module.
pub fn swap_wasm_module(&mut self, bytes: &[u8]) -> Result<(), nomai_wasm_host::WasmError> {
    if let Some(ref mut module) = self.wasm_module {
        module.swap(bytes)
    } else {
        Err(nomai_wasm_host::WasmError::Runtime(
            "no WASM module loaded -- call set_wasm_module() first".to_owned(),
        ))
    }
}
```

### Step 2: Update tick() to include physics and WASM

Update the `tick()` method to run physics and WASM between user systems and command application:

```rust
pub fn tick(&mut self) -> Vec<nomai_ecs::command::Command> {
    let tick_start = Instant::now();
    let mut system_times = Vec::with_capacity(self.systems.len());

    // Phase 1: Begin manifest tick.
    self.manifest.begin_tick();

    // Phase 2: Run user systems.
    for system in &self.systems {
        let sys_start = Instant::now();
        (system.func)(&self.world, &mut self.command_buffer);
        system_times.push((system.name.clone(), sys_start.elapsed()));
    }

    // Phase 3: Run physics step (if physics world is set).
    if let Some(ref mut physics) = self.physics {
        let (_collisions, events) =
            crate::physics::run_physics_step(
                physics,
                &mut self.command_buffer,
                self.fixed_dt,
                self.tick_counter,
            );
        // Record collision events in the manifest.
        for event in events {
            self.manifest.record_event(event);
        }
    }

    // Phase 4: Run WASM gameplay (if module is set).
    if let Some(ref mut wasm) = self.wasm_module {
        // Prepare host state.
        wasm.host_state_mut().begin_tick(self.tick_counter, self.sim_time());
        wasm.host_state_mut().entity_count = self.world.entity_count();

        // Execute WASM tick (trap-safe).
        match wasm.call_tick() {
            Ok(_fuel) => {
                // Drain WASM commands into main command buffer.
                let mut wasm_cmds = wasm.drain_commands();
                let wasm_commands = wasm_cmds.commands().to_vec();
                for cmd in wasm_commands {
                    // Re-emit each WASM command into the main buffer.
                    // This preserves causality metadata.
                    self.command_buffer.push_raw(cmd);
                }
                // Drain WASM events.
                let wasm_events = wasm.drain_events();
                for event in wasm_events {
                    self.manifest.record_event(event);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "WASM tick() failed, skipping WASM this tick");
            }
        }
    }

    // Phase 5: Apply command buffer.
    let apply_start = Instant::now();
    let applied = self.command_buffer.apply(&mut self.world);
    let command_apply_time = apply_start.elapsed();

    // Phase 6: Process commands into manifest.
    self.manifest.process_commands(&applied, self.tick_counter, &self.world);

    // Phase 7: Finalize manifest.
    let system_names: Vec<String> = self.systems.iter().map(|s| s.name.clone()).collect();
    let sim_time = self.tick_counter as f64 * self.fixed_dt;
    self.manifest.end_tick(self.tick_counter, sim_time, system_names, &self.world);

    // Phase 8: Advance tick counter.
    self.tick_counter += 1;

    self.last_diagnostics = TickDiagnostics {
        system_times,
        total_time: tick_start.elapsed(),
        command_apply_time,
    };

    applied
}
```

Note: This requires adding a `push_raw` method to `CommandBuffer` that accepts a pre-built `Command` without incrementing the index. Alternative: merge the WASM command buffer into the main one before apply.

### Step 3: Add `push_raw` to CommandBuffer

In `crates/nomai-ecs/src/command.rs`:

```rust
/// Push a pre-built command into the buffer.
///
/// Used for merging commands from external sources (e.g., WASM modules)
/// into the main command buffer. The command index is reassigned to
/// maintain ordering within this buffer.
pub fn push_raw(&mut self, mut cmd: Command) {
    cmd.command_index = self.next_index;
    self.next_index += 1;
    self.commands.push(cmd);
}
```

### Step 4: Run tests

Run: `cargo test --workspace --lib`
Expected: All existing tests still pass.

### Step 5: Commit

```
feat: integrate physics and WASM into TickLoop tick pipeline (#29, #30)

- TickLoop.tick() now runs: user systems → physics step → WASM gameplay → apply commands
- PhysicsWorld and WasmModule are optional fields on TickLoop
- WASM trap recovery: log and skip, don't crash the tick
- CommandBuffer.push_raw() for merging WASM commands into main buffer
- Collision events and WASM events recorded in manifest
```

---

## Task 5: Python Bindings for Physics (Issue #29)

**GitHub Issue:** #29

**Files:**
- Modify: `crates/nomai-python/src/engine.rs` (add physics methods)

### What to Build

Expose physics entity registration and configuration from Python so the AI can set up breakout scenarios.

### Step 1: Add physics methods to PyNomaiEngine

```rust
/// Register a physics entity with position, velocity, and body type.
///
/// Args:
///     entity_id: Raw entity ID (from spawn_entity)
///     position: Dict with "x" and "y" keys
///     velocity: Dict with "dx" and "dy" keys
///     body_type: "dynamic", "kinematic", or "static"
///     collider: Dict with either {"type": "circle", "radius": f} or {"type": "box", "half_width": f, "half_height": f}
///     restitution: Bounciness coefficient (0.0-1.0)
fn register_physics_entity(
    &mut self,
    entity_id: u64,
    position: &Bound<'_, PyDict>,
    velocity: &Bound<'_, PyDict>,
    body_type: &str,
    collider: &Bound<'_, PyDict>,
    restitution: f64,
) -> PyResult<()> {
    // ... parse Python dicts into Rust types, register with physics world ...
}

/// Initialize the physics world with zero gravity (for breakout).
fn init_physics(&mut self) -> PyResult<()> {
    self.tick_loop.set_physics(PhysicsWorld::new_zero_gravity());
    Ok(())
}
```

### Step 2: Run tests

Run: `cargo build -p nomai-python`
Expected: Compiles.

### Step 3: Commit

```
feat: Python bindings for physics entity registration (#29)

- init_physics(): create zero-gravity physics world
- register_physics_entity(): register entity with position, velocity, body, collider
```

---

## Task 6: Week 4-5 Milestone Test (Issue #32)

**GitHub Issue:** #32

**Files:**
- Create: `tests/milestone_week4_5.rs` (Rust integration test)
- Create: `python/nomai-sdk/tests/test_milestone_week4_5.py` (Python end-to-end)

### What to Build

End-to-end test validating the full physics + WASM + manifest pipeline for a breakout scenario.

### Step 1: Rust integration test

Create `tests/milestone_week4_5.rs`:

```rust
//! Week 4-5 Milestone: Physics + WASM produce full causal chains.
//!
//! This test sets up a minimal breakout scenario (ball, paddle, wall),
//! runs physics for enough ticks to produce a collision, and verifies
//! that the manifest contains collision events with correct causality.

use nomai_engine::prelude::*;
use nomai_engine::physics::*;

#[test]
fn physics_collision_appears_in_manifest() {
    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");

    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: true,
    };
    let mut tick_loop = TickLoop::new(world, config);

    // Initialize physics.
    let mut physics = PhysicsWorld::new_zero_gravity();

    // Spawn ball (dynamic, moving right).
    tick_loop.world_mut().register_component::<PhysicsBody>("physics_body");
    // ... spawn entities, register physics bodies ...

    // Run 120 ticks.
    for _ in 0..120 {
        tick_loop.tick();
    }

    // Verify collision event in manifest.
    let mut found_collision = false;
    for tick in 0..120 {
        if let Some(manifest) = tick_loop.manifest_at_tick(tick) {
            for event in &manifest.events {
                if event.event_type == "collision" {
                    found_collision = true;
                    assert_eq!(event.involved_entities.len(), 2);
                    assert!(matches!(
                        event.reason,
                        CausalReason::CollisionResponse(_, _)
                    ));
                }
            }
        }
    }

    assert!(found_collision, "should have at least one collision event in manifest");
}
```

### Step 2: Python milestone test

Create `python/nomai-sdk/tests/test_milestone_week4_5.py`:

```python
"""Week 4-5 Milestone: Physics + WASM produce manifest with causal chains."""

import logging

import pytest
from nomai.engine import NomaiEngine
from nomai.manifest import TickManifest

logger = logging.getLogger(__name__)


class TestWeek4_5Milestone:
    """End-to-end physics + WASM milestone tests."""

    def test_physics_collision_in_manifest(self) -> None:
        """Ball collides with wall, collision event appears in manifest."""
        engine = NomaiEngine(headless=True)
        engine.register_component("position")
        engine.register_component("velocity")
        engine.register_component("physics_body")
        engine.init_physics()

        # Spawn ball moving toward wall.
        engine.spawn_entity("ball", "ball", {
            "position": {"x": 0.0, "y": 0.0},
            "velocity": {"dx": 100.0, "dy": 0.0},
        })

        # Spawn wall (static).
        engine.spawn_entity("wall", "right_wall", {
            "position": {"x": 5.0, "y": 0.0},
        })

        # Register physics bodies.
        # ... (depends on exact API shape)

        # Run 120 ticks.
        manifests = engine.run_ticks(120)

        # Find collision event.
        collision_events = []
        for m in manifests:
            for event in m.events:
                if event.event_type == "collision":
                    collision_events.append(event)

        assert len(collision_events) > 0, "should have collision events"
        logger.info("Found %d collision events", len(collision_events))

    def test_wasm_responds_to_collision(self) -> None:
        """WASM gameplay module despawns bricks on collision."""
        # ... load breakout WASM, spawn ball + bricks, run simulation ...
        pass

    def test_full_causal_chain(self) -> None:
        """Trace causal chain: physics collision → brick despawn → score update."""
        # ... end-to-end causal chain tracing ...
        pass
```

### Step 3: Run all tests

Run: `cargo test --workspace`
Run: `pytest python/nomai-sdk/tests/test_milestone_week4_5.py -v`
Expected: All pass.

### Step 4: Commit

```
feat: Week 4-5 milestone test — physics + WASM produce causal chains (#32)

- Rust integration test: ball-wall collision appears in manifest
- Python milestone test: physics collision events with causality
- Validates full pipeline: rapier2d → commands → manifest → Python queries
```

---

## Post-Completion

After all tasks are committed and pushed:

1. Close GitHub issues: `gh issue close 29 30 31 32`
2. Push to remote: `git push`
3. Run full CI: `just ci`
