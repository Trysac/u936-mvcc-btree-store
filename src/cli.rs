use std::io::Write;

use crate::{FileStorage, Options, Store};

/// Runs the CLI command specified by `args`, writing output to `stdout` and errors to `stderr`.
///
/// Supported commands:
/// - `get <log> <key>`
/// - `put <log> <key> <value>`
/// - `scan <log> [--prefix p]`
/// - `check <log>`
///
/// Exit codes:
/// - `0`: Success
/// - `1`: Key not found or domain failure
/// - `2`: Usage error, I/O failure, or corrupt log
pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let Some(cmd) = args.first().map(String::as_str) else {
        let _ = writeln!(
            stderr,
            "usage: mvcc-btree-store <get|put|scan|check> <log> ..."
        );
        return 2;
    };

    match cmd {
        "get" => cmd_get(&args[1..], stdout, stderr),
        "put" => cmd_put(&args[1..], stdout, stderr),
        "scan" => cmd_scan(&args[1..], stdout, stderr),
        "check" => cmd_check(&args[1..], stdout, stderr),
        _ => {
            let _ = writeln!(stderr, "unknown command: {cmd}");
            2
        }
    }
}

fn open_file_store(path: &str, stderr: &mut dyn Write) -> Result<Store, i32> {
    let storage = match FileStorage::new(path) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(stderr, "io error: {e}");
            return Err(2);
        }
    };
    match Store::open(Box::new(storage), Options::default()) {
        Ok(store) => Ok(store),
        Err(e) => {
            let _ = writeln!(stderr, "{}: {e}", e.code());
            Err(2)
        }
    }
}

fn cmd_get(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if args.len() != 2 {
        let _ = writeln!(stderr, "usage: get <log> <key>");
        return 2;
    }
    let Ok(store) = open_file_store(&args[0], stderr) else {
        return 2;
    };
    let Ok(txn) = store.begin() else {
        return 1;
    };
    match txn.get(args[1].as_bytes()) {
        Ok(Some(val)) => {
            let _ = writeln!(stdout, "{}", String::from_utf8_lossy(&val));
            0
        }
        Ok(None) => {
            let _ = writeln!(stderr, "not found");
            1
        }
        Err(e) => {
            let _ = writeln!(stderr, "{}: {e}", e.code());
            1
        }
    }
}

fn cmd_put(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if args.len() != 3 {
        let _ = writeln!(stderr, "usage: put <log> <key> <value>");
        return 2;
    }
    let Ok(store) = open_file_store(&args[0], stderr) else {
        return 2;
    };
    let Ok(mut txn) = store.begin() else {
        return 1;
    };
    if let Err(e) = txn.put(args[1].as_bytes(), args[2].as_bytes()) {
        let _ = writeln!(stderr, "{}: {e}", e.code());
        return 1;
    }
    match txn.commit() {
        Ok(ts) => {
            let _ = writeln!(stdout, "committed ts={ts}");
            0
        }
        Err(e) => {
            let _ = writeln!(stderr, "{}: {e}", e.code());
            1
        }
    }
}

fn cmd_scan(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if args.is_empty() {
        let _ = writeln!(stderr, "usage: scan <log> [--prefix p]");
        return 2;
    }
    let log_path = &args[0];
    let prefix = match args.len() {
        1 => None,
        3 if args[1] == "--prefix" => Some(args[2].as_bytes()),
        _ => {
            let _ = writeln!(stderr, "usage: scan <log> [--prefix p]");
            return 2;
        }
    };

    let Ok(store) = open_file_store(log_path, stderr) else {
        return 2;
    };
    let Ok(txn) = store.begin() else {
        return 1;
    };
    let Ok(items) = txn.scan(..) else {
        return 1;
    };
    for (k, v) in items {
        if let Some(p) = prefix
            && !k.starts_with(p)
        {
            continue;
        }
        let _ = writeln!(
            stdout,
            "{}\t{}",
            String::from_utf8_lossy(&k),
            String::from_utf8_lossy(&v)
        );
    }
    0
}

fn cmd_check(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if args.len() != 1 {
        let _ = writeln!(stderr, "usage: check <log>");
        return 2;
    }
    let Ok(store) = open_file_store(&args[0], stderr) else {
        return 2;
    };
    let stats = store.stats();
    let _ = writeln!(
        stdout,
        "keys={} versions={} log_bytes={}",
        stats.keys, stats.versions, stats.log_bytes
    );
    0
}
