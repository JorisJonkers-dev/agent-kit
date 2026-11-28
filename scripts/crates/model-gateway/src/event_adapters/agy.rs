//! Antigravity, `--output-format stream-json`: `init`, one `step_update` per step state change, and a final
//! `result`. A step is the `user_input`, an `agent_response` (its `text_delta`s are the answer, written as one
//! message when the step is `DONE`) or a `tool` (`ACTIVE` is the start, `DONE` the end). The `result`'s usage is
//! the run's total, as `agy_output` counts it, and agy prices nothing (it is quota), so the event has no dollars.

use super::{harness_error, message, number, object_of, text, tool_finished, tool_started, usage, AdapterEvent, Mapper};
use crate::json::{Object, Value};

#[derive(Default)]
pub struct Agy {
    /// The answer's text so far.
    answer: String,
}

impl Mapper for Agy {
    fn map(&mut self, event: &Object) -> Option<Vec<AdapterEvent>> {
        match text(event, "event")? {
            "init" => Some(Vec::new()),
            "step_update" => self.step(&object_of(event, "step_update")),
            "result" => {
                let result = object_of(event, "result");
                let counted = object_of(&result, "usage");
                let mut events = vec![usage(number(&counted, "input_tokens"), number(&counted, "output_tokens"), number(&counted, "cache_read_tokens"), None)];
                if text(&result, "status") != Some("SUCCESS") {
                    events.push(harness_error(&format!("the run ended with status {}", text(&result, "status").unwrap_or("unknown"))));
                }
                Some(events)
            }
            _ => None,
        }
    }
}

impl Agy {
    fn step(&mut self, step: &Object) -> Option<Vec<AdapterEvent>> {
        let done = text(step, "state") == Some("DONE");
        match text(step, "step_type")? {
            "user_input" => Some(Vec::new()),
            "agent_response" => {
                self.answer.push_str(text(step, "text_delta").unwrap_or_default());
                // A step that answered nothing (it went on to call a tool) is no message.
                Some(if done && !self.answer.is_empty() { vec![message("assistant", "text", &std::mem::take(&mut self.answer))] } else { Vec::new() })
            }
            "tool" => {
                let info = object_of(step, "tool_info");
                let id = format!("step-{}", number(step, "step_index"));
                let name = text(step, "tool_name").unwrap_or_default();
                Some(vec![if done { tool_finished(&id, name, true, text(&info, "output").unwrap_or_default()) } else { tool_started(&id, name, info.get("parameters").unwrap_or(&Value::Null)) }])
            }
            _ => None,
        }
    }
}
