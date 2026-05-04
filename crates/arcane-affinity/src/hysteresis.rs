use std::collections::HashMap;
use uuid::Uuid;

/// Per-entity migration cooldown tracking. Prevents oscillation after a migration.
pub struct MigrationState {
    cooldowns: HashMap<Uuid, u32>,
}

impl MigrationState {
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
        }
    }

    /// Record that an entity just migrated. Sets its cooldown to cooldown_ticks.
    pub fn record_migration(&mut self, entity: Uuid, cooldown_ticks: u32) {
        self.cooldowns.insert(entity, cooldown_ticks);
    }

    /// True if entity is currently in cooldown and cannot migrate.
    pub fn is_on_cooldown(&self, entity: Uuid) -> bool {
        self.cooldowns.get(&entity).copied().unwrap_or(0) > 0
    }

    /// Decrement all cooldowns by 1. Remove entries that reach 0.
    pub fn tick(&mut self) {
        self.cooldowns.retain(|_, ticks| {
            *ticks = ticks.saturating_sub(1);
            *ticks > 0
        });
    }

    /// Remove cooldown state for an entity (on disconnect/despawn).
    pub fn remove_entity(&mut self, entity: Uuid) {
        self.cooldowns.remove(&entity);
    }

    /// Number of entities currently on cooldown. For metrics.
    pub fn cooldown_count(&self) -> usize {
        self.cooldowns.len()
    }
}

impl Default for MigrationState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u8) -> Uuid {
        Uuid::from_bytes([n; 16])
    }

    #[test]
    fn new_state_has_no_cooldowns() {
        let s = MigrationState::new();
        assert!(!s.is_on_cooldown(uuid(1)));
        assert_eq!(s.cooldown_count(), 0);
    }

    #[test]
    fn record_migration_sets_cooldown() {
        let mut s = MigrationState::new();
        s.record_migration(uuid(1), 10);
        assert!(s.is_on_cooldown(uuid(1)));
        assert_eq!(s.cooldown_count(), 1);
    }

    #[test]
    fn tick_decrements_cooldown() {
        let mut s = MigrationState::new();
        s.record_migration(uuid(1), 3);
        s.tick();
        assert!(s.is_on_cooldown(uuid(1)));
        s.tick();
        assert!(s.is_on_cooldown(uuid(1)));
        s.tick();
        assert!(!s.is_on_cooldown(uuid(1)));
        assert_eq!(s.cooldown_count(), 0);
    }

    #[test]
    fn cooldown_expires_at_exact_tick_count() {
        let mut s = MigrationState::new();
        s.record_migration(uuid(1), 50);
        for _ in 0..49 {
            s.tick();
        }
        assert!(s.is_on_cooldown(uuid(1)));
        s.tick();
        assert!(!s.is_on_cooldown(uuid(1)));
    }

    #[test]
    fn remove_entity_clears_cooldown() {
        let mut s = MigrationState::new();
        s.record_migration(uuid(1), 100);
        s.remove_entity(uuid(1));
        assert!(!s.is_on_cooldown(uuid(1)));
    }
}
