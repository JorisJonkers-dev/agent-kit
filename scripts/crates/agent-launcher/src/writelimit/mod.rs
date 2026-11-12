//! The operating-system write limit (ADR-0013): the second layer under a reach
//! level's tool configuration. One interface, [`WriteLimit`], with one backend
//! per OS; this file holds the interface, the table of writable paths per
//! reach level ([`policy_for`]), the Linux backend ([`Landlock`]), which calls
//! the kernel's Landlock syscalls directly through libc, and the macOS backend
//! ([`SandboxExec`]), which starts the agent under `sandbox-exec`.
//!
//! The limit never restricts reads or the network. When the backend for this
//! OS is not available the launcher refuses to start the agent; it never runs
//! it without the limit.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// What to run in the child between fork and exec to put the limit on it.
pub type Armed = Box<dyn Arm>;

/// The limit, ready to put on a child.
pub trait Arm: Send + Sync {
    /// Restricts the calling process. It only makes syscalls, so it is safe
    /// between fork and exec.
    fn arm(&self) -> io::Result<()>;

    /// A command that starts the agent under the limit instead: its program
    /// and leading arguments, with the agent's own command appended. Empty
    /// when the limit is put on the process itself (see [`Arm::arm`]).
    fn prefix(&self) -> Vec<OsString> {
        Vec::new()
    }
}

/// A write-limit backend.
pub trait WriteLimit {
    /// Checks the backend is usable and builds the limit for `policy`, or says
    /// why this OS cannot hold it. Nothing is restricted in this process.
    fn prepare(&self, policy: &Policy) -> Result<Armed, String>;
}

/// Where an agent may write, beyond reading everything it could already read.
#[derive(Debug, Default, PartialEq)]
pub struct Policy {
    /// Directories writable with everything beneath them.
    pub dirs: Vec<PathBuf>,
    /// Single files (devices) writable but not replaced.
    pub files: Vec<PathBuf>,
    /// Files inside the granted directories that tell git where its
    /// directories are (`.git`, `commondir`, `gitdir`). Landlock cannot carve
    /// them out of a grant, so the launcher puts them back if the agent
    /// changes them (see [`Guard`]).
    pub guarded: Vec<PathBuf>,
    /// Paths (and everything beneath them) git runs or reads its settings
    /// from, the shared `hooks` and `config` and every effective
    /// `core.hooksPath`. A backend that can deny inside a grant (`sandbox-exec`)
    /// must keep writes out of them; they lie outside every grant already.
    pub denied: Vec<PathBuf>,
}

/// What the guarded files held before the agent ran.
pub struct Guard(Vec<(PathBuf, Vec<u8>)>);

impl Guard {
    /// Remembers the content of each guarded file.
    pub fn take(policy: &Policy) -> Guard {
        Guard(
            policy
                .guarded
                .iter()
                .filter_map(|p| fs::read(p).ok().map(|bytes| (p.clone(), bytes)))
                .collect(),
        )
    }

    /// Puts back every guarded file the agent changed, replaced or removed,
    /// and names each one; a file that could not be put back says so.
    pub fn restore(&self) -> Vec<String> {
        let mut restored = Vec::new();
        for (path, bytes) in &self.0 {
            if fs::read(path).ok().as_deref() == Some(bytes.as_slice()) {
                continue;
            }
            let _ = fs::remove_dir_all(path);
            let _ = fs::remove_file(path);
            match fs::write(path, bytes) {
                Ok(()) => restored.push(path.display().to_string()),
                Err(e) => restored.push(format!("{} (could NOT be put back: {e})", path.display())),
            }
        }
        restored
    }
}

/// The directories one launch knows about.
#[derive(Debug, Default)]
pub struct Scope {
    /// The agent's own state directories; one that does not exist is skipped.
    pub state_dirs: Vec<PathBuf>,
    /// The temporary directory for this run.
    pub temp_dir: PathBuf,
    /// The declared output directory (`research`, `write-scoped`).
    pub output_dir: Option<PathBuf>,
    /// The gate-output directory (`run-one-command`).
    pub gate_dir: Option<PathBuf>,
    /// The git worktree the agent runs in (`write`).
    pub worktree: PathBuf,
    /// Other git config files (system, global) whose `core.hooksPath` git would honour.
    pub other_configs: Vec<PathBuf>,
}

