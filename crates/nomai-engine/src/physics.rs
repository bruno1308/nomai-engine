//! rapier2d physics integration with manifest causality.
//!
//! The [`PhysicsWorld`] manages a rapier2d simulation and synchronizes it with
//! the ECS [`World`](nomai_ecs::world::World). Each tick:
//!
//! 1. ECS entities with physics components are synced into rapier bodies.
//! 2. rapier steps the simulation with the engine's fixed dt.
//! 3. Collision events are collected and converted to [`GameEvent`]s.
//! 4. Updated positions and velocities are written back to the ECS via the
//!    command buffer with [`CausalReason::CollisionResponse`] or
//!    [`CausalReason::SystemInternal`].
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
use nomai_manifest::manifest::GameEvent;
use rapier2d::prelude::*;

/// System name used in manifest recording.
pub const PHYSICS_SYSTEM_NAME: &str = "physics";

// ---------------------------------------------------------------------------
// Physics component types (ECS-side)
// ---------------------------------------------------------------------------

/// 2D position component. Synced to/from rapier rigid body translation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Position {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

/// 2D velocity component. Synced to/from rapier rigid body linvel.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Velocity {
    /// Horizontal velocity.
    pub dx: f64,
    /// Vertical velocity.
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
    Box {
        /// Half-width along the x-axis.
        half_width: f64,
        /// Half-height along the y-axis.
        half_height: f64,
    },
    /// Circle with radius.
    Circle {
        /// Radius of the circle.
        radius: f64,
    },
}

/// Physics body descriptor. Attach to an entity to give it physics.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PhysicsBody {
    /// What type of body this is (Dynamic, Kinematic, or Static).
    pub body_type: PhysicsBodyType,
    /// The collider shape.
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
///
/// The physics world is stored outside the ECS because rapier owns its own
/// body and collider storage. Entity registration and unregistration map
/// between ECS [`EntityId`]s and rapier handles.
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
    /// Maps ECS EntityId (raw u64) -> rapier RigidBodyHandle.
    entity_to_body: HashMap<u64, RigidBodyHandle>,
    /// Maps rapier RigidBodyHandle -> ECS EntityId (raw u64).
    body_to_entity: HashMap<RigidBodyHandle, u64>,
    /// Maps rapier ColliderHandle -> ECS EntityId (raw u64) for collision lookup.
    collider_to_entity: HashMap<ColliderHandle, u64>,
}

impl PhysicsWorld {
    /// Create a new physics world with the given gravity vector.
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
    /// [`PhysicsBody`], [`Position`], and [`Velocity`] components.
    /// If the entity is already registered, this is a no-op.
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

        // Build rigid body based on type.
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

        // Build collider from shape descriptor.
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
    ///
    /// Removes the rigid body and all attached colliders from rapier.
    /// If the entity is not registered, this is a no-op.
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

    /// Sync ECS position/velocity into rapier for a registered entity.
    ///
    /// Call this before stepping to update rapier with the latest ECS state
    /// for kinematic bodies (e.g., paddle controlled by player input).
    /// Also useful for re-syncing dynamic bodies after external modifications.
    pub fn sync_to_rapier(
        &mut self,
        entity_id: EntityId,
        position: &Position,
        velocity: &Velocity,
    ) {
        let raw_id = entity_id.to_raw();
        if let Some(&body_handle) = self.entity_to_body.get(&raw_id) {
            if let Some(rb) = self.rigid_body_set.get_mut(body_handle) {
                rb.set_translation(vector![position.x as Real, position.y as Real], true);
                rb.set_linvel(vector![velocity.dx as Real, velocity.dy as Real], true);
            }
        }
    }

    /// Step the physics simulation by the given dt.
    ///
    /// Returns a list of collision pairs that started during the step.
    /// Uses crossbeam channels internally to collect rapier events.
    pub fn step(&mut self, dt: f64) -> Vec<CollisionPair> {
        self.integration_params.dt = dt as Real;

        // Create crossbeam channels for event collection.
        let (collision_send, collision_recv) =
            rapier2d::crossbeam::channel::unbounded::<CollisionEvent>();
        let (force_send, _force_recv) =
            rapier2d::crossbeam::channel::unbounded::<ContactForceEvent>();
        let event_handler = ChannelEventCollector::new(collision_send, force_send);

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
            &(),  // physics hooks
            &event_handler,
        );

        // Collect collision-started events.
        let mut collisions = Vec::new();
        while let Ok(event) = collision_recv.try_recv() {
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

        // Sort collision pairs by (min, max) entity ID for deterministic ordering.
        // Rapier's channel delivery order may vary across runs; sorting ensures
        // identical collision sequences given the same simulation state.
        collisions.sort_by_key(|c| {
            let a = c.entity_a.to_raw();
            let b = c.entity_b.to_raw();
            (a.min(b), a.max(b))
        });

        collisions
    }

