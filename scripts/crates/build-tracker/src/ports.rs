//! The ports the build tracker reaches the system through: files, clock, the
//! issue tracker, git and processes. Tests replace any of them.

use crate::spec::RepositoryName;
use std::collections::HashMap;

/// A file system failure as the platform words it: `code` is `ENOENT` for a missing file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsError {
    pub code: String,
    pub message: String,
}

/// The files the build tracker reads and writes.
pub trait Files {
    fn read_text(&self, path: &str) -> Result<String, FsError>;
    /// Replaces the file whole: a temporary file beside it, then a rename.
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// The names of the files in a directory.
    fn list_files(&self, dir: &str) -> Result<Vec<String>, FsError>;
}

pub trait Clock {
    /// Seconds since the Unix epoch.
    fn now(&self) -> i64;
}

/// An issue as the issue tracker reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueRecord {
    pub repository: RepositoryName,
    pub number: u64,
    pub title: String,
    pub closed: bool,
}

/// The issue tracker (GitHub), read only. Any failure is the message that says what went wrong.
pub trait Issues {
    fn issue(&self, repository: &RepositoryName, number: u64) -> Result<IssueRecord, String>;
    /// The issue's sub-issues, in the issue tracker's order.
    fn sub_issues(&self, repository: &RepositoryName, number: u64) -> Result<Vec<IssueRecord>, String>;
    /// The issues the issue's native blocked-by links name.
    fn blocked_by(&self, repository: &RepositoryName, number: u64) -> Result<Vec<IssueRecord>, String>;
}

/// Where a directory sits in its git repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryLocation {
    /// The working tree's root.
    pub root: String,
    /// The directory's path under the root, ending in `/`, or empty at the root.
    pub prefix: String,
    /// The root of the main working tree, the same for every worktree of the repository.
    pub main_root: String,
    /// Where `origin` (or the first remote) points, when the repository has a remote.
    pub remote: Option<RepositoryName>,
}

pub trait Repositories {
    /// `None` when the directory is in no git repository.
    fn locate(&self, dir: &str) -> Result<Option<RepositoryLocation>, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs one program and reports how it exited; the gh and git adapters reach the system only through it.
pub trait Process {
    /// Any exit code is an `Ok`; an `Err` is a program that could not be started.
    fn exec(&self, command: &str, args: &[String]) -> Result<ProcessOutput, String>;
}

/// Everything a command needs from its surroundings.
pub struct Runtime {
    pub files: Box<dyn Files>,
    pub clock: Box<dyn Clock>,
    pub issues: Box<dyn Issues>,
    pub repositories: Box<dyn Repositories>,
    /// The directory a relative spec path and a bare issue number are read against.
    pub cwd: String,
    /// The environment the state home comes from: XDG_STATE_HOME, else HOME.
    pub env: HashMap<String, String>,
}
