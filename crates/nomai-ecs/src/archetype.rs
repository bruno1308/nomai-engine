//! Archetype storage for the ECS.
//!
//! An [`Archetype`] stores all entities that share the exact same set of
//! component types. Within each archetype, components are laid out in a
//! Structure-of-Arrays (SoA) pattern: one [`Column`] per component type, plus
//! a parallel `Vec<EntityId>` that maps row index to entity.
//!
//! # Safety
//!
//! This module contains `unsafe` code in [`Column`] because component data is
//! stored as type-erased byte buffers. The safety invariants are maintained by
//! the higher-level [`Archetype`] and [`World`](crate::world::World) code,
//! which guarantees that every column access uses the correct
//! [`ComponentInfo`] for the column's concrete type.
// Note: unsafe_code is allowed on this module via #[allow(unsafe_code)] in lib.rs

use crate::component::{ComponentInfo, ComponentTypeId};
use crate::entity::EntityId;

use std::alloc::{self, Layout};
use std::ptr;

// ---------------------------------------------------------------------------
// ArchetypeId
// ---------------------------------------------------------------------------

/// Identifies an archetype within the world. Indices into `World::archetypes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArchetypeId(pub(crate) u32);

// ---------------------------------------------------------------------------
// ComponentVtable -- type-erased operations for a component type
// ---------------------------------------------------------------------------

/// Function pointers for type-erased drop and clone of component values.
///
/// Created via [`ComponentVtable::new::<T>()`] and stored alongside each
/// archetype column so the column can drop/clone its contents without
/// knowing the concrete type at compile time.
#[derive(Clone)]
pub struct ComponentVtable {
    /// Drop a single value in place.
    pub(crate) drop_fn: unsafe fn(*mut u8),
    /// Clone a value from `src` to `dst` (both must be properly aligned and
    /// `dst` must be uninitialized memory of the right size).
    /// Currently unused but needed for future snapshot/clone support.
    #[allow(dead_code)]
    pub(crate) clone_fn: unsafe fn(*const u8, *mut u8),
    /// Size of the component type.
    pub(crate) size: usize,
    /// Alignment of the component type.
    pub(crate) align: usize,
}

impl std::fmt::Debug for ComponentVtable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentVtable")
            .field("size", &self.size)
            .field("align", &self.align)
            .finish()
    }
}

/// Safety: The function pointers in ComponentVtable are plain `fn` items
/// created via generic monomorphization. They capture no state and are
/// safe to send/share between threads.
unsafe impl Send for ComponentVtable {}
unsafe impl Sync for ComponentVtable {}

impl ComponentVtable {
    /// Create a vtable for a concrete component type `T`.
    pub fn new<T: Clone + 'static>() -> Self {
        unsafe fn drop_fn_impl<T>(ptr: *mut u8) {
            ptr::drop_in_place(ptr as *mut T);
        }

        unsafe fn clone_fn_impl<T: Clone>(src: *const u8, dst: *mut u8) {
            let value = &*(src as *const T);
            let cloned = value.clone();
            ptr::write(dst as *mut T, cloned);
        }

        Self {
            drop_fn: drop_fn_impl::<T>,
            clone_fn: clone_fn_impl::<T>,
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
        }
    }
}

// ---------------------------------------------------------------------------
// Column -- type-erased component storage
// ---------------------------------------------------------------------------

/// A type-erased, densely packed array of component values of a single type.
///
/// Internally this is a manually managed byte buffer whose layout matches the
/// stored component type.
pub struct Column {
    /// Pointer to the heap allocation (may be null when capacity == 0).
    data: *mut u8,
    /// Number of live elements.
    len: usize,
    /// Number of elements that fit in the current allocation.
    capacity: usize,
    /// Size and alignment of a single element.
    item_size: usize,
    item_align: usize,
}

// Column only stores raw bytes; the higher-level code guarantees that the
// concrete component type is Send + Sync.
unsafe impl Send for Column {}
unsafe impl Sync for Column {}

impl Column {
    /// Create a new, empty column for a component described by `info`.
    pub fn new(info: &ComponentInfo) -> Self {
        Self {
            data: ptr::null_mut(),
            len: 0,
            capacity: 0,
            item_size: info.size,
            item_align: info.align,
        }
    }

    /// Number of stored elements.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the column is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    // -- internal helpers ---------------------------------------------------

