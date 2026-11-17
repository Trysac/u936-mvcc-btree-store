# Project record

## Objective

Embedded key-value store on a B+tree with MVCC snapshot isolation, first-committer-wins conflict detection and a CRC-checked write-ahead log that survives torn writes.

## Scope

- In-memory arena-allocated B+tree (no unsafe).
- Snapshot isolation: reads see the transaction's own writes overlaid on the committed state at snapshot time.
- Write-ahead log: CRC32-checked, supports recovery from torn trailing writes.
- Garbage collection: prunes versions older than the oldest active snapshot.
- Checkpointing: compacts logs to one record per live key.

## Out of scope

- On-disk B+tree pages or buffer pools.
- Serializable isolation (write skew is permitted).
- Multi-process locking or encryption.

## Timeline

| Date | Event |
|---|---|
| 2026 | Repository created from the factory utility template. |

## Effort metrics

| Metric | Value |
|---|---|
| Tokens (input / output) | 7531190 / 36261 |
| Hours (human) | pending measurement |
| Cost (USD) | 1.5940 |

## Factory run u936 (measured from API usage)

| Date (UTC) | Phase | Model | Batch | Input | Output | Cache write | Cache read | Cost (USD) |
|---|---|---|---|---|---|---|---|---|
| 2026-11-17 | generation | google/gemini-4-argon | yes | 6184 | 23618 | 0 | 0 | 0.4971 |
| 2026-11-17 | fix-1 | google/gemini-3.8-flash | no | 57752 | 2297 | 0 | 353684 | 0.0785 |
| 2026-11-17 | fix-2 | google/gemini-3.8-flash | no | 213838 | 1587 | 0 | 2615435 | 0.3625 |
| 2026-11-17 | fix-3 | google/gemini-3.8-flash | no | 202490 | 1570 | 0 | 2603120 | 0.3530 |
| 2026-11-17 | fix-4 | google/gemini-3.8-flash | no | 109631 | 4570 | 0 | 666086 | 0.1493 |
| 2026-11-17 | audit | google/gemini-3.8-flash | no | 133809 | 1101 | 0 | 545004 | 0.1454 |
| 2026-11-17 | docs | google/gemini-3.1-flash-lite | no | 24157 | 1518 | 0 | 0 | 0.0083 |

Total: $1.5940. Prices: vertex line, as measured by its adapter. Providers serve the current version of each model id, so the date tells which version answered; versions, launch stages and price sources per line are in the factory's model register (docs/MODELS.md).
