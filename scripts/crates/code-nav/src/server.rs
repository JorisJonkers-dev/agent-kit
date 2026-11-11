//! The MCP surface: newline-delimited JSON-RPC 2.0 over stdio, one tool.
//!
//! The server is read-only and opens no socket. It answers `initialize`,
//! `ping`, `tools/list` and `tools/call` for the one tool, `search`;
//! notifications get no reply.

use crate::boundary::Boundary;
use crate::search::{search, MAX_LIMIT};
use code_nav_eval::{json_string, parse_json, Json};
use std::io::{BufRead, Write};
use std::path::PathBuf;

const PROTOCOL_VERSION: &str = "2024-11-05";

const TOOL_LIST: &str = r#"{"tools":[{"name":"search","description":"Search the working tree for lines containing a query, ignoring case. Results are ranked (whole-identifier matches and file-name matches first, then shallower paths) and capped. Secret files and symlinks that leave the allowed roots are never read.","inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"Text to find"},"path":{"type":"string","description":"A directory or file under an allowed root; default is every allowed root"},"limit":{"type":"integer","minimum":1,"maximum":200,"description":"Result cap; the server's default when omitted"}},"required":["query"]}}]}"#;

/// Serve requests from `input` until it ends.
pub fn serve(boundary: &Boundary, default_limit: usize, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<(), String> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line).map_err(|e| format!("read: {e}"))? == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(boundary, default_limit, line.trim()) {
            writeln!(out, "{reply}").and_then(|()| out.flush()).map_err(|e| format!("write: {e}"))?;
        }
    }
}

/// The reply to one request line, or `None` for a notification.
pub fn handle(boundary: &Boundary, default_limit: usize, line: &str) -> Option<String> {
    let request = match parse_json(line) {
        Ok(r @ Json::Obj(_)) => r,
        Ok(_) => return Some(error("null", -32600, "request must be an object")),
        Err(e) => return Some(error("null", -32700, &format!("parse error: {e}"))),
    };
    let id = request.get("id")?;
    let id = id_json(id);
    let Some(method) = request.get("method").and_then(Json::as_str) else {
        return Some(error(&id, -32600, "missing method"));
    };
    let result = match method {
        "initialize" => {
            let version = request
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Json::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            format!(
                r#"{{"protocolVersion":{},"capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"code-nav","version":"{}"}}}}"#,
                json_string(version),
                env!("CARGO_PKG_VERSION")
            )
        }
        "ping" => "{}".to_string(),
        "tools/list" => TOOL_LIST.to_string(),
        "tools/call" => call(boundary, default_limit, request.get("params")),
        other => return Some(error(&id, -32601, &format!("method not found: {other}"))),
    };
    Some(format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#))
}

fn error(id: &str, code: i32, message: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":{}}}}}"#, json_string(message))
}

fn id_json(id: &Json) -> String {
    match id {
        Json::Num(n) if n.fract() == 0.0 => format!("{}", *n as i64),
        Json::Num(n) => format!("{n}"),
        Json::Str(s) => json_string(s),
        _ => "null".to_string(),
    }
}

/// A tool result carrying one text block.
fn text_result(text: &str, is_error: bool) -> String {
    format!(r#"{{"content":[{{"type":"text","text":{}}}],"isError":{is_error}}}"#, json_string(text))
}

fn call(boundary: &Boundary, default_limit: usize, params: Option<&Json>) -> String {
    let name = params.and_then(|p| p.get("name")).and_then(Json::as_str).unwrap_or("");
    if name != "search" {
        return text_result(&format!("unknown tool: {name}"), true);
    }
    let args = params.and_then(|p| p.get("arguments"));
    let Some(query) = args.and_then(|a| a.get("query")).and_then(Json::as_str).filter(|q| !q.is_empty()) else {
        return text_result("query is required", true);
    };
    let limit = args
        .and_then(|a| a.get("limit"))
        .and_then(Json::as_u64)
        .map_or(default_limit, |n| (n as usize).clamp(1, MAX_LIMIT));
    let scope: Vec<PathBuf> = match args.and_then(|a| a.get("path")).and_then(Json::as_str) {
        Some(path) => match boundary.admit(std::path::Path::new(path)) {
            Ok(canonical) => vec![canonical],
            Err(refusal) => return text_result(&refusal.to_string(), true),
        },
        None => boundary.roots().to_vec(),
    };
    let results = search(&scope, query, limit);
    let mut text = String::new();
    for hit in &results.hits {
        text.push_str(&format!("{}:{}: {}\n", hit.path.display(), hit.line, hit.text));
    }
    if results.total == 0 {
        text.push_str("no matches\n");
    } else if results.truncated() {
        text.push_str(&format!("showing {} of {} matches; narrow the query or path\n", results.hits.len(), results.total));
    }
    text_result(text.trim_end(), false)
}
