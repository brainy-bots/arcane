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
