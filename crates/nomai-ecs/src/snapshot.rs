//! World snapshot and restore support.
//!
//! Provides [`WorldSnapshot`] -- a fully serializable representation of the
//! ECS world state that can be captured, serialized to JSON, and used to
//! restore the world to an exact previous state (including entity IDs,
//! allocator generations, and all component data).

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::component::ComponentTypeId;
use crate::entity::EntityId;
use crate::world::{RawComponentBuf, World};
use crate::EcsError;

// ---------------------------------------------------------------------------
// Snapshot types
// ---------------------------------------------------------------------------

/// Serializable snapshot of the [`EntityAllocator`](crate::entity::EntityAllocator) state.
///
/// Captures generations, alive flags, and free-list so that entity ID
/// allocation is fully reproducible after restore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocatorSnapshot {
    /// Per-index generation counters.
    pub generations: Vec<u32>,
    /// Per-index alive flags.
    pub alive: Vec<bool>,
    /// Free-list indices (in FIFO order).
    pub free_indices: Vec<u32>,
}

/// Serializable snapshot of a single entity's component data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySnapshot {
    /// The entity's ID (index + generation).
    pub entity_id: EntityId,
    /// Map from component name to serialized JSON value.
    /// Uses `BTreeMap` for deterministic serialization order.
    pub components: BTreeMap<String, serde_json::Value>,
}

/// A complete, serializable snapshot of the ECS world state.
///
/// Contains the allocator state, the list of registered component names,
/// and every alive entity with its serialized component data. This can be
/// serialized to JSON for storage or transmission and used to restore the
/// world to an identical state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSnapshot {
    /// Allocator state (generations, alive flags, free list).
    pub allocator: AllocatorSnapshot,
    /// Names of all registered component types (for informational purposes).
    pub component_names: Vec<String>,
    /// All alive entities with their serialized component data.
    pub entities: Vec<EntitySnapshot>,
}

// ---------------------------------------------------------------------------
// World snapshot/restore impl
// ---------------------------------------------------------------------------

impl World {
    /// Capture a complete snapshot of the world state.
    ///
    /// Serializes all entity component data to JSON via the registered
    /// serializer functions. The resulting [`WorldSnapshot`] can be serialized
    /// to JSON and later used to restore the world to this exact state.
    pub fn capture_snapshot(&self) -> WorldSnapshot {
        // 1. Snapshot the allocator.
        let (generations, alive, free_indices) = self.allocator.snapshot_state();
        let allocator = AllocatorSnapshot {
            generations,
            alive,
            free_indices,
        };

        // 2. Collect registered component names.
        let component_names: Vec<String> = self
            .registry
            .registered_names()
            .iter()
            .map(|s| s.to_string())
            .collect();

        // 3. Build a map of ComponentTypeId -> serialize fn reference for all
        //    registered types.
        let mut serialize_fns: HashMap<
            ComponentTypeId,
            &(dyn Fn(*const u8) -> serde_json::Value + Send + Sync),
        > = HashMap::new();
        for name in &component_names {
            if let Some(type_id) = self.registry.lookup_by_name(name) {
                if let Some(ser_fn) = self.serializer_registry.get(type_id) {
                    serialize_fns.insert(type_id, ser_fn);
                } else {
                    tracing::warn!(
                        component_type_id = ?type_id,
                        component_name = %name,
                        "component type has no serializer registered -- skipping in snapshot"
                    );
                }
            }
        }

        // 4. Build a reverse map: ComponentTypeId -> name for labeling.
        let mut id_to_name: HashMap<ComponentTypeId, &str> = HashMap::new();
        for name in &component_names {
            if let Some(type_id) = self.registry.lookup_by_name(name) {
                id_to_name.insert(type_id, name.as_str());
            }
        }

        // 5. Iterate all archetypes and serialize all entities.
        let mut entities: Vec<EntitySnapshot> = Vec::new();
        for archetype in &self.archetypes {
            // Safety: serialize_fns contains the correct type-matched functions
            // registered via register_component<T>, which cast *const u8 to *const T.
            #[allow(unsafe_code)]
            let arch_entities = unsafe { archetype.serialize_all_entities(&serialize_fns) };

            for (entity_id, components) in arch_entities {
                let mut comp_map: BTreeMap<String, serde_json::Value> = BTreeMap::new();
                for (type_id, value) in components {
                    if let Some(&name) = id_to_name.get(&type_id) {
                        comp_map.insert(name.to_string(), value);
                    } else {
                        tracing::warn!(
                            component_type_id = ?type_id,
                            entity_id = ?entity_id,
                            "component type has no name in registry -- skipping in snapshot"
                        );
                    }
                }
                entities.push(EntitySnapshot {
                    entity_id,
                    components: comp_map,
                });
            }
        }

        // Sort entities by EntityId for deterministic output.
        entities.sort_by_key(|e| e.entity_id.to_raw());

        WorldSnapshot {
            allocator,
            component_names,
            entities,
        }
    }

