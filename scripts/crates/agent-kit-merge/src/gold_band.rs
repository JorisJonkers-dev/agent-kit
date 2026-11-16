//! What every Gold Band converge step shares (#810): where an instance keeps its data, and whether an instance that
//! holds that data is running, since writing under a running app races its own writes.

use crate::{Env, Failure, Io};
use std::path::{Path, PathBuf};

/// The personal instance this kit converges.
pub const INSTANCE: &str = "gold-band-personal";

/// The exit status of a refusal because an instance runs; setup stops on it.
pub const RUNNING_EXIT: i32 = 4;

/// Set to `1` it says an instance runs, set to anything else that none does; the process table is then not read.
pub const RUNNING_ENV: &str = "AGENT_KIT_GOLD_BAND_RUNNING";

/// The executable names of the desktop app, matched by name and never by install path.
pub const PROCESS_NAMES: [&str; 2] = ["gold-band-desktop", "Gold Band"];

/// One process of the current user: its pid, its executable and, where the OS shows it, the data directory it uses.
#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: u32,
    pub exe: String,
    /// `None` when the OS hides the process's environment; such a process counts as holding any data directory.
    pub data: Option<PathBuf>,
}

/// Lists the current user's processes.
pub trait Processes {
    fn list(&self) -> Vec<Proc>;
}

/// The app's data directory: `.gold-band` under `GOLD_BAND_HOME` (blank counts as unset), else under the home.
pub fn data_dir(gold_band_home: Option<&str>, home: &str) -> PathBuf {
    match gold_band_home.filter(|value| !value.trim().is_empty()) {
        Some(value) => Path::new(value).join(".gold-band"),
        None => Path::new(home).join(".gold-band"),
    }
}

/// `${GOLD_BAND_HOME:-~}/.gold-band/settings.json`.
pub fn settings_path(env: Env) -> PathBuf {
    data_dir(env("GOLD_BAND_HOME").as_deref(), &env("HOME").unwrap_or_default()).join("settings.json")
}

/// A path made absolute and free of links where it exists, else as given.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The running instance that holds `data`, described for a message, or `None`.
pub fn running(data: &Path, env: Env, processes: &dyn Processes) -> Option<String> {
    if let Some(flag) = env(RUNNING_ENV) {
        return (flag == "1").then(|| format!("{INSTANCE} (set by {RUNNING_ENV})"));
    }
    let ours = resolved(data);
    processes
        .list()
        .into_iter()
        .filter(|proc| Path::new(&proc.exe).file_name().is_some_and(|name| PROCESS_NAMES.iter().any(|known| name == *known)))
        .find(|proc| proc.data.as_deref().is_none_or(|theirs| resolved(theirs) == ours))
        .map(|proc| format!("{INSTANCE} (process {}, data {})", proc.pid, ours.display()))
}

/// Reports an instance that started between write groups; a second run finishes the rest.
pub fn stopped(tool: &str, running: &str, written: &[String], pending: &[String], io: &mut Io) -> Result<i32, Failure> {
    writeln!(io.out, "{tool}: Gold Band instance {running} started during setup; quit it and re-run setup")?;
    let written = if written.is_empty() { "nothing".to_string() } else { written.join(", ") };
    writeln!(io.out, "{tool}: written: {written}")?;
    writeln!(io.out, "{tool}: not yet written: {}", pending.join(", "))?;
    Ok(RUNNING_EXIT)
}

/// The processes of the current user: from `/proc` where it exists, else from `ps`.
pub struct SystemProcesses;

impl Processes for SystemProcesses {
    fn list(&self) -> Vec<Proc> {
        system_table(Path::new("/proc"))
    }
}

/// The processes from a `/proc`-shaped `root` when it has a `self` entry, else from `ps`.
pub fn system_table(root: &Path) -> Vec<Proc> {
    if root.join("self").exists() {
        proc_table(root, current_uid())
    } else {
        ps_table()
    }
}

/// The uid of this process, as `/proc/self` is owned.
fn current_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map_or(u32::MAX, |meta| meta.uid())
}

/// The processes `uid` owns under a `/proc`-shaped `root`.
pub fn proc_table(root: &Path, uid: u32) -> Vec<Proc> {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<Proc> = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|name| name.parse::<u32>().ok()) else { continue };
        let path = entry.path();
        if !std::fs::metadata(&path).is_ok_and(|meta| meta.uid() == uid) {
            continue;
        }
        let Ok(cmdline) = std::fs::read(path.join("cmdline")) else { continue };
        let exe = String::from_utf8_lossy(cmdline.split(|byte| *byte == 0).next().unwrap_or_default()).into_owned();
        let data = std::fs::read(path.join("environ")).ok().map(|environ| {
            let value = |name: &str| {
                environ.split(|byte| *byte == 0).find_map(|pair| pair.strip_prefix(format!("{name}=").as_bytes()).map(|v| String::from_utf8_lossy(v).into_owned()))
            };
            data_dir(value("GOLD_BAND_HOME").as_deref(), &value("HOME").unwrap_or_default())
        });
        found.push(Proc { pid, exe, data });
    }
    found.sort_by_key(|proc| proc.pid);
    found
}

/// The current user's processes as `ps` lists them (macOS): pid and executable, no environment.
pub fn ps_table() -> Vec<Proc> {
    let uid = std::process::Command::new("id").arg("-u").output().map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string()).unwrap_or_default();
    let Ok(out) = std::process::Command::new("ps").args(["-U", &uid, "-o", "pid=,comm="]).output() else { return Vec::new() };
    parse_ps(&String::from_utf8_lossy(&out.stdout))
}

/// The lines of `ps -o pid=,comm=`.
pub fn parse_ps(text: &str) -> Vec<Proc> {
    text.lines()
        .filter_map(|line| {
            let (pid, exe) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
            Some(Proc { pid: pid.parse().ok()?, exe: exe.trim().to_string(), data: None })
        })
        .collect()
}
