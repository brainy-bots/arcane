/// Dynamic named features for one entity. Games add/delete parameters freely;
/// the library NEVER enumerates game parameter names. Absent means absent (no defaults).
pub type FeatureMap = std::collections::BTreeMap<String, f64>;

/// One entity's state as published by its owning cluster: spine kinematics
/// (well-known) + dynamic features (game-defined names).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityRecord {
    pub entity_id: uuid::Uuid,
    pub cluster_id: uuid::Uuid,
    pub position: arcane_core::Vec2,
    pub velocity: arcane_core::Vec2,
    #[serde(default, skip_serializing_if = "FeatureMap::is_empty")]
    pub features: FeatureMap,
}

/// Where the Manager reads entity state from. The data of record is external
/// (production: Redis keys written by each owning cluster); the Manager holds
/// no store — only derived structures. This trait is the sans-IO boundary.
pub trait IEntityStateSource {
    /// Latest known records for all entities, grouped however the impl likes.
    fn fetch_all(&self) -> Vec<EntityRecord>;
}

/// In-memory impl for tests and in-process drivers: set/replace records directly.
#[derive(Default)]
pub struct InMemoryStateSource {
    records: std::sync::Mutex<std::collections::HashMap<uuid::Uuid, EntityRecord>>,
}

impl InMemoryStateSource {
    /// Create a new empty in-memory state source.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace an entity record.
    pub fn upsert(&self, record: EntityRecord) {
        let mut records = self.records.lock().unwrap();
        records.insert(record.entity_id, record);
    }

    /// Remove an entity record by ID.
    pub fn remove(&self, entity_id: &uuid::Uuid) {
        let mut records = self.records.lock().unwrap();
        records.remove(entity_id);
    }
}

impl IEntityStateSource for InMemoryStateSource {
    fn fetch_all(&self) -> Vec<EntityRecord> {
        let records = self.records.lock().unwrap();
        let mut sorted: Vec<_> = records.values().cloned().collect();
        sorted.sort_by_key(|r| r.entity_id);
        sorted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_with_features() {
        let mut features = FeatureMap::new();
        features.insert("squad".to_string(), 1.5);
        features.insert("tier".to_string(), 3.0);

        let record = EntityRecord {
            entity_id: uuid::Uuid::nil(),
            cluster_id: uuid::Uuid::nil(),
            position: arcane_core::Vec2::new(1.0, 2.0),
            velocity: arcane_core::Vec2::new(0.5, -0.5),
            features,
        };

        let json = serde_json::to_string(&record).expect("serialize");
        let deserialized: EntityRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, deserialized);
    }

    #[test]
    fn serde_roundtrip_empty_features() {
        let record = EntityRecord {
            entity_id: uuid::Uuid::nil(),
            cluster_id: uuid::Uuid::nil(),
            position: arcane_core::Vec2::new(1.0, 2.0),
            velocity: arcane_core::Vec2::new(0.5, -0.5),
            features: FeatureMap::new(),
        };

        let json = serde_json::to_string(&record).expect("serialize");
        assert!(!json.contains("features"));

        let deserialized: EntityRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(record, deserialized);
    }

    #[test]
    fn in_memory_upsert_remove_fetch_determinism() {
        let source = InMemoryStateSource::new();

        let record1 = EntityRecord {
            entity_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            cluster_id: uuid::Uuid::nil(),
            position: arcane_core::Vec2::new(1.0, 2.0),
            velocity: arcane_core::Vec2::new(0.0, 0.0),
            features: FeatureMap::new(),
        };

        let record2 = EntityRecord {
            entity_id: uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            cluster_id: uuid::Uuid::nil(),
            position: arcane_core::Vec2::new(3.0, 4.0),
            velocity: arcane_core::Vec2::new(1.0, 1.0),
            features: FeatureMap::new(),
        };

        source.upsert(record1.clone());
        source.upsert(record2.clone());

        let all = source.fetch_all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].entity_id, record2.entity_id);
        assert_eq!(all[1].entity_id, record1.entity_id);

        source.remove(&record2.entity_id);
        let remaining = source.fetch_all();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].entity_id, record1.entity_id);
    }

    #[test]
    fn upsert_replaces_existing_record() {
        let source = InMemoryStateSource::new();
        let id = uuid::Uuid::nil();

        let record1 = EntityRecord {
            entity_id: id,
            cluster_id: uuid::Uuid::nil(),
            position: arcane_core::Vec2::new(1.0, 2.0),
            velocity: arcane_core::Vec2::new(0.0, 0.0),
            features: FeatureMap::new(),
        };

        let record2 = EntityRecord {
            entity_id: id,
            cluster_id: uuid::Uuid::nil(),
            position: arcane_core::Vec2::new(3.0, 4.0),
            velocity: arcane_core::Vec2::new(1.0, 1.0),
            features: FeatureMap::new(),
        };

        source.upsert(record1);
        let all = source.fetch_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].position, arcane_core::Vec2::new(1.0, 2.0));

        source.upsert(record2);
        let all = source.fetch_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].position, arcane_core::Vec2::new(3.0, 4.0));
    }
}
