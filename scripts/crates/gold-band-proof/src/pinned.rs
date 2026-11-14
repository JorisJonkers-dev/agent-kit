//! The pinned shim agent: a deterministic ACP agent that a live run binds to a
//! gate, copier or verdict node in place of a model (issue #455).
//!
//! Gold Band has no command node, so a gate is a worker whose agent runs one
//! command. In a proof that agent is the stub; in a live run it is this one,
//! started by the launcher (`gold-band-proof run --pins`) with its role and its
//! one command fixed in the adapter's environment. Nothing in the prompt can
//! change the command: the prompt is read only for the run's place (project,
//! task, run and attempt), and only from the hidden runtime blocks Gold Band
//! puts at its start, never from the requirement or the goal after them.
//!
//! * A **gate** runs `gold-band-gate run` around its pinned command and answers
//!   `{"exit_code": N}`.
//! * A **copier** runs `gold-band-gate verify` for the same command and answers
//!   with the report it printed, or a report-shaped refusal carrying its code.
//! * A **verdict** reads `findings-check.json` from its round's finding-check
//!   report directory and answers `{"actionable": N}`.
//!
//! Each session runs its command once: a later prompt in the same session (Gold
//! Band's finalize turn) gets the first answer again. A prompt whose run place
//! cannot be read, or a verdict whose report is missing or for another run, is
//! answered with a protocol error, which Gold Band pauses as runtime-abnormal,
//! so it can never read as a pass or as an actionable finding.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::stub::{self, CapturingHost, Role};

/// The environment variable naming the role: `gate:<shim>`, `copy:<shim>` or
/// `verdict:<round>`.
pub const ROLE_ENV: &str = "PINNED_ACP_ROLE";

/// The environment variable holding the pinned command, a JSON array of strings.
pub const COMMAND_ENV: &str = "PINNED_ACP_COMMAND";

/// The environment variable naming the directory every run's reports go under.
pub const REPORT_ROOT_ENV: &str = "PINNED_ACP_REPORT_ROOT";

/// The environment variable holding the gate's time limit in seconds.
pub const TIMEOUT_ENV: &str = "PINNED_ACP_TIMEOUT_SECS";

/// The environment variable holding the comma separated model ids to offer.
pub const MODELS_ENV: &str = "PINNED_ACP_MODELS";

/// The environment variable holding the launcher's placeholder values, a JSON
/// object of strings (`tools`, `requirement`).
pub const VALUES_ENV: &str = "PINNED_ACP_VALUES";

/// The seconds a gate may run when no time limit is configured.
const DEFAULT_TIMEOUT_SECS: u64 = 3600;

/// The JSON-RPC error code of a prompt the agent refuses to answer.
const REFUSED: i64 = -32000;

/// How a pinned agent is configured, all of it from the adapter's environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// What the agent answers as.
    pub role: Option<Role>,
    /// The pinned command, before its placeholders are filled.
    pub command: Vec<String>,
    /// The directory every run's reports go under.
    pub report_root: PathBuf,
    /// The gate's time limit in seconds.
    pub timeout_secs: u64,
    /// The comma separated model ids the agent offers.
    pub models: String,
    /// The launcher's placeholder values.
    pub values: BTreeMap<String, String>,
}

