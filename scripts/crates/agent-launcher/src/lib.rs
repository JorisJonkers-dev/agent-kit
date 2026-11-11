//! The launcher: starts one agent id with its reach level's tool
//! configuration, and refuses, before anything starts, a level its agent
//! cannot hold on this OS (spec #392, stories 44-46, 51, 52).
//!
//! The support record is a YAML file (`registry/reach-levels.yaml`): for each
//! agent the command that starts it and, for each reach level, its tool
//! arguments and a per-OS cell, `verified`, `unverified` or `unsupported`;
//! and the agent ids, each naming one agent at one reach level (and an
//! optional skill set), the OSes it runs on and, for `research` and
//! `write-scoped`, its output directory. Only a verified cell may be used.
//!
//! Usage:
//! * `agent-launcher validate <RECORD>`: exit 0 clean, 1 rule problems on
//!   stderr, 2 bad usage or an unreadable or malformed file;
//! * `agent-launcher run <RECORD> <ID> [-- ARGS...]`: starts the id's agent
//!   with its tool arguments then ARGS, with `AGENT_KIT_REACH` (and
//!   `AGENT_KIT_OUTPUT_DIR` when the id has one, and `AGENT_KIT_SKILL_SET`
//!   with `AGENT_KIT_SKILLS_DIR` when it names a skill set) in its
//!   environment, and exits with its status. A refused id exits 1 and starts
//!   nothing.
//!
//! The operating-system write limit (ADR-0013) is the `writelimit` module: one
//! interface, with Landlock as the Linux backend. `run` refuses to start an
//! agent whose limit this OS cannot hold.

mod record;
mod writelimit;

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use record::{cell, id_rule_problems, parse, problems, Record, LEVELS, OSES};
pub use writelimit::{policy_for, Arm, Armed, Guard, Landlock, Policy, Scope, WriteLimit};

/// The crate's name, which prefixes every message line.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The record's name for the OS this runs on.
pub fn this_os() -> &'static str {
    os_name(std::env::consts::OS)
}

/// The record's name for a Rust OS name: `mac` for macOS, `linux` for Linux, and
/// `unsupported` (no cell can be verified for it) for any other OS.
pub fn os_name(rust_os: &str) -> &'static str {
    match rust_os {
        "macos" => "mac",
        "linux" => "linux",
        _ => "unsupported",
    }
}

/// Why an id may not start on `os`, or the command line it starts with.
pub fn plan(record: &Record, id: &str, os: &str) -> Result<Plan, String> {
    let entry = record
        .ids
        .get(id)
        .ok_or_else(|| format!("unknown agent id {id:?}"))?;
    if let Some(first) = id_rule_problems(record, id).into_iter().next() {
        return Err(format!("{first}; nothing started"));
    }
    if !entry.hosts.iter().any(|h| h == os) {
        return Err(format!("{id}: not declared for {os}; nothing started"));
    }
    // The id's own rules held above, so its agent and level are named, known
    // and verified on every host it lists, this OS among them.
    let agent = entry.agent.clone().unwrap_or_default();
    let level = entry.reach.clone().unwrap_or_default();
    let spec = &record.agents[&agent];
    if spec.command.first().map_or(true, String::is_empty) {
        return Err(format!("{id}: agent {agent} has no command"));
    }
    let mut argv = spec.command.clone();
    argv.extend(spec.levels[&level].args.clone());
    Ok(Plan {
        id: id.to_string(),
        argv,
        level,
        skill_set: entry.skill_set.clone(),
        output_dir: entry.output_dir.clone(),
    })
}

/// What a permitted id starts.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub id: String,
    pub argv: Vec<String>,
    pub level: String,
    pub skill_set: Option<String>,
    pub output_dir: Option<String>,
}

/// The directory setup builds for `id`'s skill set: `<root>/skill-sets/<id>`,
/// where the root is `.gold-band` under `gold_band_home` when it is set and
/// not blank (the app keeps its data there), else `<home>/.gold-band`, else
/// there is none.
pub fn skills_dir(gold_band_home: Option<&str>, home: Option<&str>, id: &str) -> Option<PathBuf> {
    let root = match gold_band_home {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir).join(".gold-band"),
        _ => match home {
            Some(dir) if !dir.is_empty() => PathBuf::from(dir).join(".gold-band"),
            _ => return None,
        },
    };
    Some(root.join("skill-sets").join(id))
}

