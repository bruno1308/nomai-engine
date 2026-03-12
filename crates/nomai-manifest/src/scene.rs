//! Scene snapshot — a full-state text representation of the game scene.
//!
//! A [`SceneSnapshot`] captures every visible entity's spatial data,
//! identity, and components at a single point in time. This is the
//! text equivalent of a rendered frame.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// A single entity's state in the scene snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneEntity {
    pub entity_id: u64,
    pub entity_type: String,
    pub role: String,
    pub tier: String,
    pub position: Option<[f64; 2]>,
    pub size: Option<[f64; 2]>,
    pub velocity: Option<[f64; 2]>,
    pub visible: bool,
    pub z_index: f64,
    pub components: HashMap<String, serde_json::Value>,
}

/// Axis-aligned bounding box of the entire scene.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// A complete snapshot of the game scene at a single tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSnapshot {
    pub schema_version: u32,
    pub tick: u64,
    pub sim_time: f64,
    pub entities: Vec<SceneEntity>,
    pub bounds: SceneBounds,
    pub entity_count: usize,
}
