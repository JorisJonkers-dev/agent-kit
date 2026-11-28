//! Codex, `exec --json`: one event per line (`thread.started`, `turn.started`, `item.started`, `item.updated`,
//! `item.completed`, `turn.completed`, `turn.failed`, `error`). An item is an `agent_message`, `reasoning`,
//! `command_execution`, `file_change`, `mcp_tool_call`, `web_search`, `todo_list` or `error`. A turn's usage is
//! its tokens, with the cached prompt counted apart from the input as the spend record counts it (`codex_output`);
//! Codex prices nothing, so a usage event has no dollars and the run's priced figure is on `run.finished`.

use super::{harness_error, message, number, object_of, plan, text, tool_finished, tool_started, usage, AdapterEvent, Mapper};
use crate::json::{Object, Value};

pub struct Codex;

impl Mapper for Codex {
    fn map(&mut self, event: &Object) -> Option<Vec<AdapterEvent>> {
        let item = object_of(event, "item");
        match (text(event, "type")?, text(&item, "type")) {
            ("thread.started" | "turn.started", _) => Some(Vec::new()),
            ("turn.completed", _) => {
                let counted = object_of(event, "usage");
                Some(vec![usage(number(&counted, "input_tokens"), number(&counted, "output_tokens"), number(&counted, "cached_input_tokens"), None)])
            }
            ("turn.failed", _) => Some(vec![harness_error(text(&object_of(event, "error"), "message").unwrap_or("the turn failed"))]),
            ("error", _) => Some(vec![harness_error(text(event, "message").unwrap_or("the client reported an error"))]),
            (phase @ ("item.started" | "item.updated" | "item.completed"), Some(kind)) => of_item(phase, kind, &item),
            _ => None,
        }
    }
}

/// What an item at one phase is. A tool item begins at `item.started` and ends at `item.completed`.
fn of_item(phase: &str, kind: &str, item: &Object) -> Option<Vec<AdapterEvent>> {
    let id = text(item, "id").unwrap_or_default();
    Some(match (kind, phase) {
        ("agent_message", "item.completed") => vec![message("assistant", "text", text(item, "text").unwrap_or_default())],
        ("reasoning", "item.completed") => vec![message("assistant", "thinking", text(item, "text").unwrap_or_default())],
        ("todo_list", _) => vec![plan(
            super::array_of(item, "items")
                .iter()
                .filter_map(Value::as_object)
                .map(|entry| (text(entry, "text").unwrap_or_default().to_string(), if entry.get("completed") == Some(&Value::Bool(true)) { "completed" } else { "pending" }.to_string()))
                .collect(),
        )],
        ("error", "item.completed") => vec![harness_error(text(item, "message").unwrap_or("the client reported an error"))],
        ("command_execution" | "file_change" | "mcp_tool_call" | "web_search", "item.started") => vec![tool_started(id, &tool_name(kind, item), &Value::Obj(item.clone()))],
        ("command_execution" | "file_change" | "mcp_tool_call" | "web_search", "item.completed") => {
            let failed = text(item, "status") == Some("failed") || item.get("exit_code").is_some_and(|code| *code != Value::Num(0.0) && *code != Value::Null);
            vec![tool_finished(id, &tool_name(kind, item), !failed, text(item, "aggregated_output").unwrap_or_default())]
        }
        ("agent_message" | "reasoning" | "error" | "command_execution" | "file_change" | "mcp_tool_call" | "web_search", _) => Vec::new(),
        _ => return None,
    })
}

/// The tool an item is: a shell command, a file change, an MCP server's tool or a web search.
fn tool_name(kind: &str, item: &Object) -> String {
    match kind {
        "mcp_tool_call" => format!("{}.{}", text(item, "server").unwrap_or_default(), text(item, "tool").unwrap_or_default()),
        "command_execution" => "shell".to_string(),
        other => other.to_string(),
    }
}
