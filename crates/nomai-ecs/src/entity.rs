//! Entity identifiers and allocation.
//!
//! An [`EntityId`] is a 64-bit handle that packs a *generation* counter in the
//! high 32 bits and an *index* in the low 32 bits. The generation is bumped
//! every time an index is recycled, which allows immediate stale-ID detection.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;

// ---------------------------------------------------------------------------
// EntityId
// ---------------------------------------------------------------------------

/// A generational entity identifier.
///
/// Layout: `[generation: u32 | index: u32]`
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId(u64);

impl EntityId {
    /// Construct an `EntityId` from an index and generation.
    #[inline]
    pub fn new(index: u32, generation: u32) -> Self {
        Self((generation as u64) << 32 | index as u64)
    }

    /// The index portion (low 32 bits).
    #[inline]
    pub fn index(self) -> u32 {
        self.0 as u32
    }

    /// The generation portion (high 32 bits).
    #[inline]
    pub fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    /// Raw `u64` representation.
    #[inline]
    pub fn to_raw(self) -> u64 {
        self.0
    }

    /// Reconstruct from a raw `u64`.
    #[inline]
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

impl fmt::Debug for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EntityId({}v{})", self.index(), self.generation())
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.index(), self.generation())
    }
}

// ---------------------------------------------------------------------------
// EntityAllocator
// ---------------------------------------------------------------------------

/// Allocates and recycles [`EntityId`]s with generational tracking.
///
/// Free indices are kept in a FIFO queue so that generations are spread out
/// over time rather than concentrated on a hot index.
#[derive(Debug)]
pub struct EntityAllocator {
    /// Current generation for each index slot.
    generations: Vec<u32>,
    /// Whether the slot is currently alive.
    alive: Vec<bool>,
    /// Free-list of recyclable indices (FIFO queue).
    free_indices: VecDeque<u32>,
}

impl EntityAllocator {
    /// Create a new, empty allocator.
    pub fn new() -> Self {
        Self {
            generations: Vec::new(),
            alive: Vec::new(),
            free_indices: VecDeque::new(),
        }
    }

    /// Allocate a fresh [`EntityId`].
    ///
    /// If a recycled index is available it will be reused with an incremented
    /// generation; otherwise a brand-new index is created.
    pub fn allocate(&mut self) -> EntityId {
        if let Some(index) = self.free_indices.pop_front() {
            // Reuse recycled index -- generation was already bumped on despawn.
            self.alive[index as usize] = true;
            EntityId::new(index, self.generations[index as usize])
        } else {
            let index = self.generations.len() as u32;
            self.generations.push(0);
            self.alive.push(true);
            EntityId::new(index, 0)
        }
    }

    /// Deallocate (despawn) an entity, incrementing the generation for that
    /// index so that any outstanding handles become stale.
    ///
    /// Returns `true` if the entity was alive and is now despawned,
    /// `false` if it was already dead or had a stale generation.
    pub fn deallocate(&mut self, id: EntityId) -> bool {
        let idx = id.index() as usize;
        if idx >= self.generations.len() {
            return false;
        }
        if self.generations[idx] != id.generation() {
            return false;
        }
        if !self.alive[idx] {
            return false;
        }
        self.alive[idx] = false;
        self.generations[idx] = self.generations[idx].wrapping_add(1);
        self.free_indices.push_back(id.index());
        true
    }

    /// Returns `true` if `id` refers to a currently alive entity whose
    /// generation matches the allocator's current generation for that index.
    pub fn is_alive(&self, id: EntityId) -> bool {
        let idx = id.index() as usize;
        if idx >= self.generations.len() {
            return false;
        }
        self.alive[idx] && self.generations[idx] == id.generation()
    }

    /// Total number of currently alive entities.
    pub fn alive_count(&self) -> usize {
        self.alive.iter().filter(|&&a| a).count()
    }

    /// Capture the allocator state for snapshot/restore.
    ///
    /// Returns `(generations, alive, free_indices)` as owned vectors.
    pub fn snapshot_state(&self) -> (Vec<u32>, Vec<bool>, Vec<u32>) {
        let free: Vec<u32> = self.free_indices.iter().copied().collect();
        (self.generations.clone(), self.alive.clone(), free)
    }

    /// Restore allocator state from a previously captured snapshot.
    ///
    /// This reconstructs the allocator exactly as it was at the time of the
    /// snapshot, preserving generations, alive flags, and the free-list order.
    pub fn restore_from_snapshot(
        generations: Vec<u32>,
        alive: Vec<bool>,
        free_indices: Vec<u32>,
    ) -> Self {
        Self {
            generations,
            alive,
            free_indices: VecDeque::from(free_indices),
        }
    }
}

impl Default for EntityAllocator {
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

    #[test]
    fn allocate_unique_ids() {
        let mut alloc = EntityAllocator::new();
        let ids: Vec<EntityId> = (0..100).map(|_| alloc.allocate()).collect();
        // All indices unique.
        let mut indices: Vec<u32> = ids.iter().map(|id| id.index()).collect();
        indices.sort();
        indices.dedup();
        assert_eq!(indices.len(), 100);
    }

    #[test]
    fn generation_increments_on_recycle() {
        let mut alloc = EntityAllocator::new();
        let e0 = alloc.allocate();
        assert_eq!(e0.generation(), 0);
        assert!(alloc.deallocate(e0));
        let e1 = alloc.allocate();
        // Same index, higher generation.
        assert_eq!(e1.index(), e0.index());
        assert_eq!(e1.generation(), 1);
    }

    #[test]
    fn stale_id_detection() {
        let mut alloc = EntityAllocator::new();
        let e0 = alloc.allocate();
        assert!(alloc.is_alive(e0));
        assert!(alloc.deallocate(e0));
        assert!(!alloc.is_alive(e0), "stale ID should not be alive");
        let _e1 = alloc.allocate(); // recycles same index
        assert!(
            !alloc.is_alive(e0),
            "stale ID still not alive after recycle"
        );
    }

    #[test]
    fn double_deallocate_returns_false() {
        let mut alloc = EntityAllocator::new();
        let e = alloc.allocate();
        assert!(alloc.deallocate(e));
        assert!(!alloc.deallocate(e));
    }

    #[test]
    fn alive_count_tracks_correctly() {
        let mut alloc = EntityAllocator::new();
        let e0 = alloc.allocate();
        let _e1 = alloc.allocate();
        assert_eq!(alloc.alive_count(), 2);
        alloc.deallocate(e0);
        assert_eq!(alloc.alive_count(), 1);
    }

    #[test]
    fn entity_id_roundtrip() {
        let id = EntityId::new(42, 7);
        assert_eq!(id.index(), 42);
        assert_eq!(id.generation(), 7);
        assert_eq!(EntityId::from_raw(id.to_raw()), id);
    }
}