fn usage() -> String {
    format!("usage: {NAME} validate <RECORD> | {NAME} run <RECORD> <ID> [-- ARGS...]")
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_on(args, this_os())
}

/// [`run`] for a named OS, so tests can stand in for either.
pub fn run_on<I>(args: I, os: &str) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_with(args, os, &Landlock, &Context::from_env())
}

/// The system and global git config files for this user.
fn global_configs() -> Vec<PathBuf> {
    configs_in(
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
    )
}

/// The git config files for a home directory and an XDG config directory
/// (which defaults to `~/.config`).
fn configs_in(home: Option<PathBuf>, xdg: Option<PathBuf>) -> Vec<PathBuf> {
    let xdg = xdg.or_else(|| home.as_ref().map(|h| h.join(".config")));
    let mut found = vec![PathBuf::from("/etc/gitconfig")];
    found.extend(home.map(|h| h.join(".gitconfig")));
    found.extend(xdg.map(|x| x.join("git/config")));
    found
}

/// What a launch knows about where it runs.
#[derive(Debug, Default)]
pub struct Context {
    /// The agent's state directories (`AGENT_KIT_STATE_DIRS`, colon separated).
    pub state_dirs: Vec<PathBuf>,
    /// The gate-output directory (`AGENT_KIT_GATE_DIR`).
    pub gate_dir: Option<PathBuf>,
    /// The git worktree the agent runs in: the current directory.
    pub worktree: PathBuf,
    /// The system and global git config files.
    pub other_configs: Vec<PathBuf>,
    /// Where each run's temporary directory is made.
    pub temp_root: PathBuf,
}

impl Context {
    /// The context of this process.
    pub fn from_env() -> Context {
        let state = std::env::var_os("AGENT_KIT_STATE_DIRS").unwrap_or_default();
        Context {
            state_dirs: std::env::split_paths(&state)
                .filter(|p| !p.as_os_str().is_empty())
                .collect(),
            gate_dir: std::env::var_os("AGENT_KIT_GATE_DIR").map(PathBuf::from),
            worktree: PathBuf::from("."),
            other_configs: global_configs(),
            temp_root: std::env::temp_dir(),
        }
    }
}

/// [`run_on`] with the write-limit backend named, so tests can stand in for
/// one that is unavailable.
pub fn run_with<I>(args: I, os: &str, limit: &dyn WriteLimit, context: &Context) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    match args.as_slice() {
        [cmd, file] if cmd == OsStr::new("validate") => validate_file(file),
        [cmd, file, id, rest @ ..] if cmd == OsStr::new("run") => {
            let extra = match rest {
                [] => &[][..],
                [dash, extra @ ..] if dash == OsStr::new("--") => extra,
                _ => return bad_usage(),
            };
            launch(file, &id.to_string_lossy(), extra, os, limit, context)
        }
        _ => bad_usage(),
    }
}

fn bad_usage() -> i32 {
    eprintln!("{}", usage());
    2
}

fn load(file: &OsStr) -> Result<Record, i32> {
    let text = fs::read_to_string(file).map_err(|error| {
        eprintln!("{NAME}: cannot read {}: {error}", file.to_string_lossy());
        2
    })?;
    parse(&text).map_err(|error| {
        eprintln!("{NAME}: {error}");
        2
    })
}

fn validate_file(file: &OsStr) -> i32 {
    let record = match load(file) {
        Ok(record) => record,
        Err(code) => return code,
    };
    let found = problems(&record);
    if found.is_empty() {
        println!("ok ({} ids)", record.ids.len());
        0
    } else {
        for problem in &found {
            eprintln!("{NAME}: {problem}");
        }
        1
    }
}

