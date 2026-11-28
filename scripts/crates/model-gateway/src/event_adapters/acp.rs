//! Hermes over ACP, the Agent Client Protocol: `hermes acp` speaks JSON-RPC 2.0 on stdin and stdout, one message
//! per line. Hermes has no native streaming JSON (`hermes -z` prints the answer text alone), so the gateway is
//! the ACP client: it writes `initialize`, `session/new` and `session/prompt` in turn, each once the answer to
//! the one before has been read, and closes stdin when the prompt's answer comes. What the agent reports as
//! `session/update` notifications are the events: `agent_message_chunk` and `agent_thought_chunk` (joined into one
//! `message` each), `tool_call` and `tool_call_update`, and `plan`. The prompt's answer holds the stop reason and
//! the run's usage. The gateway offers no file or terminal of its own, so an agent request it can answer is only
//! a permission request, which it refuses.

use super::{array_of, clip, diagnostic, harness_error, message, number, object_of, plan, text, text_in, tool_finished, tool_started, usage, Adapter, AdapterEvent, Reply};
use crate::json::{self, Object, Value};
use crate::run::{RunTokens, NO_TOKENS};
use std::collections::HashMap;

const INITIALIZE: f64 = 1.0;
const NEW_SESSION: f64 = 2.0;
const PROMPT: f64 = 3.0;

#[derive(Default)]
pub struct Acp {
    prompt: String,
    cwd: String,
    session: String,
    /// The kind (`text` or `thinking`) and text of the message being joined from chunks.
    pending: Option<(&'static str, String)>,
    /// The name of each tool call that began, by its id.
    names: HashMap<String, String>,
    out: Reply,
}

/// A JSON-RPC request line.
fn request(id: f64, method: &str, params: Value) -> String {
    format!("{}\n", json::compact(&json::object(vec![("jsonrpc", Value::str("2.0")), ("id", Value::Num(id)), ("method", Value::str(method)), ("params", params)])))
}

impl Adapter for Acp {
    fn drives(&self) -> bool {
        true
    }

    fn drive(&mut self, prompt: &str, cwd: &str) {
        self.prompt = prompt.to_string();
        self.cwd = cwd.to_string();
    }

    fn opening(&mut self) -> String {
        request(INITIALIZE, "initialize", json::object(vec![("protocolVersion", Value::Num(1.0)), ("clientCapabilities", json::object(Vec::new()))]))
    }

    fn outgoing(&mut self) -> Reply {
        std::mem::take(&mut self.out)
    }

    fn line(&mut self, line: &str) -> Vec<AdapterEvent> {
        let Ok(Value::Obj(message)) = json::parse(line) else {
            return vec![diagnostic(&format!("output line is not a JSON object: {}", clip(line, 300)))];
        };
        match (text(&message, "method"), message.get("id")) {
            (Some("session/update"), _) => self.update(&object_of(&object_of(&message, "params"), "update")),
            (Some(method), Some(id)) => {
                self.answer_request(method, id.clone(), &object_of(&message, "params"));
                Vec::new()
            }
            (Some(_), None) => vec![diagnostic(&format!("unrecognised output line: {}", clip(line, 300)))],
            (None, Some(Value::Num(id))) => self.response(*id, &message),
            (None, _) => vec![diagnostic(&format!("unrecognised output line: {}", clip(line, 300)))],
        }
    }
}

impl Acp {
    /// The agent's answer to one of the client's requests.
    fn response(&mut self, id: f64, message_object: &Object) -> Vec<AdapterEvent> {
        let mut events = self.flush();
        if let Some(error) = message_object.get("error").and_then(Value::as_object) {
            events.push(harness_error(text(error, "message").unwrap_or("the agent answered with an error")));
            self.out = Reply { text: String::new(), close: true };
            return events;
        }
        let result = object_of(message_object, "result");
        if id == INITIALIZE {
            let params = json::object(vec![("cwd", Value::str(&self.cwd)), ("mcpServers", Value::Arr(Vec::new()))]);
            self.out.text.push_str(&request(NEW_SESSION, "session/new", params));
        } else if id == NEW_SESSION {
            self.session = text(&result, "sessionId").unwrap_or_default().to_string();
            let prompt = json::object(vec![("type", Value::str("text")), ("text", Value::str(&self.prompt))]);
            let params = json::object(vec![("sessionId", Value::str(&self.session)), ("prompt", Value::Arr(vec![prompt]))]);
            self.out.text.push_str(&request(PROMPT, "session/prompt", params));
        } else if id == PROMPT {
            let counted = object_of(&result, "usage");
            events.push(usage(number(&counted, "inputTokens"), number(&counted, "outputTokens"), number(&counted, "cachedReadTokens"), None));
            if text(&result, "stopReason") != Some("end_turn") {
                events.push(harness_error(&format!("the prompt ended with stop reason {}", text(&result, "stopReason").unwrap_or("none"))));
            }
            self.out.close = true;
        }
        events
    }

