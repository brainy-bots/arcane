//! Integration tests for L1 short-term persistence (parking).
//! These tests require a live Redis instance.

use arcane_core::replication_channel::EntityStateEntry;
use arcane_core::Vec3;
use arcane_infra::parking::{park_entity, unpark_entity, ParkingConfig};
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
fn test_rejoin_within_ttl_restores_user_data() {
    // Test that reconnect within ARCANE_RECONNECT_TTL_SECS restores user_data
    // even if the anti-resurrection tombstone has been pruned.
    let redis_url = "redis://127.0.0.1:6379";
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Redis not available, skipping test");
            return;
        }
    };

    if client.get_connection().is_err() {
        eprintln!("Redis not available, skipping test");
        return;
    }

    let config = ParkingConfig {
        reconnect_ttl_secs: 120, // TTL is long
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
        "player_name": "Alice",
        "level": 99,
        "gold": 1000
    });

    // Park the entity
    park_entity(redis_url, &config, entity_id, &entry);

    // Simulate scenario: user_data should be restored from parked snapshot
    // even if the tombstone (DEPARTED_TTL_TICKS) has expired.
    let restored = unpark_entity(redis_url, entity_id);
    assert!(
        restored.is_some(),
        "Entity should be restorable within TTL window"
    );

    let snapshot = restored.unwrap();
    let restored_user_data = snapshot.get("user_data").expect("user_data should exist");
    assert_eq!(
        restored_user_data["player_name"].as_str().unwrap(),
        "Alice",
        "User data should be preserved exactly"
    );
    assert_eq!(
        restored_user_data["level"].as_i64().unwrap(),
        99,
        "Level should be preserved"
    );
    assert_eq!(
        restored_user_data["gold"].as_i64().unwrap(),
        1000,
        "Gold should be preserved"
    );
}

#[test]
fn test_rejoin_after_ttl_fresh() {
    // Test that reconnect after ARCANE_RECONNECT_TTL_SECS expires
    // results in a fresh entity (no rehydration).
    let redis_url = "redis://127.0.0.1:6379";
    let client = match redis::Client::open(redis_url) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Redis not available, skipping test");
            return;
        }
    };

    if client.get_connection().is_err() {
        eprintln!("Redis not available, skipping test");
        return;
    }

    // Use very short TTL for test (1 second)
    let config = ParkingConfig {
        reconnect_ttl_secs: 1,
    };

    let entity_id = Uuid::new_v4();
    let cluster_id = Uuid::new_v4();
    let mut entry = EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(5.0, 6.0, 7.0),
        Vec3::new(0.1, 0.1, 0.1),
    );
    entry.user_data = serde_json::json!({
        "player_name": "Bob",
        "level": 50
    });

    // Park the entity with short TTL
    park_entity(redis_url, &config, entity_id, &entry);

    // Wait for TTL to expire
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Attempt unpark after expiry
    let restored = unpark_entity(redis_url, entity_id);
    assert!(
        restored.is_none(),
        "Entity should not be found after TTL expiry; reconnect should be fresh"
    );
}