    fn layout_for_capacity(&self, cap: usize) -> Option<Layout> {
        if self.item_size == 0 || cap == 0 {
            return None;
        }
        Layout::from_size_align(self.item_size * cap, self.item_align).ok()
    }

    fn grow_if_needed(&mut self) {
        if self.len < self.capacity {
            return;
        }
        let new_cap = if self.capacity == 0 {
            4
        } else {
            self.capacity * 2
        };
        if self.item_size == 0 {
            // ZST: no actual allocation needed.
            self.capacity = new_cap;
            return;
        }
        let new_layout = self
            .layout_for_capacity(new_cap)
            .expect("column layout overflow");
        unsafe {
            let new_data = if self.capacity == 0 {
                alloc::alloc(new_layout)
            } else {
                let old_layout = self
                    .layout_for_capacity(self.capacity)
                    .expect("old layout must be valid");
                alloc::realloc(self.data, old_layout, new_layout.size())
            };
            assert!(!new_data.is_null(), "allocation failed");
            self.data = new_data;
        }
        self.capacity = new_cap;
    }

    #[inline]
    fn ptr_at(&self, index: usize) -> *mut u8 {
        debug_assert!(index < self.len);
        if self.item_size == 0 {
            // ZST -- return a dangling but aligned pointer.
            return self.item_align as *mut u8;
        }
        unsafe { self.data.add(index * self.item_size) }
    }

    // -- public typed access (through raw pointers) -------------------------

    /// Push a value onto the end of the column.
    ///
    /// # Safety
    ///
    /// `value_ptr` must point to a valid, initialised instance of the type
    /// described by the column's component info. Ownership is *moved* into the
    /// column (the caller must not drop the source).
    pub unsafe fn push_raw(&mut self, value_ptr: *const u8) {
        self.grow_if_needed();
        if self.item_size > 0 {
            let dst = self.data.add(self.len * self.item_size);
            ptr::copy_nonoverlapping(value_ptr, dst, self.item_size);
        }
        self.len += 1;
    }

    /// Get a raw pointer to the element at `index`.
    ///
    /// # Safety
    ///
    /// `index` must be less than `self.len`.
    #[inline]
    pub unsafe fn get_raw(&self, index: usize) -> *const u8 {
        self.ptr_at(index)
    }

    /// Get a mutable raw pointer to the element at `index`.
    ///
    /// # Safety
    ///
    /// `index` must be less than `self.len`.
    #[inline]
    pub unsafe fn get_raw_mut(&mut self, index: usize) -> *mut u8 {
        self.ptr_at(index)
    }

    /// Swap-remove the element at `index`, moving the last element into its
    /// place (if it wasn't the last). The removed element is dropped via
    /// `vtable.drop_fn`.
    ///
    /// # Safety
    ///
    /// `index` must be less than `self.len`. `vtable` must describe the actual
    /// component type stored in this column.
    pub unsafe fn swap_remove(&mut self, index: usize, vtable: &ComponentVtable) {
        debug_assert!(index < self.len);
        let last = self.len - 1;
        if self.item_size > 0 {
            (vtable.drop_fn)(self.ptr_at(index));
            if index != last {
                let src = self.ptr_at(last);
                let dst = self.data.add(index * self.item_size);
                ptr::copy_nonoverlapping(src, dst, self.item_size);
            }
        }
        self.len -= 1;
    }

    /// Swap-remove the element at `index` *without* dropping it, instead
    /// copying its bytes to `out_ptr`. The last element is moved into the gap.
    ///
    /// # Safety
    ///
    /// `index` must be less than `self.len`. `out_ptr` must have room for
    /// `item_size` bytes and must be properly aligned for the component type.
    pub unsafe fn swap_remove_and_move(&mut self, index: usize, out_ptr: *mut u8) {
        debug_assert!(index < self.len);
        let last = self.len - 1;
        if self.item_size > 0 {
            ptr::copy_nonoverlapping(self.ptr_at(index), out_ptr, self.item_size);
            if index != last {
                let src = self.ptr_at(last);
                let dst = self.data.add(index * self.item_size);
                ptr::copy_nonoverlapping(src, dst, self.item_size);
            }
        }
        self.len -= 1;
    }

