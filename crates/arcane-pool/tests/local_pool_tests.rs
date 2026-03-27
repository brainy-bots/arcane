//! Tests for LocalPool (IN-07). Define expected behavior; implementation must satisfy these.

use arcane_core::IServerPool;
use arcane_pool::LocalPool;

#[test]
fn new_pool_has_capacity() {
    let pool = LocalPool::new(4);
    let status = pool.get_status();
    assert_eq!(status.total_capacity, 4);
}

#[test]
fn allocate_returns_handle_when_available() {
    let pool = LocalPool::new(2);
    let result = pool.allocate();
    assert!(
        result.is_ok(),
        "allocate should succeed when pool has capacity"
    );
    let handle = result.unwrap();
    assert!(!handle.host.is_empty());
    assert!(handle.ws_port > 0);
}

#[test]
fn release_makes_server_available_again() {
    let pool = LocalPool::new(1);
    let h = pool.allocate().expect("one alloc");
    let id = h.server_id;
    let release_result = pool.release(id);
    assert!(release_result.is_ok());
    let status = pool.get_status();
    assert_eq!(status.available, 1, "after release, one available again");
}

#[test]
fn allocate_exhausted_pool_returns_error() {
    let pool = LocalPool::new(1);
    let _ = pool.allocate().expect("first alloc");
    let result = pool.allocate();
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().code,
        arcane_core::PoolErrorCode::PoolExhausted
    ));
}
