//! pi, `--mode json`: one event per line. A `message_end` of the assistant holds the whole message (text and
//! thinking) and its usage: the cached prompt (`cacheRead`) is inside the input and the dollars are
//! `cost.total`, as `pi_output` counts them. `tool_execution_start` and `tool_execution_end` are the tool calls;
//! the many `message_update` deltas, the system and user messages and the repeated `agent_end` are not events.

use super::{array_of, harness_error, message, number, object_of, text, tool_finished, tool_started, usage, AdapterEvent, Mapper};
use crate::json::{Object, Value};

pub struct Pi;

impl Mapper for Pi {
    fn map(&mut self, event: &Object) -> Option<Vec<AdapterEvent>> {
        match text(event, "type")? {
            "session" | "agent_start" | "agent_end" | "agent_settled" | "turn_start" | "turn_end" | "message_start" | "message_update" => Some(Vec::new()),
            "message_end" => Some(ended(&object_of(event, "message"))),
            "tool_execution_start" => Some(vec![tool_started(text(event, "toolCallId").unwrap_or_default(), text(event, "toolName").unwrap_or_default(), event.get("args").unwrap_or(&Value::Null))]),
            "tool_execution_end" => {
                let result = object_of(event, "result");
                let output = array_of(&result, "content").iter().filter_map(Value::as_object).filter_map(|part| text(part, "text")).collect::<Vec<_>>().join("\n");
                Some(vec![tool_finished(text(event, "toolCallId").unwrap_or_default(), text(event, "toolName").unwrap_or_default(), event.get("isError") != Some(&Value::Bool(true)), &output)])
            }
            _ => None,
        }
    }
}

/// An assistant message that ended: what it said, then its usage, then the error it stopped with, if any.
fn ended(message_object: &Object) -> Vec<AdapterEvent> {
    if text(message_object, "role") != Some("assistant") {
        return Vec::new();
    }
    let mut events = Vec::new();
    for part in array_of(message_object, "content").iter().filter_map(Value::as_object) {
        match text(part, "type") {
            Some("text") => events.push(message("assistant", "text", text(part, "text").unwrap_or_default())),
            Some("thinking") => events.push(message("assistant", "thinking", text(part, "thinking").unwrap_or_default())),
            _ => {}
        }
    }
    let counted = object_of(message_object, "usage");
    let cached = number(&counted, "cacheRead");
    let cost = Some(number(&object_of(&counted, "cost"), "total")).filter(|cost| *cost > 0.0);
    events.push(usage(number(&counted, "input") + cached, number(&counted, "output"), cached, cost));
    if matches!(text(message_object, "stopReason"), Some("error" | "aborted")) {
        events.push(harness_error(text(message_object, "errorMessage").or(text(message_object, "stopReason")).unwrap_or_default()));
    }
    events
}