    /// Drop all remaining elements using `vtable.drop_fn`, then deallocate.
    ///
    /// # Safety
    ///
    /// `vtable` must describe the type stored in this column.
    pub unsafe fn drop_all(&mut self, vtable: &ComponentVtable) {
        for i in 0..self.len {
            if self.item_size > 0 {
                (vtable.drop_fn)(self.ptr_at(i));
            }
        }
        if self.item_size > 0 && self.capacity > 0 {
            let layout = self
                .layout_for_capacity(self.capacity)
                .expect("layout must be valid");
            alloc::dealloc(self.data, layout);
        }
        self.data = ptr::null_mut();
        self.len = 0;
        self.capacity = 0;
    }
}

impl std::fmt::Debug for Column {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Column")
            .field("len", &self.len)
            .field("capacity", &self.capacity)
            .field("item_size", &self.item_size)
            .field("item_align", &self.item_align)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Archetype
// ---------------------------------------------------------------------------

/// Column entry: the Column data plus its vtable for drop/clone.
struct ColumnEntry {
    column: Column,
    vtable: ComponentVtable,
}

impl std::fmt::Debug for ColumnEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnEntry")
            .field("column", &self.column)
            .field("vtable", &self.vtable)
            .finish()
    }
}

/// An archetype stores all entities that share the exact same set of
/// component types. Components are laid out SoA-style: one [`Column`] per
/// type, plus a parallel entity ID vector.
///
/// Columns are stored in a `Vec` sorted by `ComponentTypeId` for
/// deterministic iteration order and cache-friendly binary-search lookups.
#[derive(Debug)]
pub struct Archetype {
    /// Unique identifier of this archetype.
    id: ArchetypeId,
    /// Sorted list of component types in this archetype.
    component_types: Vec<ComponentTypeId>,
    /// One column per component type, sorted by `ComponentTypeId`.
    /// Invariant: `columns[i].0 == component_types[i]` for all `i`.
    columns: Vec<(ComponentTypeId, ColumnEntry)>,
    /// Parallel entity ID vector (same indexing as columns).
    entities: Vec<EntityId>,
}

impl Archetype {
    /// Create a new, empty archetype.
    ///
    /// `component_types` should already be sorted. `infos` and `vtables`
    /// must correspond 1:1 with `component_types`.
    pub fn new(
        id: ArchetypeId,
        component_types: Vec<ComponentTypeId>,
        infos: Vec<ComponentInfo>,
        vtables: Vec<ComponentVtable>,
    ) -> Self {
        let mut columns: Vec<(ComponentTypeId, ColumnEntry)> = infos
            .iter()
            .zip(vtables)
            .map(|(info, vtable)| {
                (
                    info.id,
                    ColumnEntry {
                        column: Column::new(info),
                        vtable,
                    },
                )
            })
            .collect();
        // Sort by ComponentTypeId for deterministic order and binary search.
        columns.sort_by_key(|(id, _)| *id);

        Self {
            id,
            component_types,
            columns,
            entities: Vec::new(),
        }
    }

    /// Binary search for a column entry by ComponentTypeId.
    #[inline]
    fn column_index(&self, type_id: ComponentTypeId) -> Option<usize> {
        self.columns
            .binary_search_by_key(&type_id, |(id, _)| *id)
            .ok()
    }

    /// The archetype's unique ID.
    #[inline]
    pub fn id(&self) -> ArchetypeId {
        self.id
    }

    /// The sorted set of component type IDs that define this archetype.
    #[inline]
    pub fn component_types(&self) -> &[ComponentTypeId] {
        &self.component_types
    }

    /// Whether this archetype contains the given component type.
    #[inline]
    pub fn has_component(&self, type_id: ComponentTypeId) -> bool {
        self.column_index(type_id).is_some()
    }

    /// Number of entities stored in this archetype.
    #[inline]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether this archetype is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// The entity IDs stored in this archetype.
    #[inline]
    pub fn entities(&self) -> &[EntityId] {
        &self.entities
    }

    /// Add an entity with its component data. The caller must provide exactly
    /// one value for every component type in this archetype.
    ///
    /// # Safety
    ///
    /// Each `(ComponentTypeId, *const u8)` pair must point to a valid,
    /// initialized value of the correct component type, and ownership of each
    /// value is transferred into the archetype.
    pub unsafe fn add_entity(
        &mut self,
        entity: EntityId,
        components: &[(ComponentTypeId, *const u8)],
    ) -> usize {
        let row = self.entities.len();
        self.entities.push(entity);
        for &(type_id, value_ptr) in components {
            let idx = self
                .column_index(type_id)
                .expect("component type not in archetype");
            self.columns[idx].1.column.push_raw(value_ptr);
        }
        row
    }