impl Config {
    /// The configuration the environment lookup gives, or why it is unusable.
    pub fn from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<Config, String> {
        let role = match Role::parse(&get(ROLE_ENV).unwrap_or_default()) {
            role @ (Role::Gate(_) | Role::Copy(_) | Role::Verdict(_)) => role,
            _ => {
                return Err(format!(
                    "{} must be gate:<shim>, copy:<shim> or verdict:<round>",
                    ROLE_ENV
                ))
            }
        };
        let command: Vec<String> = match get(COMMAND_ENV) {
            Some(text) => serde_json::from_str(&text)
                .map_err(|_| format!("{} must be a JSON array of strings", COMMAND_ENV))?,
            None => Vec::new(),
        };
        if command.is_empty() && !matches!(role, Role::Verdict(_)) {
            return Err(format!("{} names no command", COMMAND_ENV));
        }
        let report_root = match get(REPORT_ROOT_ENV) {
            Some(root) if !root.is_empty() => PathBuf::from(root),
            _ => return Err(format!("{} names no directory", REPORT_ROOT_ENV)),
        };
        let timeout_secs = match get(TIMEOUT_ENV) {
            Some(text) => text
                .parse::<u64>()
                .ok()
                .filter(|secs| *secs > 0)
                .ok_or_else(|| format!("{} must be a positive number", TIMEOUT_ENV))?,
            None => DEFAULT_TIMEOUT_SECS,
        };
        let values: BTreeMap<String, String> = match get(VALUES_ENV) {
            Some(text) => serde_json::from_str(&text)
                .map_err(|_| format!("{} must be a JSON object of strings", VALUES_ENV))?,
            None => BTreeMap::new(),
        };
        Ok(Config {
            role: Some(role),
            command,
            report_root,
            timeout_secs,
            models: get(MODELS_ENV).unwrap_or_default(),
            values,
        })
    }
}

/// Where in Gold Band's run tree one prompt was sent from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    /// The project directory name.
    pub project: String,
    /// The task id, `task-001`.
    pub task: String,
    /// The run id, `run-001`.
    pub run: String,
    /// The attempt number, 1 for `attempt-001`.
    pub attempt: u32,
}

impl Place {
    /// The run id the gate report carries: `<project>/<task>/<run>`.
    pub fn run_id(&self) -> String {
        format!("{}/{}/{}", self.project, self.task, self.run)
    }

    /// The report directory of one shim of this run under `root`.
    pub fn report_dir(&self, root: &Path, shim: &str) -> PathBuf {
        root.join(&self.project)
            .join(&self.task)
            .join(&self.run)
            .join(shim)
    }
}

/// The leading hidden blocks of a prompt: Gold Band's own runtime context,
/// which comes before the requirement and the node goal. Reading only these
/// keeps a requirement that quotes a runtime line from moving the reports.
pub fn leading_hidden(prompt: &str) -> String {
    let mut rest = prompt.trim_start();
    let mut out = String::new();
    while rest.starts_with("<hidden") {
        let Some(end) = rest.find("</hidden>") else {
            break;
        };
        out.push_str(&rest[..end]);
        out.push('\n');
        rest = rest[end + "</hidden>".len()..].trim_start();
    }
    out
}

/// A name that is safe as one path component: letters, digits, `.`, `_` and
/// `-`, not starting with a dot.
fn component(text: &str) -> Option<String> {
    let ok = !text.is_empty()
        && !text.starts_with('.')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    ok.then(|| text.to_string())
}

/// The run place the leading hidden blocks of `prompt` name, or why it cannot
/// be read.
pub fn place(prompt: &str) -> Result<Place, String> {
    let context = leading_hidden(prompt);
    let field = |label: &str| -> Result<String, String> {
        let value = context
            .lines()
            .find_map(|line| line.trim().strip_prefix("- ")?.strip_prefix(label))
            .map(str::trim)
            .ok_or_else(|| format!("the prompt names no `{}`", label.trim_end()))?;
        component(value)
            .ok_or_else(|| format!("`{}` is not a plain name: {}", label.trim_end(), value))
    };
    let attempt = field("Attempt: ")?;
    let number = attempt
        .strip_prefix("attempt-")
        .and_then(|digits| digits.parse::<u32>().ok())
        .filter(|number| *number > 0)
        .ok_or_else(|| format!("`Attempt:` is not attempt-<n>: {}", attempt))?;
    Ok(Place {
        project: field("Project: ")?,
        task: field("Task: ")?,
        run: field("Run: ")?,
        attempt: number,
    })
}

