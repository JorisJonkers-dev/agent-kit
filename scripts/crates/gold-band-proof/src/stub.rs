//! A stub ACP agent: JSON-RPC 2.0 over newline-delimited JSON on stdio.
//!
//! The stub stands in for every agent id of a Gold Band workflow in a headless
//! proof. It answers the ACP handshake, hands out sessions, offers a model
//! picker and, on every prompt, streams one fenced json block back. What that
//! block holds is the stub's [`Role`]: the findings document a critic answers
//! with, `{"ok": true}` for a model node, the open questions a grill round
//! asks, the exit code a gate shim earned, the report a copier shim verified,
//! or the exit code a grant-check shim would earn. The verdict is a parameter,
//! so the same stub proves the clean path, the needs-human path, the
//! open-questions path, the failing path, a failing gate and a lapsed grant.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use gold_band_gate::{Host, Outcome, RealHost};
use gold_band_report::Clock;
use serde_json::{json, Value};

/// The environment variable naming the verdict the stub reports.
pub const VERDICT_ENV: &str = "STUB_ACP_VERDICT";

/// The environment variable holding the comma separated model ids to offer.
pub const MODELS_ENV: &str = "STUB_ACP_MODELS";

/// The environment variable naming the role the stub answers as.
pub const ROLE_ENV: &str = "STUB_ACP_ROLE";

/// The environment variable naming the directory the stub keeps gate reports in.
pub const REPORT_DIR_ENV: &str = "STUB_ACP_REPORT_DIR";

/// The verdict that makes the stub report one major finding.
pub const NEEDS_HUMAN: &str = "needs-human";

/// The verdict that makes the stub ask one open question.
pub const OPEN_QUESTIONS: &str = "open-questions";

/// The verdict that makes the stub fail the prompt instead of answering it.
pub const FAIL: &str = "fail";

/// The prefix a verdict node's verdict carries: `actionable=K`.
pub const ACTIONABLE_PREFIX: &str = "actionable=";

/// The run id every gate and copier the stub runs reports under.
const RUN_ID: &str = "proof-run";

/// The attempt number every gate and copier the stub runs reports under.
const ATTEMPT: &str = "1";

/// The seconds a gate command the stub runs may take.
const TIMEOUT_SECS: &str = "30";

/// The internal node id of an AI-DYNAMIC node that fans its workers out.
const BOOTSTRAP_NODE: &str = "bootstrap";

/// The label of the internal node id line in an AI-DYNAMIC prompt.
const NODE_LABEL: &str = "内部节点：";

/// The label of the Dynamic root line in an AI-DYNAMIC prompt.
const ROOT_LABEL: &str = "Dynamic 根目录：";

/// The label of the internal node's path relative to the Dynamic root.
const NODE_REL_LABEL: &str = "内部节点（相对 Dynamic 根目录）：";

/// The label of the attempt's path relative to the internal node.
const ATTEMPT_LABEL: &str = "当前 attempt（相对内部节点）：";

/// The label of the attachments path relative to the current attempt.
const ATTACHMENTS_LABEL: &str = "attachments（相对当前 attempt）：";

/// The kind of the completion document an AI-DYNAMIC node answers with.
const COMPLETION_KIND: &str = "dynamic-node-completion";

/// The summary the stub writes into every handoff directory.
const SUMMARY: &str = "Summary written by the headless proof stub.\n";

/// The task the stub writes for a fanned-out critic.
const CRITIC_TASK: &str = "Critic task written by the headless proof stub.\n";

/// The task the stub writes for the merge node of a fan-out.
const MERGE_TASK: &str = "Merge task written by the headless proof stub.\n";

/// The task the stub writes for the acceptance node of a fan-out.
const ACCEPTANCE_TASK: &str = "Acceptance task written by the headless proof stub.\n";

/// What a stub agent answers a prompt with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// The findings document a critic answers with.
    Findings,
    /// `{"ok": true}`, for a model node whose schema has no findings key.
    Answer,
    /// The open questions a grill round asks, for a node whose schema has an
    /// `open_questions` key.
    Questions,
    /// A gate shim: run the gate wrapper and answer its exit code.
    Gate(String),
    /// A copier shim: run the verify and answer the report it prints.
    Copy(String),
    /// A grant-check shim: answer the exit code the pinned
    /// `gold-band-run-start grant-check` wrapper would earn, 1 when the grant
    /// lapsed. No wrapper is run.
    Grant(String),
    /// An agent that serves an ai-dynamic node, listing the agent ids its
    /// fanned-out workers run as: it plans the fan-out and finishes every
    /// internal node with a `dynamic-node-completion`, and answers any other
    /// prompt as [`Role::Answer`] does.
    Dynamic(Vec<String>),
    /// The verdict node of critic round N: answer how many findings are
    /// actionable, `{"actionable": 0}` or `{"actionable": 1}`.
    Verdict(u32),
}

