//! Scene extraction -- produces a [`SceneSnapshot`] from ECS world state.
//!
//! This mirrors the renderer's entity-scanning logic in
//! [`crate::render::renderer::DebugRenderer::extract_draw_commands`] but
//! outputs a [`SceneSnapshot`] instead of GPU draw commands. The snapshot
//! captures every entity's spatial data, identity, visibility, and dynamic
//! components in a serialisable, deterministic format suitable for AI
//! verification and text-based scene diffing.

use std::collections::{HashMap, HashSet};

use nomai_ecs::entity::EntityId;
use nomai_ecs::identity::Identity;
use nomai_ecs::world::World;
use nomai_manifest::{SceneBounds, SceneEntity, SceneSnapshot};

use crate::physics::{ColliderShape, PhysicsBody, Position, Velocity};

// ---------------------------------------------------------------------------
// Identity extraction helper
// ---------------------------------------------------------------------------

/// Extract `(type_name, role, tier)` from an optional [`Identity`].
///
/// For [`Identity::Semantic`] entities the role comes from
/// [`EntityIdentity::role`]; for [`Identity::Pooled`] entities the role
/// equivalent is [`PoolIdentity::variant`].
fn extract_identity(identity: Option<&Identity>) -> (String, String, String) {
    match identity {
        Some(id) => {
            let tier = format!("{:?}", id.tier());
            let type_name = id.type_name().to_owned();
            let role = match id {
                Identity::Semantic(eid) => eid.role.clone(),
                Identity::Pooled(pid) => pid.variant.clone(),
            };
            (type_name, role, tier)
        }
        None => ("unknown".into(), "unknown".into(), "Unknown".into()),
    }
}

// ---------------------------------------------------------------------------
// Visibility / z_index helpers (mirror renderer helpers)
// ---------------------------------------------------------------------------

/// Check if an entity is visible. Returns `true` unless the entity has a
/// `"visible"` dynamic component explicitly set to `false`.
fn is_visible(world: &World, entity_id: EntityId) -> bool {
    match world.get_component_by_name(entity_id, "visible") {
        Some(val) => val.as_bool().unwrap_or(true),
        None => true,
    }
}

