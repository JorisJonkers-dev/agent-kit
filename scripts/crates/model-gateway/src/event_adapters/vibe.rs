//! Vibe, `--output streaming`: one journal entry per line, each printed once it is complete (the same entries
//! `--output json` prints as one array). A `message` entry is the user's prompt or the assistant's answer; an
//! `effect` entry is a tool call with its state, so it is both a start and an end. The stream carries no usage:
//! the tokens and dollars are in the export Vibe writes, and the run's `usage` event is written from the result
//! when the run ends.

use super::{content_text, message, object_of, text, text_in, tool_finished, tool_started, AdapterEvent, Mapper};
use crate::json::{Object, Value};

pub struct Vibe;

impl Mapper for Vibe {
    fn map(&mut self, entry: &Object) -> Option<Vec<AdapterEvent>> {
        match text(entry, "type")? {
            "message" => Some(if text(entry, "role") == Some("assistant") { vec![message("assistant", "text", &content_text(entry.get("content")))] } else { Vec::new() }),
            "effect" => {
                let detail = object_of(entry, "detail");
                let state = object_of(entry, "state");
                let (id, name) = (text(entry, "toolCallId").or_else(|| text(entry, "id")).unwrap_or_default(), text(&detail, "toolName").or_else(|| text(entry, "title")).unwrap_or_default());
                let mut events = vec![tool_started(id, name, detail.get("input").unwrap_or(&Value::Null))];
                if let Some(status @ ("completed" | "failed" | "error" | "rejected")) = text(&state, "status") {
                    let output = text_in(&state, &["display", "message"]).unwrap_or_default();
                    events.push(tool_finished(id, name, status == "completed", output));
                }
                Some(events)
            }
            _ => None,
        }
    }
}