impl Role {
    /// The role a `STUB_ACP_ROLE` value names; anything unknown or empty is
    /// [`Role::Findings`].
    pub fn parse(text: &str) -> Role {
        let text = text.trim();
        if text == "answer" {
            return Role::Answer;
        }
        if text == "questions" {
            return Role::Questions;
        }
        if let Some(name) = text.strip_prefix("gate:").filter(|name| !name.is_empty()) {
            return Role::Gate(name.to_string());
        }
        if let Some(name) = text.strip_prefix("copy:").filter(|name| !name.is_empty()) {
            return Role::Copy(name.to_string());
        }
        if let Some(name) = text.strip_prefix("grant:").filter(|name| !name.is_empty()) {
            return Role::Grant(name.to_string());
        }
        if let Some(list) = text.strip_prefix("dynamic:") {
            return Role::Dynamic(
                list.split(',')
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .collect(),
            );
        }
        if let Some(round) = text
            .strip_prefix("verdict:")
            .and_then(|round| round.parse::<u32>().ok())
        {
            return Role::Verdict(round);
        }
        Role::Findings
    }

    /// The role as `STUB_ACP_ROLE` spells it.
    pub fn as_text(&self) -> String {
        match self {
            Role::Findings => "findings".to_string(),
            Role::Answer => "answer".to_string(),
            Role::Questions => "questions".to_string(),
            Role::Gate(name) => format!("gate:{}", name),
            Role::Copy(name) => format!("copy:{}", name),
            Role::Grant(name) => format!("grant:{}", name),
            Role::Dynamic(workers) => format!("dynamic:{}", workers.join(",")),
            Role::Verdict(round) => format!("verdict:{}", round),
        }
    }
}

/// The findings document for a verdict: one finding for `needs-human`, the
/// empty verdict for anything else.
pub fn findings_document(verdict: &str) -> Value {
    if verdict.trim() == NEEDS_HUMAN {
        json!({
            "findings": [{
                "id": "stub-needs-human-1",
                "severity": "major",
                "category": "undocumented-decision",
                "check": "spec-redteam-needs-human",
                "location": "requirement.md",
                "claim": "The spec stage's verdict is needs-human: this finding is the marker the manual-check node exists to put in front of a person.",
                "evidence": [{
                    "kind": "input",
                    "value": "STUB_ACP_VERDICT=needs-human",
                }],
                "suggestion": "Answer the manual check in Gold Band: the human rules on this finding.",
            }],
        })
    } else {
        json!({
            "findings": [],
            "no_findings_reason": "The stub critic read the requirement through its lenses and raised nothing; this is the empty verdict the headless proof expects.",
        })
    }
}

/// The findings document for a verdict, wrapped in a json code fence.
pub fn fenced_findings(verdict: &str) -> String {
    fenced(&findings_document(verdict))
}

/// The open questions document for a verdict: one question for
/// `open-questions`, none for anything else. It also carries `ok`, so an agent
/// id that serves a grill round and a plain answer node answers both.
pub fn questions_document(verdict: &str) -> Value {
    if verdict.trim() == OPEN_QUESTIONS {
        json!({
            "ok": true,
            "open_questions": [{
                "id": "stub-question-1",
                "question": "The stub's open question: this is the marker a grill round's manual check puts in front of a person.",
            }],
        })
    } else {
        json!({ "ok": true, "open_questions": [] })
    }
}

/// The open questions document for a verdict, wrapped in a json code fence.
pub fn fenced_questions(verdict: &str) -> String {
    fenced(&questions_document(verdict))
}

/// The verdict document for a round: one actionable finding while the round is
/// within the count the verdict names, none after it. A verdict that does not
/// carry a count leaves nothing actionable.
pub fn verdict_document(round: u32, verdict: &str) -> Value {
    let count = verdict
        .trim()
        .strip_prefix(ACTIONABLE_PREFIX)
        .and_then(|count| count.parse::<u32>().ok());
    let actionable = match count {
        Some(count) if round <= count => 1,
        _ => 0,
    };
    json!({ "actionable": actionable })
}

/// A JSON value as one fenced json block.
fn fenced(value: &Value) -> String {
    // A document built from literals always serialises.
    fenced_text(&serde_json::to_string_pretty(value).unwrap_or_default())
}

