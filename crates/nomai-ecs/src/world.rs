//! The [`World`] is the top-level container for the ECS. It owns the entity
//! allocator, the component registry, and all archetype storage.

use std::collections::HashMap;

use crate::archetype::{Archetype, ArchetypeId, ComponentVtable};
use crate::component::{ComponentRegistry, ComponentTypeId};
use crate::entity::{EntityAllocator, EntityId};
use crate::identity::{EntityIdentity, Identity, IdentityTier, PoolIdentity};
use crate::EcsError;

// ---------------------------------------------------------------------------
// Entity location
// ---------------------------------------------------------------------------

/// Where an entity lives: which archetype and which row within that archetype.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EntityLocation {
    pub archetype_id: ArchetypeId,
    pub row: usize,
}

// ---------------------------------------------------------------------------
// VtableRegistry -- maps ComponentTypeId to its ComponentVtable
// ---------------------------------------------------------------------------

/// Stores vtables for registered component types, indexed by ComponentTypeId.
#[derive(Debug, Default)]
struct VtableRegistry {
    vtables: Vec<ComponentVtable>,
}

impl VtableRegistry {
    fn new() -> Self {
        Self {
            vtables: Vec::new(),
        }
    }

    fn register<T: Clone + 'static>(&mut self, id: ComponentTypeId) {
        let idx = id.0 as usize;
        if idx >= self.vtables.len() {
            self.vtables.resize(idx + 1, ComponentVtable::new::<()>());
        }
        self.vtables[idx] = ComponentVtable::new::<T>();
    }

    fn get(&self, id: ComponentTypeId) -> &ComponentVtable {
        &self.vtables[id.0 as usize]
    }
}

// ---------------------------------------------------------------------------
// DeserializerRegistry -- type-erased JSON -> raw bytes conversion
// ---------------------------------------------------------------------------

/// Type-erased function that deserializes a `serde_json::Value` into a
/// `RawComponentBuf` containing the component value. Returns `Err` if the JSON
/// does not match the component type's schema.
type DeserializeFn =
    Box<dyn Fn(&serde_json::Value) -> Result<RawComponentBuf, String> + Send + Sync>;

/// Registry of component deserializers, indexed by [`ComponentTypeId`].
///
/// Each registered component type gets a deserializer that converts
/// `serde_json::Value` into the raw byte representation that can be written
/// into archetype column storage.
pub(crate) struct DeserializerRegistry {
    deserializers: Vec<Option<DeserializeFn>>,
}

impl DeserializerRegistry {
    fn new() -> Self {
        Self {
            deserializers: Vec::new(),
        }
    }

    fn register<T>(&mut self, id: ComponentTypeId)
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let idx = id.0 as usize;
        if idx >= self.deserializers.len() {
            self.deserializers.resize_with(idx + 1, || None);
        }
        self.deserializers[idx] = Some(Box::new(|value: &serde_json::Value| {
            let typed: T = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            Ok(RawComponentBuf::from_value(typed))
        }));
    }

    /// Deserialize a JSON value into raw bytes for the given component type.
    ///
    /// Returns `None` if the component type has no registered deserializer,
    /// or `Some(Err(...))` if deserialization fails.
    fn deserialize(
        &self,
        id: ComponentTypeId,
        value: &serde_json::Value,
    ) -> Option<Result<RawComponentBuf, String>> {
        let idx = id.0 as usize;
        self.deserializers
            .get(idx)
            .and_then(|opt| opt.as_ref())
            .map(|f| f(value))
    }
}

impl std::fmt::Debug for DeserializerRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeserializerRegistry")
            .field(
                "count",
                &self.deserializers.iter().filter(|d| d.is_some()).count(),
            )
            .finish()
    }
}

// ---------------------------------------------------------------------------
// RawComponentBuf -- properly aligned component storage
// ---------------------------------------------------------------------------

/// A properly aligned heap buffer for storing a single component value.
///
/// Uses `std::alloc::alloc` with the correct layout to guarantee alignment.
/// Automatically deallocates on drop. Component destructor is NOT called
/// on drop -- if you need the component's Drop impl to run, call `drop_value`
/// first.
pub(crate) struct RawComponentBuf {
    /// Pointer to the heap allocation (null for ZSTs).
    ptr: *mut u8,
    /// Layout used for allocation (size may be 0 for ZSTs).
    layout: std::alloc::Layout,
}

