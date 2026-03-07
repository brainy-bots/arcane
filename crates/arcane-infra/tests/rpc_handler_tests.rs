//! Tests for RPCHandler (IN-05). Define expected behavior.

use arcane_infra::RpcHandler;

#[test]
fn new_holds_port() {
    let handler = RpcHandler::new(9200);
    assert_eq!(handler.port(), 9200);
}

#[test]
fn is_running_false_before_start() {
    let handler = RpcHandler::new(9201);
    assert!(!handler.is_running(), "not running until start()");
}