    /// Read updated positions and velocities from rapier after a step.
    ///
    /// Returns a list of `(EntityId, Position, Velocity)` for all dynamic bodies.
    /// Static and kinematic bodies are not returned because they are ECS-driven.
    ///
    /// The results are sorted by raw entity ID to ensure deterministic ordering
    /// regardless of rapier's internal iteration order.
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
        // Sort by raw entity ID for deterministic output ordering.
        results.sort_by_key(|(eid, _, _)| eid.to_raw());
        results
    }

    /// Check if an entity is registered in the physics world.
    pub fn has_entity(&self, entity_id: EntityId) -> bool {
        self.entity_to_body.contains_key(&entity_id.to_raw())
    }

    /// Number of physics bodies currently registered.
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
                self.entity_a, self.entity_b,
            ),
            involved_entities: vec![self.entity_a, self.entity_b],
            caused_by: SystemId::PHYSICS,
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
/// It requires the [`PhysicsWorld`] to be stored externally (not in the ECS)
/// since rapier owns its own body storage.
///
/// Returns collision pairs and corresponding game events for upstream
/// consumers (manifest pipeline).
pub fn run_physics_step(
    physics: &mut PhysicsWorld,
    commands: &mut CommandBuffer,
    dt: f64,
    tick: u64,
) -> (Vec<CollisionPair>, Vec<GameEvent>) {
    // Step the simulation.
    let collisions = physics.step(dt);

    // Convert collisions to game events.
    let events: Vec<GameEvent> = collisions.iter().map(|c| c.to_game_event(tick)).collect();

    // Read back updated positions and velocities from rapier for dynamic bodies.
    let results = physics.read_results();
    for (entity_id, pos, vel) in &results {
        // Emit position update command.
        commands.set_component(
            *entity_id,
            "position",
            serde_json::json!({"x": pos.x, "y": pos.y}),
            SystemId::PHYSICS,
            CausalReason::SystemInternal("physics_step".to_owned()),
        );
        // Emit velocity update command.
        commands.set_component(
            *entity_id,
            "velocity",
            serde_json::json!({"dx": vel.dx, "dy": vel.dy}),
            SystemId::PHYSICS,
            CausalReason::SystemInternal("physics_step".to_owned()),
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
    fn physics_world_creates_with_custom_gravity() {
        let pw = PhysicsWorld::new(0.0, -9.81);
        assert_eq!(pw.body_count(), 0);
        // Gravity vector is stored internally -- we verify it indirectly
        // through body movement in the gravity test below.
    }

    #[test]
    fn register_and_check_entity() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
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
    fn register_entity_is_idempotent() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
        let pos = Position { x: 0.0, y: 0.0 };
        let vel = Velocity { dx: 1.0, dy: 0.0 };
        let body = PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 0.5 },
            restitution: 1.0,
            is_sensor: false,
        };

        pw.register_entity(eid, &pos, &vel, &body);
        pw.register_entity(eid, &pos, &vel, &body); // second call is no-op
        assert_eq!(pw.body_count(), 1);
    }

    #[test]
    fn unregister_entity_removes_from_physics() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
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
    fn unregister_nonexistent_entity_is_noop() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(42, 0);
        pw.unregister_entity(eid); // should not panic
        assert_eq!(pw.body_count(), 0);
    }

    #[test]
    fn dynamic_body_moves_after_step() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
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
        assert!(
            new_pos.x > 0.0,
            "ball should move right, got x={}",
            new_pos.x
        );
    }

    #[test]
    fn static_body_does_not_appear_in_results() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
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

        let _collisions = pw.step(1.0 / 60.0);
        let results = pw.read_results();
        assert!(
            results.is_empty(),
            "static bodies should not appear in results"
        );
    }

    #[test]
    fn kinematic_body_does_not_appear_in_results() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
        pw.register_entity(
            eid,
            &Position { x: 0.0, y: 0.0 },
            &Velocity { dx: 5.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Kinematic,
                collider: ColliderShape::Box {
                    half_width: 1.0,
                    half_height: 0.2,
                },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        let _collisions = pw.step(1.0 / 60.0);
        let results = pw.read_results();
        assert!(
            results.is_empty(),
            "kinematic bodies should not appear in results"
        );
    }

    #[test]
    fn sync_to_rapier_updates_body() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
        pw.register_entity(
            eid,
            &Position { x: 0.0, y: 0.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        // Sync new position/velocity.
        pw.sync_to_rapier(
            eid,
            &Position { x: 100.0, y: 200.0 },
            &Velocity { dx: 50.0, dy: 0.0 },
        );

        // Step and read results -- the body should be near the synced position.
        let _collisions = pw.step(1.0 / 60.0);
        let results = pw.read_results();
        assert_eq!(results.len(), 1);
        let (_, pos, vel) = &results[0];
        // Position should be approximately 100 + 50/60 ~= 100.83
        assert!(
            pos.x > 99.0,
            "position should be near synced value, got x={}",
            pos.x
        );
        assert!(
            (vel.dx - 50.0).abs() < 1.0,
            "velocity should be near synced value, got dx={}",
            vel.dx
        );
    }

    #[test]
    fn box_collider_registration() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
        pw.register_entity(
            eid,
            &Position { x: 5.0, y: 5.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 2.0,
                    half_height: 0.5,
                },
                restitution: 0.5,
                is_sensor: false,
            },
        );
        assert!(pw.has_entity(eid));
        assert_eq!(pw.body_count(), 1);
    }

    #[test]
    fn sensor_body_registration() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
        pw.register_entity(
            eid,
            &Position { x: 0.0, y: 0.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 10.0,
                    half_height: 0.1,
                },
                restitution: 0.0,
                is_sensor: true,
            },
        );
        assert!(pw.has_entity(eid));
    }

    #[test]
    fn two_bodies_collide_produces_event() {
        let mut pw = PhysicsWorld::new_zero_gravity();

        // Ball moving right.
        let ball = EntityId::new(0, 0);
        pw.register_entity(
            ball,
            &Position { x: 0.0, y: 0.0 },
            &Velocity {
                dx: 100.0,
                dy: 0.0,
            },
            &PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        // Wall on the right (static).
        let wall = EntityId::new(1, 0);
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
            ids.contains(&ball.to_raw()) && ids.contains(&wall.to_raw()),
            "collision should involve ball and wall, got {:?}",
            ids
        );
    }

    #[test]
    fn collision_pair_to_game_event() {
        let pair = CollisionPair {
            entity_a: EntityId::new(0, 0),
            entity_b: EntityId::new(1, 0),
        };
        let event = pair.to_game_event(5);
        assert_eq!(event.event_type, "collision");
        assert_eq!(event.involved_entities.len(), 2);
        assert_eq!(event.tick, 5);
        assert_eq!(event.caused_by, SystemId::PHYSICS);
        assert!(matches!(
            event.reason,
            CausalReason::CollisionResponse(_, _)
        ));
    }

    #[test]
    fn run_physics_step_emits_commands_for_dynamic_body() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
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
    fn run_physics_step_no_commands_for_static_body() {
        let mut pw = PhysicsWorld::new_zero_gravity();
        let eid = EntityId::new(0, 0);
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
    fn gravity_affects_dynamic_body() {
        // Use downward gravity.
        let mut pw = PhysicsWorld::new(0.0, -9.81);
        let eid = EntityId::new(0, 0);
        pw.register_entity(
            eid,
            &Position { x: 0.0, y: 10.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 0.0,
                is_sensor: false,
            },
        );

        // Step several times.
        for _ in 0..60 {
            pw.step(1.0 / 60.0);
        }
        let results = pw.read_results();
        assert_eq!(results.len(), 1);
        let (_, pos, vel) = &results[0];
        // Body should have fallen (y decreased).
        assert!(
            pos.y < 10.0,
            "body should fall under gravity, got y={}",
            pos.y
        );
        // Velocity should be negative (falling).
        assert!(
            vel.dy < 0.0,
            "velocity should be downward, got dy={}",
            vel.dy
        );
    }

    #[test]
    fn determinism_two_identical_runs() {
        fn run_simulation() -> Vec<(f64, f64)> {
            let mut pw = PhysicsWorld::new_zero_gravity();
            pw.register_entity(
                EntityId::new(0, 0),
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
        assert_eq!(
            run1, run2,
            "two identical runs should produce identical positions"
        );
    }

    #[test]
    fn determinism_with_collision() {
        fn run_collision_sim() -> Vec<(f64, f64, usize)> {
            let mut pw = PhysicsWorld::new_zero_gravity();

            // Ball moving right.
            pw.register_entity(
                EntityId::new(0, 0),
                &Position { x: 0.0, y: 0.0 },
                &Velocity {
                    dx: 50.0,
                    dy: 0.0,
                },
                &PhysicsBody {
                    body_type: PhysicsBodyType::Dynamic,
                    collider: ColliderShape::Circle { radius: 0.5 },
                    restitution: 1.0,
                    is_sensor: false,
                },
            );

            // Static wall.
            pw.register_entity(
                EntityId::new(1, 0),
                &Position { x: 5.0, y: 0.0 },
                &Velocity { dx: 0.0, dy: 0.0 },
                &PhysicsBody {
                    body_type: PhysicsBodyType::Static,
                    collider: ColliderShape::Box {
                        half_width: 0.5,
                        half_height: 5.0,
                    },
                    restitution: 1.0,
                    is_sensor: false,
                },
            );

            let mut snapshots = Vec::new();
            for _ in 0..120 {
                let collisions = pw.step(1.0 / 60.0);
                let results = pw.read_results();
                if let Some((_, pos, _)) = results.first() {
                    snapshots.push((pos.x, pos.y, collisions.len()));
                }
            }
            snapshots
        }

        let run1 = run_collision_sim();
        let run2 = run_collision_sim();
        assert_eq!(
            run1, run2,
            "two identical collision runs should produce identical results"
        );
    }

    #[test]
    fn multiple_dynamic_bodies_all_tracked() {
        let mut pw = PhysicsWorld::new_zero_gravity();

        for i in 0..5u32 {
            pw.register_entity(
                EntityId::new(i, 0),
                &Position {
                    x: i as f64 * 10.0,
                    y: 0.0,
                },
                &Velocity { dx: 1.0, dy: 0.0 },
                &PhysicsBody {
                    body_type: PhysicsBodyType::Dynamic,
                    collider: ColliderShape::Circle { radius: 0.5 },
                    restitution: 1.0,
                    is_sensor: false,
                },
            );
        }

        assert_eq!(pw.body_count(), 5);
        pw.step(1.0 / 60.0);
        let results = pw.read_results();
        assert_eq!(results.len(), 5, "all 5 dynamic bodies should be tracked");
    }

    #[test]
    fn read_results_deterministic_ordering() {
        // Register bodies in non-sequential order to verify sorting.
        let mut pw = PhysicsWorld::new_zero_gravity();

        let ids = [
            EntityId::new(5, 0),
            EntityId::new(2, 0),
            EntityId::new(8, 0),
            EntityId::new(1, 0),
        ];

        for &eid in &ids {
            pw.register_entity(
                eid,
                &Position { x: 0.0, y: 0.0 },
                &Velocity { dx: 1.0, dy: 0.0 },
                &PhysicsBody {
                    body_type: PhysicsBodyType::Dynamic,
                    collider: ColliderShape::Circle { radius: 0.5 },
                    restitution: 1.0,
                    is_sensor: false,
                },
            );
        }

        pw.step(1.0 / 60.0);
        let results = pw.read_results();

        // Verify results are sorted by raw entity ID.
        for i in 1..results.len() {
            assert!(
                results[i - 1].0.to_raw() < results[i].0.to_raw(),
                "results should be sorted by entity ID"
            );
        }
    }

    #[test]
    fn run_physics_step_collision_produces_events() {
        let mut pw = PhysicsWorld::new_zero_gravity();

        // Ball moving right toward wall.
        pw.register_entity(
            EntityId::new(0, 0),
            &Position { x: 0.0, y: 0.0 },
            &Velocity {
                dx: 100.0,
                dy: 0.0,
            },
            &PhysicsBody {
                body_type: PhysicsBodyType::Dynamic,
                collider: ColliderShape::Circle { radius: 0.5 },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        pw.register_entity(
            EntityId::new(1, 0),
            &Position { x: 2.0, y: 0.0 },
            &Velocity { dx: 0.0, dy: 0.0 },
            &PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 0.5,
                    half_height: 5.0,
                },
                restitution: 1.0,
                is_sensor: false,
            },
        );

        let mut all_events = Vec::new();
        for tick in 0..120u64 {
            let mut cmds = CommandBuffer::new();
            let (_collisions, events) = run_physics_step(&mut pw, &mut cmds, 1.0 / 60.0, tick);
            all_events.extend(events);
        }

        assert!(
            !all_events.is_empty(),
            "collision should produce game events"
        );
        assert_eq!(all_events[0].event_type, "collision");
        assert!(matches!(
            all_events[0].reason,
            CausalReason::CollisionResponse(_, _)
        ));
    }

    #[test]
    fn game_event_fields_correct() {
        let pair = CollisionPair {
            entity_a: EntityId::new(10, 2),
            entity_b: EntityId::new(20, 3),
        };
        let event = pair.to_game_event(42);

        assert_eq!(event.event_type, "collision");
        assert!(event.description.contains("10v2"));
        assert!(event.description.contains("20v3"));
        assert_eq!(event.involved_entities.len(), 2);
        assert_eq!(event.involved_entities[0], EntityId::new(10, 2));
        assert_eq!(event.involved_entities[1], EntityId::new(20, 3));
        assert_eq!(event.caused_by, SystemId::PHYSICS);
        assert_eq!(
            event.reason,
            CausalReason::CollisionResponse(EntityId::new(10, 2), EntityId::new(20, 3))
        );
        assert_eq!(event.tick, 42);
    }
}
