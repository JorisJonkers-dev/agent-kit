//! Shared test helper: write a fake executable and run it without ETXTBSY.
//!
//! Included into a crate's `tests.rs` with
//! `#[path = "../../../test-support/fake_exe.rs"] mod fake_exe;`.
//!
//! Why this exists. Linux refuses to exec a file that any process holds open
//! for writing ("Text file busy", ETXTBSY). The writer's own fd is close-on-exec,
//! but a test thread that forks while another thread has the file open gets a
//! copy of that fd, which lives until that child reaches its exec. Renaming
//! the file into place does not help, since the check is on the inode. So the
//! write and every fork in the test binary are serialised: a test that writes
//! or runs a fake program holds [`lock`] for its whole body, and
//! [`write_executable`] demands the guard as proof.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

static EXEC: Mutex<()> = Mutex::new(());

/// The process-wide lock for writing and spawning fake executables.
pub fn lock() -> MutexGuard<'static, ()> {
    EXEC.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Write `text` to `path` as an executable: to a temporary name, synced and
/// closed, then renamed into place, so the final name never has a writer.
pub fn write_executable(_held: &MutexGuard<'static, ()>, path: &Path, text: &str) {
    let tmp = path.with_extension("partial");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o755)
        .open(&tmp)
        .unwrap();
    file.write_all(text.as_bytes()).unwrap();
    file.sync_all().unwrap();
    drop(file);
    fs::rename(&tmp, path).unwrap();
}