/// Read the z_index from an entity's `"z_index"` dynamic component.
/// Returns `0.0` if the component is absent.
fn read_z_index(world: &World, entity_id: EntityId) -> f64 {
    world
        .get_component_by_name(entity_id, "z_index")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Dynamic component collection
// ---------------------------------------------------------------------------

/// Collect ALL dynamic (named) components for an entity into a JSON map.
fn collect_dynamic_components(
    world: &World,
    entity_id: EntityId,
) -> HashMap<String, serde_json::Value> {
    let mut map = HashMap::new();
    for name in world.registry().registered_names() {
        if let Some(val) = world.get_component_by_name(entity_id, name) {
            map.insert(name.to_owned(), val);
        }
    }
    map
}

// ---------------------------------------------------------------------------
// Public extraction entry point
// ---------------------------------------------------------------------------

/// Extract a [`SceneSnapshot`] from the current ECS world state.
///
/// The extraction follows two phases, mirroring the renderer:
///
/// 1. **Phase 1 -- native physics entities**: queries `(Position, PhysicsBody)`
///    to get spatial data from the physics system. Also reads `Velocity` if
///    present.
/// 2. **Phase 2 -- dynamic JSON entities**: for entities not covered by Phase 1,
///    looks for `"position"` (`{x, y}`), `"size"` (`{w, h}`), and `"velocity"`
///    (`{dx, dy}`) dynamic components.
///
/// For ALL entities (both phases) the function extracts identity, visibility,
/// z_index, and every dynamic component.
///
/// Entities are sorted by `(z_index, entity_id)` for deterministic ordering.
/// Scene bounds are computed from all entities that have position data.
pub fn extract_scene_snapshot(world: &World, tick: u64, sim_time: f64) -> SceneSnapshot {
    let mut entities: Vec<SceneEntity> = Vec::new();
    let mut visited: HashSet<EntityId> = HashSet::new();

    // Track bounds across both phases.
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;

    // ----- Phase 1: Native physics entities --------------------------------
    for (entity_id, (pos, body)) in world.query::<(&Position, &PhysicsBody)>() {
        visited.insert(entity_id);

        let visible = is_visible(world, entity_id);
        let z_index = read_z_index(world, entity_id);
        let identity = world.get_component::<Identity>(entity_id);
        let (entity_type, role, tier) = extract_identity(identity);
        let components = collect_dynamic_components(world, entity_id);

        let (w, h) = match &body.collider {
            ColliderShape::Box {
                half_width,
                half_height,
            } => (*half_width * 2.0, *half_height * 2.0),
            ColliderShape::Circle { radius } => {
                let d = *radius * 2.0;
                (d, d)
            }
        };

        // Velocity from native component.
        let velocity = world
            .get_component::<Velocity>(entity_id)
            .map(|v| [v.dx, v.dy]);

        // Update bounds.
        let half_w = w / 2.0;
        let half_h = h / 2.0;
        min_x = min_x.min(pos.x - half_w);
        min_y = min_y.min(pos.y - half_h);
        max_x = max_x.max(pos.x + half_w);
        max_y = max_y.max(pos.y + half_h);

        entities.push(SceneEntity {
            entity_id: entity_id.to_raw(),
            entity_type,
            role,
            tier,
            position: Some([pos.x, pos.y]),
            size: Some([w, h]),
            velocity,
            visible,
            z_index,
            components,
        });
    }

    // ----- Phase 2: Dynamic JSON entities ----------------------------------
    for entity_id in world.all_entity_ids() {
        if visited.contains(&entity_id) {
            continue;
        }

        let visible = is_visible(world, entity_id);
        let z_index = read_z_index(world, entity_id);
        let identity = world.get_component::<Identity>(entity_id);
        let (entity_type, role, tier) = extract_identity(identity);
        let components = collect_dynamic_components(world, entity_id);

        // Try to read position from dynamic component.
        let position = world
            .get_component_by_name(entity_id, "position")
            .and_then(|v| {
                let x = v.get("x")?.as_f64()?;
                let y = v.get("y")?.as_f64()?;
                Some([x, y])
            });

        // Try to read size from dynamic component.
        let size = world
            .get_component_by_name(entity_id, "size")
            .and_then(|v| {
                let w = v.get("w").and_then(|w| w.as_f64()).unwrap_or(1.0);
                let h = v.get("h").and_then(|h| h.as_f64()).unwrap_or(1.0);
                Some([w, h])
            });

        // Try to read velocity from dynamic component.
        let velocity = world
            .get_component_by_name(entity_id, "velocity")
            .and_then(|v| {
                let dx = v.get("dx")?.as_f64()?;
                let dy = v.get("dy")?.as_f64()?;
                Some([dx, dy])
            });

        // Update bounds if position data is available.
        if let Some([x, y]) = position {
            let (half_w, half_h) = match size {
                Some([w, h]) => (w / 2.0, h / 2.0),
                None => (0.0, 0.0),
            };
            min_x = min_x.min(x - half_w);
            min_y = min_y.min(y - half_h);
            max_x = max_x.max(x + half_w);
            max_y = max_y.max(y + half_h);
        }

        entities.push(SceneEntity {
            entity_id: entity_id.to_raw(),
            entity_type,
            role,
            tier,
            position,
            size,
            velocity,
            visible,
            z_index,
            components,
        });
    }

    // ----- Sort by (z_index, entity_id) for deterministic ordering ---------
    entities.sort_by(|a, b| {
        a.z_index
            .partial_cmp(&b.z_index)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.entity_id.cmp(&b.entity_id))
    });

    // ----- Compute scene bounds --------------------------------------------
    let bounds = if min_x <= max_x && min_y <= max_y {
        SceneBounds {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    } else {
        // No entities with position data -- default to zero bounds.
        SceneBounds {
            min_x: 0.0,
            min_y: 0.0,
            max_x: 0.0,
            max_y: 0.0,
        }
    };

    let entity_count = entities.len();

    SceneSnapshot {
        schema_version: 1,
        tick,
        sim_time,
        entities,
        bounds,
        entity_count,
    }
}
