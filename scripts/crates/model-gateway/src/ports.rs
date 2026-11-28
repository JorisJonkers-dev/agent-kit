//! The ports the gateway reaches the system through: the clock, the files and
//! the environment. Tests replace any of them.
//!
//! Every port is `Send + Sync`, so a `Runtime` is `Sync` and a batch can hand
//! one delegation to each of several threads at once.

use std::collections::{BTreeMap, HashMap};

/// A file system failure as the platform words it: `code` is `ENOENT` for a missing file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsError {
    pub code: String,
    pub message: String,
}

/// The files the gateway reads and writes.
pub trait Files: Send + Sync {
    fn read_text(&self, path: &str) -> Result<String, FsError>;
    /// Replaces the file whole: a temporary file beside it, then a rename.
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// `mkdir -p`.
    fn mkdir_all(&self, path: &str) -> Result<(), FsError>;
    /// `mkdir -p` with mode 0700 for what it makes: a directory a confined launch is granted to write, which the
    /// launcher refuses when it is not there yet.
    fn mkdir_private(&self, path: &str) -> Result<(), FsError> {
        self.mkdir_all(path)
    }
    /// Creates the file, or reports `false` when it is already there.
    fn create_exclusive(&self, path: &str) -> Result<bool, FsError>;
    /// Removes the file, and says nothing when it is not there.
    fn remove_file(&self, path: &str) -> Result<(), FsError>;
    /// The file's modification time in epoch milliseconds; `None` when it cannot be read.
    fn modified_ms(&self, path: &str) -> Option<i64>;
    /// `Ok(false)` only when the file is not there (ENOENT or ENOTDIR); any other failure to look is an error.
    fn exists(&self, path: &str) -> Result<bool, FsError>;
    /// Appends text to the file, creating it when it is not there.
    fn append(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// What makes a file the one it is: its real path, change time, size, inode and SHA-256; `None` when it cannot be read.
    fn stamp(&self, path: &str) -> Option<FileStamp>;
    /// Like `write_atomic`, in a directory only its owner may enter (0700) and a file only its owner may read (0600).
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// Writes the file whole, making its directory first.
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError>;
    /// A fresh directory under `parent` named `prefix` and six letters or digits, as `mkdtemp` makes one; its path.
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError>;
    /// `rm -rf`: says nothing when the path is not there.
    fn remove(&self, path: &str) -> Result<(), FsError>;
    /// Copies a file, or a folder with everything in it, to `to`, making its parent first; symlinks are copied as what they point at.
    fn copy_tree(&self, _from: &str, _to: &str) -> Result<(), FsError> {
        Err(FsError { code: "ENOSYS".into(), message: "ENOSYS: this file system cannot copy".into() })
    }
    /// The names in a directory, sorted; a file system that cannot list says so.
    fn list_dir(&self, _path: &str) -> Result<Vec<String>, FsError> {
        Err(FsError { code: "ENOSYS".into(), message: "ENOSYS: this file system cannot list".into() })
    }
}

/// What `stamp` reports of a file, followed through every symlink.
#[derive(Clone, Debug, PartialEq)]
pub struct FileStamp {
    pub realpath: String,
    pub ctime_ms: f64,
    pub ino: f64,
    pub sha256: String,
    pub size: f64,
}

/// What a finished command left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub timed_out: bool,
}

/// One launch of a program: what the node adapter's `ProcessCommand` carries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessCommand {
    pub command: String,
    pub args: Vec<String>,
    /// The directory it starts in; the caller's own when `None`.
    pub cwd: Option<String>,
    /// Variables added to the environment the adapter holds, which they win over.
    pub env: BTreeMap<String, String>,
    /// How long it may run before it is stopped; no limit when `None`.
    pub timeout_ms: Option<u64>,
    /// Written to its stdin, which is then closed; without it stdin is closed from the start.
    pub stdin: Option<String>,
}

impl ProcessCommand {
    pub fn new(command: &str, args: &[&str]) -> ProcessCommand {
        ProcessCommand { command: command.to_string(), args: args.iter().map(|arg| (*arg).to_string()).collect(), ..ProcessCommand::default() }
    }
}

/// Which output stream of a launched program a chunk came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// What a delegation's launch reports while it runs: its process id once it started, and each chunk of output
/// the moment it is read. The run's event log is one; a caller that needs neither passes `Unwatched`.
pub trait LaunchWatch: Sync {
    fn spawned(&self, pid: u32);
    fn output(&self, stream: Stream, chunk: &[u8]);
    /// Something the runner itself did to the process (the time limit's SIGTERM, a SIGKILL after the grace).
    fn note(&self, text: &str);
    /// Whether the client is driven through its stdin (ACP): its stdin then stays open after `opening` is written,
    /// and `reply` is asked what to write after each chunk of its stdout.
    fn interactive(&self) -> bool {
        false
    }
    /// What an interactive client is sent first.
    fn opening(&self) -> String {
        String::new()
    }
    /// What an interactive client is sent after a chunk of its stdout, and whether its stdin is then closed.
    fn reply(&self, _chunk: &[u8]) -> Reply {
        Reply::default()
    }
}

/// What is written to an interactive client's stdin, and whether the pipe is closed after it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    pub close: bool,
}

/// A watch that wants nothing.
pub struct Unwatched;

