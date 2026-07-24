//! IPersistence trait (IF-04) — durable entity state persistence seam (L2, epic #305).
//!
//! Defines a pluggable contract for storing and restoring entity state snapshots.
//! Implementations provide bucket-4 durable persistence; the platform calls them at key
//! lifecycle moments (snapshot on tick, final write on leave, load on join).
//!
//! ## Three-level ladder
//!
//! | Level | Behavior | Responsibility |
//! |-------|----------|-----------------|
//! | **L0** | No persistence | N/A |
//! | **L1** (parking) | Short-term Redis snapshots (TTL-based reconnection) | `crate::parking` |
//! | **L2** (durable) | Long-term implementation via this trait (SpacetimeDB, custom DB, etc.) | [`IPersistence`] |
//!
//! The L2 seam lets games choose their durability backend without coupling to SpacetimeDB.
//! Multi-level orchestration ensures L1 (fresher) takes precedence over L2 on rehydration.

use crate::replication_channel::EntityStateEntry;
use uuid::Uuid;

/// Contract for storing and retrieving durable entity state snapshots (L2 persistence, epic #305).
///
/// **Lifecycle:**
/// - `should_snapshot()` — cheap cadence check to avoid building full snapshots on non-persist ticks.
/// - `snapshot()` — throttled cadence (impl-specific); mirrors live state into durable backend.
/// - `persist_final()` — guaranteed final write at session end (when entity leaves). Blocking OK;
///   implementations should retry internally and log failures.
/// - `load()` — restore entity from durable storage (fallback when L1 parked snapshot absent).
///
/// **Bucket model:** stores buckets 1 (spine: position, velocity, id) and 2 (user_data).
/// Bucket 3 (cluster-local) is never persisted; bucket 4 (durable) is the implementation's responsibility.
pub trait IPersistence: Send + Sync {
    /// Cheap cadence check: returns true if this tick should produce a snapshot.
    /// Caller uses this to avoid expensive snapshot clones on non-persist ticks.
    /// Default: always true (every tick snapshots). Implementations with throttled cadence
    /// should override this for the performance optimization.
    fn should_snapshot(&self, tick: u64) -> bool {
        let _ = tick;
        true
    }

    /// Throttled mirror of live state. Called at a cadence determined by the implementation
    /// (e.g., every N ticks), or always if `should_snapshot()` returns false. Implementations
    /// may drop snapshots if the queue is full. Non-blocking preferred; should not hold locks.
    fn snapshot(&self, entries: &[EntityStateEntry]);

    /// Guaranteed final write at session end (entity is leaving). Blocking OK (leave is rare).
    /// Implementations should retry internally (e.g., 3 attempts with backoff) and log
    /// failures **loudly** — a lost final write is data loss.
    /// Called BEFORE [`crate::ArcaneNode::remove_entity`] to guarantee ordering.
    fn persist_final(&self, entry: &EntityStateEntry);

    /// Durable rehydration: retrieve the persisted state for an entity, if one exists.
    /// Returns the reconstructed [`EntityStateEntry`] or None. Used as a fallback
    /// when L1 (parked snapshot) is absent.
    /// Non-blocking preferred; implementations may use sync HTTP or similar.
    fn load(&self, entity_id: Uuid) -> Option<EntityStateEntry>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Vec3;

    struct MockPersistence {
        snapshot_calls: std::sync::Arc<std::sync::Mutex<Vec<Vec<EntityStateEntry>>>>,
        persist_final_calls: std::sync::Arc<std::sync::Mutex<Vec<EntityStateEntry>>>,
        load_data:
            std::sync::Arc<std::sync::Mutex<std::collections::HashMap<Uuid, EntityStateEntry>>>,
    }

    impl MockPersistence {
        fn new() -> Self {
            Self {
                snapshot_calls: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                persist_final_calls: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
                load_data: std::sync::Arc::new(std::sync::Mutex::new(
                    std::collections::HashMap::new(),
                )),
            }
        }
    }

    impl IPersistence for MockPersistence {
        fn snapshot(&self, entries: &[EntityStateEntry]) {
            let mut calls = self.snapshot_calls.lock().unwrap();
            calls.push(entries.to_vec());
        }

        fn persist_final(&self, entry: &EntityStateEntry) {
            let mut calls = self.persist_final_calls.lock().unwrap();
            calls.push(entry.clone());
        }

        fn load(&self, entity_id: Uuid) -> Option<EntityStateEntry> {
            let data = self.load_data.lock().unwrap();
            data.get(&entity_id).cloned()
        }
    }

    #[test]
    fn mock_persistence_snapshot_records_calls() {
        let mock = MockPersistence::new();
        let entry = EntityStateEntry::new(
            Uuid::from_u128(1),
            Uuid::nil(),
            Vec3::new(1.0, 2.0, 3.0),
            Vec3::new(0.0, 0.0, 0.0),
        );

        mock.snapshot(&[entry.clone()]);

        let calls = mock.snapshot_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].len(), 1);
        assert_eq!(calls[0][0].entity_id, entry.entity_id);
    }

    #[test]
    fn mock_persistence_persist_final_records_calls() {
        let mock = MockPersistence::new();
        let entry = EntityStateEntry::new(
            Uuid::from_u128(2),
            Uuid::nil(),
            Vec3::new(5.0, 6.0, 7.0),
            Vec3::new(0.0, 0.0, 0.0),
        );

        mock.persist_final(&entry);

        let calls = mock.persist_final_calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].entity_id, entry.entity_id);
    }

    #[test]
    fn mock_persistence_load_returns_stored_entry() {
        let mock = MockPersistence::new();
        let entity_id = Uuid::from_u128(3);
        let entry = EntityStateEntry::new(
            entity_id,
            Uuid::nil(),
            Vec3::new(10.0, 11.0, 12.0),
            Vec3::new(0.0, 0.0, 0.0),
        );

        {
            let mut data = mock.load_data.lock().unwrap();
            data.insert(entity_id, entry.clone());
        }

        let loaded = mock.load(entity_id);
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().entity_id, entity_id);
    }

    #[test]
    fn mock_persistence_load_returns_none_when_not_found() {
        let mock = MockPersistence::new();
        let loaded = mock.load(Uuid::from_u128(999));
        assert!(loaded.is_none());
    }
}
