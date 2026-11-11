//! The MCP surface: newline-delimited JSON-RPC 2.0 over stdio, four tools.
//!
//! The server is read-only and opens no socket. It answers `initialize`,
//! `ping`, `tools/list` and `tools/call` for `search`, `outline`, `expand` and `map`;
//! notifications get no reply.

use crate::boundary::Boundary;
use crate::map::{map, RepoMap};
use crate::outline;
use crate::search::{search, MAX_FILE_BYTES, MAX_LIMIT};
use code_nav_eval::{json_string, parse_json, Json};
use std::fs;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Source lines one `expand` call returns before it says it capped.
const MAX_EXPAND_LINES: usize = 400;

// One line: the transport is newline-delimited, so a reply may hold no newline.
const TOOL_LIST: &str = r#"{"tools":[{"name":"search","description":"Search the working tree for lines containing a query, ignoring case. Results are ranked (whole-identifier matches and file-name matches first, then shallower paths) and capped. Read-only; secret files and symlinks that leave the allowed roots are never read.","inputSchema":{"type":"object","properties":{"query":{"type":"string","description":"Text to find"},"path":{"type":"string","description":"A directory or file under an allowed root; default is every allowed root"},"limit":{"type":"integer","minimum":1,"maximum":200,"description":"Result cap; the server's default when omitted"}},"required":["query"]}},{"name":"outline","description":"List the symbols a file declares, one line each: line, kind, name and range. Read-only; secret files and symlinks that leave the allowed roots are never read.","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"A file under an allowed root"}},"required":["path"]}},{"name":"expand","description":"Return one symbol's source lines, numbered, without the rest of the file. Read-only; secret files and symlinks that leave the allowed roots are never read.","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"A file under an allowed root"},"symbol":{"type":"string","description":"The symbol name, exactly as outline prints it"}},"required":["path","symbol"]}},{"name":"map","description":"Rank a tree's files and symbols by how often each symbol is referenced, most referenced first. Read-only; secret files and symlinks that leave the allowed roots are never read.","inputSchema":{"type":"object","properties":{"path":{"type":"string","description":"A directory or file under an allowed root; default is every allowed root"},"limit":{"type":"integer","minimum":1,"maximum":200,"description":"Entries per list; the server's default when omitted"}}}}]}"#;

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
    let args = params.and_then(|p| p.get("arguments"));
    match name {
        "search" => search_tool(boundary, default_limit, args),
        "outline" => outline_tool(boundary, args),
        "expand" => expand_tool(boundary, args),
        "map" => map_tool(boundary, default_limit, args),
        other => text_result(&format!("unknown tool: {other}"), true),
    }
}

