//! Component type registration and metadata.
//!
//! Every component type used in the ECS must be registered at runtime in a
//! [`ComponentRegistry`]. Registration produces a [`ComponentTypeId`] that is
//! used as the key for archetype column lookups and query matching.

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;

// ---------------------------------------------------------------------------
// ComponentTypeId
// ---------------------------------------------------------------------------

/// Opaque, lightweight identifier for a registered component type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentTypeId(pub(crate) u32);

impl fmt::Debug for ComponentTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ComponentTypeId({})", self.0)
    }
}

// ---------------------------------------------------------------------------
// ComponentInfo
// ---------------------------------------------------------------------------

/// Metadata about a registered component type.
///
/// This struct stores only safe metadata. The type-erased drop and clone
/// operations live in the archetype module's [`ComponentVtable`](crate::archetype::ComponentVtable).
#[derive(Debug, Clone)]
pub struct ComponentInfo {
    /// Unique ID assigned at registration time.
    pub id: ComponentTypeId,
    /// Human-readable name (supplied by the caller).
    pub name: String,
    /// `std::mem::size_of::<T>()`
    pub size: usize,
    /// `std::mem::align_of::<T>()`
    pub align: usize,
    /// Rust `TypeId` for runtime type checking.
    pub type_id: TypeId,
}

// ---------------------------------------------------------------------------
// ComponentRegistry
// ---------------------------------------------------------------------------

/// Registry mapping Rust types to [`ComponentTypeId`]s and their metadata.
///
/// A type can only be registered once; subsequent registrations of the same
/// Rust `TypeId` return the existing [`ComponentTypeId`].
#[derive(Debug)]
pub struct ComponentRegistry {
    /// TypeId -> ComponentTypeId for dedup.
    by_type: HashMap<TypeId, ComponentTypeId>,
    /// Name -> ComponentTypeId for lookup by string name (used by command buffer).
    by_name: HashMap<String, ComponentTypeId>,
    /// Indexed by ComponentTypeId.0.
    infos: Vec<ComponentInfo>,
}

impl ComponentRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            by_type: HashMap::new(),
            by_name: HashMap::new(),
            infos: Vec::new(),
        }
    }

    /// Register a component type under the given `name`.
    ///
    /// If the type has already been registered, the existing
    /// [`ComponentTypeId`] is returned and `name` is ignored.
    pub fn register<T>(&mut self, name: &str) -> ComponentTypeId
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        let rust_type_id = TypeId::of::<T>();
        if let Some(&existing) = self.by_type.get(&rust_type_id) {
            return existing;
        }

        let id = ComponentTypeId(self.infos.len() as u32);

        let info = ComponentInfo {
            id,
            name: name.to_owned(),
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            type_id: rust_type_id,
        };
        self.infos.push(info);
        self.by_type.insert(rust_type_id, id);
        if self.by_name.contains_key(name) {
            panic!(
                "component name '{}' is already registered for a different type",
                name
            );
        }
        self.by_name.insert(name.to_owned(), id);
        id
    }

    /// Register a dynamic component type by name.
    ///
    /// Unlike [`register`], this creates a unique [`ComponentTypeId`] per name
    /// even if the backing Rust type is the same. Used for Python/JSON
    /// components where all values are `serde_json::Value` but need distinct
    /// component identities.
    ///
    /// Returns the existing id if the name is already registered.
    pub fn register_dynamic<T>(&mut self, name: &str) -> ComponentTypeId
    where
        T: Clone + Send + Sync + 'static + serde::Serialize + for<'de> serde::Deserialize<'de>,
    {
        // If the name is already registered, return the existing id.
        if let Some(&existing) = self.by_name.get(name) {
            return existing;
        }

        let rust_type_id = TypeId::of::<T>();
        let id = ComponentTypeId(self.infos.len() as u32);

        let info = ComponentInfo {
            id,
            name: name.to_owned(),
            size: std::mem::size_of::<T>(),
            align: std::mem::align_of::<T>(),
            type_id: rust_type_id,
        };
        self.infos.push(info);
        // NOTE: We do NOT insert into by_type here. The by_type map
        // deduplicates by Rust TypeId, which we intentionally skip for
        // dynamic components so each name gets a unique ComponentTypeId.
        self.by_name.insert(name.to_owned(), id);
        id
    }

    /// Look up a component type by its Rust `TypeId`.
    pub fn lookup<T: 'static>(&self) -> Option<ComponentTypeId> {
        self.by_type.get(&TypeId::of::<T>()).copied()
    }

    /// Look up a component type by its registered string name.
    ///
    /// This is used by the command buffer to resolve component names from
    /// serialized commands back to their type IDs.
    pub fn lookup_by_name(&self, name: &str) -> Option<ComponentTypeId> {
        self.by_name.get(name).copied()
    }

    /// Get the [`ComponentInfo`] for a registered component type ID.
    pub fn get_info(&self, id: ComponentTypeId) -> Option<&ComponentInfo> {
        self.infos.get(id.0 as usize)
    }

    /// Total number of registered component types.
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    /// Whether any component types have been registered.
    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }

    /// Returns the names of all registered component types, sorted.
    pub fn registered_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.by_name.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }
}

impl Default for ComponentRegistry {
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

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct Pos {
        x: f32,
        y: f32,
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct Vel {
        dx: f32,
        dy: f32,
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = ComponentRegistry::new();
        let id = reg.register::<Pos>("position");
        assert_eq!(reg.lookup::<Pos>(), Some(id));
    }

    #[test]
    fn same_type_same_id() {
        let mut reg = ComponentRegistry::new();
        let id1 = reg.register::<Pos>("position");
        let id2 = reg.register::<Pos>("position_again");
        assert_eq!(id1, id2);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn different_types_different_ids() {
        let mut reg = ComponentRegistry::new();
        let p = reg.register::<Pos>("position");
        let v = reg.register::<Vel>("velocity");
        assert_ne!(p, v);
    }

    #[test]
    fn info_correctness() {
        let mut reg = ComponentRegistry::new();
        let id = reg.register::<Pos>("position");
        let info = reg.get_info(id).unwrap();
        assert_eq!(info.name, "position");
        assert_eq!(info.size, std::mem::size_of::<Pos>());
        assert_eq!(info.align, std::mem::align_of::<Pos>());
        assert_eq!(info.type_id, TypeId::of::<Pos>());
    }
}
