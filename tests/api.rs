//! Integration and property tests covering all MVCC, WAL, B+tree, concurrency, and CLI acceptance items.

use std::collections::BTreeMap;
use std::io;
use std::ops::Bound;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::thread;

use proptest::prelude::*;

use mvcc_btree_store::{Error, FileStorage, MemStorage, Options, Storage, Store, cli, crc32};

fn open_mem(opts: Options) -> (Store, MemStorage) {
    let mem = MemStorage::new();
    let store = Store::open(Box::new(mem.clone()), opts).expect("open mem store");
    (store, mem)
}

#[test]
fn acceptance_1_basic_put_commit_and_monotonic_ts() {
    let (store, _mem) = open_mem(Options::default());

    let mut t1 = store.begin().unwrap();
    t1.put(b"a", b"1").unwrap();
    assert_eq!(t1.commit().unwrap(), 1);

    let t2 = store.begin().unwrap();
    assert_eq!(t2.get(b"a").unwrap(), Some(b"1".to_vec()));
    // Read-only commit returns snapshot ts without writing a log record.
    assert_eq!(t2.commit().unwrap(), 1);

    let mut t3 = store.begin().unwrap();
    t3.put(b"b", b"2").unwrap();
    assert_eq!(t3.commit().unwrap(), 2);
}

#[test]
fn acceptance_2_snapshot_isolation_repeatable_read_and_no_phantom() {
    let (store, _mem) = open_mem(Options::default());

    let mut init = store.begin().unwrap();
    init.put(b"a", b"1").unwrap();
    init.commit().unwrap();

    let t1 = store.begin().unwrap();
    let mut t2 = store.begin().unwrap();
    t2.put(b"a", b"2").unwrap();
    t2.put(b"ab", b"99").unwrap();
    t2.commit().unwrap();

    assert_eq!(t1.get(b"a").unwrap(), Some(b"1".to_vec()));
    let scanned = t1.scan(b"a".to_vec()..b"b".to_vec()).unwrap();
    assert_eq!(scanned, vec![(b"a".to_vec(), b"1".to_vec())]);
    t1.rollback();

    let t3 = store.begin().unwrap();
    assert_eq!(t3.get(b"a").unwrap(), Some(b"2".to_vec()));
    assert_eq!(t3.get(b"ab").unwrap(), Some(b"99".to_vec()));
    let scanned3 = t3.scan(b"a".to_vec()..b"b".to_vec()).unwrap();
    assert_eq!(
        scanned3,
        vec![
            (b"a".to_vec(), b"2".to_vec()),
            (b"ab".to_vec(), b"99".to_vec())
        ]
    );
}

#[test]
fn acceptance_3_first_committer_wins_and_disjoint_commits() {
    let (store, _mem) = open_mem(Options::default());

    let mut t1 = store.begin().unwrap();
    let mut t2 = store.begin().unwrap();

    t1.put(b"a", b"from_t1").unwrap();
    t2.put(b"a", b"from_t2").unwrap();
    t2.put(b"other", b"unseen").unwrap();

    assert!(t1.commit().is_ok());
    let err = t2.commit().unwrap_err();
    assert_eq!(err.code(), "write_conflict");

    let check = store.begin().unwrap();
    assert_eq!(check.get(b"a").unwrap(), Some(b"from_t1".to_vec()));
    assert_eq!(check.get(b"other").unwrap(), None);
    check.rollback();

    // Disjoint keys both commit.
    let mut d1 = store.begin().unwrap();
    let mut d2 = store.begin().unwrap();
    d1.put(b"k1", b"v1").unwrap();
    d2.put(b"k2", b"v2").unwrap();
    assert!(d1.commit().is_ok());
    assert!(d2.commit().is_ok());
}

