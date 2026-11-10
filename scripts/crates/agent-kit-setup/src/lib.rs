//! The setup canary crate: the Rust workspace's proof of path.
//!
//! The standards repo's ADR-0014 puts all of agent-kit's Rust in one workspace
//! under `scripts/`, behind the 7-day crate cooldown. This member is
//! deliberately trivial and std-only, with no dependency of any kind, so the
//! workspace lockfile holds nothing whose age the cooldown could question.
//! `--version` proves the build end to end; the command also validates a path.
//! New Rust joins this workspace as a new member, never as its own Cargo
//! project.

use std::ffi::{OsStr, OsString};
use std::path::Path;

pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The line `--version` prints.
pub fn version() -> String {
    format!("{NAME} {}", env!("CARGO_PKG_VERSION"))
}

/// The line `validate PATH` prints, with the exit code it earns: 0 for a path
/// that exists, 1 for one that does not.
pub fn validate(path: &OsStr) -> (String, i32) {
    let shown = path.to_string_lossy();
    if Path::new(path).exists() {
        (format!("{NAME}: {shown} exists"), 0)
    } else {
        (format!("{NAME}: {shown} does not exist"), 1)
    }
}

/// The usage text a bad command line prints.
fn usage() -> String {
    format!("usage: {NAME} --version\n       {NAME} validate <PATH>")
}

/// One answered command line: the line to print and the exit code it earns.
fn answer(first: Option<OsString>, second: Option<OsString>) -> (String, i32) {
    match first.as_deref() {
        Some(flag) if flag == OsStr::new("--version") => (version(), 0),
        Some(cmd) if cmd == OsStr::new("validate") => match second {
            Some(path) => validate(&path),
            None => (format!("{NAME}: validate needs a path"), 2),
        },
        _ => (usage(), 2),
    }
}

/// Runs one command line and returns its exit code: 0, 1 (validated path
/// missing) or 2 (bad usage). Success prints to stdout, failure to stderr.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let (message, code) = answer(args.next(), args.next());
    if code == 0 {
        println!("{message}");
    } else {
        eprintln!("{message}");
    }
    code
}

// The unit tests below execute every line of this file, which is the
// workspace's 100 percent line-coverage standard (docs/RUST.md); the
// executable line in main.rs, the exit wrapper, is the one line they cannot.
#[cfg(test)]
mod tests;
