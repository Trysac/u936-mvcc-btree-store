# Arena-Allocated Multiversion B+Tree with Checksummed WAL Recovery

Status: accepted (generated, pending human review)

## Context

Embedded Rust applications need a lightweight, readable key-value store with concurrent non-blocking snapshot reads, first-committer-wins conflict detection, and deterministic crash recovery without `unsafe` pointer graphs or external C dependencies.

## Decision

Represent the B+tree as an arena `Vec<Node>` indexed by `usize` with leaf-level version chains ordered newest-first, serialize writers at commit time via a commit mutex that checks `latest_ts(key) > snapshot` before appending/syncing a CRC32-framed WAL record, and run exact watermark garbage collection whenever transactions complete.

## Consequences

Readers never block writers and observe repeatable reads without phantoms; torn trailing log records are safely truncated on reopen while mid-log corruption is rejected; memory usage scales with live data plus active snapshot history.
