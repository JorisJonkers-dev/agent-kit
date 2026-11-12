//! Replays every scenario under tests/fixtures through the library and compares
//! stdout, stderr, the exit code and every file under the sandbox with the
//! goldens the node runner produced.

use crate::adapters::fs::SystemFiles;
use crate::cli::run_cli;
use crate::json::{parse, Value};
use crate::ports::{Clock, LockTimings, Runtime};
use crate::timestamps;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// 2026-11-03T12:00:00Z, the instant fake-clock.mjs gives the node runner.
pub const FIXED_NOW: i64 = 1_793_707_200_000;

pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
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
        let name = format!("model-gateway-test-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst));
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
        let mut env: HashMap<String, String> = HashMap::new();
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
        let runtime = Runtime {
            clock: Box::new(FixedClock(now)),
            files: Box::new(SystemFiles { cwd: sandbox.join("work").to_string_lossy().into_owned() }),
            env,
            lock: LockTimings::default(),
        };
        let argv: Vec<String> = list(step, "argv").iter().map(|arg| self.substitute(arg.as_str().unwrap_or_default())).collect();
        let result = run_cli(&argv, &runtime);
        let normalise = |text: &str| text.replace(&self.sandbox.text(), "@SANDBOX@");
        let files = snapshot(&self.sandbox, &["state", "home", "work"])
            .into_iter()
            .map(|(path, content)| (path, content.map(|content| normalise(&content))))
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

#[test]
fn every_scenario_gives_the_bytes_the_node_runner_gave() {
    let names = scenario_names();
    assert!(names.len() >= 7, "the fixture scenarios are missing: {names:?}");
    for name in names {
        let replay = Replay::new(&name);
        let golden = read_json(&replay.dir.join("expected.json"));
        let steps = list(&replay.scenario, "steps");
        assert!(!steps.is_empty(), "{name} has no steps");
        for (index, step) in steps.iter().enumerate() {
            let want = expected(&golden, index);
            let got = replay.step(step);
            let label = format!("{name} step {index} {:?}", list(step, "argv"));
            assert_eq!(got.stdout, want.stdout, "{label}: stdout");
            assert_eq!(got.stderr, want.stderr, "{label}: stderr");
            assert_eq!(got.exit, want.exit, "{label}: exit code");
            assert_eq!(got.files, want.files, "{label}: files");
        }
    }
}
