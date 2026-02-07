//! Windowed application runner for the debug renderer.
//!
//! Provides [`run_windowed`], which takes ownership of a [`TickLoop`] and
//! drives it inside a winit event loop with debug rendering. Each
//! `RedrawRequested` event runs one tick, extracts draw commands from the
//! ECS world, and renders a frame.
//!
//! This module is feature-gated behind `renderer`.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{WindowAttributes, WindowId};

use super::renderer::DebugRenderer;
use crate::tick::TickLoop;

/// Run the tick loop in a window with debug rendering.
///
/// Takes ownership of the tick loop and blocks until the window is closed.
/// Each frame:
///
/// 1. Runs one tick of the simulation.
/// 2. Extracts draw commands from the ECS world.
/// 3. Renders the frame via the debug renderer.
///
/// # Arguments
///
/// * `tick_loop` - The simulation tick loop to drive. Ownership is taken.
/// * `window_title` - Title for the OS window.
/// * `width` - Initial window width in physical pixels.
/// * `height` - Initial window height in physical pixels.
///
/// # Errors
///
/// Returns an error if the event loop cannot be created or if a fatal
/// rendering error occurs.
pub fn run_windowed(
    tick_loop: TickLoop,
    window_title: &str,
    width: u32,
    height: u32,
) -> Result<(), anyhow::Error> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = App {
        state: AppState::Pending {
            tick_loop,
            title: window_title.to_owned(),
            width,
            height,
        },
        init_failed: false,
    };

    event_loop.run_app(&mut app)?;

    if app.init_failed {
        return Err(anyhow::anyhow!(
            "failed to initialize windowed renderer (see logs for details)"
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal state machine
// ---------------------------------------------------------------------------

/// Internal state of the windowed app.
///
/// Winit 0.30 requires that window creation happens inside the
/// `ApplicationHandler::resumed` callback, so we use a two-phase state
/// machine: `Pending` (before window creation) and `Running` (window +
/// renderer are initialized).
enum AppState {
    /// Waiting for `resumed` to create the window and renderer.
    Pending {
        tick_loop: TickLoop,
        title: String,
        width: u32,
        height: u32,
    },
    /// Window and renderer are initialized; simulation is running.
    Running {
        tick_loop: TickLoop,
        renderer: DebugRenderer,
    },
    /// Temporary placeholder used during state transitions.
    Transitioning,
}

/// The winit application handler that drives the tick loop with rendering.
struct App {
    state: AppState,
    /// Set to `true` if initialization fails (window or renderer), so
    /// `run_windowed` can return an error after the event loop exits.
    init_failed: bool,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Only transition from Pending -> Running.
        let state = std::mem::replace(&mut self.state, AppState::Transitioning);
        match state {
            AppState::Pending {
                tick_loop,
                title,
                width,
                height,
            } => {
                let window_attrs = WindowAttributes::default()
                    .with_title(title)
                    .with_inner_size(winit::dpi::PhysicalSize::new(width, height));

                match event_loop.create_window(window_attrs) {
                    Ok(window) => {
                        let window = Arc::new(window);
                        match pollster::block_on(DebugRenderer::new(window.clone())) {
                            Ok(renderer) => {
                                tracing::info!(
                                    width,
                                    height,
                                    "debug renderer window created successfully"
                                );
                                // Kick off the first frame so the render loop starts
                                // even on backends that don't send an initial
                                // RedrawRequested event.
                                window.request_redraw();
                                self.state = AppState::Running {
                                    tick_loop,
                                    renderer,
                                };
                            }
                            Err(e) => {
                                tracing::error!(error = %e, "failed to initialize debug renderer -- exiting");
                                self.init_failed = true;
                                self.state = AppState::Pending {
                                    tick_loop,
                                    title: String::new(),
                                    width,
                                    height,
                                };
                                event_loop.exit();
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "failed to create window -- exiting");
                        self.init_failed = true;
                        self.state = AppState::Pending {
                            tick_loop,
                            title: String::new(),
                            width,
                            height,
                        };
                        event_loop.exit();
                    }
                }
            }
            AppState::Running {
                tick_loop,
                renderer,
            } => {
                // Already running; put state back.
                self.state = AppState::Running {
                    tick_loop,
                    renderer,
                };
            }
            AppState::Transitioning => {
                // Should not happen; no-op.
                tracing::warn!("resumed called during state transition");
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match &mut self.state {
            AppState::Running {
                tick_loop,
                renderer,
            } => match event {
                WindowEvent::CloseRequested => {
                    tracing::info!(
                        ticks = tick_loop.tick_count(),
                        "window close requested -- shutting down"
                    );
                    event_loop.exit();
                }
                WindowEvent::Resized(new_size) => {
                    tracing::debug!(
                        width = new_size.width,
                        height = new_size.height,
                        "window resized"
                    );
                    renderer.resize(new_size);
                }
                WindowEvent::RedrawRequested => {
                    // Phase 1: Run one tick of the simulation.
                    tick_loop.tick();

                    // Phase 2: Render the current world state.
                    match renderer.render_world(tick_loop.world()) {
                        Ok(()) => {}
                        Err(wgpu::SurfaceError::Lost) => {
                            // Reconfigure surface on loss.
                            let size = renderer.window().inner_size();
                            renderer.resize(size);
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            tracing::error!("GPU out of memory -- exiting");
                            event_loop.exit();
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "surface error during render");
                        }
                    }

                    // Request the next frame.
                    renderer.window().request_redraw();
                }
                _ => {}
            },
            _ => {
                // Not yet initialized; ignore window events.
            }
        }
    }
}
