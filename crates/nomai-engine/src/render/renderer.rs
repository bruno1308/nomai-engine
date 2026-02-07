//! Game-agnostic 2D renderer using wgpu.
//!
//! Renders ANY entity that has spatial data (position + size) as colored
//! rectangles. Works with both native physics components (`Position` +
//! `PhysicsBody`) and dynamic JSON components (`"position"` + `"size"`).
//!
//! # Architecture
//!
//! The renderer does NOT own the event loop -- the tick loop drives it.
//! Each frame:
//!
//! 1. [`DebugRenderer::extract_draw_commands`] scans ALL entities for spatial
//!    data (native physics OR dynamic JSON components), maps identity to color.
//! 2. [`DebugRenderer::render`] builds a vertex buffer from draw commands
//!    and renders a frame.
//!
//! # Color Mapping
//!
//! Colors are assigned automatically based on entity identity:
//! - Each unique role/type name gets a consistent color from a palette
//! - Entities with a "type" component (e.g., `tile_type`) get per-type colors
//! - Entities without identity get a default magenta color

use std::collections::HashSet;
use std::sync::Arc;

use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::Identity;
use nomai_ecs::world::World;
use wgpu::util::DeviceExt;

use crate::physics::{ColliderShape, PhysicsBody, Position};

// ---------------------------------------------------------------------------
// Vertex
// ---------------------------------------------------------------------------

/// A single vertex with 2D position and RGBA color, sent to the GPU.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck_derive::Pod, bytemuck_derive::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

impl Vertex {
    /// Vertex buffer layout for the shader.
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

// ---------------------------------------------------------------------------
// Camera2D
// ---------------------------------------------------------------------------

/// A simple 2D orthographic camera.
///
/// The camera defines the visible area of the game world. The
/// [`orthographic_matrix`](Self::orthographic_matrix) method produces a
/// column-major 4x4 matrix that maps world coordinates to clip space
/// `[-1, 1]`.
#[derive(Debug, Clone)]
pub struct Camera2D {
    /// Width of the visible area in world units.
    pub width: f32,
    /// Height of the visible area in world units.
    pub height: f32,
    /// Camera center X in world units.
    pub x: f32,
    /// Camera center Y in world units.
    pub y: f32,
}

impl Camera2D {
    /// Produce a column-major 4x4 orthographic projection matrix.
    ///
    /// Maps world coordinates to clip space `[-1, 1]` for both axes.
    /// The camera is centered at `(x, y)` and spans `width` x `height`
    /// world units.
    pub fn orthographic_matrix(&self) -> [f32; 16] {
        let left = self.x - self.width / 2.0;
        let right = self.x + self.width / 2.0;
        let bottom = self.y - self.height / 2.0;
        let top = self.y + self.height / 2.0;

        // Column-major orthographic projection.
        // Maps [left, right] -> [-1, 1] on x
        // Maps [bottom, top] -> [-1, 1] on y
        // Z is unused (2D), set near=0.0, far=1.0.
        let sx = 2.0 / (right - left);
        let sy = 2.0 / (top - bottom);
        let tx = -(right + left) / (right - left);
        let ty = -(top + bottom) / (top - bottom);

        // Column-major layout:
        // col0     col1     col2     col3
        [
            sx, 0.0, 0.0, 0.0, // column 0
            0.0, sy, 0.0, 0.0, // column 1
            0.0, 0.0, 1.0, 0.0, // column 2
            tx, ty, 0.0, 1.0, // column 3
        ]
    }
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 600.0,
            x: 400.0,
            y: 300.0,
        }
    }
}

// ---------------------------------------------------------------------------
// DrawCommand
// ---------------------------------------------------------------------------

