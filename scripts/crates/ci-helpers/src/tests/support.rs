//! Shared helpers for the tests: scratch directories and a captured run.

use crate::{dispatch, Io};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory, removed when it goes out of scope.
pub struct Tmp(pub PathBuf);

impl Tmp {
    pub fn new() -> Tmp {
        let dir = std::env::temp_dir().join(format!(
            "ci-helpers-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn join(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    pub fn write(&self, rel: &str, content: &str) -> PathBuf {
        let path = self.0.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
        path
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
}

/// Run the dispatcher with `args` and `stdin`, capturing both output streams.
pub fn run(args: &[&str], stdin: &str) -> Run {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    run_with(|io| dispatch(&args, io), stdin)
}

pub fn run_with(f: impl FnOnce(&mut Io) -> i32, stdin: &str) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut input = stdin.as_bytes();
    let code = f(&mut Io {
        stdin: &mut input,
        out: &mut out,
        err: &mut err,
    });
    Run {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}
