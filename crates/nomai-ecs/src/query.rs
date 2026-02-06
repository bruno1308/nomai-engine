//! Query system for iterating entities by component set.
//!
//! Queries resolve which archetypes contain the required components and yield
//! `(EntityId, (&C1, &C2, ...))` tuples for each matching row.
//!
//! ## Soundness
//!
//! Read-only queries (`&T`) use [`World::query`] which takes `&self`.
//! Mutable queries (`&mut T`) use [`World::query_mut`] which takes `&mut self`,
//! guaranteeing exclusive access to the world and preventing aliasing UB.
//!
//! The query traits and their implementations delegate all unsafe archetype
//! access to the `archetype` module (which has `#[allow(unsafe_code)]`).
//! This module itself only uses `#[allow(unsafe_code)]` on the specific
//! implementations and fetch operations that need it.

use crate::component::ComponentTypeId;
use crate::entity::EntityId;
use crate::world::World;

// ---------------------------------------------------------------------------
// QueryItem trait -- describes one element in a query tuple
// ---------------------------------------------------------------------------

/// Trait for a single element of a query fetch: `&T` (read) or `&mut T` (write).
///
/// Implementors must correctly report the component type they access and
/// produce valid references from archetype data.
pub trait QueryItem {
    /// The output type yielded per-row.
    type Item<'w>;
    /// Whether this item borrows mutably.
    const MUTABLE: bool;
    /// The concrete component type's ID.
    fn component_type_id(world: &World) -> Option<ComponentTypeId>;
    /// Fetch one item from an archetype row (immutable world reference).
    ///
    /// # Safety
    ///
    /// For `&T` items this is safe with `&World`. For `&mut T` items, the
    /// caller must guarantee exclusive access (typically via `&mut World`
    /// at a higher level).
    fn fetch(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_>;
}

// Impl for `&T` -- immutable borrow
impl<T: 'static> QueryItem for &T {
    type Item<'w> = &'w T;
    const MUTABLE: bool = false;

    fn component_type_id(world: &World) -> Option<ComponentTypeId> {
        world.component_type_id::<T>()
    }

    fn fetch(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_> {
        let type_id = world.component_type_id::<T>().unwrap();
        #[allow(unsafe_code)]
        unsafe {
            world.archetypes[archetype_idx as usize]
                .get_component::<T>(row, type_id)
                .unwrap()
        }
    }
}

// Impl for `&mut T` -- mutable borrow via pointer cast.
//
// Safety: This is only called through `World::query_mut(&mut self)` which
// guarantees exclusive access to the world. The `&mut self` receiver on
// `query_mut` prevents any concurrent borrows, making the cast from
// `*const World` to `*mut World` sound.
impl<T: 'static> QueryItem for &mut T {
    type Item<'w> = &'w mut T;
    const MUTABLE: bool = true;

    fn component_type_id(world: &World) -> Option<ComponentTypeId> {
        world.component_type_id::<T>()
    }

    fn fetch(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_> {
        let type_id = world.component_type_id::<T>().unwrap();
        // Safety: The caller (World::query_mut) holds &mut World, guaranteeing
        // exclusive access. The cast is sound because no other references exist.
        #[allow(unsafe_code)]
        unsafe {
            let world_ptr = world as *const World as *mut World;
            (*world_ptr).archetypes[archetype_idx as usize]
                .get_component_mut::<T>(row, type_id)
                .unwrap()
        }
    }
}

// ---------------------------------------------------------------------------
// Query trait -- describes a tuple of QueryItems
// ---------------------------------------------------------------------------

/// Trait for a tuple of query items: `(&A, &B)`, `(&mut A, &B)`, etc.
pub trait Query {
    /// The per-row output type.
    type Item<'w>;
    /// Whether any item in this query borrows mutably.
    const HAS_MUTABLE: bool;
    /// Collect all required component type IDs.
    fn type_ids(world: &World) -> Option<Vec<ComponentTypeId>>;
    /// Validate that no component type appears as mutable more than once.
    /// Panics if it does.
    fn validate_no_duplicate_muts(world: &World);
    /// Fetch one row.
    fn fetch_row(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_>;
}

/// Validate that no component type has overlapping mutable and immutable access.
/// Panics if the same ComponentTypeId appears as both `&mut T` and `&T`, or
/// as `&mut T` twice. This prevents aliasing UB in mutable queries.
fn validate_no_access_conflicts(items: &[(bool, Option<ComponentTypeId>)]) {
    let mut mutable_ids: Vec<ComponentTypeId> = Vec::new();
    let mut read_ids: Vec<ComponentTypeId> = Vec::new();
    for &(is_mutable, type_id) in items {
        if let Some(id) = type_id {
            if is_mutable {
                if mutable_ids.contains(&id) {
                    panic!("query contains duplicate mutable access to the same component type");
                }
                if read_ids.contains(&id) {
                    panic!(
                        "query contains overlapping read and mutable access to the same component type"
                    );
                }
                mutable_ids.push(id);
            } else {
                if mutable_ids.contains(&id) {
                    panic!(
                        "query contains overlapping read and mutable access to the same component type"
                    );
                }
                read_ids.push(id);
            }
        }
    }
}

// -- Query impls for tuples of 1..4 ----------------------------------------

impl<A: QueryItem> Query for (A,) {
    type Item<'w> = (A::Item<'w>,);
    const HAS_MUTABLE: bool = A::MUTABLE;