    /// Answers a request the agent made of the client: a permission request is refused, anything else is a method
    /// the gateway does not offer.
    fn answer_request(&mut self, method: &str, id: Value, params: &Object) {
        let mut reply = Object::new();
        reply.set("jsonrpc", Value::str("2.0"));
        reply.set("id", id);
        if method == "session/request_permission" {
            let refusal = array_of(params, "options").iter().filter_map(Value::as_object).find(|option| text(option, "kind").is_some_and(|kind| kind.starts_with("reject")));
            let outcome = match refusal {
                Some(option) => json::object(vec![("outcome", Value::str("selected")), ("optionId", Value::str(text(option, "optionId").unwrap_or_default()))]),
                None => json::object(vec![("outcome", Value::str("cancelled"))]),
            };
            reply.set("result", json::object(vec![("outcome", outcome)]));
        } else {
            reply.set("error", json::object(vec![("code", Value::Num(-32601.0)), ("message", Value::str(&format!("the gateway offers no {method}")))]));
        }
        self.out.text.push_str(&format!("{}\n", json::compact(&Value::Obj(reply))));
    }

    /// The message being joined from chunks, as an event.
    fn flush(&mut self) -> Vec<AdapterEvent> {
        self.pending.take().map(|(kind, body)| message("assistant", kind, &body)).into_iter().collect()
    }

    fn chunk(&mut self, kind: &'static str, update: &Object) -> Vec<AdapterEvent> {
        let mut events = Vec::new();
        if self.pending.as_ref().is_some_and(|(pending, _)| *pending != kind) {
            events = self.flush();
        }
        let piece = text_in(update, &["content", "text"]).unwrap_or_default();
        self.pending.get_or_insert((kind, String::new())).1.push_str(piece);
        events
    }

    fn update(&mut self, update: &Object) -> Vec<AdapterEvent> {
        let Some(kind) = text(update, "sessionUpdate") else {
            return vec![diagnostic("a session update names no kind")];
        };
        let id = text(update, "toolCallId").unwrap_or_default();
        match kind {
            "agent_message_chunk" => self.chunk("text", update),
            "agent_thought_chunk" => self.chunk("thinking", update),
            "tool_call" => {
                let mut events = self.flush();
                let name = text(update, "title").or_else(|| text(update, "kind")).unwrap_or_default();
                self.names.insert(id.to_string(), name.to_string());
                events.push(tool_started(id, name, update.get("rawInput").or_else(|| update.get("locations")).unwrap_or(&Value::Null)));
                events
            }
            "tool_call_update" => match text(update, "status") {
                Some(status @ ("completed" | "failed")) => {
                    let output = array_of(update, "content").iter().filter_map(Value::as_object).map(|item| text_in(item, &["content", "text"]).unwrap_or_default()).collect::<Vec<_>>().join("\n");
                    vec![tool_finished(id, self.names.get(id).map_or("", String::as_str), status == "completed", &output)]
                }
                _ => Vec::new(),
            },
            "plan" => {
                let mut events = self.flush();
                events.push(plan(array_of(update, "entries").iter().filter_map(Value::as_object).map(|entry| (text(entry, "content").unwrap_or_default().to_string(), text(entry, "status").unwrap_or("pending").to_string())).collect()));
                events
            }
            "usage_update" | "available_commands_update" | "current_mode_update" | "config_option_update" | "session_info_update" => Vec::new(),
            other => vec![diagnostic(&format!("unrecognised session update: {other}"))],
        }
    }
}

/// What an ACP run's stdout says: the answer (the text after the last tool call), the tokens the prompt's answer
/// counted and how the run failed, if it did.
pub struct Transcript {
    pub answer: Option<String>,
    pub tokens: RunTokens,
    pub error: Option<String>,
}

/// Reads an ACP run's whole stdout; `None` when it holds no JSON-RPC line, so it is not an ACP run's.
pub fn transcript(stdout: &str) -> Option<Transcript> {
    let messages: Vec<Object> = stdout.lines().filter_map(|line| match json::parse(line) {
        Ok(Value::Obj(message)) if text(&message, "jsonrpc") == Some("2.0") => Some(message),
        _ => None,
    }).collect();
    if messages.is_empty() {
        return None;
    }
    let (mut answer, mut tokens, mut error, mut ended) = (String::new(), NO_TOKENS, None, false);
    for message in &messages {
        let update = object_of(&object_of(message, "params"), "update");
        match text(&update, "sessionUpdate") {
            Some("agent_message_chunk") => answer.push_str(text_in(&update, &["content", "text"]).unwrap_or_default()),
            Some("tool_call") => answer.clear(),
            _ => {}
        }
        if let Some(failure) = message.get("error").and_then(Value::as_object) {
            error = Some(text(failure, "message").unwrap_or("the agent answered with an error").to_string());
        }
        if message.get("id") == Some(&Value::Num(PROMPT)) && message.get("result").is_some() {
            let result = object_of(message, "result");
            let counted = object_of(&result, "usage");
            tokens = RunTokens { input: number(&counted, "inputTokens"), output: number(&counted, "outputTokens"), cached: number(&counted, "cachedReadTokens") };
            ended = true;
            if text(&result, "stopReason") != Some("end_turn") {
                error = Some(format!("the prompt ended with stop reason {}", text(&result, "stopReason").unwrap_or("none")));
            }
        }
    }
    if !ended && error.is_none() {
        error = Some("the agent never answered the prompt".to_string());
    }
    let trimmed = answer.trim();
    Some(Transcript { answer: (!trimmed.is_empty()).then(|| trimmed.to_string()), tokens, error })
}