    /// Remove the entity at `row`, using swap-remove to keep storage dense.
    ///
    /// Returns the entity that was moved into `row` (the previous last entity)
    /// if any swap occurred, or `None` if the removed entity was the last.
    pub fn remove_entity(&mut self, row: usize) -> Option<EntityId> {
        let last = self.entities.len() - 1;
        self.entities.swap_remove(row);
        for (_type_id, entry) in &mut self.columns {
            unsafe {
                entry.column.swap_remove(row, &entry.vtable);
            }
        }
        if row < last {
            Some(self.entities[row])
        } else {
            None
        }
    }

    /// Remove the entity at `row` *without* dropping its components; instead,
    /// copy the component bytes into properly aligned temporary buffers
    /// allocated with `std::alloc::alloc`. The `out` closure is called once
    /// per component type with `(ComponentTypeId, *const u8, &ComponentVtable)`
    /// where the pointer points to the properly aligned temp buffer. The caller
    /// takes ownership of the buffer and must deallocate it using
    /// `std::alloc::dealloc` with the same layout.
    ///
    /// Returns the entity that was swapped into `row`, if any.
    ///
    /// # Safety
    ///
    /// The caller takes ownership of the extracted component values and must
    /// arrange for them to be properly dropped or moved, and must deallocate
    /// the temp buffers with the correct layout.
    pub unsafe fn remove_entity_and_move(
        &mut self,
        row: usize,
        mut out: impl FnMut(ComponentTypeId, *const u8, &ComponentVtable),
    ) -> Option<EntityId> {
        let last = self.entities.len() - 1;
        self.entities.swap_remove(row);

        for (type_id, entry) in &mut self.columns {
            let size = entry.vtable.size;
            let align = entry.vtable.align;
            if size > 0 {
                let layout =
                    Layout::from_size_align(size, align).expect("invalid component layout");
                let buf = alloc::alloc(layout);
                assert!(!buf.is_null(), "allocation failed");
                entry.column.swap_remove_and_move(row, buf);
                out(*type_id, buf, &entry.vtable);
                // Caller takes ownership of buf -- do NOT dealloc here.
            } else {
                // ZST: no allocation needed, just do the swap_remove bookkeeping.
                let zst_ptr = align as *mut u8;
                entry.column.swap_remove_and_move(row, zst_ptr);
                out(*type_id, zst_ptr, &entry.vtable);
            }
        }

        if row < last {
            Some(self.entities[row])
        } else {
            None
        }
    }

    /// Get a reference to a component value.
    ///
    /// # Safety
    ///
    /// `T` must be the actual type stored in the column for `type_id`.
    pub unsafe fn get_component<T: 'static>(
        &self,
        row: usize,
        type_id: ComponentTypeId,
    ) -> Option<&T> {
        let idx = self.column_index(type_id)?;
        let entry = &self.columns[idx].1;
        if row >= entry.column.len() {
            return None;
        }
        Some(&*(entry.column.get_raw(row) as *const T))
    }

    /// Get a mutable reference to a component value.
    ///
    /// # Safety
    ///
    /// `T` must be the actual type stored in the column for `type_id`.
    pub unsafe fn get_component_mut<T: 'static>(
        &mut self,
        row: usize,
        type_id: ComponentTypeId,
    ) -> Option<&mut T> {
        let idx = self.column_index(type_id)?;
        let entry = &mut self.columns[idx].1;
        if row >= entry.column.len() {
            return None;
        }
        Some(&mut *(entry.column.get_raw_mut(row) as *mut T))
    }

    /// Get a raw mutable pointer to a component value at `row` for the given
    /// `type_id`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the pointer is used with the correct concrete
    /// type and that no aliasing occurs.
    pub unsafe fn get_component_raw_mut(
        &mut self,
        row: usize,
        type_id: ComponentTypeId,
    ) -> Option<*mut u8> {
        let idx = self.column_index(type_id)?;
        let entry = &mut self.columns[idx].1;
        if row >= entry.column.len() {
            return None;
        }
        Some(entry.column.get_raw_mut(row))
    }

    /// Get the vtable for a given component type in this archetype.
    pub fn vtable(&self, type_id: ComponentTypeId) -> Option<&ComponentVtable> {
        let idx = self.column_index(type_id)?;
        Some(&self.columns[idx].1.vtable)
    }
}