impl RawComponentBuf {
    /// Create a new buffer from a typed value. The value is moved into the
    /// buffer and `mem::forget`-ed -- ownership transfers to the buffer.
    fn from_value<T>(value: T) -> Self {
        let size = std::mem::size_of::<T>();
        let align = std::mem::align_of::<T>();
        let layout = if size > 0 {
            std::alloc::Layout::from_size_align(size, align).expect("invalid layout")
        } else {
            // ZST: use a layout with size 0.
            std::alloc::Layout::from_size_align(0, align).expect("invalid ZST layout")
        };

        if size > 0 {
            #[allow(unsafe_code)]
            let ptr = unsafe {
                let ptr = std::alloc::alloc(layout);
                assert!(!ptr.is_null(), "allocation failed");
                std::ptr::copy_nonoverlapping(&value as *const T as *const u8, ptr, size);
                ptr
            };
            std::mem::forget(value);
            Self { ptr, layout }
        } else {
            std::mem::forget(value);
            Self {
                ptr: std::ptr::null_mut(),
                layout,
            }
        }
    }

    /// Create a buffer by taking ownership of an existing allocation.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated with `std::alloc::alloc(layout)` and
    /// must contain a valid, initialized component value.
    unsafe fn from_raw(ptr: *mut u8, layout: std::alloc::Layout) -> Self {
        Self { ptr, layout }
    }

    /// Get a pointer to the stored data.
    fn as_ptr(&self) -> *const u8 {
        if self.layout.size() > 0 {
            self.ptr
        } else {
            // ZST: return a dangling aligned pointer.
            self.layout.align() as *const u8
        }
    }

    /// Drop the component value in place using the provided vtable,
    /// then mark the buffer as consumed (ptr set to null, will not dealloc again).
    #[allow(unsafe_code)]
    unsafe fn drop_value(&mut self, vtable: &ComponentVtable) {
        if vtable.size > 0 && !self.ptr.is_null() {
            (vtable.drop_fn)(self.ptr);
        }
    }
}

