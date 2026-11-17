# Testing

## Running

```sh
cargo test
cargo llvm-cov
```

## Existing Tests

- `tests/api.rs`: Integration tests covering:
    - Basic CRUD operations and monotonic timestamps.
    - Snapshot isolation (no non-repeatable reads, no phantoms).
    - Conflict detection: first-committer-wins vs disjoint commit success.
    - Concurrency: bank transfer simulation with 8 threads; asserts invariant consistency and no lost updates.
    - Crash recovery: verifies recovery from torn log writes using `MemStorage` and `crash_at`.
    - B+tree integrity: validates order 4 constraints, key sorting, and leaf depth uniformity with 10,000 keys.
- Unit tests:
    - `src/btree.rs`: B+tree structural logic (via `validate_invariants`).
    - `src/wal.rs`: Payload encoding/decoding and CRC validation.
    - `src/crc.rs`: IEEE 802.3 CRC-32 consistency (tested against `0xCBF4_3926`).