    fn type_ids(world: &World) -> Option<Vec<ComponentTypeId>> {
        Some(vec![A::component_type_id(world)?])
    }

    fn validate_no_duplicate_muts(_world: &World) {
        // Single item -- no duplicates possible.
    }

    fn fetch_row(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_> {
        (A::fetch(world, archetype_idx, row),)
    }
}

impl<A: QueryItem, B: QueryItem> Query for (A, B) {
    type Item<'w> = (A::Item<'w>, B::Item<'w>);
    const HAS_MUTABLE: bool = A::MUTABLE || B::MUTABLE;

    fn type_ids(world: &World) -> Option<Vec<ComponentTypeId>> {
        Some(vec![
            A::component_type_id(world)?,
            B::component_type_id(world)?,
        ])
    }

    fn validate_no_duplicate_muts(world: &World) {
        let a_id = A::component_type_id(world);
        let b_id = B::component_type_id(world);
        // Reject &mut T + &mut T
        if A::MUTABLE && B::MUTABLE && a_id.is_some() && a_id == b_id {
            panic!("query contains duplicate mutable access to the same component type");
        }
        // Reject &mut T + &T or &T + &mut T (read/write overlap on same type)
        if (A::MUTABLE || B::MUTABLE) && a_id.is_some() && a_id == b_id {
            panic!("query contains overlapping read and mutable access to the same component type");
        }
    }

    fn fetch_row(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_> {
        (
            A::fetch(world, archetype_idx, row),
            B::fetch(world, archetype_idx, row),
        )
    }
}

impl<A: QueryItem, B: QueryItem, C: QueryItem> Query for (A, B, C) {
    type Item<'w> = (A::Item<'w>, B::Item<'w>, C::Item<'w>);
    const HAS_MUTABLE: bool = A::MUTABLE || B::MUTABLE || C::MUTABLE;

    fn type_ids(world: &World) -> Option<Vec<ComponentTypeId>> {
        Some(vec![
            A::component_type_id(world)?,
            B::component_type_id(world)?,
            C::component_type_id(world)?,
        ])
    }

    fn validate_no_duplicate_muts(world: &World) {
        let ids: [(bool, Option<ComponentTypeId>); 3] = [
            (A::MUTABLE, A::component_type_id(world)),
            (B::MUTABLE, B::component_type_id(world)),
            (C::MUTABLE, C::component_type_id(world)),
        ];
        validate_no_access_conflicts(&ids);
    }

    fn fetch_row(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_> {
        (
            A::fetch(world, archetype_idx, row),
            B::fetch(world, archetype_idx, row),
            C::fetch(world, archetype_idx, row),
        )
    }
}

impl<A: QueryItem, B: QueryItem, C: QueryItem, D: QueryItem> Query for (A, B, C, D) {
    type Item<'w> = (A::Item<'w>, B::Item<'w>, C::Item<'w>, D::Item<'w>);
    const HAS_MUTABLE: bool = A::MUTABLE || B::MUTABLE || C::MUTABLE || D::MUTABLE;

    fn type_ids(world: &World) -> Option<Vec<ComponentTypeId>> {
        Some(vec![
            A::component_type_id(world)?,
            B::component_type_id(world)?,
            C::component_type_id(world)?,
            D::component_type_id(world)?,
        ])
    }

    fn validate_no_duplicate_muts(world: &World) {
        let ids: [(bool, Option<ComponentTypeId>); 4] = [
            (A::MUTABLE, A::component_type_id(world)),
            (B::MUTABLE, B::component_type_id(world)),
            (C::MUTABLE, C::component_type_id(world)),
            (D::MUTABLE, D::component_type_id(world)),
        ];
        validate_no_access_conflicts(&ids);
    }