/// A drawable entity extracted from ECS state.
///
/// Represents a colored rectangle to be rendered. Position is in world
/// coordinates (center of the rectangle). Width and height are full extents.
#[derive(Debug, Clone)]
pub struct DrawCommand {
    /// Center X position in world coordinates.
    pub x: f32,
    /// Center Y position in world coordinates.
    pub y: f32,
    /// Full width of the rectangle.
    pub width: f32,
    /// Full height of the rectangle.
    pub height: f32,
    /// RGBA color (each channel 0.0..1.0).
    pub color: [f32; 4],
}

// ---------------------------------------------------------------------------
// Color palette -- game-agnostic, visually distinct colors
// ---------------------------------------------------------------------------

/// A palette of visually distinct, saturated colors for entity rendering.
/// Colors are assigned by hashing the entity's identity/type information.
const COLOR_PALETTE: &[[f32; 4]] = &[
    [0.267, 0.533, 1.0, 1.0],   // Blue
    [1.0, 0.2, 0.2, 1.0],       // Red
    [0.2, 1.0, 0.2, 1.0],       // Green
    [1.0, 1.0, 0.2, 1.0],       // Yellow
    [1.0, 0.6, 0.2, 1.0],       // Orange
    [0.6, 0.2, 1.0, 1.0],       // Purple
    [0.2, 1.0, 1.0, 1.0],       // Cyan
    [1.0, 0.4, 0.7, 1.0],       // Pink
    [1.0, 1.0, 1.0, 1.0],       // White
    [0.533, 0.533, 0.533, 1.0], // Gray
    [0.7, 0.5, 0.2, 1.0],       // Brown
    [0.5, 1.0, 0.5, 1.0],       // Light green
];

/// Default color for entities without any distinguishing identity.
const COLOR_DEFAULT: [f32; 4] = [0.8, 0.2, 0.8, 1.0];

// ---------------------------------------------------------------------------
// Entity color mapping -- game-agnostic
// ---------------------------------------------------------------------------

/// Simple string hash for deterministic palette selection.
fn hash_str(s: &str) -> usize {
    let mut h: usize = 5381;
    for b in s.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as usize);
    }
    h
}

/// Determine the color for an entity based on its identity and optional
/// type-distinguishing info from dynamic components.
///
/// The color is chosen by hashing the entity's identity role/type name
/// (and any `type_id` from a dynamic "type" component) into the palette.
/// This is fully game-agnostic: no hard-coded game entity names.
fn color_for_entity(
    identity: Option<&Identity>,
    type_component_value: Option<&serde_json::Value>,
) -> [f32; 4] {
    let Some(identity) = identity else {
        return COLOR_DEFAULT;
    };

    // Build a hash key from identity.
    let role = identity.role().unwrap_or("");
    let type_name = identity.type_name();

    // If there's a type-distinguishing component (like tile_type), use its
    // content to differentiate entities with the same role.
    if let Some(type_val) = type_component_value {
        // Try "type_id" field first (integer), then "name" field (string).
        let distinguisher = type_val
            .get("type_id")
            .and_then(|v| v.as_u64())
            .map(|id| id.to_string())
            .or_else(|| {
                type_val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_owned())
            });
        if let Some(d) = distinguisher {
            let key = format!("{role}:{type_name}:{d}");
            let idx = hash_str(&key) % COLOR_PALETTE.len();
            return COLOR_PALETTE[idx];
        }
    }

    // Hash the role + type_name for a base color.
    let key = format!("{role}:{type_name}");
    let idx = hash_str(&key) % COLOR_PALETTE.len();
    COLOR_PALETTE[idx]
}

// ---------------------------------------------------------------------------
// Max entities for vertex buffer sizing
// ---------------------------------------------------------------------------

/// Maximum number of entities we can render (determines vertex buffer size).
/// Each entity uses 6 vertices (two triangles for a quad).
const MAX_ENTITIES: usize = 2048;
const VERTICES_PER_QUAD: usize = 6;
const MAX_VERTICES: usize = MAX_ENTITIES * VERTICES_PER_QUAD;