impl Drop for RawComponentBuf {
    fn drop(&mut self) {
        // Deallocate the heap buffer. This does NOT drop the component value.
        // The caller must have already consumed or dropped the value.
        if self.layout.size() > 0 && !self.ptr.is_null() {
            #[allow(unsafe_code)]
            unsafe {
                std::alloc::dealloc(self.ptr, self.layout);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ComponentBundle helpers
// ---------------------------------------------------------------------------

/// A helper for building a set of components to pass to `World::spawn_bundle`.
///
/// Usage:
/// ```ignore
/// let mut bundle = ComponentBundle::new();
/// bundle.add(world.registry(), Position { x: 0.0, y: 0.0 });
/// bundle.add(world.registry(), Velocity { dx: 1.0, dy: 0.0 });
/// world.spawn_bundle(bundle);
/// ```
pub struct ComponentBundle {
    /// (ComponentTypeId, properly-aligned buffer, drop vtable)
    entries: Vec<(ComponentTypeId, RawComponentBuf, Option<ComponentVtable>)>,
}

impl ComponentBundle {
    /// Create an empty bundle.
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a component to the bundle. The component type must already be
    /// registered in the world's registry.
    ///
    /// # Panics
    ///
    /// Panics if the component type is not registered, or if a component of
    /// the same type has already been added to this bundle.
    pub fn add<T>(&mut self, registry: &ComponentRegistry, value: T)
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let type_id = registry
            .lookup::<T>()
            .expect("component type not registered -- call world.register_component::<T>() first");

        // M3: Check for duplicate component types.
        if self.entries.iter().any(|(id, _, _)| *id == type_id) {
            panic!(
                "duplicate component type {:?} in ComponentBundle -- each component type can only be added once",
                type_id
            );
        }

        let buf = RawComponentBuf::from_value(value);
        self.entries
            .push((type_id, buf, Some(ComponentVtable::new::<T>())));
    }

    /// The sorted set of component type IDs in this bundle.
    pub(crate) fn type_ids(&self) -> Vec<ComponentTypeId> {
        let mut ids: Vec<_> = self.entries.iter().map(|(id, _, _)| *id).collect();
        ids.sort();
        ids
    }

    /// Consume the bundle, yielding `(ComponentTypeId, RawComponentBuf)` pairs.
    /// The caller takes ownership of the buffers.
    pub(crate) fn into_raw_parts(mut self) -> Vec<(ComponentTypeId, RawComponentBuf)> {
        let entries: Vec<_> = self
            .entries
            .drain(..)
            .map(|(id, buf, _vtable)| (id, buf))
            .collect();
        entries
    }
}

impl Default for ComponentBundle {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ComponentBundle {
    fn drop(&mut self) {
        // Drop any remaining component values using their vtables.
        for (_id, mut buf, vtable) in self.entries.drain(..) {
            if let Some(vt) = vtable {
                #[allow(unsafe_code)]
                unsafe {
                    buf.drop_value(&vt);
                }
            }
            // RawComponentBuf::drop handles deallocation automatically.
        }
    }
}

// ---------------------------------------------------------------------------
// World
// ---------------------------------------------------------------------------

/// The top-level ECS container.
///
/// Owns the entity allocator, component registry, and all archetype storage.
/// Provides the primary API for entity lifecycle and component access.
pub struct World {
    /// Entity ID allocator.
    pub(crate) allocator: EntityAllocator,
    /// Component type registry.
    pub(crate) registry: ComponentRegistry,
    /// Vtable registry for drop/clone fns.
    vtable_registry: VtableRegistry,
    /// Deserializer registry for JSON -> typed component conversion.
    pub(crate) deserializer_registry: DeserializerRegistry,
    /// All archetypes, indexed by `ArchetypeId.0`.
    pub(crate) archetypes: Vec<Archetype>,
    /// Maps a sorted set of component type IDs to an archetype.
    archetype_index: HashMap<Vec<ComponentTypeId>, ArchetypeId>,
    /// Maps entity ID -> (archetype, row).
    pub(crate) entity_locations: HashMap<EntityId, EntityLocation>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World")
            .field("entity_count", &self.entity_locations.len())
            .field("archetype_count", &self.archetypes.len())
            .finish()
    }
}

impl World {
    /// Create a new, empty world.
    ///
    /// The built-in [`Identity`] component is automatically registered so that
    /// [`spawn_semantic`](Self::spawn_semantic) and
    /// [`spawn_pooled`](Self::spawn_pooled) can attach it without extra setup.
    pub fn new() -> Self {
        let mut world = Self {
            allocator: EntityAllocator::new(),
            registry: ComponentRegistry::new(),
            vtable_registry: VtableRegistry::new(),
            deserializer_registry: DeserializerRegistry::new(),
            archetypes: Vec::new(),
            archetype_index: HashMap::new(),
            entity_locations: HashMap::new(),
        };
        // Register the built-in Identity component type.
        world.register_component::<Identity>("__identity");
        world
    }

    /// Read-only access to the component registry.
    pub fn registry(&self) -> &ComponentRegistry {
        &self.registry
    }

    /// Register a component type. Convenience wrapper.
    ///
    /// This registers the type in the component registry, vtable registry,
    /// and deserializer registry so that it can be used with typed access,
    /// archetype storage, and command buffer JSON deserialization.
    pub fn register_component<T>(&mut self, name: &str) -> ComponentTypeId
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let id = self.registry.register::<T>(name);
        self.vtable_registry.register::<T>(id);
        self.deserializer_registry.register::<T>(id);
        id
    }

    // -- archetype management -----------------------------------------------

    /// Find or create the archetype for a given sorted set of component types.
    fn get_or_create_archetype(&mut self, type_ids: &[ComponentTypeId]) -> ArchetypeId {
        if let Some(&id) = self.archetype_index.get(type_ids) {
            return id;
        }
        let id = ArchetypeId(self.archetypes.len() as u32);
        let infos: Vec<_> = type_ids
            .iter()
            .map(|tid| {
                self.registry
                    .get_info(*tid)
                    .expect("component type not registered")
                    .clone()
            })
            .collect();
        let vtables: Vec<_> = type_ids
            .iter()
            .map(|tid| self.vtable_registry.get(*tid).clone())
            .collect();
        let archetype = Archetype::new(id, type_ids.to_vec(), infos, vtables);
        self.archetypes.push(archetype);
        self.archetype_index.insert(type_ids.to_vec(), id);
        id
    }

    // -- entity lifecycle ---------------------------------------------------

    /// Spawn a new entity from a [`ComponentBundle`].
    ///
    /// Returns the newly allocated [`EntityId`].
    pub fn spawn_bundle(&mut self, bundle: ComponentBundle) -> EntityId {
        let entity = self.allocator.allocate();
        let type_ids = bundle.type_ids();
        let archetype_id = self.get_or_create_archetype(&type_ids);
        let raw_parts = bundle.into_raw_parts();

        // Build pointer pairs in the archetype's expected sort order.
        let mut components: Vec<(ComponentTypeId, *const u8)> = raw_parts
            .iter()
            .map(|(id, buf)| (*id, buf.as_ptr()))
            .collect();
        components.sort_by_key(|(id, _)| *id);

        #[allow(unsafe_code)]
        let row =
            unsafe { self.archetypes[archetype_id.0 as usize].add_entity(entity, &components) };

        // Archetype copied the bytes via add_entity. Drop the RawComponentBufs
        // to free their temporary heap allocations (data already in columns).
        drop(raw_parts);

        self.entity_locations
            .insert(entity, EntityLocation { archetype_id, row });
        entity
    }

    /// Spawn a new entity with a single component.
    pub fn spawn_with<T>(&mut self, component: T) -> EntityId
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let mut bundle = ComponentBundle::new();
        bundle.add(&self.registry, component);
        self.spawn_bundle(bundle)
    }

    /// Despawn an entity, removing it from its archetype and recycling the ID.
    pub fn despawn(&mut self, entity: EntityId) -> Result<(), EcsError> {
        let loc = self
            .entity_locations
            .remove(&entity)
            .ok_or(EcsError::StaleEntity(entity))?;
        if !self.allocator.is_alive(entity) {
            return Err(EcsError::StaleEntity(entity));
        }
        let archetype = &mut self.archetypes[loc.archetype_id.0 as usize];
        let swapped = archetype.remove_entity(loc.row);

        // If an entity was swapped into the removed row, update its location.
        if let Some(moved_entity) = swapped {
            if let Some(moved_loc) = self.entity_locations.get_mut(&moved_entity) {
                moved_loc.row = loc.row;
            }
        }

        self.allocator.deallocate(entity);
        Ok(())
    }

    // -- component access ---------------------------------------------------

    /// Get an immutable reference to a component on an entity.
    pub fn get_component<T: 'static>(&self, entity: EntityId) -> Option<&T> {
        let loc = self.entity_locations.get(&entity)?;
        let type_id = self.registry.lookup::<T>()?;
        #[allow(unsafe_code)]
        unsafe {
            self.archetypes[loc.archetype_id.0 as usize].get_component::<T>(loc.row, type_id)
        }
    }

