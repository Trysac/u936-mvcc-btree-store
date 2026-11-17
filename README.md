# mvcc-btree-store

Embedded key-value store on a B+tree with MVCC snapshot isolation, first-committer-wins conflict detection, multiversion garbage collection, and a CRC32-checked write-ahead log (`WAL`) that recovers cleanly from torn writes.

## Problem

Applications that embed a key-value store need concurrent readers that never block writers and a store that comes back consistent after a crash, and hand-rolled maps behind a mutex give neither. Full databases are large, and many small stores silently lose updates under concurrency.

## Installation

Add `mvcc-btree-store` to your `Cargo.toml`. This library is written in pure Rust (edition 2024), relies only on the standard library, and is tested for safety.

## Usage

```rust
use mvcc_btree_store::{MemStorage, Options, Store};

let store = Store::open(Box::new(MemStorage::new()), Options::default()).unwrap();
let mut tx = store.begin().unwrap();
tx.put(b"a", b"1").unwrap();
assert_eq!(tx.commit().unwrap(), 1);

let reader = store.begin().unwrap();
assert_eq!(reader.get(b"a").unwrap(), Some(b"1".to_vec()));
```

## Public API

- `Store`: The main handle. Create with `Store::open(storage, opts)`, begin transactions with `begin()`, or checkpoint logs with `checkpoint()`. Stats available via `stats()`.
- `Txn`: A snapshot-isolated transaction. Provides `get`, `put`, `delete`, `scan`, `commit` (returns timestamp on success or `Error::WriteConflict`), and `rollback`.
- `Storage` trait: Abstracts file or memory backends. `FileStorage` handles atomic replacement with temporary files.
- `Error`: Includes `write_conflict`, `key_too_large`, `value_too_large`, `too_many_transactions`, `corrupt_log`, and `io` variants. Use `code()` for stable identifiers.
- `Options`: Configure `order` (default 64), `max_key_bytes` (1,024), `max_value_bytes` (1,048,576), and `max_active_txns` (1,024).

## Why not existing alternatives?

This is a small, readable reference implementation of MVCC and WAL recovery with every isolation and crash claim tested; it prioritizes correct multiversion semantics over the feature set found in larger stores like `sled` or `redb`.

See [docs/SECURITY.md](docs/SECURITY.md) for threat modeling.