/// The pinned command with its placeholders filled: `{repo}` (the session's
/// working directory), `{report_dir}`, `{run_id}`, `{attempt}` and each
/// launcher value by its key.
pub fn fill(command: &[String], values: &BTreeMap<String, String>) -> Vec<String> {
    command
        .iter()
        .map(|argument| {
            values.iter().fold(argument.clone(), |text, (key, value)| {
                text.replace(&format!("{{{}}}", key), value)
            })
        })
        .collect()
}

/// The pinned agent: its configuration, the working directory of each session
/// and the answer each session gave first.
pub struct Pinned {
    config: Config,
    next_session: u64,
    cwd: BTreeMap<String, String>,
    answers: BTreeMap<String, Result<String, String>>,
}

impl Pinned {
    /// An agent with no sessions yet.
    pub fn new(config: Config) -> Pinned {
        Pinned {
            config,
            next_session: 0,
            cwd: BTreeMap::new(),
            answers: BTreeMap::new(),
        }
    }

    /// The frames to emit for one request, in order.
    pub fn handle(&mut self, request: &Value) -> Vec<Value> {
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params");
        match method {
            "initialize" => vec![stub::reply(
                request,
                json!({
                    "protocolVersion": 1,
                    "agentCapabilities": { "loadSession": false },
                    "authMethods": [],
                }),
            )],
            "session/new" => {
                self.next_session += 1;
                let session_id = format!("pinned-session-{}", self.next_session);
                let cwd = stub::param_str(params, "cwd").unwrap_or("").to_string();
                self.cwd.insert(session_id.clone(), cwd);
                vec![stub::reply(
                    request,
                    json!({
                        "sessionId": session_id,
                        "configOptions": stub::model_option(&self.config.models, None),
                    }),
                )]
            }
            "session/set_config_option" => {
                let current = match stub::param_str(params, "configId") {
                    Some("model") => stub::param_str(params, "value"),
                    _ => None,
                };
                vec![stub::reply(
                    request,
                    json!({ "configOptions": stub::model_option(&self.config.models, current) }),
                )]
            }
            "session/list" => vec![stub::reply(request, json!({ "sessions": [] }))],
            "session/set_mode" | "session/delete" | "session/close" => {
                vec![stub::reply(request, json!({}))]
            }
            "session/prompt" => self.prompt(request, params),
            _ => match request.get("id") {
                Some(id) if !id.is_null() => {
                    vec![stub::error(request, -32601, "Method not found")]
                }
                _ => Vec::new(),
            },
        }
    }

