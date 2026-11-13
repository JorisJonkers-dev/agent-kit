//! Replays every scenario under tests/fixtures through the library and compares
//! stdout, stderr, the exit code and every file under the sandbox with the
//! goldens the node runner produced.

use crate::cli::run_cli;
use crate::git::SystemGit;
use crate::Runtime;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::json::{parse, Value};
use model_gateway::ports::Clock;
use model_gateway::timestamps;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Held across writing a fake client program and running anything, so no thread forks while a script is still open for writing (ETXTBSY).
pub static EXEC_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The first program of that name on the host's PATH.
fn host_program(name: &str) -> PathBuf {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':').map(|dir| Path::new(dir).join(name)).find(|candidate| candidate.exists()).unwrap_or_else(|| panic!("no {name} on PATH"))
}

/// 2026-11-03T12:00:00Z, the instant fake-clock.mjs gives the node runner.
pub const FIXED_NOW: i64 = 1_793_707_200_000;

pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }

    /// Frozen at 0, as the node runner's clock fake freezes `performance.now`.
    fn monotonic_ms(&self) -> f64 {
        0.0
    }
}

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

pub fn read_json(path: &Path) -> Value {
    parse(&std::fs::read_to_string(path).expect("fixture file")).expect("fixture JSON")
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_object().and_then(|object| object.get(key))
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key).and_then(Value::as_str).unwrap_or_default()
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Some(Value::Arr(items)) => items,
        _ => &[],
    }
}