fn search_tool(boundary: &Boundary, default_limit: usize, args: Option<&Json>) -> String {
    let Some(query) = args.and_then(|a| a.get("query")).and_then(Json::as_str).filter(|q| !q.is_empty()) else {
        return text_result("query is required", true);
    };
    let limit = limit_arg(args, default_limit);
    let scope = match scope_arg(boundary, args) {
        Ok(scope) => scope,
        Err(message) => return text_result(&message, true),
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

/// The `limit` argument clamped to the allowed range, or the default.
fn limit_arg(args: Option<&Json>, default_limit: usize) -> usize {
    args.and_then(|a| a.get("limit")).and_then(Json::as_u64).map_or(default_limit, |n| (n as usize).clamp(1, MAX_LIMIT))
}

/// The paths the optional `path` argument names, every root when it is absent.
fn scope_arg(boundary: &Boundary, args: Option<&Json>) -> Result<Vec<PathBuf>, String> {
    match args.and_then(|a| a.get("path")).and_then(Json::as_str) {
        Some(path) => boundary.admit(Path::new(path)).map(|canonical| vec![canonical]).map_err(|r| r.to_string()),
        None => Ok(boundary.roots().to_vec()),
    }
}

fn map_tool(boundary: &Boundary, default_limit: usize, args: Option<&Json>) -> String {
    let scope = match scope_arg(boundary, args) {
        Ok(scope) => scope,
        Err(message) => return text_result(&message, true),
    };
    text_result(&render_map(&map(&scope, limit_arg(args, default_limit))), false)
}

/// The text of a repository map.
pub(crate) fn render_map(repo: &RepoMap) -> String {
    if repo.symbols.is_empty() {
        return "no symbols".to_string();
    }
    let mut out = String::from("files:\n");
    for file in &repo.files {
        out.push_str(&format!("{} ({} refs, {} symbols)\n", file.path.display(), file.refs, file.symbols));
    }
    out.push_str("symbols:\n");
    for s in &repo.symbols {
        out.push_str(&format!("{}:{} {} {} ({} refs)\n", s.path.display(), s.line, s.kind, s.name, s.refs));
    }
    if repo.total_files > repo.files.len() || repo.total_symbols > repo.symbols.len() {
        out.push_str(&format!(
            "showing {} of {} files and {} of {} symbols; narrow the path\n",
            repo.files.len(),
            repo.total_files,
            repo.symbols.len(),
            repo.total_symbols
        ));
    }
    if repo.walk_capped {
        out.push_str("stopped after the file cap; narrow the path\n");
    }
    out.trim_end().to_string()
}

fn outline_tool(boundary: &Boundary, args: Option<&Json>) -> String {
    let text = match path_arg(args).and_then(|path| admitted_text(boundary, path)) {
        Ok((_, text)) => text,
        Err(message) => return text_result(&message, true),
    };
    let found = outline::symbols(&text);
    if found.is_empty() {
        return text_result("no symbols", false);
    }
    let mut out = String::new();
    for symbol in &found {
        out.push_str(&format!("{} {} {} ({}-{})\n", symbol.line, symbol.kind, symbol.name, symbol.start, symbol.end));
    }
    text_result(out.trim_end(), false)
}

fn expand_tool(boundary: &Boundary, args: Option<&Json>) -> String {
    let path = match path_arg(args) {
        Ok(path) => path,
        Err(message) => return text_result(&message, true),
    };
    let Some(name) = args.and_then(|a| a.get("symbol")).and_then(Json::as_str) else {
        return text_result("symbol is required", true);
    };
    let (canonical, text) = match admitted_text(boundary, path) {
        Ok(found) => found,
        Err(message) => return text_result(&message, true),
    };
    let found = outline::expand(&text, name);
    if found.is_empty() {
        return text_result(&format!("no symbol named {name} in {}", canonical.display()), true);
    }
    let mut out = String::new();
    let mut shown = 0;
    let mut capped = false;
    for (symbol, source) in &found {
        if shown >= MAX_EXPAND_LINES {
            capped = true;
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("{}:{}-{} {} {}\n", canonical.display(), symbol.start, symbol.end, symbol.kind, symbol.name));
        for (offset, line) in source.lines().enumerate() {
            if shown == MAX_EXPAND_LINES {
                capped = true;
                break;
            }
            out.push_str(&format!("{}\t{}\n", symbol.start + offset, line));
            shown += 1;
        }
    }
    if capped {
        out.push_str(&format!("output capped at {MAX_EXPAND_LINES} lines\n"));
    }
    text_result(out.trim_end(), false)
}

/// The `path` argument, or the message to report.
fn path_arg(args: Option<&Json>) -> Result<&str, String> {
    args.and_then(|a| a.get("path")).and_then(Json::as_str).ok_or_else(|| "path is required".to_string())
}

/// The canonical path and text of the `path` argument, or the message to report.
fn admitted_text(boundary: &Boundary, path: &str) -> Result<(PathBuf, String), String> {
    let canonical = boundary.admit(Path::new(path)).map_err(|refusal| refusal.to_string())?;
    let text = read_text(path, &canonical)?;
    Ok((canonical, text))
}

/// The text of an admitted file, or the message to report.
fn read_text(path: &str, canonical: &Path) -> Result<String, String> {
    if !canonical.is_file() {
        return Err(format!("{path} is not a file"));
    }
    // A file that vanished since the check reads as empty and fails below.
    if fs::metadata(canonical).map(|meta| meta.len()).unwrap_or(0) > MAX_FILE_BYTES {
        return Err("file is too large".to_string());
    }
    match fs::read_to_string(canonical) {
        Ok(text) if !text.contains('\0') => Ok(text),
        _ => Err("not a text file".to_string()),
    }
}
