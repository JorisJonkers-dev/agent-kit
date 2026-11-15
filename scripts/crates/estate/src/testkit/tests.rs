//! Shared test helpers: scratch directories, fake tools on a PATH, and a
//! runner that reads what a subcommand printed.

use crate::{run, Ctx};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

static COUNTER: AtomicUsize = AtomicUsize::new(0);
static SPAWN: Mutex<()> = Mutex::new(());

/// Tests that write a tool and run it hold this, so a concurrent fork never
/// keeps a just-written script open ("text file busy").
pub fn spawn_lock() -> MutexGuard<'static, ()> {
    SPAWN.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct Dir(PathBuf);

impl Dir {
    pub fn new() -> Dir {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("estate-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Dir(path)
    }

    pub fn str(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }

    pub fn join(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    pub fn write(&self, rel: &str, text: &str) -> PathBuf {
        self.write_bytes(rel, text.as_bytes())
    }

    pub fn write_bytes(&self, rel: &str, data: &[u8]) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, data).unwrap();
        path
    }

    /// An executable shell script at `rel`.
    pub fn tool(&self, rel: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = self.write(rel, &format!("#!/bin/sh\n{script}\n"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct Out {
    pub code: i32,
    pub out: String,
    pub err: String,
}

pub fn run_in(
    args: &[&str],
    input: &str,
    path: &str,
    timeout: Option<Duration>,
    home: Option<&str>,
) -> Out {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut stdin = input.as_bytes();
    let code = run(
        &args,
        &mut Ctx {
            out: &mut out,
            err: &mut err,
            input: &mut stdin,
            path: path.to_string(),
            home: home.map(String::from),
            timeout,
        },
    );
    Out {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
}

/// Runs with an empty PATH and no stdin.
pub fn run_plain(args: &[&str]) -> Out {
    run_in(args, "", "/nonexistent", None, None)
}