    /// The frames for a prompt: the session's answer, or a refusal.
    fn prompt(&mut self, request: &Value, params: Option<&Value>) -> Vec<Value> {
        let session_id = stub::param_str(params, "sessionId")
            .unwrap_or("")
            .to_string();
        let answer = match self.answers.get(&session_id) {
            Some(answer) => answer.clone(),
            None => {
                let cwd = self.cwd.get(&session_id).cloned().unwrap_or_default();
                let answer = self.answer(&cwd, &stub::prompt_text(params));
                self.answers.insert(session_id.clone(), answer.clone());
                answer
            }
        };
        match answer {
            Ok(text) => vec![
                stub::notify(
                    "session/update",
                    json!({
                        "sessionId": session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": text },
                        },
                    }),
                ),
                stub::reply(request, json!({ "stopReason": "end_turn" })),
            ],
            Err(problem) => vec![stub::error(
                request,
                REFUSED,
                &format!("pinned shim agent: {}", problem),
            )],
        }
    }

    /// The answer text for the first prompt of a session.
    fn answer(&self, cwd: &str, prompt: &str) -> Result<String, String> {
        let place = place(prompt)?;
        match self.config.role.as_ref() {
            Some(Role::Gate(shim)) => Ok(stub::fenced(&self.gate(cwd, &place, shim)?)),
            Some(Role::Copy(shim)) => self.copy(cwd, &place, shim),
            Some(Role::Verdict(round)) => Ok(stub::fenced(&self.verdict(&place, *round)?)),
            _ => Err("no gate, copy or verdict role".to_string()),
        }
    }

    /// The command and the report directory of one shim for this run, the
    /// directory created.
    fn command(
        &self,
        cwd: &str,
        place: &Place,
        shim: &str,
    ) -> Result<(Vec<String>, PathBuf), String> {
        let dir = place.report_dir(&self.config.report_root, shim);
        fs::create_dir_all(&dir)
            .map_err(|error| format!("cannot create {}: {}", dir.display(), error))?;
        let mut values = self.config.values.clone();
        values.insert("repo".to_string(), cwd.to_string());
        values.insert("report_dir".to_string(), dir.to_string_lossy().into_owned());
        values.insert("run_id".to_string(), place.run_id());
        values.insert("attempt".to_string(), place.attempt.to_string());
        Ok((fill(&self.config.command, &values), dir))
    }

    /// Runs the gate wrapper around the pinned command.
    fn gate(&self, cwd: &str, place: &Place, shim: &str) -> Result<Value, String> {
        let (command, dir) = self.command(cwd, place, shim)?;
        let mut args: Vec<OsString> = vec![
            "run".into(),
            "--run-id".into(),
            place.run_id().into(),
            "--attempt".into(),
            place.attempt.to_string().into(),
            "--timeout-secs".into(),
            self.config.timeout_secs.to_string().into(),
            "--report-dir".into(),
            dir.into_os_string(),
            "--".into(),
        ];
        args.extend(command.into_iter().map(OsString::from));
        let host = CapturingHost::default();
        Ok(json!({ "exit_code": gold_band_gate::run_with(args, &host) }))
    }

    /// Runs the copier for the pinned command.
    fn copy(&self, cwd: &str, place: &Place, shim: &str) -> Result<String, String> {
        let (command, dir) = self.command(cwd, place, shim)?;
        let args: Vec<OsString> = vec![
            "verify".into(),
            "--run-id".into(),
            place.run_id().into(),
            "--attempt".into(),
            place.attempt.to_string().into(),
            "--command".into(),
            command.join(" ").into(),
            "--report-dir".into(),
            dir.into_os_string(),
        ];
        let host = CapturingHost::default();
        let code = gold_band_gate::run_with(args, &host);
        if code == 0 {
            return Ok(stub::fenced_text(host.out.borrow().trim()));
        }
        Ok(stub::fenced(&stub::report_document(code)))
    }

    /// Reads the actionable count of round `round`'s finding check.
    fn verdict(&self, place: &Place, round: u32) -> Result<Value, String> {
        let path = place
            .report_dir(&self.config.report_root, &format!("findings-{}", round))
            .join("findings-check.json");
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {}: {}", path.display(), error))?;
        let report: Value =
            serde_json::from_str(&text).map_err(|_| format!("{} is not JSON", path.display()))?;
        if report.get("run_id").and_then(Value::as_str) != Some(place.run_id().as_str()) {
            return Err(format!("{} is for another run", path.display()));
        }
        let actionable = report
            .get("actionable")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{} has no actionable count", path.display()))?;
        Ok(json!({ "actionable": actionable }))
    }
}

/// Serves the pinned agent over a reader and a writer until the reader ends.
///
/// Blank lines and lines that are not a JSON object are skipped; every frame is
/// written as one line of compact JSON and flushed.
pub fn serve<R: BufRead, W: Write>(
    input: R,
    output: &mut W,
    config: Config,
) -> std::io::Result<()> {
    let mut agent = Pinned::new(config);
    for line in input.lines() {
        let line = line?;
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !request.is_object() {
            continue;
        }
        for frame in agent.handle(&request) {
            writeln!(output, "{frame}")?;
            output.flush()?;
        }
    }
    Ok(())
}
