# Security & Threat Model

## Defended

- Torn writes: The WAL is structured as `[len u32][crc32 u32][payload]`. `Store::open` detects incomplete or invalid final records and removes them to prevent corruption.
- Resource exhaustion: Keys are limited to `Options::max_key_bytes`, values to `Options::max_value_bytes`, and concurrent transactions to `Options::max_active_txns`.
- Memory safety: The crate is strictly `#![forbid(unsafe_code)]` and uses `RwLock` and `Mutex` to mediate access to the B+tree and WAL.

## Not defended

- Maliciously constructed log files: The `decode_log` function performs CRC checks, but is not designed to defend against active adversarial modification of the file system; once corrupted beyond the final record, `Error::CorruptLog` is returned.
- Denial of Service via large inputs: While limits are enforced, the B+tree resides in memory; total memory usage is proportional to keys and versions, which can grow until the system OOMs.