    /// Restore the world state from a previously captured snapshot.
    ///
    /// This clears all existing entities and archetypes, restores the entity
    /// allocator to its snapshotted state, and re-creates all entities with
    /// their exact original IDs and deserialized component data.
    ///
    /// # Prerequisites
    ///
    /// The same component types must be registered in the world before calling
    /// this method. Component registration is NOT restored by this function --
    /// only entity data and allocator state are restored.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::UnknownComponent`] if the snapshot references a
    /// component type that is not registered in the world.
    /// Returns [`EcsError::ComponentDeserializationError`] if a component
    /// value fails to deserialize.
    pub fn restore_from_snapshot(&mut self, snapshot: &WorldSnapshot) -> Result<(), EcsError> {
        // 0a. Pre-validate: ensure all component names in snapshot are known
        //     BEFORE clearing any world state. This prevents leaving the world
        //     in a broken state if a component name is unrecognised.
        for entity_snapshot in &snapshot.entities {
            for comp_name in entity_snapshot.components.keys() {
                if self.registry.lookup_by_name(comp_name).is_none() {
                    return Err(EcsError::UnknownComponent {
                        name: comp_name.clone(),
                        registered: self.registry.registered_names().join(", "),
                    });
                }
            }
        }

        // 0b. Validate allocator snapshot consistency.
        if snapshot.allocator.generations.len() != snapshot.allocator.alive.len() {
            return Err(EcsError::ComponentDeserializationError {
                component: "__allocator".to_owned(),
                details: format!(
                    "allocator snapshot inconsistent: {} generations vs {} alive flags",
                    snapshot.allocator.generations.len(),
                    snapshot.allocator.alive.len()
                ),
            });
        }
        let alloc_len = snapshot.allocator.generations.len();
        for &free_idx in &snapshot.allocator.free_indices {
            if (free_idx as usize) >= alloc_len {
                return Err(EcsError::ComponentDeserializationError {
                    component: "__allocator".to_owned(),
                    details: format!(
                        "allocator free index {} out of bounds (allocator has {} slots)",
                        free_idx, alloc_len
                    ),
                });
            }
        }

        // 0c. Validate free list entries only reference dead slots and are unique.
        {
            let mut seen = std::collections::HashSet::new();
            for &free_idx in &snapshot.allocator.free_indices {
                if snapshot.allocator.alive[free_idx as usize] {
                    return Err(EcsError::ComponentDeserializationError {
                        component: "__allocator".to_owned(),
                        details: format!(
                            "free list contains index {} which is marked alive",
                            free_idx
                        ),
                    });
                }
                if !seen.insert(free_idx) {
                    return Err(EcsError::ComponentDeserializationError {
                        component: "__allocator".to_owned(),
                        details: format!(
                            "free list contains duplicate index {}",
                            free_idx
                        ),
                    });
                }
            }
        }

        // 0d. Validate alive flags match snapshot entities: every allocator
        //     slot marked alive must have a corresponding entity in the
        //     snapshot, and every snapshot entity must be in an alive slot.
        {
            let entity_indices: std::collections::HashSet<u32> = snapshot
                .entities
                .iter()
                .map(|e| e.entity_id.index())
                .collect();
            for (idx, &is_alive) in snapshot.allocator.alive.iter().enumerate() {
                let has_entity = entity_indices.contains(&(idx as u32));
                if is_alive && !has_entity {
                    return Err(EcsError::ComponentDeserializationError {
                        component: "__allocator".to_owned(),
                        details: format!(
                            "allocator slot {} is marked alive but has no entity in snapshot",
                            idx
                        ),
                    });
                }
                if !is_alive && has_entity {
                    return Err(EcsError::ComponentDeserializationError {
                        component: "__allocator".to_owned(),
                        details: format!(
                            "allocator slot {} is marked dead but has entity data in snapshot",
                            idx
                        ),
                    });
                }
            }
        }

        // 1. Clear all existing entity data.
        //    Drop all entities in all archetypes.
        for archetype in &mut self.archetypes {
            #[allow(unsafe_code)]
            unsafe {
                archetype.clear();
            }
        }
        self.entity_locations.clear();
        self.archetypes.clear();
        self.archetype_index.clear();
        self.archetype_generation += 1;
        self.query_cache.borrow_mut().clear();

        // 2. Restore the allocator.
        self.allocator = crate::entity::EntityAllocator::restore_from_snapshot(
            snapshot.allocator.generations.clone(),
            snapshot.allocator.alive.clone(),
            snapshot.allocator.free_indices.clone(),
        );

        // 3. Re-create each entity with its exact EntityId.
        for entity_snapshot in &snapshot.entities {
            let entity_id = entity_snapshot.entity_id;

            // Deserialize all components into raw buffers.
            let mut raw_parts: Vec<(ComponentTypeId, RawComponentBuf)> = Vec::new();

            for (comp_name, value) in &entity_snapshot.components {
                let type_id = self.registry.lookup_by_name(comp_name).ok_or_else(|| {
                    EcsError::UnknownComponent {
                        name: comp_name.clone(),
                        registered: self.registry.registered_names().join(", "),
                    }
                })?;

                let raw_buf = self
                    .deserializer_registry
                    .deserialize(type_id, value)
                    .ok_or_else(|| EcsError::ComponentDeserializationError {
                        component: comp_name.clone(),
                        details: "no deserializer registered".to_owned(),
                    })?
                    .map_err(|e| EcsError::ComponentDeserializationError {
                        component: comp_name.clone(),
                        details: e,
                    })?;

                raw_parts.push((type_id, raw_buf));
            }

            // Sort type IDs to form the archetype key.
            raw_parts.sort_by_key(|(id, _)| *id);
            let type_ids: Vec<ComponentTypeId> = raw_parts.iter().map(|(id, _)| *id).collect();

            // Get or create the target archetype.
            let archetype_id = self.get_or_create_archetype(&type_ids);

            // Build pointer pairs in sorted order.
            let components: Vec<(ComponentTypeId, *const u8)> = raw_parts
                .iter()
                .map(|(id, buf)| (*id, buf.as_ptr()))
                .collect();

            // Add entity to archetype with its exact EntityId.
            #[allow(unsafe_code)]
            let row = unsafe {
                self.archetypes[archetype_id.0 as usize].add_entity(entity_id, &components)
            };

            // Safety: add_entity performed a bitwise copy of the raw bytes into
            // the archetype column. RawComponentBuf::Drop only deallocates the
            // outer heap buffer -- it does NOT call the component's destructor.
            // The column now owns the component data and will drop it via the
            // vtable when needed.
            drop(raw_parts);

            // Record entity location.
            self.entity_locations.insert(
                entity_id,
                crate::world::EntityLocation { archetype_id, row },
            );
        }

        Ok(())
    }
}