/// Text as one fenced json block.
fn fenced_text(body: &str) -> String {
    format!("```json\n{}\n```", body)
}

/// A report-shaped refusal: the fields a copier's success condition reads,
/// carrying the exit code the verify earned.
fn report_document(exit_code: i32) -> Value {
    json!({
        "run_id": "",
        "attempt": 0,
        "command": "",
        "exit_code": exit_code,
        "output_hash": "",
        "timestamp": "",
    })
}

/// The command a gate runs: `false` when the verdict names this gate as
/// failing, else `true`. The copier derives its `--command` from the same
/// function, so the two always agree.
fn gate_command(verdict: &str, name: &str) -> Vec<String> {
    let failing = verdict.trim().strip_prefix("gate-fail=") == Some(name);
    if failing {
        vec!["false".to_string()]
    } else {
        vec!["true".to_string()]
    }
}

/// The answer a grant-check shim gives: the exit code the pinned wrapper would
/// earn, 1 when the verdict says this grant lapsed and 0 otherwise. The wrapper
/// itself is not run; the real shim and gateway are proved by their own tests.
fn grant_answer(verdict: &str, name: &str) -> Value {
    let lapsed = verdict.trim().strip_prefix("grant-lapse=") == Some(name);
    json!({ "exit_code": if lapsed { 1 } else { 0 } })
}

/// The ACP model picker for the comma separated model ids, with `current`
/// selected when it names one of them.
pub fn model_option(models: &str, current: Option<&str>) -> Vec<Value> {
    let ids: Vec<&str> = models
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .collect();
    let Some(&first) = ids.first() else {
        return Vec::new();
    };
    let selected = current.filter(|value| !value.is_empty()).unwrap_or(first);
    let options: Vec<Value> = ids
        .iter()
        .map(|id| json!({ "value": id, "name": id }))
        .collect();
    vec![json!({
        "id": "model",
        "name": "Model",
        "category": "model",
        "type": "select",
        "currentValue": selected,
        "options": options,
    })]
}

/// A host that runs commands for real but keeps what the wrapper prints,
/// because stdout is the ACP stream.
#[derive(Default)]
struct CapturingHost {
    /// The text the wrapper printed on stdout; stderr is discarded.
    out: RefCell<String>,
}

impl Clock for CapturingHost {
    fn unix_seconds(&self) -> u64 {
        RealHost.unix_seconds()
    }
}

impl Host for CapturingHost {
    fn execute(&self, argv: &[String], timeout: Duration, output: &Path) -> Outcome {
        RealHost.execute(argv, timeout, output)
    }

    fn print(&self, stderr: bool, text: &str) {
        if !stderr {
            self.out.borrow_mut().push_str(text);
        }
    }
}

/// The stub agent: the sessions it knows, the next session id to hand out, the
/// role it answers as and the directory its gate reports live under.
pub struct Stub {
    sessions: BTreeSet<String>,
    next_session: u64,
    role: Role,
    report_dir: String,
    /// The last completion the dynamic role answered in each session, which it
    /// repeats when Gold Band sends a repair or reminder prompt in that session.
    completions: BTreeMap<String, String>,
}

impl Stub {
    /// A stub that knows no sessions, has handed out no ids yet and answers as
    /// a critic.
    pub fn new() -> Stub {
        Stub::with_role(Role::Findings, String::new())
    }

    /// A stub that answers as `role` and keeps its gate reports under
    /// `report_dir`.
    pub fn with_role(role: Role, report_dir: String) -> Stub {
        Stub {
            sessions: BTreeSet::new(),
            next_session: 0,
            role,
            report_dir,
            completions: BTreeMap::new(),
        }
    }

    /// The frames to emit for one request, in order.
    pub fn handle(&mut self, request: &Value, verdict: &str, models: &str) -> Vec<Value> {
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params");
        match method {
            "initialize" => vec![reply(
                request,
                json!({
                    "protocolVersion": 1,
                    "agentCapabilities": { "loadSession": false },
                    "authMethods": [],
                }),
            )],
            "session/new" => {
                self.next_session += 1;
                let session_id = format!("stub-session-{}", self.next_session);
                self.sessions.insert(session_id.clone());
                vec![reply(
                    request,
                    json!({
                        "sessionId": session_id,
                        "configOptions": model_option(models, None),
                    }),
                )]
            }
            "session/list" => vec![reply(request, json!({ "sessions": [] }))],
            "session/set_config_option" => {
                let current = match param_str(params, "configId") {
                    Some("model") => param_str(params, "value"),
                    _ => None,
                };
                vec![reply(
                    request,
                    json!({ "configOptions": model_option(models, current) }),
                )]
            }
            "session/set_mode" => vec![reply(request, json!({}))],
            "session/delete" | "session/close" => {
                if let Some(session_id) = param_str(params, "sessionId") {
                    self.sessions.remove(session_id);
                }
                vec![reply(request, json!({}))]
            }
            "session/prompt" => self.prompt(request, params, verdict),
            _ => match request.get("id") {
                Some(id) if !id.is_null() => vec![error(request, -32601, "Method not found")],
                _ => Vec::new(),
            },
        }
    }