/// Devices every agent writes to.
const DEVICES: [&str; 2] = ["/dev/null", "/dev/tty"];

/// ADR-0013's table. `None` is `full`: no limit, which the table allows only
/// in a throwaway repository.
pub fn policy_for(level: &str, scope: &Scope) -> Result<Option<Policy>, String> {
    let mut dirs: Vec<PathBuf> = scope
        .state_dirs
        .iter()
        .filter(|d| d.is_dir())
        .cloned()
        .collect();
    let granted = dirs
        .iter()
        .chain(scope.output_dir.iter())
        .chain(scope.gate_dir.iter());
    if let Some(bad) = granted
        .into_iter()
        .find(|d| d.components().any(|c| c.as_os_str() == ".git"))
    {
        return Err(format!(
            "{} is inside a .git directory and cannot be writable",
            bad.display()
        ));
    }
    dirs.push(scope.temp_dir.clone());
    let mut guarded = Vec::new();
    let mut denied = Vec::new();
    match level {
        "full" => return Ok(None),
        "read" | "read-web" => {}
        "research" | "write-scoped" => {
            dirs.push(need(&scope.output_dir, level, "an output directory")?)
        }
        "run-one-command" => dirs.push(need(&scope.gate_dir, level, "a gate-output directory")?),
        "write" => {
            let layout = worktree_dirs(&scope.worktree, &scope.other_configs)?;
            dirs.extend(layout.dirs);
            guarded = layout.guarded;
            denied = layout.denied;
        }
        other => {
            return Err(format!(
                "no write-limit table row for reach level {other:?}"
            ))
        }
    }
    let files = DEVICES
        .iter()
        .map(PathBuf::from)
        .filter(|f| f.exists())
        .collect();
    Ok(Some(Policy {
        dirs,
        files,
        guarded,
        denied,
    }))
}

fn need(dir: &Option<PathBuf>, level: &str, what: &str) -> Result<PathBuf, String> {
    dir.clone()
        .ok_or_else(|| format!("reach level {level} needs {what}"))
}

/// What `write` gets in a linked worktree.
struct Layout {
    dirs: Vec<PathBuf>,
    guarded: Vec<PathBuf>,
    denied: Vec<PathBuf>,
}