fn copy_tree(from: &Path, to: &Path) {
    if !from.exists() {
        return;
    }
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir").flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

/// A scratch directory that goes away with it.
pub struct Sandbox {
    pub path: PathBuf,
}

impl Sandbox {
    pub fn new() -> Sandbox {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let name = format!("critique-test-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).expect("create sandbox");
        Sandbox { path: std::fs::canonicalize(path).expect("canonical sandbox") }
    }

    pub fn text(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Every file and directory under the roots, as `path -> Some(text)` (a directory is `path/ -> None`).
fn snapshot(sandbox: &Sandbox, roots: &[&str]) -> BTreeMap<String, Option<String>> {
    fn walk(sandbox: &Path, path: &Path, files: &mut BTreeMap<String, Option<String>>) {
        let Ok(kind) = std::fs::metadata(path) else {
            return;
        };
        let relative = path.strip_prefix(sandbox).expect("under sandbox").to_string_lossy().into_owned();
        if kind.is_dir() {
            files.insert(format!("{relative}/"), None);
            for entry in std::fs::read_dir(path).expect("read dir").flatten() {
                walk(sandbox, &entry.path(), files);
            }
        } else {
            files.insert(relative, Some(String::from_utf8_lossy(&std::fs::read(path).expect("read file")).into_owned()));
        }
    }
    let mut files = BTreeMap::new();
    for root in roots {
        walk(&sandbox.path, &sandbox.path.join(root), &mut files);
    }
    files
}

/// The identity the repositories a step makes are given, so no setting of the host's is read.
const GIT_IDENTITY: [&str; 8] = ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "-c", "init.defaultBranch=main"];

/// Runs one git command line of a step in the sandbox, with the host's git, a fixed identity and fixed instants.
fn run_git(sandbox: &Path, args: &[String]) {
    let output = std::process::Command::new(host_program("git"))
        .args(GIT_IDENTITY)
        .args(args)
        .current_dir(sandbox)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", sandbox.join("home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "2026-11-03T12:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-03T12:00:00Z")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

/// What one step printed, and the files it left behind.
pub struct Outcome {
    pub exit: i64,
    pub stdout: String,
    pub stderr: String,
    pub files: BTreeMap<String, Option<String>>,
}

/// One scenario's sandbox, with the steps replayed in it.
pub struct Replay {
    pub dir: PathBuf,
    pub scenario: Value,
    pub sandbox: Sandbox,
}

impl Replay {
    pub fn new(name: &str) -> Replay {
        let dir = fixtures().join(name);
        let scenario = read_json(&dir.join("scenario.json"));
        let sandbox = Sandbox::new();
        std::fs::create_dir_all(sandbox.path.join("home")).expect("create home");
        std::fs::create_dir_all(sandbox.path.join("work")).expect("create work");
        copy_tree(&dir.join("state"), &sandbox.path.join("state"));
        Replay { dir, scenario, sandbox }
    }

    fn substitute(&self, text: &str) -> String {
        text.replace("@SANDBOX@", &self.sandbox.text())
    }

    /// Applies a step's file changes, runs its command line and reports what happened.
    pub fn step(&self, step: &Value) -> Outcome {
        let _held = EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let sandbox = &self.sandbox.path;
        for path in list(step, "remove") {
            let target = sandbox.join(path.as_str().unwrap_or_default());
            let _ = std::fs::remove_dir_all(&target);
            let _ = std::fs::remove_file(&target);
        }
        for path in list(step, "mkdir") {
            std::fs::create_dir_all(sandbox.join(path.as_str().unwrap_or_default())).expect("mkdir");
        }
        if let Some(Value::Obj(writes)) = field(step, "write") {
            for (path, content) in writes.entries() {
                let target = sandbox.join(path);
                std::fs::create_dir_all(target.parent().expect("parent")).expect("create parent");
                std::fs::write(target, self.substitute(content.as_str().unwrap_or_default())).expect("write file");
            }
        }
        for path in list(step, "exec") {
            use std::os::unix::fs::PermissionsExt;
            let target = sandbox.join(path.as_str().unwrap_or_default());
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        for name in list(step, "shims") {
            let name = name.as_str().unwrap_or_default();
            std::fs::create_dir_all(sandbox.join("bin")).expect("bin");
            let link = sandbox.join("bin").join(name);
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(host_program(name), link).expect("symlink");
        }
        for argv in list(step, "git") {
            let args: Vec<String> = match argv {
                Value::Arr(items) => items.iter().map(|arg| self.substitute(arg.as_str().unwrap_or_default())).collect(),
                _ => Vec::new(),
            };
            run_git(sandbox, &args);
        }
        let mut env: HashMap<String, String> = HashMap::new();
        let path = match field(step, "path") {
            Some(Value::Arr(dirs)) => dirs.iter().map(|dir| sandbox.join(dir.as_str().unwrap_or_default()).to_string_lossy().into_owned()).collect::<Vec<_>>().join(":"),
            _ => std::env::var("PATH").unwrap_or_default(),
        };
        env.insert("PATH".into(), path);
        env.insert("HOME".into(), sandbox.join("home").to_string_lossy().into_owned());
        env.insert("XDG_STATE_HOME".into(), sandbox.join("state").to_string_lossy().into_owned());
        if let Some(Value::Obj(overrides)) = field(step, "env") {
            for (key, value) in overrides.entries() {
                match value {
                    Value::Str(value) => {
                        env.insert(key.to_string(), self.substitute(value));
                    }
                    _ => {
                        env.remove(key);
                    }
                }
            }
        }
        let now = match field(step, "now").and_then(Value::as_str) {
            Some(now) => timestamps::parse(now).expect("a step's now is an ISO instant"),
            None => FIXED_NOW,
        };
        let stdin = self.substitute(text(step, "stdin"));
        let runtime = Runtime {
            gateway: model_gateway::ports::Runtime {
                clock: Box::new(FixedClock(now)),
                files: Box::new(SystemFiles { cwd: sandbox.join("work").to_string_lossy().into_owned() }),
                lock: model_gateway::ports::LockTimings::default(),
                process: Box::new(model_gateway::adapters::process::SystemProcess { env: env.clone() }),
                git: Box::new(model_gateway::adapters::git::SystemGit { env: env.clone(), timeout_ms: model_gateway::adapters::git::TIMEOUT_MS }),
                env: env.clone(),
                defaults: model_gateway::SHIPPED_DEFAULTS.to_string(),
                defaults_path: "defaults.json".to_string(),
                routing: model_gateway::SHIPPED_ROUTING.to_string(),
            },
            clock: Box::new(FixedClock(now)),
            files: Box::new(SystemFiles { cwd: sandbox.join("work").to_string_lossy().into_owned() }),
            dirs: Box::new(crate::dirs::SystemDirs),
            git: Box::new(SystemGit { env: env.clone() }),
            env,
            assets_root: "/assets".to_string(),
            stdin: Box::new(move || stdin.clone()),
        };
        let argv: Vec<String> = list(step, "argv").iter().map(|arg| self.substitute(arg.as_str().unwrap_or_default())).collect();
        let result = run_cli(&argv, &runtime);
        let normalise = |text: &str| text.replace(&self.sandbox.text(), "@SANDBOX@");
        let extra: Vec<&str> = list(step, "roots").iter().filter_map(Value::as_str).collect();
        let roots: Vec<&str> = ["state", "home", "work"].into_iter().chain(extra).collect();
        let files = snapshot(&self.sandbox, &roots)
            .into_iter()
            .map(|(path, content)| (normalise(&path), content.map(|content| normalise(&content))))
            .collect();
        Outcome { exit: i64::from(result.exit_code), files, stderr: normalise(&result.stderr), stdout: normalise(&result.stdout) }
    }
}

/// The golden of one step: its exit code, output and files.
pub fn expected(golden: &Value, index: usize) -> Outcome {
    let step = match field(golden, "steps") {
        Some(Value::Arr(steps)) => &steps[index],
        _ => panic!("golden without steps"),
    };
    let files = match field(step, "files") {
        Some(Value::Obj(files)) => files.entries().into_iter().map(|(path, content)| (path.to_string(), content.as_str().map(str::to_string))).collect(),
        _ => BTreeMap::new(),
    };
    let exit = match field(step, "exit") {
        Some(Value::Num(code)) => *code as i64,
        _ => -1,
    };
    Outcome { exit, files, stderr: text(step, "stderr").to_string(), stdout: text(step, "stdout").to_string() }
}

pub fn scenario_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .expect("fixtures")
        .flatten()
        .filter(|entry| entry.path().join("scenario.json").exists())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Whether `SCENARIO` (an exact name, or a prefix ending in `-`) asks for this scenario; without it every scenario is replayed.
fn is_wanted(name: &str, only: Option<&str>) -> bool {
    match only {
        Some(only) => only == name || (only.ends_with('-') && name.starts_with(only)),
        None => true,
    }
}

#[test]
fn which_scenarios_are_replayed() {
    assert!(is_wanted("cli-input", None) && is_wanted("allow-basic", None));
    assert!(is_wanted("allow-basic", Some("allow-basic")) && !is_wanted("allow-files", Some("allow-basic")));
    assert!(is_wanted("allow-files", Some("allow-")) && !is_wanted("cli-input", Some("allow-")));
}

#[test]
fn every_scenario_gives_the_bytes_the_node_runner_gave() {
    let names = scenario_names();
    assert!(names.len() >= 7, "the fixture scenarios are missing: {names:?}");
    // Every mismatch is collected, so one run shows all of them; SCENARIO=<name> replays one scenario.
    let only = std::env::var("SCENARIO").ok();
    let mut mismatches: Vec<String> = Vec::new();
    for name in names.iter().filter(|name| is_wanted(name, only.as_deref())) {
        let replay = Replay::new(name);
        let golden = read_json(&replay.dir.join("expected.json"));
        let steps = list(&replay.scenario, "steps");
        assert!(!steps.is_empty(), "{name} has no steps");
        for (index, step) in steps.iter().enumerate() {
            let want = expected(&golden, index);
            let got = replay.step(step);
            let label = format!("{name} step {index} {:?}", list(step, "argv"));
            if got.stdout != want.stdout {
                mismatches.push(format!("{label}: stdout\n--- want\n{}\n--- got\n{}", want.stdout, got.stdout));
            }
            if got.stderr != want.stderr {
                mismatches.push(format!("{label}: stderr\n--- want\n{}\n--- got\n{}", want.stderr, got.stderr));
            }
            if got.exit != want.exit {
                mismatches.push(format!("{label}: exit code want {} got {}", want.exit, got.exit));
            }
            if got.files != want.files {
                let mut changed: Vec<&String> = want.files.keys().chain(got.files.keys()).filter(|path| want.files.get(*path) != got.files.get(*path)).collect();
                changed.dedup();
                let detail: Vec<String> = changed.iter().map(|path| format!("{path}\n--- want {:?}\n--- got {:?}", want.files.get(*path), got.files.get(*path))).collect();
                mismatches.push(format!("{label}: files differ\n{}", detail.join("\n")));
            }
        }
    }
    let shown: Vec<&String> = mismatches.iter().take(6).collect();
    assert!(mismatches.is_empty(), "{} mismatches, first {}:\n{}", mismatches.len(), shown.len(), shown.iter().map(|m| m.as_str()).collect::<Vec<_>>().join("\n====\n"));
}
