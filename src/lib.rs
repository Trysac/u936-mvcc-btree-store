// The README is the crate documentation, so its usage example runs as a doc test.
#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod btree;
/// Command-line interface for inspecting and mutating a store log.
pub mod cli;
mod crc;
mod error;
mod storage;
mod wal;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Bound, RangeBounds};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub use crate::crc::crc32;
pub use crate::error::Error;
pub use crate::storage::{FileStorage, MemStorage, Storage};

use crate::btree::{BPlusTree, Version};
use crate::wal::{LogEntry, LogOp, LogRecord};

/// Configuration options for opening a [`Store`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Maximum branching factor / entries per node before splitting (minimum 4, default 64).
    pub order: usize,
    /// Maximum allowed key length in bytes (default `1_024`).
    pub max_key_bytes: usize,
    /// Maximum allowed value length in bytes (default `1_048_576`).
    pub max_value_bytes: usize,
    /// Maximum concurrent active transactions allowed (default `1_024`).
    pub max_active_txns: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            order: 64,
            max_key_bytes: 1_024,
            max_value_bytes: 1_048_576,
            max_active_txns: 1_024,
        }
    }
}

/// Point-in-time operational statistics for a [`Store`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Number of keys currently present in the B+tree.
    pub keys: usize,
    /// Total number of MVCC versions currently retained across all keys.
    pub versions: usize,
    /// Size of the backing write-ahead log in bytes.
    pub log_bytes: u64,
    /// Number of currently active transactions.
    pub active_txns: usize,
}

#[derive(Debug, Default)]
struct ActiveTxns {
    /// Maps transaction id -> snapshot timestamp.
    by_id: BTreeMap<u64, u64>,
    /// Keys modified since some transaction was active, awaiting GC cleanup.
    pending_gc_keys: BTreeSet<Vec<u8>>,
}

struct StoreInner {
    opts: Options,
    tree: RwLock<BPlusTree>,
    storage: Mutex<(Box<dyn Storage>, u64)>,
    commit_lock: Mutex<()>,
    active: Mutex<ActiveTxns>,
    last_ts: AtomicU64,
    next_tx_id: AtomicU64,
}

/// An embedded key-value store backed by an in-memory multiversion B+tree and a durable WAL.
///
/// `Store` is `Send + Sync` and cheaply cloneable (`Arc` internally).
#[derive(Clone)]
pub struct Store {
    inner: Arc<StoreInner>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let stats = self.stats();
        f.debug_struct("Store")
            .field("opts", &self.inner.opts)
            .field("stats", &stats)
            .finish()
    }
}

impl Store {
    /// Opens a store over `storage` with the given `opts`, replaying any committed WAL records.
    ///
    /// If the final record in `storage` is truncated or fails its CRC check with no subsequent
    /// bytes, it is treated as a torn write from a prior crash, dropped, and the storage is
    /// rewritten cleanly via [`Storage::replace`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::CorruptLog`] if a mid-log record fails validation, or [`Error::Io`] if
    /// reading or rewriting the storage fails.
    pub fn open(mut storage: Box<dyn Storage>, mut opts: Options) -> Result<Self, Error> {
        if opts.order < 4 {
            opts.order = 4;
        }
        let raw = storage.read_all()?;
        let decoded = wal::decode_log(&raw)?;
        let log_bytes = if decoded.torn {
            let clean = &raw[..decoded.valid_bytes];
            storage.replace(clean)?;
            clean.len() as u64
        } else {
            raw.len() as u64
        };

        let mut tree = BPlusTree::new(opts.order);
        let mut max_ts = 0_u64;
        let mut touched = BTreeSet::new();
        for record in decoded.records {
            if record.ts > max_ts {
                max_ts = record.ts;
            }
            for entry in record.entries {
                let value = match entry.op {
                    LogOp::Put => Some(entry.value),
                    LogOp::Delete => None,
                };
                touched.insert(entry.key.clone());
                tree.insert_version(
                    entry.key,
                    Version {
                        ts: record.ts,
                        value,
                    },
                );
            }
        }
        for key in touched {
            tree.gc_key(&key, max_ts);
        }

        Ok(Self {
            inner: Arc::new(StoreInner {
                opts,
                tree: RwLock::new(tree),
                storage: Mutex::new((storage, log_bytes)),
                commit_lock: Mutex::new(()),
                active: Mutex::new(ActiveTxns::default()),
                last_ts: AtomicU64::new(max_ts),
                next_tx_id: AtomicU64::new(1),
            }),
        })
    }

