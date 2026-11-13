//! The memory bank resolver (ADR-0012, spec #392 B5-01): a session's bank is
//! chosen by its repository's remote owner. A repository with a `nedap`-owned
//! remote, on any host or alias, goes to the Nedap bank; every other
//! repository, and a directory with no remote, goes to Shared Memory. The owner
//! rule is the model gateway's (`git_remote::identify_repository`), so memory,
//! delegation and critique agree on what a `nedap` repository is.
//!
//! [`Bank`] has two values and no Private Memory: Private Memory is reachable
//! only from `vibe-private`, behind the passphrase, and no code path here can
//! name it. A [`Session`] holds one bank and hands only that bank to its
//! [`Memory`] backend for every recall and retain, so a nedap session cannot
//! touch Shared Memory and a personal session cannot touch the Nedap bank.
//!
//! ```text
//! memory-bank resolve [PATH]
//! ```
//!
//! prints `{"bank":"nedap","bank_id":"nedap"}` or
//! `{"bank":"shared","bank_id":"default"}` for the repository at `PATH` (default
//! `.`). Git failing to read the remotes is an error (exit 1, nothing on stdout):
//! an unreadable remote never falls back to Shared Memory. Exit 2 is bad usage.

#[cfg(test)]
mod tests;

use model_gateway::adapters::git::{SystemGit, TIMEOUT_MS};
use model_gateway::git_remote::{identify_repository, RepositoryIdentity};
use model_gateway::ports::{Git, GitRemote};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write;

/// A bank a session may reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bank {
    /// Shared Memory: the bank every personal agent and launcher reads and writes.
    Shared,
    /// The Nedap bank: employer facts, reached from work surfaces only.
    Nedap,
}

impl Bank {
    /// The word the registry and the documents use.
    pub fn name(self) -> &'static str {
        match self {
            Bank::Shared => "shared",
            Bank::Nedap => "nedap",
        }
    }

    /// The Hindsight bank id: Shared Memory is the bank `default`.
    pub fn id(self) -> &'static str {
        match self {
            Bank::Shared => "default",
            Bank::Nedap => "nedap",
        }
    }
}

/// The bank for a repository with these remotes.
pub fn resolve(remotes: &[GitRemote]) -> Bank {
    match identify_repository(remotes) {
        RepositoryIdentity::DeniedOwner { .. } => Bank::Nedap,
        RepositoryIdentity::NoRemote | RepositoryIdentity::Identified { .. } => Bank::Shared,
    }
}

/// What the resolver asks git: whether a path is in a repository, and where its remotes point.
pub trait Remotes {
    /// `Ok(false)` when git says `path` is no repository.
    fn in_repository(&self, path: &str) -> Result<bool, String>;
    fn remotes(&self, path: &str) -> Result<Vec<GitRemote>, String>;
}

impl<G: Git> Remotes for G {
    fn in_repository(&self, path: &str) -> Result<bool, String> {
        Ok(self.root(path)?.is_some())
    }
    fn remotes(&self, path: &str) -> Result<Vec<GitRemote>, String> {
        Git::remotes(self, path)
    }
}

/// The bank for the repository at `path`. A path that is no repository has no remote, so it is Shared Memory;
/// a failure to read the remotes is an `Err` and never a bank.
pub fn resolve_path(git: &dyn Remotes, path: &str) -> Result<Bank, String> {
    if !git.in_repository(path)? {
        return Ok(Bank::Shared);
    }
    Ok(resolve(&git.remotes(path)?))
}

/// What a memory server does for one bank.
pub trait Memory {
    fn recall(&self, bank: Bank, query: &str) -> Result<Vec<String>, String>;
    fn retain(&self, bank: Bank, content: &str) -> Result<(), String>;
}

/// One session's view of memory: the bank its repository resolves to, and no other.
pub struct Session<'a> {
    bank: Bank,
    memory: &'a dyn Memory,
}

impl<'a> Session<'a> {
    /// Opens the session for the repository at `path`.
    pub fn open(git: &dyn Remotes, path: &str, memory: &'a dyn Memory) -> Result<Session<'a>, String> {
        Ok(Session { bank: resolve_path(git, path)?, memory })
    }

    pub fn bank(&self) -> Bank {
        self.bank
    }

    pub fn recall(&self, query: &str) -> Result<Vec<String>, String> {
        self.memory.recall(self.bank, query)
    }

    pub fn retain(&self, content: &str) -> Result<(), String> {
        self.memory.retain(self.bank, content)
    }
}

const USAGE: &str = "usage: memory-bank resolve [PATH]\n";

/// The command line: `resolve [PATH]` on the real git.
pub fn run(args: impl IntoIterator<Item = OsString>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let git = SystemGit { env: std::env::vars().collect::<HashMap<_, _>>(), timeout_ms: TIMEOUT_MS };
    run_with(args, &git, out, err)
}

/// [`run`] on any git.
pub fn run_with(args: impl IntoIterator<Item = OsString>, git: &dyn Remotes, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let args: Vec<String> = args.into_iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
    let path = match args.as_slice() {
        [command] if command == "resolve" => ".",
        [command, path] if command == "resolve" => path.as_str(),
        _ => {
            let _ = err.write_all(USAGE.as_bytes());
            return 2;
        }
    };
    match resolve_path(git, path) {
        Ok(bank) => {
            let _ = writeln!(out, "{{\"bank\":\"{}\",\"bank_id\":\"{}\"}}", bank.name(), bank.id());
            0
        }
        Err(message) => {
            let _ = writeln!(err, "memory-bank: cannot read the remotes of {path}: {message}");
            1
        }
    }
}