// ---------------------------------------------------------------------------
// DebugRenderer
// ---------------------------------------------------------------------------

/// Debug 2D renderer using wgpu.
///
/// Reads ECS state and draws entities as colored rectangles. The renderer
/// does not own the event loop -- the tick loop drives it by calling
/// [`extract_draw_commands`](Self::extract_draw_commands) and then
/// [`render`](Self::render) each frame.
///
/// # GPU Initialization
///
/// Call [`DebugRenderer::new`] with an `Arc<winit::window::Window>`. This
/// performs async wgpu device/adapter selection, surface creation, and
/// pipeline setup. If no suitable GPU is available, the error is returned
/// and the engine can fall back to headless mode.
pub struct DebugRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    window: Arc<winit::window::Window>,
    /// The 2D orthographic camera.
    pub camera: Camera2D,
}

impl DebugRenderer {
    /// Initialize wgpu: window, surface, device, queue, pipeline.
    ///
    /// This is an async function because wgpu adapter/device selection is
    /// asynchronous. Call with `.await` or use `pollster::block_on`.
    ///
    /// # Errors
    ///
    /// Returns an error if no suitable GPU adapter or device is available.
    pub async fn new(window: Arc<winit::window::Window>) -> Result<Self, anyhow::Error> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        // Create wgpu instance.
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Create surface.
        let surface = instance.create_surface(window.clone())?;

