//! A machine for the tests of the guards and the escalation: a sandbox for the
//! state and configuration homes with the real files in it, a clock that stands
//! still, a git that finds no repository and a process port that knows which
//! programs are installed and what a status command answers.

use super::scenarios::{FixedClock, Sandbox};
use crate::adapters::fs::SystemFiles;
use crate::config::CostClass;
use crate::grants::{grant_file_text, Grant};
use crate::ledger::{LedgerEntry, ReservationRecord};
use crate::ledger_file::LedgerStore;
use crate::locations::{grants_path, ledger_path, override_path};
use crate::ports::{Git, GitRemote, LockTimings, Process, ProcessResult, Runtime};
use crate::state_file::workflows_path;
use crate::timestamps;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 2026-11-04T10:00:00.000Z.
pub fn now_ms() -> i64 {
    timestamps::parse("2026-11-04T10:00:00.000Z").expect("a timestamp")
}

/// The programs a machine has installed and what their commands answer.
#[derive(Clone, Default)]
pub struct FakeProcess {
    installed: Arc<Mutex<Vec<String>>>,
    answers: Arc<Mutex<HashMap<String, Result<ProcessResult, String>>>>,
}

impl FakeProcess {
    pub fn install(&self, binary: &str) {
        self.installed.lock().expect("the installed list").push(binary.to_string());
    }

    pub fn answer(&self, command: &str, result: Result<ProcessResult, String>) {
        self.answers.lock().expect("the answers").insert(command.to_string(), result);
    }
}

impl Process for FakeProcess {
    fn resolve(&self, binary: &str) -> Option<String> {
        self.installed.lock().expect("the installed list").iter().any(|name| name == binary).then(|| format!("/bin/{binary}"))
    }

    fn exec(&self, command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.answers.lock().expect("the answers").get(command).cloned().unwrap_or_else(|| Err("no answer was stubbed".to_string()))
    }

    fn run(&self, command: &crate::ports::ProcessCommand) -> Result<ProcessResult, String> {
        let args: Vec<&str> = command.args.iter().map(String::as_str).collect();
        self.exec(&command.command, &args, command.timeout_ms)
    }
}

/// A git that finds no repository anywhere.
struct NoRepositories;

impl Git for NoRepositories {
    fn common_dir(&self, _path: &str) -> Result<String, String> {
        unreachable!("the test asks for no common directory")
    }

    fn add_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!("the test adds no worktree")
    }

    fn root(&self, _path: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        Ok(Vec::new())
    }

    fn remove_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete_branch(&self, _repo: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    fn git_dir(&self, _path: &str) -> Result<String, String> {
        unreachable!("the test asks for no git directory")
    }

    fn create_worktree(&self, _repo: &str, _path: &str, _branch: &str, _base: &str) -> Result<(), String> {
        unreachable!("the test creates no worktree")
    }

    fn stage_all(&self, _path: &str) -> Result<(), String> {
        unreachable!("the test stages nothing")
    }

    fn staged_files(&self, _path: &str, _base: Option<&str>) -> Result<Vec<String>, String> {
        unreachable!("the test lists nothing")
    }

    fn list_worktrees(&self, _repo: &str) -> Result<Vec<String>, String> {
        unreachable!("the test watches no worktree")
    }

    fn status(&self, _path: &str) -> Result<String, String> {
        unreachable!("the test reads no status")
    }
}

pub struct Machine {
    pub sandbox: Sandbox,
    pub runtime: Runtime,
    pub process: FakeProcess,
}

impl Machine {
    /// A machine whose shipped defaults are `defaults`, and whose clock reads `now`.
    pub fn at(defaults: &str, now: i64) -> Machine {
        let sandbox = Sandbox::new();
        let root = sandbox.text();
        let env = HashMap::from([
            ("HOME".to_string(), format!("{root}/home")),
            ("XDG_STATE_HOME".to_string(), format!("{root}/state")),
        ]);
        let process = FakeProcess::default();
        let runtime = Runtime {
            clock: Box::new(FixedClock(now)),
            files: Box::new(SystemFiles { cwd: root }),
            lock: LockTimings::default(),
            process: Box::new(process.clone()),
            git: Box::new(NoRepositories),
            env,
            defaults: defaults.to_string(),
            defaults_path: "defaults.json".to_string(),
            routing: crate::SHIPPED_ROUTING.to_string(),
        };
        Machine { sandbox, runtime, process }
    }

    pub fn new(defaults: &str) -> Machine {
        Machine::at(defaults, now_ms())
    }

    /// A file written under the sandbox, its directories made.
    pub fn write_at(&self, path: &str, text: &str) {
        let parent = std::path::Path::new(path).parent().expect("a parent");
        std::fs::create_dir_all(parent).expect("create dirs");
        std::fs::write(path, text).expect("write file");
    }

    pub fn write_override(&self, text: &str) {
        self.write_at(&override_path(&self.runtime.env).expect("override path"), text);
    }

