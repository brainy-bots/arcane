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
    // Integration test for L1 reconnect rehydration (epic #305, issue #321).
    // Verifies that user_data is restored when a client reconnects within TTL,
    // even if the anti-resurrection tombstone has expired.
    // This test exercises the is_entity_parked check independent of departed state.

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
        reconnect_ttl_secs: 120,
    };

    let entity_id = Uuid::new_v4();
    let cluster_id = Uuid::new_v4();
    let mut entry = EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(10.0, 20.0, 30.0),
        Vec3::new(1.0, 2.0, 3.0),
    );
    entry.user_data = serde_json::json!({
        "level": 50,
        "inventory": ["sword", "shield", "potion"],
        "position_x": 10.0
    });

    // Park the entity
    park_entity(redis_url, &config, entity_id, &entry);

    // Verify the entity is parked
    use arcane_infra::parking::is_entity_parked;
    assert!(
        is_entity_parked(redis_url, entity_id),
        "Entity should be parked after park_entity"
    );

    // Simulate the scenario: entity is no longer in the departed tombstone
    // (simulating TTL expiration), but is still parked in Redis.
    // This is the gap condition described in issue #321.

    // Unpark and verify that user_data is restored
    let restored = unpark_entity(redis_url, entity_id);
    assert!(restored.is_some(), "Entity should be restorable within TTL");

    let snapshot = restored.unwrap();
    let restored_user_data = &snapshot["user_data"];

    // Verify user_data is correctly restored
    assert_eq!(restored_user_data["level"].as_i64().unwrap(), 50);
    assert_eq!(
        restored_user_data["inventory"][0].as_str().unwrap(),
        "sword"
    );
    assert_eq!(restored_user_data["position_x"].as_f64().unwrap(), 10.0);

    // Verify position and velocity are also in the snapshot
    assert_eq!(snapshot["position"]["x"].as_f64().unwrap(), 10.0);
    assert_eq!(snapshot["position"]["y"].as_f64().unwrap(), 20.0);
    assert_eq!(snapshot["position"]["z"].as_f64().unwrap(), 30.0);
    assert_eq!(snapshot["velocity"]["x"].as_f64().unwrap(), 1.0);
}

#[test]
fn test_rejoin_after_ttl_fresh() {
    // Verifies that after the parked key expires (TTL exceeded),
    // a rejoin request gets a fresh entity (no user_data restoration).

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

    // Use a very short TTL so the key expires quickly
    let config = ParkingConfig {
        reconnect_ttl_secs: 1, // 1 second TTL
    };

    let entity_id = Uuid::new_v4();
    let cluster_id = Uuid::new_v4();
    let mut entry = EntityStateEntry::new(
        entity_id,
        cluster_id,
        Vec3::new(100.0, 200.0, 300.0),
        Vec3::new(5.0, 6.0, 7.0),
    );
    entry.user_data = serde_json::json!({
        "level": 99,
        "ephemeral_data": "should_not_restore"
    });

    // Park with short TTL
    park_entity(redis_url, &config, entity_id, &entry);

    // Immediately verify it's parked
    use arcane_infra::parking::is_entity_parked;
    assert!(
        is_entity_parked(redis_url, entity_id),
        "Entity should be parked initially"
    );

    // Wait for TTL to expire
    std::thread::sleep(std::time::Duration::from_secs(2));

    // Verify the key has expired
    assert!(
        !is_entity_parked(redis_url, entity_id),
        "Entity should no longer be parked after TTL expiration"
    );

    // Attempt to unpark (should find nothing)
    let restored = unpark_entity(redis_url, entity_id);
    assert!(
        restored.is_none(),
        "Expired parked entity should not be restored"
    );
}