    /// The frames for a prompt: the role's answer, or the failure error.
    fn prompt(&mut self, request: &Value, params: Option<&Value>, verdict: &str) -> Vec<Value> {
        let session_id = param_str(params, "sessionId")
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| self.sessions.iter().next().cloned())
            .unwrap_or_default();
        if verdict.trim() == FAIL {
            return vec![error(request, -32000, "stub failure")];
        }
        let mut text = self.answer(verdict, &prompt_text(params));
        if matches!(self.role, Role::Dynamic(_)) {
            // A repair or reminder prompt carries no runtime context: answer it
            // with the completion the session gave last.
            if text.contains(COMPLETION_KIND) {
                self.completions.insert(session_id.clone(), text.clone());
            } else if let Some(last) = self.completions.get(&session_id) {
                text = last.clone();
            }
        }
        vec![
            notify(
                "session/update",
                json!({
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": text },
                    },
                }),
            ),
            reply(request, json!({ "stopReason": "end_turn" })),
        ]
    }

    /// The answer text for a prompt: one fenced json block, chosen by the role.
    /// Only the dynamic role reads the prompt text.
    fn answer(&self, verdict: &str, prompt: &str) -> String {
        match &self.role {
            Role::Findings => fenced_findings(verdict),
            Role::Answer => fenced(&json!({ "ok": true })),
            Role::Questions => fenced_questions(verdict),
            Role::Gate(name) => fenced(&self.gate_answer(verdict, name)),
            Role::Copy(name) => self.copy_answer(verdict, name),
            Role::Grant(name) => fenced(&grant_answer(verdict, name)),
            Role::Dynamic(workers) => self.dynamic_answer(prompt, workers),
            Role::Verdict(round) => fenced(&verdict_document(*round, verdict)),
        }
    }

    /// The answer to a gate prompt: run the wrapper and report its exit code.
    fn gate_answer(&self, verdict: &str, name: &str) -> Value {
        let Some(dir) = self.report_dir_for(name) else {
            return json!({ "exit_code": 2 });
        };
        let mut args: Vec<OsString> = vec![
            "run".into(),
            "--run-id".into(),
            RUN_ID.into(),
            "--attempt".into(),
            ATTEMPT.into(),
            "--timeout-secs".into(),
            TIMEOUT_SECS.into(),
            "--report-dir".into(),
            dir.into_os_string(),
            "--".into(),
        ];
        args.extend(gate_command(verdict, name).into_iter().map(OsString::from));
        let host = CapturingHost::default();
        json!({ "exit_code": gold_band_gate::run_with(args, &host) })
    }

    /// The answer to a copier prompt: the report the verify printed, or a
    /// report-shaped refusal carrying its exit code.
    fn copy_answer(&self, verdict: &str, name: &str) -> String {
        let dir = Path::new(&self.report_dir).join(name);
        let args: Vec<OsString> = vec![
            "verify".into(),
            "--run-id".into(),
            RUN_ID.into(),
            "--attempt".into(),
            ATTEMPT.into(),
            "--command".into(),
            gate_command(verdict, name).join(" ").into(),
            "--report-dir".into(),
            dir.into_os_string(),
        ];
        let host = CapturingHost::default();
        let code = gold_band_gate::run_with(args, &host);
        if code == 0 {
            return fenced_text(host.out.borrow().trim());
        }
        fenced(&report_document(code))
    }

    /// The report directory for one shim, created; `None` when it cannot be.
    fn report_dir_for(&self, name: &str) -> Option<PathBuf> {
        let dir = Path::new(&self.report_dir).join(name);
        fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    /// The answer to an AI-DYNAMIC prompt: write the node's handoff files and
    /// answer the completion the node's kind calls for. A prompt without the
    /// runtime context block is not an AI-DYNAMIC one, and a handoff that
    /// cannot be written falls back to the plain answer, so the proof fails
    /// visibly instead of passing on a missing handoff.
    fn dynamic_answer(&self, prompt: &str, workers: &[String]) -> String {
        let field = |label: &str| {
            prompt.lines().find_map(|line| {
                // Gold Band lists the context as a markdown bullet list.
                line.trim()
                    .trim_start_matches("- ")
                    .strip_prefix(label)
                    .map(|value| value.trim().to_string())
            })
        };
        let (Some(node), Some(root), Some(node_rel), Some(attempt), Some(attachments)) = (
            field(NODE_LABEL),
            field(ROOT_LABEL),
            field(NODE_REL_LABEL),
            field(ATTEMPT_LABEL),
            field(ATTACHMENTS_LABEL),
        ) else {
            return fenced(&json!({ "ok": true }));
        };
        let handoff = Path::new(&root)
            .join(&node_rel)
            .join(&attempt)
            .join(&attachments)
            .join("handoff");
        let written = (|| -> std::io::Result<()> {
            fs::create_dir_all(&handoff)?;
            fs::write(handoff.join("summary.md"), SUMMARY)?;
            if node != BOOTSTRAP_NODE {
                return Ok(());
            }
            for index in 1..=workers.len() {
                fs::write(handoff.join(format!("critic-{}.md", index)), CRITIC_TASK)?;
            }
            fs::write(handoff.join("merge.md"), MERGE_TASK)?;
            fs::write(handoff.join("acceptance.md"), ACCEPTANCE_TASK)
        })();
        if written.is_err() {
            return fenced(&json!({ "ok": true }));
        }
        if node != BOOTSTRAP_NODE {
            return fenced(&json!({
                "version": "0.1",
                "kind": "dynamic-node-completion",
                "status": "success",
                "summaryPath": "handoff/summary.md",
                "next": { "type": "end" },
            }));
        }
        let nodes: Vec<Value> = workers
            .iter()
            .enumerate()
            .map(|(index, provider)| {
                json!({
                    "id": format!("critic-{}", index + 1),
                    "kind": "worker",
                    "title": format!("Critic {}", index + 1),
                    "taskPath": format!("handoff/critic-{}.md", index + 1),
                    "provider": provider,
                    "dependsOn": [node],
                })
            })
            .collect();
        fenced(&json!({
            "version": "0.1",
            "kind": "dynamic-node-completion",
            "status": "success",
            "summaryPath": "handoff/summary.md",
            "next": {
                "type": "fanout",
                "groupId": "critics",
                "nodes": nodes,
                "merge": { "title": "Merge critics", "taskPath": "handoff/merge.md" },
                "acceptance": { "title": "Accept critics", "taskPath": "handoff/acceptance.md" },
            },
        }))
    }
}