        // Request adapter.
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| anyhow::anyhow!("no suitable GPU adapter found"))?;

        // Request device and queue.
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("nomai_debug_renderer"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
            )
            .await?;

        // Configure surface.
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Load shader.
        let shader_source = include_str!("shaders.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("debug_renderer_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // Camera uniform buffer.
        let camera = Camera2D::default();
        let camera_matrix = camera.orthographic_matrix();
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniform"),
            contents: bytemuck::cast_slice(&camera_matrix),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Camera bind group layout.
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera_bind_group_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Pipeline layout.
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("debug_renderer_pipeline_layout"),
            bind_group_layouts: &[&camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        // Render pipeline.
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("debug_renderer_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Pre-allocate vertex buffer for max entities.
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vertex_buffer"),
            size: (MAX_VERTICES * std::mem::size_of::<Vertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Ok(Self {
            surface,
            device,
            queue,
            config,
            render_pipeline,
            vertex_buffer,
            camera_buffer,
            camera_bind_group,
            window,
            camera,
        })
    }

    /// Extract draw commands from ECS world state.
    ///
    /// Scans ALL entities for spatial data, supporting two sources:
    ///
    /// 1. **Native physics**: entities with [`Position`] + [`PhysicsBody`]
    /// 2. **Dynamic JSON**: entities with `"position"` (`{x, y}`) +
    ///    `"size"` (`{w, h}`) dynamic components
    ///
    /// This makes the renderer game-agnostic: any game that sets position
    /// and size on its entities will have them rendered automatically.
    ///
    /// This is a pure function that does not require a GPU -- suitable for
    /// headless testing and verification.
    pub fn extract_draw_commands(world: &World) -> Vec<DrawCommand> {
        let mut commands = Vec::new();
        let mut rendered: HashSet<EntityId> = HashSet::new();

        // --- Phase 1: Native physics entities ---
        for (entity_id, (pos, body)) in world.query::<(&Position, &PhysicsBody)>() {
            let x = pos.x as f32;
            let y = pos.y as f32;

            let (width, height) = match &body.collider {
                ColliderShape::Box {
                    half_width,
                    half_height,
                } => (*half_width as f32 * 2.0, *half_height as f32 * 2.0),
                ColliderShape::Circle { radius } => {
                    let d = *radius as f32 * 2.0;
                    (d, d)
                }
            };

            let identity = world.get_component::<Identity>(entity_id);
            let type_comp = Self::find_type_component(world, entity_id);
            let color = color_for_entity(identity, type_comp.as_ref());

            commands.push(DrawCommand {
                x,
                y,
                width,
                height,
                color,
            });
            rendered.insert(entity_id);
        }

        // --- Phase 2: Dynamic JSON component entities ---
        // NOTE: Native physics components must NOT be registered under the names
        // "position" or "size", as Phase 2 uses those names to discover dynamic
        // JSON spatial data. Naming collisions would cause double-rendering.
        for entity_id in world.all_entity_ids() {
            if rendered.contains(&entity_id) {
                continue;
            }

            // Must have a "position" dynamic component with x and y fields.
            let pos_json = match world.get_component_by_name(entity_id, "position") {
                Some(v) => v,
                None => continue,
            };
            let x = match pos_json.get("x").and_then(|v| v.as_f64()) {
                Some(v) => v as f32,
                None => continue,
            };
            let y = match pos_json.get("y").and_then(|v| v.as_f64()) {
                Some(v) => v as f32,
                None => continue,
            };

            // Must have a "size" dynamic component with w and h fields.
            let size_json = match world.get_component_by_name(entity_id, "size") {
                Some(v) => v,
                None => continue,
            };
            let width = size_json.get("w").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            let height = size_json.get("h").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;

            let identity = world.get_component::<Identity>(entity_id);
            let type_comp = Self::find_type_component(world, entity_id);
            let color = color_for_entity(identity, type_comp.as_ref());

            commands.push(DrawCommand {
                x,
                y,
                width,
                height,
                color,
            });
        }

        // Warn once if entities exist but nothing is renderable.
        // Using a static flag to avoid spamming the log every frame.
        if commands.is_empty() && world.entity_count() > 0 {
            use std::sync::atomic::{AtomicBool, Ordering};
            static WARNED: AtomicBool = AtomicBool::new(false);
            if !WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    entity_count = world.entity_count(),
                    "renderer found 0 drawable entities out of {} total -- \
                     entities need either (Position + PhysicsBody) or \
                     (dynamic \"position\" + dynamic \"size\") components to render",
                    world.entity_count()
                );
            }
        }

        commands
    }

    /// Search for a "type" component on an entity (e.g., `tile_type`,
    /// `entity_type`). Returns its JSON value if found.
    ///
    /// Looks for any registered dynamic component whose name ends with
    /// `_type` or equals `type`.
    ///
    /// NOTE: This scans all registered component names per entity, making it
    /// O(N_components * N_entities) per frame. At MVP scale this is fine, but
    /// for larger registries consider caching the list of `_type`-suffixed
    /// component names once and reusing it.
    fn find_type_component(world: &World, entity_id: EntityId) -> Option<serde_json::Value> {
        for name in world.registry().registered_names() {
            if name.ends_with("_type") || name == "type" {
                if let Some(val) = world.get_component_by_name(entity_id, name) {
                    return Some(val);
                }
            }
        }
        None
    }

    /// Render a frame from draw commands.
    ///
    /// Builds a vertex buffer from the provided [`DrawCommand`]s, uploads
    /// the camera uniform, and issues a render pass. The frame is presented
    /// to the surface.
    ///
    /// # Errors
    ///
    /// Returns a [`wgpu::SurfaceError`] if the surface cannot provide an
    /// output texture (e.g., window minimized, surface lost).
    pub fn render(&mut self, commands: &[DrawCommand]) -> Result<(), wgpu::SurfaceError> {
        // Update camera uniform.
        let camera_matrix = self.camera.orthographic_matrix();
        self.queue
            .write_buffer(&self.camera_buffer, 0, bytemuck::cast_slice(&camera_matrix));

        // Build vertex data from draw commands.
        let mut vertices: Vec<Vertex> = Vec::with_capacity(commands.len() * VERTICES_PER_QUAD);
        for cmd in commands.iter().take(MAX_ENTITIES) {
            let half_w = cmd.width / 2.0;
            let half_h = cmd.height / 2.0;
            let (x, y, c) = (cmd.x, cmd.y, cmd.color);

            // Two triangles forming a quad (CCW winding).
            // Triangle 1: bottom-left, bottom-right, top-right
            vertices.push(Vertex {
                position: [x - half_w, y - half_h],
                color: c,
            });
            vertices.push(Vertex {
                position: [x + half_w, y - half_h],
                color: c,
            });
            vertices.push(Vertex {
                position: [x + half_w, y + half_h],
                color: c,
            });
            // Triangle 2: bottom-left, top-right, top-left
            vertices.push(Vertex {
                position: [x - half_w, y - half_h],
                color: c,
            });
            vertices.push(Vertex {
                position: [x + half_w, y + half_h],
                color: c,
            });
            vertices.push(Vertex {
                position: [x - half_w, y + half_h],
                color: c,
            });
        }

        // Upload vertices.
        if !vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }

        // Get surface texture.
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Build and submit render pass.
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("debug_renderer_encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("debug_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.05,
                            b: 0.1,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

            let vertex_count = vertices.len() as u32;
            if vertex_count > 0 {
                render_pass.draw(0..vertex_count, 0..1);
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    /// Extract draw commands from the world and render them in one call.
    ///
    /// This is a convenience method that combines
    /// [`extract_draw_commands`](Self::extract_draw_commands) and
    /// [`render`](Self::render) into a single step. Automatically fits
    /// the camera to the bounding box of all draw commands with padding.
    ///
    /// # Errors
    ///
    /// Returns a [`wgpu::SurfaceError`] if the surface cannot provide an
    /// output texture (e.g., window minimized, surface lost).
    pub fn render_world(&mut self, world: &World) -> Result<(), wgpu::SurfaceError> {
        let commands = Self::extract_draw_commands(world);
        self.auto_fit_camera(&commands);
        self.render(&commands)
    }

    /// Fit the camera to the bounding box of the draw commands with padding.
    ///
    /// Ensures all entities are visible regardless of their coordinate system.
    /// Adds 10% padding around the bounding box for visual breathing room.
    pub fn auto_fit_camera(&mut self, commands: &[DrawCommand]) {
        if commands.is_empty() {
            return;
        }

        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;

        for cmd in commands {
            let half_w = cmd.width / 2.0;
            let half_h = cmd.height / 2.0;
            min_x = min_x.min(cmd.x - half_w);
            min_y = min_y.min(cmd.y - half_h);
            max_x = max_x.max(cmd.x + half_w);
            max_y = max_y.max(cmd.y + half_h);
        }

        let content_w = max_x - min_x;
        let content_h = max_y - min_y;

        // Add 10% padding on each side.
        let padding_x = content_w * 0.1;
        let padding_y = content_h * 0.1;

        // Minimum dimensions to avoid degenerate cameras.
        let cam_w = (content_w + padding_x * 2.0).max(1.0);
        let cam_h = (content_h + padding_y * 2.0).max(1.0);

        // Maintain aspect ratio of the window.
        let window_aspect = self.config.width as f32 / self.config.height.max(1) as f32;
        let content_aspect = cam_w / cam_h;

        let (final_w, final_h) = if content_aspect > window_aspect {
            // Content is wider than window -- fit width, expand height.
            (cam_w, cam_w / window_aspect)
        } else {
            // Content is taller than window -- fit height, expand width.
            (cam_h * window_aspect, cam_h)
        };

        self.camera.x = (min_x + max_x) / 2.0;
        self.camera.y = (min_y + max_y) / 2.0;
        self.camera.width = final_w;
        self.camera.height = final_h;
    }

    /// Resize the surface when the window size changes.
    ///
    /// Must be called in response to window resize events. The new size
    /// must have non-zero width and height.
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);
        }
    }

    /// Get a reference to the window.
    pub fn window(&self) -> &winit::window::Window {
        &self.window
    }
}
