# Architecture

## Overview

`mvcc_btree_store` is a single-process embedded key-value store composed of four internal modules:

1. `src/btree.rs`: An arena-allocated B+tree (`Vec<Node>`) with index links (`usize`) and zero `unsafe` code. Internal nodes hold separator keys and child indices; leaf nodes form a singly-linked list (`next: Option<usize>`) for range scans and map each key to a descending list of `Version { ts: u64, value: Option<Vec<u8>> }` (`None` represents a tombstone). Nodes split at `Options::order` (default 64, minimum 4).
2. `src/wal.rs` & `src/crc.rs`: Binary write-ahead log encoder/decoder protected by IEEE 802.3 CRC-32 (`0xEDB88320`).
3. `src/storage.rs`: The `Storage` trait along with `FileStorage` (atomic temp-file + sync + rename on `replace`) and `MemStorage` (in-memory buffer supporting deterministic `crash_at` torn-write simulation).
4. `src/lib.rs`: Transaction coordinator (`Store` and `Txn`) implementing snapshot isolation, first-committer-wins conflict detection, deterministic version garbage collection, and log checkpointing.
