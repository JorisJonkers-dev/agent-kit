//! Shared test helper: write a fake executable and run it without ETXTBSY.
//!
//! Included into a crate's `tests.rs` with
//! `#[path = "../../../test-support/fake_exe.rs"] mod fake_exe;`.
//!
//! Why this exists. Linux refuses to exec a file that any process holds open
//! for writing ("Text file busy", ETXTBSY). The writer's own fd is close-on-exec,
//! but a test thread that forks while another thread has the file open gets a
//! copy of that fd, which lives until that child reaches its exec. Renaming
//! the file into place does not help, since the check is on the inode.
//!
//! [`write_script`] avoids the race outright: a child `sh` writes the file and
//! makes it executable, so this process never holds it open and no fork of
//! another test thread can carry a writer into its child. A crate that writes
//! its fakes through it needs no lock (#880). [`lock`] and [`write_executable`]
//! stay for the crates that serialise their own spawns.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, PoisonError};

static EXEC: Mutex<()> = Mutex::new(());

/// The process-wide lock for writing and spawning fake executables.
pub fn lock() -> MutexGuard<'static, ()> {
    EXEC.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Write `text` to `path` as an executable.
pub fn write_executable(_held: &MutexGuard<'static, ()>, path: &Path, text: &str) {
    write_script(path, text);
}

/// Write `text` to `path` as an executable through a child `sh`, so no descriptor of the file is ever open in this
/// process: it can be run at once, from any thread, without ETXTBSY.
pub fn write_script(path: &Path, text: &str) {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "cat > \"$1\" && chmod 755 \"$1\"", "sh"])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    assert!(child.wait().unwrap().success(), "could not write {}", path.display());
}
