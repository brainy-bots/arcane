# WS + Channel Backpressure Validation

This note documents current backpressure behavior in `arcane-infra` and how it is validated.

## Scope

- `crates/arcane-infra/src/ws_server.rs`
- Cluster loop handoff path: `node_runner` tick output -> mpsc sender -> ws broadcast channel -> client socket writes

## Current policy

1. **Server-side state fanout is best-effort.**
   - Broadcast queue is bounded (`tokio::sync::broadcast` capacity: 256 messages).
2. **When a client lags, old frames may be dropped.**
   - `RecvError::Lagged(_)` is treated as recoverable.
   - The connection stays alive and resumes from newer state messages.
3. **Only terminal channel close ends broadcast receive path.**
   - `RecvError::Closed` ends the loop.
4. **Client update ingest is non-blocking at parse level.**
   - Invalid payloads are ignored; valid `PLAYER_STATE` messages are forwarded to the cluster tick loop via mpsc.

## Validation coverage

- Unit tests in `ws_server.rs` validate:
  - `PLAYER_STATE` parse success/failure behavior.
  - Backpressure error policy (`Lagged` continues, `Closed` stops).
- Cluster merge/cadence seam tests in `node_runner.rs` and `spacetimedb_persist.rs` validate:
  - merged-delta composition from local + neighbor snapshots.
  - persistence cadence gate logic (`tick % interval == 0` and non-empty updates).

## Residual risk and operational guidance

- Under sustained overload, slow clients can miss intermediate frames by design.
- For stricter delivery guarantees, use sequence-gap detection/recovery at protocol level (future enhancement).
- During performance runs, monitor:
  - send failures/disconnect rates,
  - cluster tick duration,
  - downstream client interpolation quality.
