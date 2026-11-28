//! Claude Code, `--output-format stream-json --verbose`: one object per line. `assistant` lines hold whole content
//! blocks (text, thinking, `tool_use`), `user` lines hold the `tool_result` blocks, and the last line is the
//! `result`, whose usage and `total_cost_usd` are the run's totals, so the usage event is written there alone: the
//! per-message usage repeats for each block of one message.

use super::{array_of, content_text, harness_error, message, number, object_of, text, tool_finished, tool_started, usage, AdapterEvent, Mapper};
use crate::json::{Object, Value};
use std::collections::HashMap;

#[derive(Default)]
pub struct Claude {
    /// The name of each tool call that began, by its id: its result names only the id.
    names: HashMap<String, String>,
}

impl Mapper for Claude {
    fn map(&mut self, event: &Object) -> Option<Vec<AdapterEvent>> {
        match text(event, "type")? {
            "system" | "rate_limit_event" => Some(Vec::new()),
            "assistant" => Some(self.blocks(event, "assistant")),
            "user" => Some(self.blocks(event, "user")),
            "result" => Some(result(event)),
            _ => None,
        }
    }
}

impl Claude {
    fn blocks(&mut self, event: &Object, role: &str) -> Vec<AdapterEvent> {
        let content = object_of(event, "message");
        let mut events = Vec::new();
        for block in array_of(&content, "content").iter().filter_map(Value::as_object) {
            match text(block, "type") {
                Some("text") if role == "assistant" => events.push(message(role, "text", text(block, "text").unwrap_or_default())),
                // The thinking is empty when the client redacts it.
                Some("thinking") => events.extend(Some(text(block, "thinking").unwrap_or_default()).filter(|thought| !thought.is_empty()).map(|thought| message(role, "thinking", thought))),
                Some("tool_use") => {
                    let (id, name) = (text(block, "id").unwrap_or_default(), text(block, "name").unwrap_or_default());
                    self.names.insert(id.to_string(), name.to_string());
                    events.push(tool_started(id, name, block.get("input").unwrap_or(&Value::Null)));
                }
                Some("tool_result") => {
                    let id = text(block, "tool_use_id").unwrap_or_default();
                    let name = self.names.get(id).map_or("", String::as_str);
                    events.push(tool_finished(id, name, block.get("is_error") != Some(&Value::Bool(true)), &content_text(block.get("content"))));
                }
                _ => {}
            }
        }
        events
    }
}

/// The run's totals, counted as the spend record counts them (`claude_output`): the cached and cache-written
/// prompt are inside the input. A result that reports an error is a diagnostic as well.
fn result(event: &Object) -> Vec<AdapterEvent> {
    let counted = object_of(event, "usage");
    let cached = number(&counted, "cache_read_input_tokens");
    let input = number(&counted, "input_tokens") + cached + number(&counted, "cache_creation_input_tokens");
    let cost = Some(number(event, "total_cost_usd")).filter(|cost| *cost > 0.0);
    let mut events = vec![usage(input, number(&counted, "output_tokens"), cached, cost)];
    if event.get("is_error") == Some(&Value::Bool(true)) {
        events.push(harness_error(text(event, "result").unwrap_or("the client reported an error")));
    }
    events
}
