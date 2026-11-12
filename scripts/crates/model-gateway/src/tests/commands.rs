//! What the goldens cannot reach in the commands: a ledger that reads once and then fails.

use super::scenarios::{FixedClock, Sandbox, FIXED_NOW};
use crate::adapters::fs::SystemFiles;
use crate::adapters::git::{SystemGit, TIMEOUT_MS};
use crate::adapters::process::SystemProcess;
use crate::cli::run_cli;
use crate::ports::{FileStamp, Files, FsError, LockTimings, Runtime};
use std::cell::Cell;
use std::collections::HashMap;

/// The real files, except that the ledger can be read only so many times: then it fails as a disk would.
struct FlakyLedger {
    inner: SystemFiles,
    reads: Cell<usize>,
    readable: usize,
}

impl Files for FlakyLedger {
    fn write_text(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        unreachable!("the test does not write text")
    }

    fn temp_dir(&self, _parent: &str, _prefix: &str) -> Result<String, FsError> {
        unreachable!("the test makes no temporary directory")
    }

    fn remove(&self, _path: &str) -> Result<(), FsError> {
        unreachable!("the test removes nothing")
    }

    fn read_text(&self, path: &str) -> Result<String, FsError> {
        if path.ends_with("ledger.jsonl") {
            self.reads.set(self.reads.get() + 1);
            if self.reads.get() > self.readable {
                return Err(FsError { code: "EIO".into(), message: "EIO: i/o error, read".into() });
            }
        }
        self.inner.read_text(path)
    }

    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.write_atomic(path, content)
    }

    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        self.inner.mkdir_all(path)
    }

    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        self.inner.create_exclusive(path)
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        self.inner.remove_file(path)
    }

    fn modified_ms(&self, path: &str) -> Option<i64> {
        self.inner.modified_ms(path)
    }

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        self.inner.exists(path)
    }

    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.append(path, content)
    }

    fn stamp(&self, path: &str) -> Option<FileStamp> {
        self.inner.stamp(path)
    }

    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.inner.write_private(path, content)
    }
}

fn runtime(sandbox: &Sandbox, readable: usize) -> Runtime {
    let root = sandbox.text();
    std::fs::create_dir_all(format!("{root}/home")).expect("home");
    let env = HashMap::from([
        ("HOME".to_string(), format!("{root}/home")),
        ("XDG_STATE_HOME".to_string(), format!("{root}/state")),
        ("PATH".to_string(), std::env::var("PATH").unwrap_or_default()),
    ]);
    Runtime {
        clock: Box::new(FixedClock(FIXED_NOW)),
        files: Box::new(FlakyLedger { inner: SystemFiles { cwd: root.clone() }, reads: Cell::new(0), readable }),
        lock: LockTimings::default(),
        process: Box::new(SystemProcess { env: env.clone() }),
        git: Box::new(SystemGit { env: env.clone(), timeout_ms: TIMEOUT_MS }),
        env,
        defaults: crate::SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    }
}

fn grant_write_input() -> Vec<String> {
    let input = r#"{"workflow":"w","client":"codex","cap_usd":2,"question":"q","answer":"a"}"#;
    ["grant", "write", "--json", input].iter().map(|word| word.to_string()).collect()
}

#[test]
fn a_ledger_that_fails_after_the_grant_was_written_is_a_warning_not_an_error() {
    // Reads of the ledger in order: the settle (two), the standing-grant check, then the report.
    let sandbox = Sandbox::new();
    let result = run_cli(&grant_write_input(), &runtime(&sandbox, 3));
    assert_eq!(result.exit_code, 0, "{}", result.stderr);
    assert!(result.stdout.contains("\"room\": null"), "{}", result.stdout);
    assert!(result.stdout.contains("\"warning\": \"the grant is written, but the ledger could not be read, so what it has left is unknown: EIO: i/o error, read\""), "{}", result.stdout);
    let written = std::fs::read_to_string(format!("{}/home/.config/agent-kit/model-gateway-grants.json", sandbox.text())).expect("the grant was written");
    assert!(written.contains("\"cap_usd\": 2"));
}

#[test]
fn a_ledger_that_cannot_be_read_before_the_write_refuses_it() {
    let sandbox = Sandbox::new();
    let result = run_cli(&grant_write_input(), &runtime(&sandbox, 2));
    assert_eq!(result.exit_code, 1);
    assert!(result.stdout.contains("\"code\":\"ledger-unreadable\""), "{}", result.stdout);
    assert!(result.stdout.contains("a standing grant cannot be ruled out and no grant is written: EIO: i/o error, read"), "{}", result.stdout);
    assert!(!std::path::Path::new(&format!("{}/home/.config/agent-kit/model-gateway-grants.json", sandbox.text())).exists());
}
