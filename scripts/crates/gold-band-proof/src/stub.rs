//! A stub ACP agent: JSON-RPC 2.0 over newline-delimited JSON on stdio.
//!
//! The stub stands in for every agent id of a Gold Band workflow in a headless
//! proof. It answers the ACP handshake, hands out sessions, offers a model
//! picker and, on every prompt, streams one fenced findings document back. The
//! verdict is a parameter, so the same stub proves the clean path, the
//! needs-human path and the failing path.

use std::collections::BTreeSet;
use std::io::{BufRead, Write};

use serde_json::{json, Value};

/// The environment variable naming the verdict the stub reports.
pub const VERDICT_ENV: &str = "STUB_ACP_VERDICT";

/// The environment variable holding the comma separated model ids to offer.
pub const MODELS_ENV: &str = "STUB_ACP_MODELS";

/// The verdict that makes the stub report one major finding.
pub const NEEDS_HUMAN: &str = "needs-human";

/// The verdict that makes the stub fail the prompt instead of answering it.
pub const FAIL: &str = "fail";

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
    // A document built from literals always serialises.
    let body = serde_json::to_string_pretty(&findings_document(verdict)).unwrap_or_default();
    format!("```json\n{body}\n```")
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

/// The stub agent: the sessions it knows and the next session id to hand out.
pub struct Stub {
    sessions: BTreeSet<String>,
    next_session: u64,
}

impl Stub {
    /// A stub that knows no sessions and has handed out no ids yet.
    pub fn new() -> Stub {
        Stub {
            sessions: BTreeSet::new(),
            next_session: 0,
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

    /// The frames for a prompt: the fenced findings, or the failure error.
    fn prompt(&mut self, request: &Value, params: Option<&Value>, verdict: &str) -> Vec<Value> {
        let session_id = param_str(params, "sessionId")
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .or_else(|| self.sessions.iter().next().cloned())
            .unwrap_or_default();
        if verdict.trim() == FAIL {
            return vec![error(request, -32000, "stub failure")];
        }
        vec![
            notify(
                "session/update",
                json!({
                    "sessionId": session_id,
                    "update": {
                        "sessionUpdate": "agent_message_chunk",
                        "content": { "type": "text", "text": fenced_findings(verdict) },
                    },
                }),
            ),
            reply(request, json!({ "stopReason": "end_turn" })),
        ]
    }
}

impl Default for Stub {
    fn default() -> Stub {
        Stub::new()
    }
}

/// Serve the stub over a reader and a writer until the reader ends.
///
/// Blank lines and lines that are not a JSON object are skipped; every frame is
/// written as one line of compact JSON and flushed.
pub fn serve<R: BufRead, W: Write>(
    input: R,
    output: &mut W,
    verdict: &str,
    models: &str,
) -> std::io::Result<()> {
    let mut stub = Stub::new();
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