    pub fn write_grants(&self, grants: &[Grant]) {
        self.write_at(&grants_path(&self.runtime.env).expect("grants path"), &grant_file_text(grants));
    }

    /// The workflows the machine has started, as `(name, started_at)`.
    pub fn write_workflows(&self, started: &[(&str, &str)]) {
        let records: Vec<String> = started.iter().map(|(name, at)| format!(r#"{{"name":"{name}","started_at":"{at}"}}"#)).collect();
        self.write_at(&workflows_path(&self.runtime.env).expect("workflows path"), &format!(r#"{{"version":1,"workflows":[{}]}}"#, records.join(",")));
    }

    pub fn ledger_file(&self) -> String {
        ledger_path(&self.runtime.env).expect("ledger path")
    }

    /// The ledger's own text.
    pub fn ledger_text(&self) -> String {
        std::fs::read_to_string(self.ledger_file()).unwrap_or_default()
    }

    pub fn write_ledger(&self, lines: &[String]) {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        self.write_at(&self.ledger_file(), &text);
    }

    pub fn ledger(&self) -> Vec<LedgerEntry> {
        (LedgerStore { runtime: &self.runtime }).read().expect("the ledger")
    }

    /// The reservation lines the ledger holds.
    pub fn reservations(&self) -> Vec<ReservationRecord> {
        self.ledger()
            .into_iter()
            .filter_map(|entry| match entry {
                LedgerEntry::Reservation(record) => Some(record),
                _ => None,
            })
            .collect()
    }
}

/// A grant of `cap_usd` for workflow `w` and client `codex`, written at 09:00 and lapsing in `expires_in_s` seconds from 10:00.
pub fn grant(cap_usd: f64, expires_in_s: f64) -> Grant {
    Grant {
        answer: "yes".to_string(),
        cap_usd,
        max_starts: None,
        client: "codex".to_string(),
        expires_at: timestamps::to_iso(now_ms() + (expires_in_s * 1000.0) as i64),
        id: "grant-1".to_string(),
        question: "may I?".to_string(),
        workflow: "w".to_string(),
        written_at: "2026-11-04T09:00:00.000Z".to_string(),
        ended_at: None,
    }
}

/// A reservation line for a metered run.
pub fn reservation_line(run_id: &str, run_dir: &str, estimate_usd: f64, expires_at: &str, cost_class: CostClass) -> String {
    format!(
        r#"{{"client":"codex","cost_class":"{}","estimate_usd":{estimate_usd},"expires_at":"{expires_at}","kind":"reservation","model":"gpt-5.5","reserved_at":"2026-11-04T09:30:00.000Z","run_dir":"{run_dir}","run_id":"{run_id}","workflow":"w"}}"#,
        cost_class.as_str()
    )
}

/// A catalogue the guards and the escalation are tested against: subsidised, metered-personal and
/// metered-employer clients, three Anthropic models and one of each other family.
pub const DEFAULTS: &str = r#"{
  "version": 1,
  "clients": {
    "claude": {"binary": "claude", "cost_class": "subsidised", "model_source": "declared"},
    "vibe": {"binary": "vibe", "cost_class": "subsidised", "model_source": "declared"},
    "codex": {"binary": "codex", "cost_class": "metered-employer", "model_source": "declared"},
    "hermes": {"binary": "hermes", "cost_class": "metered-personal", "model_source": "declared"},
    "opencode": {"binary": "opencode", "cost_class": "metered-personal", "model_source": "declared",
                 "provider_cost_classes": {"opencode-go": "subsidised"}}
  },
  "models": [
    {"client": "claude", "family": "anthropic", "model": "claude-opus-5", "notes": "n", "tier": "frontier"},
    {"client": "claude", "family": "anthropic", "model": "claude-sonnet-5-5", "notes": "n", "tier": "hard"},
    {"client": "claude", "family": "anthropic", "model": "claude-haiku-5", "notes": "n", "tier": "routine"},
    {"client": "vibe", "family": "zai", "model": "zai-glm-5-3", "notes": "n", "tier": "hard"},
    {"client": "codex", "family": "openai", "model": "gpt-5.5", "notes": "n", "tier": "frontier",
     "prices": {"input": 5, "cached_input": 0.5, "output": 30}},
    {"client": "hermes", "family": "anthropic", "model": "hermes-hard", "notes": "n", "tier": "hard",
     "prices": {"input": 3, "cached_input": 0.3, "output": 15}},
    {"client": "opencode", "family": "moonshot", "model": "opencode-go/kimi-k2", "notes": "n", "tier": "hard"}
  ],
  "settings": {
    "availability_window_minutes": 60,
    "max_batch_delegations": 50,
    "max_parallel_per_client": 3,
    "metered_personal_caps": {"per_day_usd": 5, "per_workflow_usd": 2},
    "output_allowance_tokens": {"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500},
    "plan_warning_ratio": 0.8,
    "summary_max_chars": 1500
  }
}"#;