    fn fetch_row(world: &World, archetype_idx: u32, row: usize) -> Self::Item<'_> {
        (
            A::fetch(world, archetype_idx, row),
            B::fetch(world, archetype_idx, row),
            C::fetch(world, archetype_idx, row),
            D::fetch(world, archetype_idx, row),
        )
    }
}

// ---------------------------------------------------------------------------
// QueryIter (read-only)
// ---------------------------------------------------------------------------

/// Iterator that yields `(EntityId, Q::Item)` for all matching entities.
/// Used for read-only queries via `World::query`.
pub struct QueryIter<'w, Q: Query> {
    world: &'w World,
    /// Matching archetype indices (into world.archetypes).
    archetypes: Vec<u32>,
    /// Current position: which archetype in `archetypes` and which row.
    arch_cursor: usize,
    row_cursor: usize,
    _marker: std::marker::PhantomData<Q>,
}

impl<'w, Q: Query> QueryIter<'w, Q> {
    pub(crate) fn new(world: &'w World, archetypes: Vec<u32>) -> Self {
        Self {
            world,
            archetypes,
            arch_cursor: 0,
            row_cursor: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'w, Q: Query> Iterator for QueryIter<'w, Q> {
    type Item = (EntityId, Q::Item<'w>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.arch_cursor >= self.archetypes.len() {
                return None;
            }
            let arch_idx = self.archetypes[self.arch_cursor];
            let archetype = &self.world.archetypes[arch_idx as usize];
            if self.row_cursor < archetype.len() {
                let entity = archetype.entities()[self.row_cursor];
                let item = Q::fetch_row(self.world, arch_idx, self.row_cursor);
                self.row_cursor += 1;
                return Some((entity, item));
            }
            self.arch_cursor += 1;
            self.row_cursor = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// QueryIterMut (mutable)
// ---------------------------------------------------------------------------

/// Iterator that yields `(EntityId, Q::Item)` for all matching entities.
/// Used for mutable queries via `World::query_mut`.
///
/// Safety: This borrows `&mut World` at construction, so the lifetime `'w`
/// is tied to the exclusive borrow. The internal `world` pointer is derived
/// from `&mut World` and is sound because no other references can exist.
pub struct QueryIterMut<'w, Q: Query> {
    world: &'w World,
    /// Matching archetype indices (into world.archetypes).
    archetypes: Vec<u32>,
    /// Current position: which archetype in `archetypes` and which row.
    arch_cursor: usize,
    row_cursor: usize,
    _marker: std::marker::PhantomData<Q>,
}

impl<'w, Q: Query> QueryIterMut<'w, Q> {
    /// Create a new mutable query iterator.
    ///
    /// # Safety
    ///
    /// The caller must guarantee that `world` was derived from a `&mut World`
    /// borrow, ensuring exclusive access for the lifetime `'w`.
    pub(crate) fn new(world: &'w World, archetypes: Vec<u32>) -> Self {
        Self {
            world,
            archetypes,
            arch_cursor: 0,
            row_cursor: 0,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'w, Q: Query> Iterator for QueryIterMut<'w, Q> {
    type Item = (EntityId, Q::Item<'w>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.arch_cursor >= self.archetypes.len() {
                return None;
            }
            let arch_idx = self.archetypes[self.arch_cursor];
            let archetype = &self.world.archetypes[arch_idx as usize];
            if self.row_cursor < archetype.len() {
                let entity = archetype.entities()[self.row_cursor];
                let item = Q::fetch_row(self.world, arch_idx, self.row_cursor);
                self.row_cursor += 1;
                return Some((entity, item));
            }
            self.arch_cursor += 1;
            self.row_cursor = 0;
        }
    }
}

// ---------------------------------------------------------------------------
// World query methods
// ---------------------------------------------------------------------------

impl World {
    /// Run a read-only query over all matching entities.
    ///
    /// This method takes `&self` and is only sound for queries containing
    /// `&T` items (no `&mut T`). For mutable queries, use [`query_mut`](Self::query_mut).
    ///
    /// # Panics
    ///
    /// Panics if the query contains mutable items. Use `query_mut` instead.
    ///
    /// ```ignore
    /// for (entity, (pos, vel)) in world.query::<(&Pos, &Vel)>() {
    ///     println!("{entity:?}: pos={pos:?} vel={vel:?}");
    /// }
    /// ```
    pub fn query<Q: Query>(&self) -> QueryIter<'_, Q> {
        assert!(
            !Q::HAS_MUTABLE,
            "World::query() cannot be used with mutable query items (&mut T). \
             Use World::query_mut() instead, which requires &mut self."
        );
        let type_ids = Q::type_ids(self).unwrap_or_default();
        let matching = self.matching_archetypes(&type_ids);
        let arch_indices: Vec<u32> = matching.iter().map(|id| id.0).collect();
        QueryIter::new(self, arch_indices)
    }

    /// Run a mutable query over all matching entities.
    ///
    /// This method takes `&mut self`, guaranteeing exclusive access to the
    /// world. This makes it sound to produce `&mut T` references from query
    /// items.
    ///
    /// # Panics
    ///
    /// Panics if the same component type appears as `&mut T` more than once
    /// in the query tuple.
    ///
    /// ```ignore
    /// for (entity, (pos, vel)) in world.query_mut::<(&mut Pos, &Vel)>() {
    ///     pos.x += vel.dx;
    ///     pos.y += vel.dy;
    /// }
    /// ```
    pub fn query_mut<Q: Query>(&mut self) -> QueryIterMut<'_, Q> {
        Q::validate_no_duplicate_muts(self);
        let type_ids = Q::type_ids(self).unwrap_or_default();
        let matching = self.matching_archetypes(&type_ids);
        let arch_indices: Vec<u32> = matching.iter().map(|id| id.0).collect();
        QueryIterMut::new(self, arch_indices)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::world::{ComponentBundle, World};

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
    fn query_matching_only() {
        let mut world = setup_world();

        let mut b1 = ComponentBundle::new();
        b1.add(world.registry(), Pos { x: 1.0, y: 2.0 });
        b1.add(world.registry(), Vel { dx: 3.0, dy: 4.0 });
        let e1 = world.spawn_bundle(b1);

        let _e2 = world.spawn_with(Pos { x: 10.0, y: 20.0 });

        let results: Vec<_> = world.query::<(&Pos, &Vel)>().collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, e1);
        assert_eq!(results[0].1 .0, &Pos { x: 1.0, y: 2.0 });
        assert_eq!(results[0].1 .1, &Vel { dx: 3.0, dy: 4.0 });
    }

    #[test]
    fn query_skips_missing_components() {
        let mut world = setup_world();

        for i in 0..5 {
            world.spawn_with(Pos {
                x: i as f32,
                y: 0.0,
            });
        }

        let results: Vec<_> = world.query::<(&Pos, &Vel)>().collect();
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn mutable_query_modifies() {
        let mut world = setup_world();

        let mut b = ComponentBundle::new();
        b.add(world.registry(), Pos { x: 0.0, y: 0.0 });
        b.add(world.registry(), Vel { dx: 1.0, dy: 2.0 });
        let e = world.spawn_bundle(b);

        // Use query_mut for mutable access.
        for (_entity, (pos, vel)) in world.query_mut::<(&mut Pos, &Vel)>() {
            pos.x += vel.dx;
            pos.y += vel.dy;
        }

        assert_eq!(world.get_component::<Pos>(e), Some(&Pos { x: 1.0, y: 2.0 }));
    }

    #[test]
    fn single_component_query() {
        let mut world = setup_world();
        world.spawn_with(Pos { x: 1.0, y: 2.0 });
        world.spawn_with(Pos { x: 3.0, y: 4.0 });

        let results: Vec<_> = world.query::<(&Pos,)>().collect();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn query_multiple_archetypes() {
        let mut world = setup_world();

        world.spawn_with(Pos { x: 1.0, y: 0.0 });

        let mut b = ComponentBundle::new();
        b.add(world.registry(), Pos { x: 2.0, y: 0.0 });
        b.add(world.registry(), Vel { dx: 0.0, dy: 0.0 });
        world.spawn_bundle(b);

        let results: Vec<_> = world.query::<(&Pos,)>().collect();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn query_mut_requires_mut_self() {
        // This test verifies that query_mut takes &mut self (compile-time guarantee).
        // If query_mut only took &self, mutable queries would be unsound.
        let mut world = setup_world();
        let mut b = ComponentBundle::new();
        b.add(world.registry(), Pos { x: 0.0, y: 0.0 });
        let e = world.spawn_bundle(b);

        // query_mut with a single mutable component.
        for (_entity, (pos,)) in world.query_mut::<(&mut Pos,)>() {
            pos.x = 42.0;
        }

        assert_eq!(
            world.get_component::<Pos>(e),
            Some(&Pos { x: 42.0, y: 0.0 })
        );
    }

    #[test]
    #[should_panic(expected = "cannot be used with mutable query items")]
    fn query_rejects_mutable_items() {
        let mut world = setup_world();
        world.spawn_with(Pos { x: 0.0, y: 0.0 });

        // query() with &mut T should panic.
        let _results: Vec<_> = world.query::<(&mut Pos,)>().collect();
    }
}