#[test]
fn snapshot_isolation_permits_write_skew() {
    let (store, _mem) = open_mem(Options::default());
    let mut setup = store.begin().unwrap();
    setup.put(b"doc_on_call_1", b"true").unwrap();
    setup.put(b"doc_on_call_2", b"true").unwrap();
    setup.commit().unwrap();

    let mut t1 = store.begin().unwrap();
    let mut t2 = store.begin().unwrap();

    assert_eq!(t1.get(b"doc_on_call_2").unwrap(), Some(b"true".to_vec()));
    assert_eq!(t2.get(b"doc_on_call_1").unwrap(), Some(b"true".to_vec()));

    t1.put(b"doc_on_call_1", b"false").unwrap();
    t2.put(b"doc_on_call_2", b"false").unwrap();

    assert!(t1.commit().is_ok());
    assert!(t2.commit().is_ok());
}

#[test]
fn acceptance_4_concurrent_bank_transfers_preserve_invariant() {
    let (store, _mem) = open_mem(Options::default());

    let mut init = store.begin().unwrap();
    for acct in 0..10_u8 {
        init.put(&[acct], &100_u64.to_le_bytes()).unwrap();
    }
    init.commit().unwrap();

    let barrier = Arc::new(Barrier::new(8));
    let succeeded = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(8);

    for tid in 0..8_u64 {
        let store = store.clone();
        let barrier = Arc::clone(&barrier);
        let succeeded = Arc::clone(&succeeded);
        handles.push(thread::spawn(move || {
            let mut rng = (tid + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            barrier.wait();
            for step in 0..500_u64 {
                rng ^= rng >> 12;
                rng ^= rng << 25;
                rng ^= rng >> 27;
                let r = rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
                let from = (r % 10) as u8;
                let mut to = ((r >> 8) % 10) as u8;
                if to == from {
                    to = (to + 1) % 10;
                }
                loop {
                    let mut txn = store.begin().unwrap();
                    let b_from =
                        u64::from_le_bytes(txn.get(&[from]).unwrap().unwrap().try_into().unwrap());
                    let b_to =
                        u64::from_le_bytes(txn.get(&[to]).unwrap().unwrap().try_into().unwrap());
                    txn.put(&[from], &b_from.wrapping_sub(1).to_le_bytes())
                        .unwrap();
                    txn.put(&[to], &b_to.wrapping_add(1).to_le_bytes()).unwrap();
                    match txn.commit() {
                        Ok(_) => {
                            succeeded.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                        Err(e) => assert_eq!(e.code(), "write_conflict"),
                    }
                }
                if step % 25 == 0 {
                    let snap = store.begin().unwrap();
                    let mut sum = 0_u64;
                    for acct in 0..10_u8 {
                        let v = snap.get(&[acct]).unwrap().unwrap();
                        sum = sum.wrapping_add(u64::from_le_bytes(v.try_into().unwrap()));
                    }
                    assert_eq!(sum, 1000);
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(succeeded.load(Ordering::SeqCst), 4000);

    let final_snap = store.begin().unwrap();
    let mut final_sum = 0_u64;
    for acct in 0..10_u8 {
        let v = final_snap.get(&[acct]).unwrap().unwrap();
        final_sum = final_sum.wrapping_add(u64::from_le_bytes(v.try_into().unwrap()));
    }
    assert_eq!(final_sum, 1000);
    drop(final_snap);
    assert_eq!(store.stats().versions, store.stats().keys);
}

#[test]
fn acceptance_5_wal_crash_recovery_and_corrupt_mid_log() {
    let mem = MemStorage::new();
    let store = Store::open(Box::new(mem.clone()), Options::default()).unwrap();

    for i in 1..=4_u8 {
        let mut txn = store.begin().unwrap();
        txn.put(&[b'k', i], &[b'v', i]).unwrap();
        txn.commit().unwrap();
        if i == 3 {
            let mut uncommitted = store.begin().unwrap();
            uncommitted.put(b"never", b"logged").unwrap();
            uncommitted.rollback();
        }
    }

    let full_bytes = mem.bytes();

    // Locate offset after 3rd record by opening a store on just 3 commits.
    let mem3 = MemStorage::new();
    let s3 = Store::open(Box::new(mem3.clone()), Options::default()).unwrap();
    for i in 1..=3_u8 {
        let mut txn = s3.begin().unwrap();
        txn.put(&[b'k', i], &[b'v', i]).unwrap();
        txn.commit().unwrap();
    }
    let offset_after_3 = mem3.bytes().len();
    assert!(full_bytes.len() > offset_after_3);

    // Cut the 4th record at every byte offset inside it.
    for cut in offset_after_3..full_bytes.len() {
        let crash_mem = MemStorage::new();
        let mut handle = crash_mem.clone();
        handle.replace(&full_bytes).unwrap();
        crash_mem.crash_at(cut);

        let recovered = Store::open(Box::new(crash_mem.clone()), Options::default()).unwrap();
        let txn = recovered.begin().unwrap();
        assert_eq!(txn.get(b"k\x01").unwrap(), Some(b"v\x01".to_vec()));
        assert_eq!(txn.get(b"k\x02").unwrap(), Some(b"v\x02".to_vec()));
        assert_eq!(txn.get(b"k\x03").unwrap(), Some(b"v\x03".to_vec()));
        assert_eq!(txn.get(b"k\x04").unwrap(), None);
        assert_eq!(txn.get(b"never").unwrap(), None);
        assert_eq!(crash_mem.bytes().len(), offset_after_3);
    }

    // Flip a payload byte inside record 2 while record 3 follows -> corrupt_log.
    let rec1_len = u32::from_le_bytes(full_bytes[0..4].try_into().unwrap()) as usize;
    let rec2_payload_byte = 8 + rec1_len + 8;
    let bad_mem = MemStorage::new();
    bad_mem
        .clone()
        .replace(&full_bytes[..offset_after_3])
        .unwrap();
    let orig = bad_mem.bytes()[rec2_payload_byte];
    bad_mem.corrupt_byte(rec2_payload_byte, orig ^ 0xFF);

    let err = Store::open(Box::new(bad_mem), Options::default()).unwrap_err();
    assert_eq!(err.code(), "corrupt_log");
}

#[test]
fn acceptance_6_order_4_ten_thousand_keys_and_invariants() {
    let opts = Options {
        order: 4,
        ..Options::default()
    };
    let (store, _mem) = open_mem(opts);
    let mut model = BTreeMap::new();

    let mut state = 7_u64;
    let mut next_u64 = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state
    };

    let mut keys = Vec::with_capacity(10_000);
    let mut txn = store.begin().unwrap();
    for i in 0..10_000_u32 {
        let r = next_u64();
        let mut k = Vec::with_capacity(12);
        k.extend_from_slice(&r.to_be_bytes());
        k.extend_from_slice(&i.to_be_bytes());
        let v = i.to_le_bytes().to_vec();
        txn.put(&k, &v).unwrap();
        model.insert(k.clone(), v);
        keys.push(k);
    }
    txn.commit().unwrap();
    assert!(store.debug_validate_tree());

    let mut del_txn = store.begin().unwrap();
    for key in keys.iter().take(5_000) {
        del_txn.delete(key).unwrap();
        model.remove(key);
    }
    del_txn.commit().unwrap();
    assert!(store.debug_validate_tree());

    let read_txn = store.begin().unwrap();
    let scanned = read_txn.scan(..).unwrap();
    let expected: Vec<(Vec<u8>, Vec<u8>)> = model.into_iter().collect();
    assert_eq!(scanned, expected);
    drop(read_txn);

    let st = store.stats();
    assert_eq!(st.keys, 5_000);
    assert_eq!(st.versions, st.keys);
}

#[test]
fn acceptance_7_limits_crc32_gc_and_checkpoint() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);

    let (store, mem) = open_mem(Options::default());
    let mut txn = store.begin().unwrap();

    let big_key = vec![b'k'; 1_025];
    assert_eq!(txn.put(&big_key, b"v").unwrap_err().code(), "key_too_large");
    assert_eq!(txn.get(&big_key).unwrap_err().code(), "key_too_large");
    assert_eq!(txn.delete(&big_key).unwrap_err().code(), "key_too_large");

    let big_val = vec![b'v'; 1_048_577];
    assert_eq!(
        txn.put(b"ok", &big_val).unwrap_err().code(),
        "value_too_large"
    );

    txn.put(b"k1", b"v1").unwrap();
    txn.put(b"k2", b"v2").unwrap();
    txn.commit().unwrap();

    for round in 0..5_u8 {
        let mut t = store.begin().unwrap();
        t.put(b"k1", &[b'v', round]).unwrap();
        t.commit().unwrap();
        let st = store.stats();
        assert_eq!(st.versions, st.keys);
    }

    let mut del = store.begin().unwrap();
    del.delete(b"k2").unwrap();
    del.commit().unwrap();
    assert_eq!(store.stats().keys, 1);
    assert_eq!(store.stats().versions, 1);

    let before_scan = {
        let r = store.begin().unwrap();
        r.scan(..).unwrap()
    };
    let log_before = store.stats().log_bytes;
    store.checkpoint().unwrap();
    let log_after = store.stats().log_bytes;
    assert!(log_after < log_before);

    let reopened = Store::open(Box::new(mem), Options::default()).unwrap();
    let r2 = reopened.begin().unwrap();
    assert_eq!(r2.scan(..).unwrap(), before_scan);
}

#[test]
fn max_active_transactions_and_error_formatting_safety() {
    let opts = Options {
        order: 2, // Clamped to minimum 4
        max_active_txns: 2,
        ..Options::default()
    };
    let (store, _mem) = open_mem(opts);
    let _t1 = store.begin().unwrap();
    let _t2 = store.begin().unwrap();
    let err = store.begin().unwrap_err();
    assert_eq!(err.code(), "too_many_transactions");
    assert!(err.to_string().contains("too_many_transactions"));

    for e in [
        Error::WriteConflict,
        Error::KeyTooLarge,
        Error::ValueTooLarge,
        Error::TooManyTransactions,
        Error::CorruptLog,
        Error::Io(io::Error::other("disk full")),
    ] {
        let msg = e.to_string();
        assert!(msg.starts_with(e.code()));
    }
}

#[test]
fn scan_bounds_and_uncommitted_write_merging() {
    let (store, _mem) = open_mem(Options::default());
    let mut t = store.begin().unwrap();
    for k in [b"a", b"b", b"c", b"d"] {
        t.put(k, k).unwrap();
    }
    t.commit().unwrap();

    let mut t2 = store.begin().unwrap();
    t2.put(b"b", b"B").unwrap();
    t2.delete(b"c").unwrap();
    t2.put(b"bb", b"BB").unwrap();
    assert_eq!(t2.get(b"b").unwrap(), Some(b"B".to_vec()));
    assert_eq!(t2.get(b"c").unwrap(), None);

    let res = t2
        .scan((
            Bound::Excluded(b"a".to_vec()),
            Bound::Included(b"d".to_vec()),
        ))
        .unwrap();
    assert_eq!(
        res,
        vec![
            (b"b".to_vec(), b"B".to_vec()),
            (b"bb".to_vec(), b"BB".to_vec()),
            (b"d".to_vec(), b"d".to_vec()),
        ]
    );
}

#[test]
fn file_storage_and_cli_commands() {
    let dir = std::env::temp_dir().join(format!(
        "mvcc_btree_test_{}_{}",
        std::process::id(),
        crc32(b"cli_test")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let log_path = dir.join("store.wal");
    let log_str = log_path.to_string_lossy().into_owned();

    let run_cli = |args: &[&str]| -> (i32, String, String) {
        let vec_args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = cli::run(&vec_args, &mut out, &mut err);
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    };

    assert_eq!(run_cli(&[]).0, 2);
    assert_eq!(run_cli(&["unknown"]).0, 2);
    assert_eq!(run_cli(&["get", &log_str]).0, 2);
    assert_eq!(run_cli(&["put", &log_str, "only_key"]).0, 2);
    assert_eq!(run_cli(&["scan"]).0, 2);
    assert_eq!(run_cli(&["scan", &log_str, "--bad"]).0, 2);
    assert_eq!(run_cli(&["check"]).0, 2);

    let (code, out, _) = run_cli(&["put", &log_str, "alpha", "100"]);
    assert_eq!(code, 0);
    assert!(out.contains("committed ts=1"));

    let (code, _, _) = run_cli(&["put", &log_str, "alphabet", "200"]);
    assert_eq!(code, 0);
    let (code, _, _) = run_cli(&["put", &log_str, "beta", "300"]);
    assert_eq!(code, 0);

    let (code, out, _) = run_cli(&["get", &log_str, "alpha"]);
    assert_eq!(code, 0);
    assert_eq!(out.trim(), "100");

    let (code, _, _) = run_cli(&["get", &log_str, "missing"]);
    assert_eq!(code, 1);

    let (code, out, _) = run_cli(&["scan", &log_str, "--prefix", "alph"]);
    assert_eq!(code, 0);
    assert_eq!(out, "alpha\t100\nalphabet\t200\n");

    let (code, out, _) = run_cli(&["scan", &log_str]);
    assert_eq!(code, 0);
    assert_eq!(out.lines().count(), 3);

    let (code, out, _) = run_cli(&["check", &log_str]);
    assert_eq!(code, 0);
    assert!(out.contains("keys=3 versions=3"));

    let fs = FileStorage::new(&log_path).unwrap();
    assert_eq!(fs.path(), log_path.as_path());
    let store = Store::open(Box::new(fs), Options::default()).unwrap();
    store.checkpoint().unwrap();

    let _ = std::fs::remove_dir_all(&dir);
}

#[derive(Debug, Clone)]
enum Op {
    Put(u8, u8),
    Delete(u8),
    Commit,
    Rollback,
}

fn op_strategy() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(
        prop_oneof![
            (0_u8..32, 0_u8..100).prop_map(|(k, v)| Op::Put(k, v)),
            (0_u8..32).prop_map(Op::Delete),
            Just(Op::Commit),
            Just(Op::Rollback),
        ],
        1..60,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn prop_random_txns_match_btreemap_model(ops in op_strategy()) {
        let opts = Options {
            order: 4,
            ..Options::default()
        };
        let (store, _mem) = open_mem(opts);
        let mut committed: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
        let mut staged: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
        let mut txn = store.begin().unwrap();

        for op in ops {
            match op {
                Op::Put(k, v) => {
                    txn.put(&[k], &[v]).unwrap();
                    staged.insert(vec![k], Some(vec![v]));
                }
                Op::Delete(k) => {
                    txn.delete(&[k]).unwrap();
                    staged.insert(vec![k], None);
                }
                Op::Commit => {
                    txn.commit().unwrap();
                    for (k, v_opt) in std::mem::take(&mut staged) {
                        match v_opt {
                            Some(v) => { committed.insert(k, v); }
                            None => { committed.remove(&k); }
                        }
                    }
                    txn = store.begin().unwrap();
                }
                Op::Rollback => {
                    txn.rollback();
                    staged.clear();
                    txn = store.begin().unwrap();
                }
            }
        }
        txn.commit().unwrap();
        for (k, v_opt) in staged {
            match v_opt {
                Some(v) => { committed.insert(k, v); }
                None => { committed.remove(&k); }
            }
        }

        prop_assert!(store.debug_validate_tree());
        let verify = store.begin().unwrap();
        let scanned = verify.scan(..).unwrap();
        let expected: Vec<(Vec<u8>, Vec<u8>)> = committed.into_iter().collect();
        prop_assert_eq!(scanned, expected);
        drop(verify);

        let st = store.stats();
        prop_assert_eq!(st.versions, st.keys);
    }
}