/// The directories `write` may change in a linked git worktree: the worktree,
/// its own git administration directory, and the shared object, ref and log
/// directories, but never the shared `hooks` or `config`. Landlock only grants,
/// so a primary checkout (whose `.git` directory holds hooks and config beside
/// everything else) cannot be limited and is refused.
fn worktree_dirs(worktree: &Path, other_configs: &[PathBuf]) -> Result<Layout, String> {
    let root = fs::canonicalize(worktree)
        .map_err(|e| format!("write: cannot resolve worktree {}: {e}", worktree.display()))?;
    let dot_git = root.join(".git");
    if dot_git.is_dir() {
        return Err(format!(
            "write: {} is a primary checkout; the limit cannot keep .git/hooks and .git/config out of reach there, use a linked worktree",
            root.display()
        ));
    }
    let pointer = fs::read_to_string(&dot_git).map_err(|_| {
        format!(
            "write: {} is not a git worktree (no .git file)",
            root.display()
        )
    })?;
    let admin = pointer
        .trim()
        .strip_prefix("gitdir:")
        .map(|p| root.join(p.trim()))
        .ok_or_else(|| format!("write: {} is not a gitdir pointer", dot_git.display()))?;
    let admin = fs::canonicalize(&admin).map_err(|e| {
        format!(
            "write: cannot resolve git directory {}: {e}",
            admin.display()
        )
    })?;
    let common = fs::read_to_string(admin.join("commondir"))
        .map_err(|e| format!("write: {} has no commondir: {e}", admin.display()))?;
    let common = fs::canonicalize(admin.join(common.trim()))
        .map_err(|e| format!("write: cannot resolve the common git directory: {e}"))?;
    // Believe the layout only when it is the one git makes: the worktree's
    // directory is `<common>/worktrees/<name>`, the common directory is not
    // inside anything the agent can write, and the directory points back at
    // this worktree's `.git`. A previous run's residue fails this.
    if common.starts_with(&root)
        || common.starts_with(&admin)
        || admin.parent().and_then(Path::parent) != Some(common.as_path())
        || admin.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("worktrees"))
    {
        return Err(format!(
            "write: {} is not laid out as git makes a worktree's directory; not trusting it",
            admin.display()
        ));
    }
    let back = fs::read_to_string(admin.join("gitdir"))
        .map_err(|e| format!("write: {} has no gitdir: {e}", admin.display()))?;
    if fs::canonicalize(back.trim()).ok().as_deref() != Some(dot_git.as_path()) {
        return Err(format!(
            "write: {} does not point back at {}; not trusting it",
            admin.display(),
            dot_git.display()
        ));
    }
    let config_text = fs::read_to_string(common.join("config"))
        .map_err(|e| format!("write: cannot read the repository config: {e}"))?;
    let config = config_text.to_ascii_lowercase();
    if config.contains("worktreeconfig") || config.contains("include") {
        return Err("write: the repository config enables extensions.worktreeConfig or includes another file, so a config the agent can write could change hooks; remove it".into());
    }
    let mut dirs = vec![root, admin.clone()];
    for shared in ["objects", "refs", "logs"] {
        let dir = common.join(shared);
        // git makes these itself; `logs` is missing until the first reflog.
        fs::create_dir_all(&dir)
            .map_err(|e| format!("write: cannot create {}: {e}", dir.display()))?;
        // A symlink here would make the grant land somewhere else, such as hooks.
        if fs::canonicalize(&dir).ok().as_deref() != Some(dir.as_path()) {
            return Err(format!(
                "write: {} is a symlink; refusing to grant it",
                dir.display()
            ));
        }
        dirs.push(dir);
    }
    // The hooks git will run, and the config that names them, must be out of
    // every grant (a symlinked `hooks`, a hooksPath such as `.husky`).
    let mut protected = vec![common.join("hooks"), common.join("config")];
    let others = other_configs
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok());
    for text in std::iter::once(config_text).chain(others) {
        // Git uses the last one it reads, so every occurrence is checked.
        for path in hooks_paths(&text) {
            protected.push(dirs[0].join(expand_home(&path)));
        }
    }
    let mut denied = Vec::new();
    for path in protected {
        let real = match fs::canonicalize(&path) {
            Ok(real) => real,
            // A link that leads nowhere may be made to lead somewhere later.
            Err(_) if fs::symlink_metadata(&path).is_ok() => {
                return Err(format!(
                    "write: {} is a dangling link; refusing",
                    path.display()
                ))
            }
            Err(_) => path.clone(),
        };
        if let Some(grant) = dirs
            .iter()
            .find(|g| real.starts_with(g) || g.starts_with(&real))
        {
            return Err(format!(
                "write: {} (hooks or config git uses) lies inside the writable {}; refusing",
                real.display(),
                grant.display()
            ));
        }
        denied.push(real);
    }
    // Files inside the grants that git reads settings or object sources from:
    // a per-worktree config (only read with extensions.worktreeConfig, which
    // is refused above, but cheap to keep shut) and the alternates list.
    denied.push(admin.join("config.worktree"));
    denied.push(common.join("objects/info/alternates"));
    let guarded = vec![dot_git, admin.join("commondir"), admin.join("gitdir")];
    Ok(Layout {
        dirs,
        guarded,
        denied,
    })
}

/// Every `hooksPath` value in a git config. Keys are matched
/// case-insensitively in any section; a quoted value loses its quotes.
fn hooks_paths(config: &str) -> Vec<String> {
    config
        .lines()
        .filter_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            key.trim()
                .eq_ignore_ascii_case("hookspath")
                .then(|| value.trim().trim_matches('"').to_string())
        })
        .collect()
}

/// `~/x` as the home directory's `x`; anything else unchanged.
fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

#[cfg(target_os = "linux")]
mod landlock;
#[cfg(target_os = "linux")]
pub use landlock::Landlock;

mod sandbox_exec;
pub use sandbox_exec::SandboxExec;

/// The backend for an OS that has none: it always refuses.
pub struct Unsupported;

impl WriteLimit for Unsupported {
    fn prepare(&self, _policy: &Policy) -> Result<Armed, String> {
        Err("this OS has no write-limit backend".into())
    }
}

/// The backend for the OS this was built for.
pub fn native() -> Box<dyn WriteLimit> {
    #[cfg(target_os = "linux")]
    return Box::new(Landlock);
    #[cfg(target_os = "macos")]
    return Box::new(SandboxExec::system());
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    return Box::new(Unsupported);
}

#[cfg(test)]
mod tests;
