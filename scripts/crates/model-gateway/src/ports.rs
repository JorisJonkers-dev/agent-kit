//! The ports the gateway reaches the system through: the clock, the files and
//! the environment. Tests replace any of them.

use std::collections::HashMap;

/// A file system failure as the platform words it: `code` is `ENOENT` for a missing file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsError {
    pub code: String,
    pub message: String,
}

/// The files the gateway reads and writes.
pub trait Files {
    fn read_text(&self, path: &str) -> Result<String, FsError>;
    /// Replaces the file whole: a temporary file beside it, then a rename.
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// `mkdir -p`.
    fn mkdir_all(&self, path: &str) -> Result<(), FsError>;
    /// Creates the file, or reports `false` when it is already there.
    fn create_exclusive(&self, path: &str) -> Result<bool, FsError>;
    /// Removes the file, and says nothing when it is not there.
    fn remove_file(&self, path: &str) -> Result<(), FsError>;
    /// The file's modification time in epoch milliseconds; `None` when it cannot be read.
    fn modified_ms(&self, path: &str) -> Option<i64>;
}

pub trait Clock {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
}

/// How long a writer waits for the workflows lock: how long it sleeps before it
/// looks again, how old a lock must be to belong to a process that died, and
/// how long it waits before it gives up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockTimings {
    pub retry_ms: u64,
    pub stale_ms: i64,
    pub timeout_ms: i64,
}

impl Default for LockTimings {
    fn default() -> Self {
        LockTimings { retry_ms: 10, stale_ms: 5_000, timeout_ms: 10_000 }
    }
}

/// Everything a command needs from its surroundings.
pub struct Runtime {
    pub clock: Box<dyn Clock>,
    pub files: Box<dyn Files>,
    /// The environment the state home comes from: XDG_STATE_HOME, else HOME.
    pub env: HashMap<String, String>,
    pub lock: LockTimings,
}

impl Runtime {
    /// The text of the file `--input` names.
    pub fn read_input(&self, path: &str) -> Result<String, FsError> {
        self.files.read_text(path)
    }
}
