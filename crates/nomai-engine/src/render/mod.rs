//! Debug 2D renderer for visual validation of engine state.
//!
//! This module is feature-gated behind `renderer`. When the feature is not
//! enabled, this module compiles to nothing. The renderer reads ECS world
//! state and draws colored rectangles/circles for entities, providing a
//! visual debug overlay that lets humans validate what the AI built.
//!
//! This is NOT a production renderer. It is a debug visualization for human
//! validation: colored rectangles representing entities by type, basic text
//! rendering (score, debug info), fixed 2D orthographic camera.

#[cfg(feature = "renderer")]
pub mod renderer;

#[cfg(feature = "renderer")]
pub use renderer::{Camera2D, DebugRenderer, DrawCommand};