impl Default for Stub {
    fn default() -> Stub {
        Stub::new()
    }
}

/// Serve the stub over a reader and a writer until the reader ends, answering
/// as a critic.
///
/// Blank lines and lines that are not a JSON object are skipped; every frame is
/// written as one line of compact JSON and flushed.
pub fn serve<R: BufRead, W: Write>(
    input: R,
    output: &mut W,
    verdict: &str,
    models: &str,
) -> std::io::Result<()> {
    serve_with_role(input, output, verdict, models, &Role::Findings, "")
}

/// Serve the stub over a reader and a writer until the reader ends, answering
/// as `role` and keeping its gate reports under `report_dir`.
///
/// Blank lines and lines that are not a JSON object are skipped; every frame is
/// written as one line of compact JSON and flushed.
pub fn serve_with_role<R: BufRead, W: Write>(
    input: R,
    output: &mut W,
    verdict: &str,
    models: &str,
    role: &Role,
    report_dir: &str,
) -> std::io::Result<()> {
    let mut stub = Stub::with_role(role.clone(), report_dir.to_string());
    for line in input.lines() {
        let line = line?;
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if !request.is_object() {
            continue;
        }
        for frame in stub.handle(&request, verdict, models) {
            writeln!(output, "{frame}")?;
            output.flush()?;
        }
    }
    Ok(())
}

/// The string value of `key` in the request's params, when it is a string.
fn param_str<'a>(params: Option<&'a Value>, key: &str) -> Option<&'a str> {
    params?.get(key)?.as_str()
}

/// The text of every `params.prompt` entry that carries one, joined with
/// newlines; empty when the prompt has no text.
fn prompt_text(params: Option<&Value>) -> String {
    let Some(entries) = params
        .and_then(|params| params.get("prompt"))
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    let texts: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.get("text").and_then(Value::as_str))
        .collect();
    texts.join("\n")
}

/// A JSON-RPC result frame for the request's id.
fn reply(request: &Value, result: Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// A JSON-RPC error frame for the request's id.
fn error(request: &Value, code: i64, message: &str) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A JSON-RPC notification frame.
fn notify(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}