impl Drop for Archetype {
    fn drop(&mut self) {
        for (_type_id, entry) in &mut self.columns {
            unsafe {
                entry.column.drop_all(&entry.vtable);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::ComponentRegistry;

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

    fn setup() -> (ComponentRegistry, ComponentTypeId, ComponentTypeId) {
        let mut reg = ComponentRegistry::new();
        let pos_id = reg.register::<Pos>("position");
        let vel_id = reg.register::<Vel>("velocity");
        (reg, pos_id, vel_id)
    }

    #[test]
    fn add_and_get_component() {
        let (reg, pos_id, _vel_id) = setup();
        let mut types = vec![pos_id];
        types.sort();
        let infos: Vec<_> = types
            .iter()
            .map(|id| reg.get_info(*id).unwrap().clone())
            .collect();
        let vtables = vec![ComponentVtable::new::<Pos>()];
        let mut arch = Archetype::new(ArchetypeId(0), types, infos, vtables);

        let entity = EntityId::new(0, 0);
        let pos = Pos { x: 1.0, y: 2.0 };
        unsafe {
            let ptr = &pos as *const Pos as *const u8;
            arch.add_entity(entity, &[(pos_id, ptr)]);
        }

        assert_eq!(arch.len(), 1);
        unsafe {
            let got: &Pos = arch.get_component(0, pos_id).unwrap();
            assert_eq!(got, &Pos { x: 1.0, y: 2.0 });
        }
    }

    #[test]
    fn remove_entity_from_archetype() {
        let (reg, pos_id, _vel_id) = setup();
        let mut types = vec![pos_id];
        types.sort();
        let infos: Vec<_> = types
            .iter()
            .map(|id| reg.get_info(*id).unwrap().clone())
            .collect();
        let vtables = vec![ComponentVtable::new::<Pos>()];
        let mut arch = Archetype::new(ArchetypeId(0), types, infos, vtables);

        let e0 = EntityId::new(0, 0);
        let e1 = EntityId::new(1, 0);

        let p0 = Pos { x: 0.0, y: 0.0 };
        let p1 = Pos { x: 1.0, y: 1.0 };
        unsafe {
            arch.add_entity(e0, &[(pos_id, &p0 as *const Pos as *const u8)]);
            arch.add_entity(e1, &[(pos_id, &p1 as *const Pos as *const u8)]);
        }

        assert_eq!(arch.len(), 2);
        let swapped = arch.remove_entity(0);
        assert_eq!(swapped, Some(e1));
        assert_eq!(arch.len(), 1);
        unsafe {
            let got: &Pos = arch.get_component(0, pos_id).unwrap();
            assert_eq!(got, &Pos { x: 1.0, y: 1.0 });
        }
    }

    #[test]
    fn archetype_with_multiple_components() {
        let (reg, pos_id, vel_id) = setup();
        let mut types = vec![pos_id, vel_id];
        types.sort();
        let infos: Vec<_> = types
            .iter()
            .map(|id| reg.get_info(*id).unwrap().clone())
            .collect();
        let vtables = vec![ComponentVtable::new::<Pos>(), ComponentVtable::new::<Vel>()];
        let mut arch = Archetype::new(ArchetypeId(0), types, infos, vtables);

        let entity = EntityId::new(0, 0);
        let pos = Pos { x: 5.0, y: 6.0 };
        let vel = Vel { dx: 1.0, dy: -1.0 };
        unsafe {
            let mut comps = vec![
                (pos_id, &pos as *const Pos as *const u8),
                (vel_id, &vel as *const Vel as *const u8),
            ];
            comps.sort_by_key(|(id, _)| *id);
            arch.add_entity(entity, &comps);
        }

        assert_eq!(arch.len(), 1);
        unsafe {
            assert_eq!(
                arch.get_component::<Pos>(0, pos_id).unwrap(),
                &Pos { x: 5.0, y: 6.0 }
            );
            assert_eq!(
                arch.get_component::<Vel>(0, vel_id).unwrap(),
                &Vel { dx: 1.0, dy: -1.0 }
            );
        }
    }
}
