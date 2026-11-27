//! The setup canary crate: the Rust workspace's proof of path.
//!
//! The standards repo's ADR-0014 puts all of agent-kit's Rust in one workspace
//! under `scripts/`. This member is deliberately trivial and std-only, with no
//! dependency of any kind.
//! `--version` proves the build end to end; the command also validates a path.
//! New Rust joins this workspace as a new member, never as its own Cargo
//! project.
//!
//! The `gh` side of a claim: a small seam for tests plus the slug check that
//! decides whether a repository argument names a GitHub repository.

use std::process::Command;

/// One GitHub CLI call, separated from the git store so tests can record it.
pub trait Gh {
    fn gh(&self, args: &[&str]) -> Result<(), String>;
}

/// The GitHub CLI backed by the real executable.
pub struct RealGh;

impl Gh for RealGh {
    fn gh(&self, args: &[&str]) -> Result<(), String> {
        run_command(Command::new("gh").args(args))
    }
}

/// Runs one prepared command: stderr on failure, a spawn error if it cannot start.
pub(crate) fn run_command(command: &mut Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("gh spawn: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(())
}

/// Whether `s` is exactly `owner/name`: one slash, two non-empty parts, no
/// spaces, no colon, and neither part starting with `/` or `.`.
pub fn is_slug(s: &str) -> bool {
    let Some((owner, name)) = s.split_once('/') else {
        return false;
    };
    !owner.is_empty()
        && !name.is_empty()
        && !owner.contains('/')
        && !name.contains('/')
        && !s.contains(char::is_whitespace)
        && !s.contains(':')
        && !owner.starts_with('.')
        && !name.starts_with('.')
}