fn launch(
    file: &OsStr,
    id: &str,
    extra: &[OsString],
    os: &str,
    limit: &dyn WriteLimit,
    context: &Context,
) -> i32 {
    let record = match load(file) {
        Ok(record) => record,
        Err(code) => return code,
    };
    let plan = match plan(&record, id, os) {
        Ok(plan) => plan,
        Err(refusal) => {
            eprintln!("{NAME}: {refusal}");
            return 1;
        }
    };
    // The temporary directory is made before the limit, which names it, and
    // removed after the agent ends.
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let temp = context.temp_root.join(format!(
        "{NAME}-{}-{}-{nanos}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    let outcome = start(&plan, extra, &temp, limit, context);
    let _ = fs::remove_dir_all(&temp);
    match outcome {
        Ok((code, restored)) if restored.is_empty() => code,
        Ok((_, restored)) => {
            eprintln!(
                "{NAME}: the agent changed files that tell git where its directories are: {}",
                restored.join(", ")
            );
            1
        }
        Err(message) => {
            eprintln!("{NAME}: {message}; nothing started");
            1
        }
    }
}

/// The directories a launch knows, for the write-limit table.
fn scope_of(plan: &Plan, temp: &std::path::Path, context: &Context) -> Scope {
    Scope {
        state_dirs: context.state_dirs.clone(),
        temp_dir: temp.to_path_buf(),
        output_dir: plan.output_dir.as_ref().map(PathBuf::from),
        gate_dir: context.gate_dir.clone(),
        worktree: context.worktree.clone(),
        other_configs: context.other_configs.clone(),
    }
}

/// Makes `dir` new and private (mode 0700) beneath `root`: a path someone else
/// planted there is refused, not followed.
fn private_dir(root: &std::path::Path, dir: &std::path::Path) -> Result<(), String> {
    match fs::create_dir_all(root) {
        Err(e) => Err(format!("cannot create {}: {e}", root.display())),
        Ok(()) => fs::DirBuilder::new()
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display())),
    }
}

/// What runs in the child between fork and exec.
fn child_hook(armed: Armed) -> impl FnMut() -> std::io::Result<()> + Send + Sync + 'static {
    move || armed.arm()
}

/// Puts the write limit on, then starts the agent; its exit code, or why not.
fn start(
    plan: &Plan,
    extra: &[OsString],
    temp: &std::path::Path,
    limit: &dyn WriteLimit,
    context: &Context,
) -> Result<(i32, Vec<String>), String> {
    let scope = scope_of(plan, temp, context);
    let policy = policy_for(&plan.level, &scope)?;
    // The directories the policy grants must exist to be granted.
    for dir in scope.output_dir.iter().chain(scope.gate_dir.iter()) {
        fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    private_dir(&context.temp_root, &scope.temp_dir)?;
    let guard = policy.as_ref().map(Guard::take);
    let armed = match &policy {
        Some(policy) => Some(
            limit
                .prepare(policy)
                .map_err(|e| format!("no write limit for reach level {}: {e}", plan.level))?,
        ),
        None => None,
    };
    let mut command = Command::new(&plan.argv[0]);
    command
        .args(&plan.argv[1..])
        .args(extra)
        .env("AGENT_KIT_REACH", &plan.level);
    if let Some(dir) = &plan.output_dir {
        command.env("AGENT_KIT_OUTPUT_DIR", dir);
    }
    command.env("TMPDIR", temp);
    if let Some(armed) = armed {
        // SAFETY: `armed` only makes syscalls between fork and exec.
        unsafe { command.pre_exec(child_hook(armed)) };
    }
    match &plan.skill_set {
        Some(name) => {
            command.env("AGENT_KIT_SKILL_SET", name);
            let gold_band_home = std::env::var("GOLD_BAND_HOME").ok();
            let home = std::env::var("HOME").ok();
            if let Some(dir) = skills_dir(gold_band_home.as_deref(), home.as_deref(), &plan.id) {
                command.env("AGENT_KIT_SKILLS_DIR", dir);
            }
        }
        None => {
            command.env_remove("AGENT_KIT_SKILL_SET");
            command.env_remove("AGENT_KIT_SKILLS_DIR");
        }
    }
    match command.status() {
        Ok(status) => {
            let restored = guard.map(|g| g.restore()).unwrap_or_default();
            Ok((status.code().unwrap_or(1), restored))
        }
        Err(error) => Err(format!(
            "cannot start {} (or put the write limit on it): {error}",
            plan.argv[0]
        )),
    }
}

#[cfg(test)]
mod tests;