    /// Begins a new snapshot-isolated transaction.
    ///
    /// # Errors
    ///
    /// Returns [`Error::TooManyTransactions`] if `opts.max_active_txns` transactions are
    /// already active.
    pub fn begin(&self) -> Result<Txn, Error> {
        let mut active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.by_id.len() >= self.inner.opts.max_active_txns {
            return Err(Error::TooManyTransactions);
        }
        let snapshot = self.inner.last_ts.load(Ordering::Acquire);
        let id = self.inner.next_tx_id.fetch_add(1, Ordering::Relaxed);
        active.by_id.insert(id, snapshot);
        drop(active);

        Ok(Txn {
            store: self.clone(),
            id,
            snapshot,
            writes: BTreeMap::new(),
            finished: false,
        })
    }

    /// Compacts the write-ahead log by writing one record per currently live key (preserving its
    /// latest committed version and original timestamp) via [`Storage::replace`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if replacing the underlying storage fails.
    pub fn checkpoint(&self) -> Result<(), Error> {
        let _commit_guard = self
            .inner
            .commit_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let current_ts = self.inner.last_ts.load(Ordering::Acquire);
        let live_entries = {
            let mut tree = self
                .inner
                .tree
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let min_snap = {
                let mut active = self
                    .inner
                    .active
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let floor = active.by_id.values().copied().min().unwrap_or(current_ts);
                let keys: Vec<Vec<u8>> = active.pending_gc_keys.iter().cloned().collect();
                for k in keys {
                    tree.gc_key(&k, floor);
                }
                if active.by_id.is_empty() {
                    active.pending_gc_keys.clear();
                }
                floor
            };
            let _ = min_snap;
            tree.latest_live_versions(current_ts)
        };

        let mut buf = Vec::new();
        for (key, version) in live_entries {
            if let Some(val) = version.value {
                let rec = LogRecord {
                    ts: version.ts,
                    entries: vec![LogEntry {
                        op: LogOp::Put,
                        key,
                        value: val,
                    }],
                };
                rec.encode_into(&mut buf);
            }
        }

        let mut storage_guard = self
            .inner
            .storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        storage_guard.0.replace(&buf)?;
        storage_guard.1 = buf.len() as u64;
        Ok(())
    }

    /// Returns current key, version, log byte and active transaction counts.
    #[must_use]
    pub fn stats(&self) -> Stats {
        let (keys, versions) = {
            let tree = self
                .inner
                .tree
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (tree.key_count(), tree.version_count())
        };
        let log_bytes = self
            .inner
            .storage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .1;
        let active_txns = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .by_id
            .len();
        Stats {
            keys,
            versions,
            log_bytes,
            active_txns,
        }
    }

    /// Validates internal B+tree invariants (leaf depth uniformity and key ordering).
    #[doc(hidden)]
    #[must_use]
    pub fn debug_validate_tree(&self) -> bool {
        let tree = self
            .inner
            .tree
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tree.validate_invariants().is_ok()
    }