impl LaunchWatch for Unwatched {
    fn spawned(&self, _pid: u32) {}
    fn output(&self, _stream: Stream, _chunk: &[u8]) {}
    fn note(&self, _text: &str) {}
}

/// How a launched program ended, past the exit code `ProcessResult` carries (which reads 124 for a signal).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ending {
    /// The signal that ended it (9 for SIGKILL), when one did; its exit code is then none.
    pub signal: Option<i32>,
    /// The signal the gateway itself was stopped with (SIGINT, SIGTERM, SIGHUP) and passed on to the launch.
    pub cancelled_by: Option<i32>,
    /// The process id it ran as; `None` when the runner does not know it.
    pub pid: Option<u32>,
}

/// A watched launch's result and how it ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launched {
    pub result: ProcessResult,
    pub ending: Ending,
}

/// The programs the gateway starts: the clients' own listing and status commands, and a delegation's client.
pub trait Process: Send + Sync {
    /// `run`, telling `watch` what happens as it happens and saying how the program ended. A runner that cannot
    /// stream hands the whole output over once the program is done, and knows no signal.
    fn run_watched(&self, command: &ProcessCommand, watch: &dyn LaunchWatch) -> Result<Launched, String> {
        let result = self.run(command)?;
        watch.output(Stream::Stdout, result.stdout.as_bytes());
        watch.output(Stream::Stderr, result.stderr.as_bytes());
        Ok(Launched { result, ending: Ending::default() })
    }
    /// Whether a process with this id is running. A runner that cannot tell says it is, so nothing is declared
    /// lost on a guess.
    fn is_alive(&self, _pid: u32) -> bool {
        true
    }
    /// The path of the first executable file named `binary` in a folder of PATH; `None` when there is none.
    fn resolve(&self, binary: &str) -> Option<String>;
    /// Runs `command args` to its end, or kills it after `timeout_ms`; an `Err` is a command that could not start.
    fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String>;
    /// Runs one launch with its own directory, environment and stdin: stopped with SIGTERM when it outlives its
    /// time limit, and killed when it ignores that; an `Err` is a command that could not start.
    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String>;
}

/// One remote of a repository, as `git config` names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitRemote {
    pub name: String,
    pub url: String,
}

/// What the gateway asks git: where a path's repository is, where it points, and the worktrees of it.
pub trait Git: Send + Sync {
    /// The repository root of `path`; `Ok(None)` when git says it is no repository; any other failure is an `Err` with git's own words.
    fn root(&self, path: &str) -> Result<Option<String>, String>;
    fn remotes(&self, path: &str) -> Result<Vec<GitRemote>, String>;
    /// `git worktree remove --force`, which takes the directory with it. A worktree git no longer knows is
    /// deleted by hand and pruned; the `Err` is git's own words (or the platform's) for what still failed.
    fn remove_worktree(&self, repo: &str, path: &str) -> Result<(), String>;
    /// `git branch -D`.
    fn delete_branch(&self, repo: &str, branch: &str) -> Result<(), String>;
    /// The repository's common directory as an absolute path, which every worktree of it shares.
    fn common_dir(&self, path: &str) -> Result<String, String>;
    /// A detached worktree of the repository's HEAD at `path`, which may be an empty directory already.
    fn add_worktree(&self, repo: &str, path: &str) -> Result<(), String>;
    /// The git directory of `path`'s worktree as an absolute path, which sits outside the worktree itself.
    fn git_dir(&self, path: &str) -> Result<String, String>;
    /// A worktree of `repo` at `path`, on a branch of its own named `branch`, created off `base`.
    fn create_worktree(&self, repo: &str, path: &str, branch: &str, base: &str) -> Result<(), String>;
    /// `git add -A`: everything in the worktree, staged.
    fn stage_all(&self, path: &str) -> Result<(), String>;
    /// The files staged in `path`, against `base` when one is named, so a change committed on the worktree's
    /// branch is listed too.
    fn staged_files(&self, path: &str, base: Option<&str>) -> Result<Vec<String>, String>;
    /// Every worktree of `repo`, without the blocks git marks `prunable`: a worktree whose directory is gone.
    fn list_worktrees(&self, repo: &str) -> Result<Vec<String>, String>;
    /// `git status --porcelain`, as git printed it.
    fn status(&self, path: &str) -> Result<String, String>;
    /// `git --exec-path`: where the `git` on `PATH` keeps its programs, which tells its install. A git that cannot
    /// say gives an `Err`, and a confined launch then runs no git of its own install.
    fn exec_path(&self) -> Result<String, String> {
        Err("git cannot say where it is installed".to_string())
    }
}

pub trait Clock: Send + Sync {
    /// Milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
    /// A monotonic reading in milliseconds, from which how long something took is measured.
    fn monotonic_ms(&self) -> f64;
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
    pub process: Box<dyn Process>,
    pub git: Box<dyn Git>,
    /// The shipped `defaults.json`, as text.
    pub defaults: String,
    /// The path the shipped defaults are named by in an error.
    pub defaults_path: String,
    /// Critique's shipped `routing.json`, as text: the read-only proofs are matched against it.
    pub routing: String,
}

impl Runtime {
    /// The text of the file `--input` names.
    pub fn read_input(&self, path: &str) -> Result<String, FsError> {
        self.files.read_text(path)
    }
}
