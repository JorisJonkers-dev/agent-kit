//! OpenCode, `run --format json`: one event per line. `text` holds a whole text part, `tool_use` a tool part with
//! its state (written once the tool is `completed` or `error`), `step_finish` the step's dollars and tokens (the
//! cache counted apart from the input, reasoning billed as output, as `open_code_output` counts them), and
//! `error` an error the client reports.

use super::{harness_error, message, number, object_of, text, text_in, tool_finished, tool_started, usage, AdapterEvent, Mapper};
use crate::json::Object;

pub struct OpenCode;

impl Mapper for OpenCode {
    fn map(&mut self, event: &Object) -> Option<Vec<AdapterEvent>> {
        let part = object_of(event, "part");
        match text(event, "type")? {
            "step_start" => Some(Vec::new()),
            "text" => Some(vec![message("assistant", "text", text(&part, "text").unwrap_or_default())]),
            "reasoning" => Some(vec![message("assistant", "thinking", text(&part, "text").unwrap_or_default())]),
            "tool_use" => {
                let state = object_of(&part, "state");
                let (id, name) = (text(&part, "callID").or_else(|| text(&part, "id")).unwrap_or_default(), text(&part, "tool").unwrap_or_default());
                let mut events = vec![tool_started(id, name, state.get("input").unwrap_or(&crate::json::Value::Null))];
                if let Some(status @ ("completed" | "error")) = text(&state, "status") {
                    events.push(tool_finished(id, name, status == "completed", text(&state, "output").or_else(|| text(&state, "error")).unwrap_or_default()));
                }
                Some(events)
            }
            "step_finish" => {
                let tokens = object_of(&part, "tokens");
                let cache = object_of(&tokens, "cache");
                let input = number(&tokens, "input") + number(&cache, "read") + number(&cache, "write");
                let output = number(&tokens, "output") + number(&tokens, "reasoning");
                Some(vec![usage(input, output, number(&cache, "read"), Some(number(&part, "cost")).filter(|cost| *cost > 0.0))])
            }
            "error" => Some(vec![harness_error(text_in(event, &["error", "data", "message"]).or_else(|| text_in(event, &["error", "name"])).unwrap_or("the client reported an error"))]),
            _ => None,
        }
    }
}