    fn finish_txn(&self, tx_id: u64, newly_touched: Vec<Vec<u8>>) {
        let (min_snapshot, keys_to_gc) = {
            let mut active = self
                .inner
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            active.by_id.remove(&tx_id);
            for k in newly_touched {
                active.pending_gc_keys.insert(k);
            }
            if active.pending_gc_keys.is_empty() {
                return;
            }
            let last = self.inner.last_ts.load(Ordering::Acquire);
            let min_snap = active.by_id.values().copied().min().unwrap_or(last);
            let keys: Vec<Vec<u8>> = active.pending_gc_keys.iter().cloned().collect();
            (min_snap, keys)
        };

        let mut tree = self
            .inner
            .tree
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut completed = Vec::new();
        for key in &keys_to_gc {
            if tree.gc_key(key, min_snapshot) {
                completed.push(key.clone());
            }
        }
        if !completed.is_empty() {
            let mut active = self
                .inner
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for key in &completed {
                active.pending_gc_keys.remove(key);
            }
        }
    }
}

/// A snapshot-isolated MVCC transaction.
///
/// Reads see the transaction's own uncommitted writes overlaid on top of the committed state as
/// of the transaction's snapshot timestamp. Dropping a `Txn` without calling [`Txn::commit`]
/// automatically rolls it back and triggers version garbage collection.
pub struct Txn {
    store: Store,
    id: u64,
    snapshot: u64,
    writes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    finished: bool,
}

impl std::fmt::Debug for Txn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Txn")
            .field("id", &self.id)
            .field("snapshot", &self.snapshot)
            .field("buffered_writes", &self.writes.len())
            .finish_non_exhaustive()
    }
}

