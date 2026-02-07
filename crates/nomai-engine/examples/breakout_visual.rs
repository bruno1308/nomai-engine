//! Interactive breakout demo -- move paddle with arrow keys, ball destroys bricks.
//!
//! Run with:
//!   cargo run --example breakout_visual --features renderer -p nomai-engine
//!
//! Controls:
//!   Left/Right arrows or A/D -- move paddle
//!   Escape -- quit

use std::collections::HashSet;
use std::sync::Arc;

use nomai_engine::prelude::*;
use nomai_engine::render::DebugRenderer;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{WindowAttributes, WindowId};

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

enum RenderState {
    Pending,
    Running { renderer: DebugRenderer },
}

struct BreakoutApp {
    tick_loop: TickLoop,
    render_state: RenderState,
    paddle_id: EntityId,
    ball_id: EntityId,
    brick_ids: HashSet<u64>,
    left_pressed: bool,
    right_pressed: bool,
}

impl ApplicationHandler for BreakoutApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if matches!(self.render_state, RenderState::Running { .. }) {
            return;
        }

        let attrs = WindowAttributes::default()
            .with_title("Nomai Breakout -- arrows/A/D to move, ESC to quit")
            .with_inner_size(winit::dpi::PhysicalSize::new(800u32, 600));

        match event_loop.create_window(attrs) {
            Ok(window) => {
                let window = Arc::new(window);
                match pollster::block_on(DebugRenderer::new(window.clone())) {
                    Ok(renderer) => {
                        window.request_redraw();
                        self.render_state = RenderState::Running { renderer };
                    }
                    Err(e) => {
                        eprintln!("renderer init failed: {e}");
                        event_loop.exit();
                    }
                }
            }
            Err(e) => {
                eprintln!("window creation failed: {e}");
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let RenderState::Running { renderer } = &mut self.render_state else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(size) => renderer.resize(size),

            WindowEvent::KeyboardInput { event: key_ev, .. } => {
                let pressed = key_ev.state == ElementState::Pressed;
                match key_ev.physical_key {
                    PhysicalKey::Code(KeyCode::ArrowLeft) | PhysicalKey::Code(KeyCode::KeyA) => {
                        self.left_pressed = pressed;
                    }
                    PhysicalKey::Code(KeyCode::ArrowRight) | PhysicalKey::Code(KeyCode::KeyD) => {
                        self.right_pressed = pressed;
                    }
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
                    _ => {}
                }
            }

            WindowEvent::RedrawRequested => {
                // -- 1. Update paddle velocity from keyboard input --
                let paddle_speed = 400.0;
                let paddle_dx = match (self.left_pressed, self.right_pressed) {
                    (true, false) => -paddle_speed,
                    (false, true) => paddle_speed,
                    _ => 0.0,
                };

                // Sync paddle to rapier with new velocity.
                let paddle_pos = self
                    .tick_loop
                    .world()
                    .get_component::<Position>(self.paddle_id)
                    .cloned()
                    .unwrap_or(Position { x: 400.0, y: 550.0 });

                if let Some(physics) = self.tick_loop.physics_mut() {
                    physics.sync_to_rapier(
                        self.paddle_id,
                        &paddle_pos,
                        &Velocity {
                            dx: paddle_dx,
                            dy: 0.0,
                        },
                    );
                }

                // -- 2. Run one tick --
                self.tick_loop.tick();

                // -- 3. Check for ball-brick collisions and destroy bricks --
                let mut bricks_to_destroy: Vec<EntityId> = Vec::new();
                if let Some(manifest) = self.tick_loop.last_manifest() {
                    for event in &manifest.events {
                        if event.event_type != "collision" {
                            continue;
                        }
                        let involves_ball =
                            event.involved_entities.iter().any(|e| *e == self.ball_id);
                        if !involves_ball {
                            continue;
                        }
                        for eid in &event.involved_entities {
                            if self.brick_ids.contains(&eid.to_raw()) {
                                bricks_to_destroy.push(*eid);
                            }
                        }
                    }
                }

                // Despawn destroyed bricks.
                // Use deferred_unregister so rapier's solver can finish
                // resolving the ball's bounce impulse before the collider
                // is removed. The ECS entity is despawned immediately
                // (for rendering), but the rapier body persists until the
                // next physics step.
                for brick_eid in bricks_to_destroy {
                    self.brick_ids.remove(&brick_eid.to_raw());
                    if let Some(physics) = self.tick_loop.physics_mut() {
                        physics.deferred_unregister(brick_eid);
                    }
                    let _ = self.tick_loop.world_mut().despawn(brick_eid);
                }

                // -- 4. Render --
                match renderer.render_world(self.tick_loop.world()) {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost) => {
                        let size = renderer.window().inner_size();
                        renderer.resize(size);
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        eprintln!("GPU out of memory");
                        event_loop.exit();
                    }
                    Err(_) => {}
                }

                renderer.window().request_redraw();
            }

            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Scene setup
// ---------------------------------------------------------------------------

fn main() -> Result<(), anyhow::Error> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let mut world = World::new();
    world.register_component::<Position>("position");
    world.register_component::<Velocity>("velocity");
    world.register_component::<PhysicsBody>("physics_body");

    let mut physics = PhysicsWorld::new_zero_gravity();

    // Helper: spawn + register physics in one step.
    fn spawn_and_register(
        world: &mut World,
        physics: &mut PhysicsWorld,
        identity: EntityIdentity,
        pos: Position,
        vel: Velocity,
        body: PhysicsBody,
    ) -> EntityId {
        let mut bundle = ComponentBundle::new();
        bundle.add(world.registry(), pos.clone());
        bundle.add(world.registry(), vel.clone());
        bundle.add(world.registry(), body.clone());
        let id = world
            .spawn_semantic(identity, bundle)
            .expect("spawn entity");
        physics.register_entity(id, &pos, &vel, &body);
        id
    }

    // Paddle
    let paddle_id = spawn_and_register(
        &mut world,
        &mut physics,
        EntityIdentity {
            entity_type: "paddle".to_owned(),
            role: "paddle".to_owned(),
            spawned_by: SystemId(0),
            requirement_id: None,
        },
        Position { x: 400.0, y: 550.0 },
        Velocity { dx: 0.0, dy: 0.0 },
        PhysicsBody {
            body_type: PhysicsBodyType::Kinematic,
            collider: ColliderShape::Box {
                half_width: 50.0,
                half_height: 7.5,
            },
            restitution: 1.0,
            is_sensor: false,
        },
    );

    // Ball
    let ball_id = spawn_and_register(
        &mut world,
        &mut physics,
        EntityIdentity {
            entity_type: "ball".to_owned(),
            role: "ball".to_owned(),
            spawned_by: SystemId(0),
            requirement_id: None,
        },
        Position { x: 400.0, y: 500.0 },
        Velocity {
            dx: 180.0,
            dy: -250.0,
        },
        PhysicsBody {
            body_type: PhysicsBodyType::Dynamic,
            collider: ColliderShape::Circle { radius: 6.0 },
            restitution: 1.0,
            is_sensor: false,
        },
    );

    // Walls
    let wall_specs: &[(&str, f64, f64, f64, f64)] = &[
        ("wall_top", 400.0, 0.0, 400.0, 5.0),
        ("wall_bottom", 400.0, 600.0, 400.0, 5.0),
        ("wall_left", 0.0, 300.0, 5.0, 300.0),
        ("wall_right", 800.0, 300.0, 5.0, 300.0),
    ];
    for &(role, x, y, hw, hh) in wall_specs {
        spawn_and_register(
            &mut world,
            &mut physics,
            EntityIdentity {
                entity_type: "wall".to_owned(),
                role: role.to_owned(),
                spawned_by: SystemId(0),
                requirement_id: None,
            },
            Position { x, y },
            Velocity { dx: 0.0, dy: 0.0 },
            PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: hw,
                    half_height: hh,
                },
                restitution: 1.0,
                is_sensor: false,
            },
        );
    }

    // Bricks (6 rows x 10 columns)
    let mut brick_ids = HashSet::new();
    for row in 0..6u32 {
        for col in 0..10u32 {
            let x = 75.0 + col as f64 * 75.0;
            let y = 80.0 + row as f64 * 25.0;

            let mut bundle = ComponentBundle::new();
            let pos = Position { x, y };
            let vel = Velocity { dx: 0.0, dy: 0.0 };
            let body = PhysicsBody {
                body_type: PhysicsBodyType::Static,
                collider: ColliderShape::Box {
                    half_width: 30.0,
                    half_height: 8.0,
                },
                restitution: 1.0,
                is_sensor: false,
            };
            bundle.add(world.registry(), pos.clone());
            bundle.add(world.registry(), vel.clone());
            bundle.add(world.registry(), body.clone());
            let id = world
                .spawn_pooled(
                    PoolIdentity {
                        pool_type: "destructible".to_owned(),
                        variant: "brick".to_owned(),
                    },
                    bundle,
                )
                .expect("spawn brick");
            physics.register_entity(id, &pos, &vel, &body);
            brick_ids.insert(id.to_raw());
        }
    }

    println!(
        "Breakout initialized: {} entities, {} physics bodies",
        world.entity_count(),
        physics.body_count()
    );
    println!("Controls: Left/Right arrows or A/D to move paddle, ESC to quit");

    // Build tick loop
    let config = TickConfig {
        fixed_dt: 1.0 / 60.0,
        headless: false,
    };
    let mut tick_loop = TickLoop::new(world, config);
    tick_loop.set_physics(physics);

    // Run windowed
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = BreakoutApp {
        tick_loop,
        render_state: RenderState::Pending,
        paddle_id,
        ball_id,
        brick_ids,
        left_pressed: false,
        right_pressed: false,
    };

    event_loop.run_app(&mut app)?;
    Ok(())
}
