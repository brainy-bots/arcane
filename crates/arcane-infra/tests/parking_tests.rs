//! Integration tests for L1 short-term persistence (parking).
//! These tests require a live Redis instance.

use arcane_core::replication_channel::EntityStateEntry;
use arcane_core::Vec3;
use arcane_infra::parking::{is_entity_parked, park_entity, unpark_entity, ParkingConfig};
use uuid::Uuid;

#[test]
fn test_parking_config_enabled() {
    let config = ParkingConfig {
        reconnect_ttl_secs: 120,
    };
    assert!(config.is_enabled());
}

#[test]
fn test_parking_config_disabled() {
    let config = ParkingConfig {
        reconnect_ttl_secs: 0,
    };
    assert!(!config.is_enabled());
}

#[test]
fn test_park_and_unpark_entity() {
    // Skip if Redis is not available.
    let redis_url = "redis://127.0.0.1:6379";
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Redis not available, skipping parking test");
            return;
        }
    };

    if client.get_connection().is_err() {
        eprintln!("Redis not available, skipping parking test");
        return;
    }

    let config = ParkingConfig {
        reconnect_ttl_secs: 60,
    };

    let entity_id = Uuid::new_v4();
    let cluster_id = Uuid::new_v4();
    let mut entry = EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(0.5, 0.5, 0.5),
    );
    entry.user_data = serde_json::json!({
        "level": 42,
        "inventory": ["sword", "shield"]
    });

    // Park the entity
    park_entity(redis_url, &config, entity_id, &entry);

    // Unpark and verify
    let restored = unpark_entity(redis_url, entity_id);
    assert!(restored.is_some(), "Entity should be found after parking");

    let snapshot = restored.unwrap();
    assert_eq!(
        snapshot["entity_id"].as_str().unwrap(),
        entity_id.to_string()
    );
    assert_eq!(
        snapshot["cluster_id"].as_str().unwrap(),
        cluster_id.to_string()
    );

    // Verify position
    let pos = &snapshot["position"];
    assert_eq!(pos["x"].as_f64().unwrap(), 1.0);
    assert_eq!(pos["y"].as_f64().unwrap(), 2.0);
    assert_eq!(pos["z"].as_f64().unwrap(), 3.0);

    // Verify user_data
    let user_data = &snapshot["user_data"];
    assert_eq!(user_data["level"].as_i64().unwrap(), 42);
    assert!(user_data["inventory"].is_array());

    // Verify local_data is not present
    assert!(!snapshot.get("local_data").is_some());

    // Verify key is consumed (second unpark returns None)
    let second_unpark = unpark_entity(redis_url, entity_id);
    assert!(second_unpark.is_none(), "Parked entity should be consumed");
}

#[test]
fn test_parking_disabled_skips_write() {
    let redis_url = "redis://127.0.0.1:6379";
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Redis not available, skipping parking test");
            return;
        }
    };

    if client.get_connection().is_err() {
        eprintln!("Redis not available, skipping parking test");
        return;
    }

    let config = ParkingConfig {
        reconnect_ttl_secs: 0, // disabled
    };

    let entity_id = Uuid::new_v4();
    let cluster_id = Uuid::new_v4();
    let entry = EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(0.5, 0.5, 0.5),
    );

    // Park the entity (should be a no-op)
    park_entity(redis_url, &config, entity_id, &entry);

    // Try to unpark (should find nothing)
    let restored = unpark_entity(redis_url, entity_id);
    assert!(
        restored.is_none(),
        "Disabled parking should not write to Redis"
    );
}

#[test]
fn test_rehydration_gate_on_parked_key_presence() {
    // Regression test for issue #321: L1 reconnect rehydration must be gated on
    // parked-key presence (ARCANE_RECONNECT_TTL_SECS), not on anti-resurrection
    // tombstone (DEPARTED_TTL_TICKS). These are two different, decoupled lifetimes.
    //
    // Scenario: the anti-resurrection tombstone has already expired (pruned from
    // self.departed) but the parked key is still live in Redis. A reconnect should
    // still rehydrate user_data.
    //
    // This test verifies the gate logic directly at the parking layer.

    let redis_url = "redis://127.0.0.1:6379";
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Redis not available, skipping rehydration gate test");
            return;
        }
    };

    if client.get_connection().is_err() {
        eprintln!("Redis not available, skipping rehydration gate test");
        return;
    }

    let config = ParkingConfig {
        reconnect_ttl_secs: 60,
    };

    let entity_id = Uuid::new_v4();
    let cluster_id = Uuid::new_v4();
    let original_user_data = serde_json::json!({
        "level": 42,
        "inventory": ["sword", "shield"]
    });

    let mut entry = EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(0.5, 0.5, 0.5),
    );
    entry.user_data = original_user_data.clone();

    // Step 1: Park the entity snapshot.
    park_entity(redis_url, &config, entity_id, &entry);

    // Step 2: Verify the parked key exists in Redis (even though anti-resurrection
    // tombstone might have expired).
    let is_parked = is_entity_parked(redis_url, entity_id);
    assert!(
        is_parked,
        "Parked key should exist for recently parked entity"
    );

    // Step 3: Verify that unpark succeeds and restores user_data.
    let restored = unpark_entity(redis_url, entity_id);
    assert!(
        restored.is_some(),
        "Entity should be restorable when parked key is live"
    );

    let snapshot = restored.unwrap();
    let restored_user_data = snapshot.get("user_data");
    assert_eq!(
        restored_user_data,
        Some(&original_user_data),
        "user_data should be restored from parked snapshot"
    );
}

#[test]
fn test_rejoin_after_ttl_expired_fresh_entity() {
    // Regression test for issue #321: reconnect after the parked key has actually
    // expired (passed ARCANE_RECONNECT_TTL_SECS) should yield a fresh entity, not a
    // restore. In practice, Redis expiry handles this, but we verify the behavior
    // when a key has been manually removed.

    let redis_url = "redis://127.0.0.1:6379";
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Redis not available, skipping TTL expiry test");
            return;
        }
    };

    if client.get_connection().is_err() {
        eprintln!("Redis not available, skipping TTL expiry test");
        return;
    }

    let config = ParkingConfig {
        reconnect_ttl_secs: 60,
    };

    let entity_id = Uuid::new_v4();
    let cluster_id = Uuid::new_v4();
    let mut entry = EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::new(0.5, 0.5, 0.5),
    );
    entry.user_data = serde_json::json!({
        "level": 42,
        "inventory": ["sword", "shield"]
    });

    // Park the entity.
    park_entity(redis_url, &config, entity_id, &entry);

    // Verify it's parked.
    let is_parked = is_entity_parked(redis_url, entity_id);
    assert!(is_parked, "Parked key should exist after parking");

    // Simulate TTL expiry by unparkingly it (consume once).
    let _ = unpark_entity(redis_url, entity_id);

    // Verify the key is now gone (expired from Redis perspective, or consumed).
    let is_parked_after = is_entity_parked(redis_url, entity_id);
    assert!(
        !is_parked_after,
        "Parked key should be gone after unpark (consumed) or expiry"
    );

    // Attempting to unpark again should return None (fresh join).
    let fresh_unpark = unpark_entity(redis_url, entity_id);
    assert!(
        fresh_unpark.is_none(),
        "After TTL expiry, rejoin should result in fresh entity (no restore)"
    );
}