impl Txn {
    /// Looks up the value for `key` at this transaction's snapshot (including its own writes).
    ///
    /// # Errors
    ///
    /// Returns [`Error::KeyTooLarge`] if `key.len() > opts.max_key_bytes`.
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        if key.len() > self.store.inner.opts.max_key_bytes {
            return Err(Error::KeyTooLarge);
        }
        if let Some(local) = self.writes.get(key) {
            return Ok(local.clone());
        }
        let tree = self
            .store
            .inner
            .tree
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(tree.get_visible(key, self.snapshot))
    }

    /// Stages an insert or update of `key` to `value` in this transaction's write set.
    ///
    /// # Errors
    ///
    /// Returns [`Error::KeyTooLarge`] if `key.len() > opts.max_key_bytes` or
    /// [`Error::ValueTooLarge`] if `value.len() > opts.max_value_bytes`.
    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Error> {
        if key.len() > self.store.inner.opts.max_key_bytes {
            return Err(Error::KeyTooLarge);
        }
        if value.len() > self.store.inner.opts.max_value_bytes {
            return Err(Error::ValueTooLarge);
        }
        self.writes.insert(key.to_vec(), Some(value.to_vec()));
        Ok(())
    }

    /// Stages a deletion (tombstone) of `key` in this transaction's write set.
    ///
    /// # Errors
    ///
    /// Returns [`Error::KeyTooLarge`] if `key.len() > opts.max_key_bytes`.
    pub fn delete(&mut self, key: &[u8]) -> Result<(), Error> {
        if key.len() > self.store.inner.opts.max_key_bytes {
            return Err(Error::KeyTooLarge);
        }
        self.writes.insert(key.to_vec(), None);
        Ok(())
    }

    /// Scans all visible `(key, value)` pairs within `range` in ascending key order.
    ///
    /// Merges the committed B+tree snapshot with this transaction's uncommitted write set.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] if an internal state check fails.
    #[allow(clippy::type_complexity)]
    pub fn scan(&self, range: impl RangeBounds<Vec<u8>>) -> Result<Vec<(Vec<u8>, Vec<u8>)>, Error> {
        let start: Bound<Vec<u8>> = match range.start_bound() {
            Bound::Included(k) => Bound::Included(k.clone()),
            Bound::Excluded(k) => Bound::Excluded(k.clone()),
            Bound::Unbounded => Bound::Unbounded,
        };
        let end: Bound<Vec<u8>> = match range.end_bound() {
            Bound::Included(k) => Bound::Included(k.clone()),
            Bound::Excluded(k) => Bound::Excluded(k.clone()),
            Bound::Unbounded => Bound::Unbounded,
        };

        let tree_items = {
            let tree = self
                .store
                .inner
                .tree
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            tree.scan_visible((start.as_ref(), end.as_ref()), self.snapshot)
        };

        if self.writes.is_empty() {
            return Ok(tree_items);
        }

        let mut merged: BTreeMap<Vec<u8>, Vec<u8>> = tree_items.into_iter().collect();
        for (k, v_opt) in self.writes.range((start, end)) {
            match v_opt {
                Some(v) => {
                    merged.insert(k.clone(), v.clone());
                }
                None => {
                    merged.remove(k);
                }
            }
        }
        Ok(merged.into_iter().collect())
    }

    /// Commits this transaction using first-committer-wins conflict detection.
    ///
    /// Read-only transactions return their snapshot timestamp immediately without writing a WAL
    /// record. Write transactions allocate `ts = last_committed_ts + 1`, append and sync a
    /// checksummed WAL record, and publish their versions into the B+tree.
    ///
    /// # Errors
    ///
    /// Returns [`Error::WriteConflict`] if any key in this transaction's write set has been
    /// committed with `ts > self.snapshot` since this transaction began, or [`Error::Io`] if
    /// appending or syncing the WAL fails.
    pub fn commit(mut self) -> Result<u64, Error> {
        if self.writes.is_empty() {
            let snap = self.snapshot;
            self.finished = true;
            self.store.finish_txn(self.id, Vec::new());
            return Ok(snap);
        }

        let commit_guard = self
            .store
            .inner
            .commit_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        {
            let tree = self
                .store
                .inner
                .tree
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for key in self.writes.keys() {
                if let Some(latest_ts) = tree.latest_ts(key)
                    && latest_ts > self.snapshot
                {
                    drop(tree);
                    drop(commit_guard);
                    self.finished = true;
                    self.store.finish_txn(self.id, Vec::new());
                    return Err(Error::WriteConflict);
                }
            }
        }

        let new_ts = self.store.inner.last_ts.load(Ordering::Acquire) + 1;

        let entries: Vec<LogEntry> = self
            .writes
            .iter()
            .map(|(k, v)| match v {
                Some(bytes) => LogEntry {
                    op: LogOp::Put,
                    key: k.clone(),
                    value: bytes.clone(),
                },
                None => LogEntry {
                    op: LogOp::Delete,
                    key: k.clone(),
                    value: Vec::new(),
                },
            })
            .collect();

        let record = LogRecord {
            ts: new_ts,
            entries,
        };
        let mut encoded = Vec::new();
        record.encode_into(&mut encoded);

        {
            let mut storage_guard = self
                .store
                .inner
                .storage
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Err(e) = storage_guard
                .0
                .append(&encoded)
                .and_then(|()| storage_guard.0.sync())
            {
                drop(storage_guard);
                drop(commit_guard);
                self.finished = true;
                self.store.finish_txn(self.id, Vec::new());
                return Err(Error::Io(e));
            }
            storage_guard.1 += encoded.len() as u64;
        }

        let writes = std::mem::take(&mut self.writes);
        let mut touched_keys = Vec::with_capacity(writes.len());
        {
            let mut tree = self
                .store
                .inner
                .tree
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (key, value) in writes {
                touched_keys.push(key.clone());
                tree.insert_version(key, Version { ts: new_ts, value });
            }
        }

        self.store.inner.last_ts.store(new_ts, Ordering::Release);
        drop(commit_guard);

        self.finished = true;
        self.store.finish_txn(self.id, touched_keys);
        Ok(new_ts)
    }

    /// Explicitly aborts this transaction, discarding any buffered writes.
    pub fn rollback(mut self) {
        self.finished = true;
        self.writes.clear();
        self.store.finish_txn(self.id, Vec::new());
    }
}

impl Drop for Txn {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            self.writes.clear();
            self.store.finish_txn(self.id, Vec::new());
        }
    }
}
