//! claim: a ticket claim is the git ref `refs/claims/<key>` in a repository, taken with a
//! create-only push, so exactly one agent wins. The lease is recorded in the claim commit.

pub mod cli;
pub mod gh;
pub mod lease;
pub mod store;

#[cfg(test)]
mod tests;

use crate::cli::run_with;
use crate::gh::RealGh;
use crate::lease::SystemClock;
use crate::store::{Git, RealGit};

/// Runs one command with the real git, `gh`, clock and standard streams. The
/// scratch repository is removed even when the command reports an error.
pub fn run(args: Vec<String>) -> i32 {
    let dir = std::env::temp_dir().join(format!("claim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    let git = RealGit { dir: dir.clone() };
    let _ = git.git(&["init", "-q"], "");
    let code = run_with(
        &args,
        &env_var,
        &git,
        &RealGh,
        &SystemClock,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    );
    let _ = std::fs::remove_dir_all(&dir);
    code
}

/// One environment variable, if set and valid.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}