    /// Get a mutable reference to a component on an entity.
    pub fn get_component_mut<T: 'static>(&mut self, entity: EntityId) -> Option<&mut T> {
        let loc = *self.entity_locations.get(&entity)?;
        let type_id = self.registry.lookup::<T>()?;
        #[allow(unsafe_code)]
        unsafe {
            self.archetypes[loc.archetype_id.0 as usize].get_component_mut::<T>(loc.row, type_id)
        }
    }

    /// Check whether an entity has a given component type.
    pub fn has_component<T: 'static>(&self, entity: EntityId) -> bool {
        let Some(loc) = self.entity_locations.get(&entity) else {
            return false;
        };
        let Some(type_id) = self.registry.lookup::<T>() else {
            return false;
        };
        self.archetypes[loc.archetype_id.0 as usize].has_component(type_id)
    }

    /// Extract component data from an archetype row into properly-aligned buffers.
    ///
    /// Helper that calls `remove_entity_and_move` on the archetype, collecting
    /// all extracted components into `(ComponentTypeId, RawComponentBuf)` pairs.
    /// Also updates the location of any entity swapped into the removed row.
    ///
    /// Returns `(extracted_components, swapped_entity)`.
    #[allow(unsafe_code)]
    fn extract_entity_components(
        &mut self,
        loc: EntityLocation,
    ) -> (Vec<(ComponentTypeId, RawComponentBuf)>, Option<EntityId>) {
        let mut extracted: Vec<(ComponentTypeId, RawComponentBuf)> = Vec::new();
        let swapped = unsafe {
            self.archetypes[loc.archetype_id.0 as usize].remove_entity_and_move(
                loc.row,
                |tid, ptr, vtable| {
                    // The archetype allocated this buffer with alloc::alloc(layout).
                    // We take ownership by wrapping it in a RawComponentBuf.
                    let layout = if vtable.size > 0 {
                        std::alloc::Layout::from_size_align(vtable.size, vtable.align).unwrap()
                    } else {
                        std::alloc::Layout::from_size_align(0, vtable.align).unwrap()
                    };
                    let buf = RawComponentBuf::from_raw(ptr as *mut u8, layout);
                    extracted.push((tid, buf));
                },
            )
        };

        // Update location of the entity that was swapped into the old row.
        if let Some(moved_entity) = swapped {
            if let Some(moved_loc) = self.entity_locations.get_mut(&moved_entity) {
                moved_loc.row = loc.row;
            }
        }

        (extracted, swapped)
    }

    /// Insert extracted components (plus optionally new ones) into a target archetype.
    ///
    /// Returns the new row index.
    #[allow(unsafe_code)]
    fn insert_extracted_entity(
        &mut self,
        entity: EntityId,
        arch_id: ArchetypeId,
        extracted: Vec<(ComponentTypeId, RawComponentBuf)>,
    ) -> usize {
        let mut components: Vec<(ComponentTypeId, *const u8)> = extracted
            .iter()
            .map(|(id, buf)| (*id, buf.as_ptr()))
            .collect();
        components.sort_by_key(|(id, _)| *id);

        let new_row =
            unsafe { self.archetypes[arch_id.0 as usize].add_entity(entity, &components) };

        // Archetype copied the bytes via add_entity. Drop the RawComponentBufs
        // to free their temporary heap allocations (data already in columns).
        drop(extracted);

        new_row
    }

    /// Insert a component on an entity. If the entity already has this
    /// component type, the value is overwritten in place. Otherwise, the
    /// entity migrates to a new archetype that includes the additional type.
    pub fn insert_component<T>(&mut self, entity: EntityId, value: T) -> Result<(), EcsError>
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let type_id = self
            .registry
            .lookup::<T>()
            .ok_or_else(|| EcsError::UnknownComponent(std::any::type_name::<T>().to_owned()))?;

        let loc = *self
            .entity_locations
            .get(&entity)
            .ok_or(EcsError::StaleEntity(entity))?;

        let archetype = &self.archetypes[loc.archetype_id.0 as usize];

        if archetype.has_component(type_id) {
            // Overwrite in place.
            #[allow(unsafe_code)]
            let slot = unsafe {
                self.archetypes[loc.archetype_id.0 as usize]
                    .get_component_mut::<T>(loc.row, type_id)
            };
            if let Some(slot) = slot {
                *slot = value;
            }
            return Ok(());
        }

        // Migrate to a new archetype with the additional component.
        let old_types = archetype.component_types().to_vec();
        let mut new_types = old_types;
        new_types.push(type_id);
        new_types.sort();

        // Extract entity from old archetype.
        let (mut extracted, _swapped) = self.extract_entity_components(loc);

        // Add the new component to extracted set.
        let new_buf = RawComponentBuf::from_value(value);
        extracted.push((type_id, new_buf));

        // Get or create the target archetype.
        let new_arch_id = self.get_or_create_archetype(&new_types);

        let new_row = self.insert_extracted_entity(entity, new_arch_id, extracted);

        self.entity_locations.insert(
            entity,
            EntityLocation {
                archetype_id: new_arch_id,
                row: new_row,
            },
        );
        Ok(())
    }

    /// Remove a component type from an entity. If the entity does not have
    /// the component, this is a no-op (returns Ok). Otherwise the entity
    /// migrates to a new archetype without that type.
    pub fn remove_component<T>(&mut self, entity: EntityId) -> Result<(), EcsError>
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let type_id = self
            .registry
            .lookup::<T>()
            .ok_or_else(|| EcsError::UnknownComponent(std::any::type_name::<T>().to_owned()))?;

        let loc = *self
            .entity_locations
            .get(&entity)
            .ok_or(EcsError::StaleEntity(entity))?;

        let archetype = &self.archetypes[loc.archetype_id.0 as usize];
        if !archetype.has_component(type_id) {
            return Ok(()); // Nothing to remove.
        }

        let old_types = archetype.component_types().to_vec();
        let new_types: Vec<_> = old_types
            .iter()
            .copied()
            .filter(|t| *t != type_id)
            .collect();

        // Extract from old archetype.
        let (extracted, _swapped) = self.extract_entity_components(loc);

        // Separate: drop the removed component, keep the rest.
        let vtable = self.vtable_registry.get(type_id).clone();
        let mut kept: Vec<(ComponentTypeId, RawComponentBuf)> = Vec::new();
        for (tid, mut buf) in extracted {
            if tid == type_id {
                // Drop the removed component's value, then dealloc via Drop.
                #[allow(unsafe_code)]
                unsafe {
                    buf.drop_value(&vtable);
                }
                // buf drops here, deallocating the buffer.
            } else {
                kept.push((tid, buf));
            }
        }

        let new_arch_id = self.get_or_create_archetype(&new_types);

        let new_row = self.insert_extracted_entity(entity, new_arch_id, kept);

        self.entity_locations.insert(
            entity,
            EntityLocation {
                archetype_id: new_arch_id,
                row: new_row,
            },
        );

        Ok(())
    }

    /// Total number of alive entities tracked by the world.
    pub fn entity_count(&self) -> usize {
        self.entity_locations.len()
    }

    /// Total number of archetypes.
    pub fn archetype_count(&self) -> usize {
        self.archetypes.len()
    }

    // -- query helpers (used by query.rs) -----------------------------------

    /// Find all archetype IDs whose component set is a superset of `required`.
    pub(crate) fn matching_archetypes(&self, required: &[ComponentTypeId]) -> Vec<ArchetypeId> {
        self.archetypes
            .iter()
            .filter(|arch| required.iter().all(|req| arch.has_component(*req)))
            .map(|arch| arch.id())
            .collect()
    }

    /// Look up the `ComponentTypeId` for a Rust type.
    pub(crate) fn component_type_id<T: 'static>(&self) -> Option<ComponentTypeId> {
        self.registry.lookup::<T>()
    }

    /// Check whether an entity is alive.
    pub fn is_alive(&self, entity: EntityId) -> bool {
        self.allocator.is_alive(entity)
    }

    // -- tiered entity spawning ---------------------------------------------

    /// Spawn a **Semantic-tier** entity with full identity and manifest tracking.
    ///
    /// The [`Identity::Semantic`] component is automatically added. Additional
    /// components can be provided via the `bundle`.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::UnknownComponent`] if a component type in the bundle
    /// is not registered (this would panic before reaching here via
    /// `ComponentBundle::add`, so in practice this only applies to future
    /// bundle construction paths).
    pub fn spawn_semantic(
        &mut self,
        identity: EntityIdentity,
        bundle: ComponentBundle,
    ) -> Result<EntityId, EcsError> {
        let identity_component = Identity::Semantic(identity);
        let entity = self.spawn_bundle_with_identity(bundle, identity_component);
        Ok(entity)
    }

    /// Spawn a **Pooled-tier** entity with type-level identity.
    ///
    /// The [`Identity::Pooled`] component is automatically added. Additional
    /// components can be provided via the `bundle`.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::UnknownComponent`] if a component type in the bundle
    /// is not registered.
    pub fn spawn_pooled(
        &mut self,
        identity: PoolIdentity,
        bundle: ComponentBundle,
    ) -> Result<EntityId, EcsError> {
        let identity_component = Identity::Pooled(identity);
        let entity = self.spawn_bundle_with_identity(bundle, identity_component);
        Ok(entity)
    }

    /// Internal helper: spawn an entity from a bundle plus an identity component.
    fn spawn_bundle_with_identity(
        &mut self,
        mut bundle: ComponentBundle,
        identity: Identity,
    ) -> EntityId {
        bundle.add(&self.registry, identity);
        self.spawn_bundle(bundle)
    }

    // -- identity queries ---------------------------------------------------

    /// Get the [`Identity`] of an entity.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::StaleEntity`] if the entity does not exist or has
    /// no identity component (e.g., was spawned via the low-level `spawn_bundle`).
    pub fn get_identity(&self, entity: EntityId) -> Result<&Identity, EcsError> {
        self.get_component::<Identity>(entity)
            .ok_or(EcsError::StaleEntity(entity))
    }

    /// Get the [`IdentityTier`] of an entity.
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::StaleEntity`] if the entity does not exist or has
    /// no identity component.
    pub fn get_tier(&self, entity: EntityId) -> Result<IdentityTier, EcsError> {
        self.get_identity(entity).map(|id| id.tier())
    }

    /// Query all entities with a specific [`IdentityTier`].
    ///
    /// Iterates all entities that have an [`Identity`] component and returns
    /// those whose tier matches.
    pub fn entities_by_tier(&self, tier: IdentityTier) -> Vec<EntityId> {
        let Some(identity_type_id) = self.registry.lookup::<Identity>() else {
            return Vec::new();
        };
        let matching = self.matching_archetypes(&[identity_type_id]);
        let mut result = Vec::new();
        for arch_id in matching {
            let archetype = &self.archetypes[arch_id.0 as usize];
            for (row, &entity) in archetype.entities().iter().enumerate() {
                #[allow(unsafe_code)]
                let identity =
                    unsafe { archetype.get_component::<Identity>(row, identity_type_id) };
                if let Some(id) = identity {
                    if id.tier() == tier {
                        result.push(entity);
                    }
                }
            }
        }
        result
    }

    // -- command buffer support ------------------------------------------------

    /// Set a component on an entity using a JSON value and the component's
    /// registered string name.
    ///
    /// This is the primary mechanism for the command buffer's `SetComponent`
    /// operation. It deserializes the JSON value into the correct type using the
    /// registered deserializer, then either overwrites the existing component or
    /// inserts it (triggering an archetype migration).
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::UnknownComponent`] if the component name is not
    /// registered, or [`EcsError::StaleEntity`] if the entity does not exist.
    /// Returns a deserialization error (wrapped in `UnknownComponent`) if the
    /// JSON value does not match the component's schema.
    pub fn set_component_by_name(
        &mut self,
        entity: EntityId,
        component_name: &str,
        value: &serde_json::Value,
    ) -> Result<(), EcsError> {
        let type_id = self
            .registry
            .lookup_by_name(component_name)
            .ok_or_else(|| EcsError::UnknownComponent(component_name.to_owned()))?;

        let _loc = self
            .entity_locations
            .get(&entity)
            .ok_or(EcsError::StaleEntity(entity))?;

        // Deserialize JSON -> raw bytes in a properly-aligned buffer.
        let raw_buf = self
            .deserializer_registry
            .deserialize(type_id, value)
            .ok_or_else(|| {
                EcsError::UnknownComponent(format!(
                    "no deserializer for component '{component_name}'"
                ))
            })?
            .map_err(|e| {
                EcsError::UnknownComponent(format!(
                    "failed to deserialize component '{component_name}': {e}"
                ))
            })?;

        let loc = *self.entity_locations.get(&entity).unwrap();
        let archetype = &self.archetypes[loc.archetype_id.0 as usize];

        if archetype.has_component(type_id) {
            // Overwrite in place: copy the new raw bytes over the existing slot.
            let info = self.registry.get_info(type_id).unwrap();
            let vtable = self.vtable_registry.get(type_id).clone();
            let archetype_mut = &mut self.archetypes[loc.archetype_id.0 as usize];
            #[allow(unsafe_code)]
            unsafe {
                let ptr = archetype_mut.get_component_raw_mut(loc.row, type_id);
                if let Some(ptr) = ptr {
                    // Drop old value, then copy new bytes.
                    (vtable.drop_fn)(ptr);
                    std::ptr::copy_nonoverlapping(raw_buf.as_ptr(), ptr, info.size);
                }
            }
            // Archetype overwrote the column bytes. Drop raw_buf to free
            // the temporary heap allocation.
            drop(raw_buf);
            return Ok(());
        }

        // Migration: add the new component type.
        let old_types = archetype.component_types().to_vec();
        let mut new_types = old_types;
        new_types.push(type_id);
        new_types.sort();

        // Extract entity from old archetype.
        let (mut extracted, _swapped) = self.extract_entity_components(loc);

        // Add the new component.
        extracted.push((type_id, raw_buf));

        let new_arch_id = self.get_or_create_archetype(&new_types);
        let new_row = self.insert_extracted_entity(entity, new_arch_id, extracted);

        self.entity_locations.insert(
            entity,
            EntityLocation {
                archetype_id: new_arch_id,
                row: new_row,
            },
        );
        Ok(())
    }

    /// Remove a component from an entity by the component's registered string
    /// name.
    ///
    /// This is the primary mechanism for the command buffer's `RemoveComponent`
    /// operation. If the entity does not have the named component, this is a
    /// no-op (returns `Ok`).
    ///
    /// # Errors
    ///
    /// Returns [`EcsError::UnknownComponent`] if the component name is not
    /// registered, or [`EcsError::StaleEntity`] if the entity does not exist.
    pub fn remove_component_by_name(
        &mut self,
        entity: EntityId,
        component_name: &str,
    ) -> Result<(), EcsError> {
        let type_id = self
            .registry
            .lookup_by_name(component_name)
            .ok_or_else(|| EcsError::UnknownComponent(component_name.to_owned()))?;

        let loc = *self
            .entity_locations
            .get(&entity)
            .ok_or(EcsError::StaleEntity(entity))?;

        let archetype = &self.archetypes[loc.archetype_id.0 as usize];
        if !archetype.has_component(type_id) {
            return Ok(()); // Nothing to remove.
        }

        let old_types = archetype.component_types().to_vec();
        let new_types: Vec<_> = old_types
            .iter()
            .copied()
            .filter(|t| *t != type_id)
            .collect();

        // Extract from old archetype.
        let (extracted, _swapped) = self.extract_entity_components(loc);

        // Separate: drop the removed component, keep the rest.
        let vtable = self.vtable_registry.get(type_id).clone();
        let mut kept: Vec<(ComponentTypeId, RawComponentBuf)> = Vec::new();
        for (tid, mut buf) in extracted {
            if tid == type_id {
                #[allow(unsafe_code)]
                unsafe {
                    buf.drop_value(&vtable);
                }
                // buf drops here, deallocating.
            } else {
                kept.push((tid, buf));
            }
        }

        let new_arch_id = self.get_or_create_archetype(&new_types);
        let new_row = self.insert_extracted_entity(entity, new_arch_id, kept);

        self.entity_locations.insert(
            entity,
            EntityLocation {
                archetype_id: new_arch_id,
                row: new_row,
            },
        );

        Ok(())
    }

    /// Query all semantic entities with a specific role.
    ///
    /// Only entities with [`Identity::Semantic`] are considered. The `role`
    /// parameter is matched exactly against [`EntityIdentity::role`].
    pub fn entities_by_role(&self, role: &str) -> Vec<EntityId> {
        let Some(identity_type_id) = self.registry.lookup::<Identity>() else {
            return Vec::new();
        };
        let matching = self.matching_archetypes(&[identity_type_id]);
        let mut result = Vec::new();
        for arch_id in matching {
            let archetype = &self.archetypes[arch_id.0 as usize];
            for (row, &entity) in archetype.entities().iter().enumerate() {
                #[allow(unsafe_code)]
                let identity =
                    unsafe { archetype.get_component::<Identity>(row, identity_type_id) };
                if let Some(Identity::Semantic(eid)) = identity {
                    if eid.role == role {
                        result.push(entity);
                    }
                }
            }
        }
        result
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Pos {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Vel {
        dx: f32,
        dy: f32,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Health(u32);

    fn setup_world() -> World {
        let mut world = World::new();
        world.register_component::<Pos>("position");
        world.register_component::<Vel>("velocity");
        world.register_component::<Health>("health");
        world
    }

    #[test]
    fn spawn_and_get() {
        let mut world = setup_world();
        let mut bundle = ComponentBundle::new();
        bundle.add(&world.registry, Pos { x: 1.0, y: 2.0 });
        bundle.add(&world.registry, Vel { dx: 3.0, dy: 4.0 });
        let e = world.spawn_bundle(bundle);

        assert_eq!(world.get_component::<Pos>(e), Some(&Pos { x: 1.0, y: 2.0 }));
        assert_eq!(
            world.get_component::<Vel>(e),
            Some(&Vel { dx: 3.0, dy: 4.0 })
        );
        assert!(!world.has_component::<Health>(e));
    }

    #[test]
    fn spawn_with_single() {
        let mut world = setup_world();
        let e = world.spawn_with(Pos { x: 10.0, y: 20.0 });
        assert_eq!(
            world.get_component::<Pos>(e),
            Some(&Pos { x: 10.0, y: 20.0 })
        );
    }

    #[test]
    fn despawn_removes_entity() {
        let mut world = setup_world();
        let e = world.spawn_with(Pos { x: 0.0, y: 0.0 });
        assert!(world.is_alive(e));
        world.despawn(e).unwrap();
        assert!(!world.is_alive(e));
        assert_eq!(world.get_component::<Pos>(e), None);
    }

    #[test]
    fn insert_component_migrates_archetype() {
        let mut world = setup_world();
        let e = world.spawn_with(Pos { x: 1.0, y: 2.0 });
        assert!(!world.has_component::<Vel>(e));

        world.insert_component(e, Vel { dx: 5.0, dy: 6.0 }).unwrap();
        assert!(world.has_component::<Vel>(e));
        assert_eq!(
            world.get_component::<Vel>(e),
            Some(&Vel { dx: 5.0, dy: 6.0 })
        );
        assert_eq!(world.get_component::<Pos>(e), Some(&Pos { x: 1.0, y: 2.0 }));
    }

    #[test]
    fn remove_component_migrates_archetype() {
        let mut world = setup_world();
        let mut bundle = ComponentBundle::new();
        bundle.add(&world.registry, Pos { x: 1.0, y: 2.0 });
        bundle.add(&world.registry, Vel { dx: 3.0, dy: 4.0 });
        let e = world.spawn_bundle(bundle);
        assert!(world.has_component::<Vel>(e));

        world.remove_component::<Vel>(e).unwrap();
        assert!(!world.has_component::<Vel>(e));
        assert_eq!(world.get_component::<Pos>(e), Some(&Pos { x: 1.0, y: 2.0 }));
    }

    #[test]
    fn get_component_mut_modifies() {
        let mut world = setup_world();
        let e = world.spawn_with(Pos { x: 0.0, y: 0.0 });
        if let Some(pos) = world.get_component_mut::<Pos>(e) {
            pos.x = 99.0;
        }
        assert_eq!(
            world.get_component::<Pos>(e),
            Some(&Pos { x: 99.0, y: 0.0 })
        );
    }

    #[test]
    fn entity_count_updates() {
        let mut world = setup_world();
        assert_eq!(world.entity_count(), 0);
        let e1 = world.spawn_with(Pos { x: 0.0, y: 0.0 });
        let _e2 = world.spawn_with(Pos { x: 1.0, y: 1.0 });
        assert_eq!(world.entity_count(), 2);
        world.despawn(e1).unwrap();
        assert_eq!(world.entity_count(), 1);
    }

    #[test]
    #[should_panic(expected = "duplicate component type")]
    fn component_bundle_rejects_duplicates() {
        let mut world = setup_world();
        let mut bundle = ComponentBundle::new();
        bundle.add(&world.registry, Pos { x: 1.0, y: 2.0 });
        bundle.add(&world.registry, Pos { x: 3.0, y: 4.0 }); // should panic
        let _ = world.spawn_bundle(bundle);
    }
}
