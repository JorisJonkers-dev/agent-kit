//! `critique search-server`: the runner's MCP stdio server for one critic's packet copy (ADR-0006). It answers
//! `initialize`, `ping`, `tools/list` and `tools/call` until stdin closes, then exits 0. It reads the packet and nothing
//! else: it writes no file, runs no command and opens no socket. Arguments it cannot use exit 2; a root it cannot open
//! exits 1. The node runner's `adapters/search/server.ts` and `engine.ts`, and the answers of `domain/packet-search.ts`.
//!
//! A critic's `grep` pattern is a JavaScript regular expression, run on the crate's JavaScript-semantics engine. A
//! pattern that uses syntax the engine does not implement (a back reference, a lookbehind, a named group, a Unicode
//! property) is answered as an invalid pattern, where the node runner would search with it.

use crate::js_regex::Regex;
use crate::manifest::REPO_MAP_PATH;
use crate::packet_search::{default_search_limits, SearchLimits};
use crate::triage::matches_glob;
use model_gateway::adapters::fs::fail;
use model_gateway::json::{self, Object, Value};
use model_gateway::text::{is_js_whitespace, join, js_slice, js_trim};
use std::io::{BufRead, Write};

/// The MCP version the server speaks when the client asks for none.
const PROTOCOL_VERSION: &str = "2025-06-18";

pub const SEARCH_SERVER_USAGE: &str = "usage: critique search-server --root DIR [--max-matches N] [--max-bytes N]\n";

const OUTSIDE: &str = "Nothing: that path is outside the packet, or is not in it.";

/// A search line holds at most this many UTF-16 code units of the matched line, so one long line cannot fill an answer.
const LINE_MAX_CHARS: f64 = 300.0;

/// The tools' definitions as `tools/list` gives them, copied from the node runner's `SEARCH_TOOL_DEFINITIONS`.
const TOOL_DEFINITIONS: &str = include_str!("search_tools.json");

/// Runs the server over `input` until it closes.
pub fn serve(argv: &[String], input: &mut dyn BufRead, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let Some((root, limits)) = read_options(argv) else {
        let _ = stderr.write_all(SEARCH_SERVER_USAGE.as_bytes());
        return 2;
    };
    let engine = match Engine::open(&root, limits) {
        Ok(engine) => engine,
        Err(message) => {
            let _ = stderr.write_all(format!("critique search-server: {message}\n").as_bytes());
            return 1;
        }
    };
    let mut chunk = Vec::new();
    while input.read_until(b'\n', &mut chunk).unwrap_or(0) > 0 {
        let text = String::from_utf8_lossy(&chunk).into_owned();
        chunk.clear();
        let text = text.strip_suffix('\n').map_or(text.as_str(), |line| line.strip_suffix('\r').unwrap_or(line));
        // `readline` ends a line at `\n`, `\r\n` or a lone `\r`.
        for line in text.split('\r') {
            if js_trim(line).is_empty() {
                continue;
            }
            if let Some(reply) = answer(line, &engine) {
                let _ = stdout.write_all(format!("{}\n", json::compact(&reply)).as_bytes());
                let _ = stdout.flush();
            }
        }
    }
    0
}

/// The root and limits the arguments give, or `None` for arguments the server cannot use.
fn read_options(argv: &[String]) -> Option<(String, SearchLimits)> {
    let mut root = None;
    let mut limits = default_search_limits();
    for pair in argv.chunks(2) {
        let [flag, value] = pair else { return None };
        match flag.as_str() {
            "--root" => root = Some(value.clone()),
            "--max-matches" | "--max-bytes" => {
                let number = js_number(value).filter(|number| number.fract() == 0.0 && *number >= 1.0)?;
                if flag == "--max-matches" {
                    limits.max_matches = number;
                } else {
                    limits.max_bytes = number;
                }
            }
            _ => return None,
        }
    }
    root.map(|root| (root, limits))
}

/// `Number(text)` for what a command line can hold: decimal, hexadecimal, octal or binary, `Infinity`, and empty or
/// white space as 0; `None` where JavaScript gives NaN.
pub(crate) fn js_number(text: &str) -> Option<f64> {
    let text = text.trim_matches(is_js_whitespace);
    if text.is_empty() {
        return Some(0.0);
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = text.strip_prefix(prefix) {
            return (!digits.is_empty() && digits.chars().all(|c| c.is_digit(radix))).then(|| digits.chars().fold(0.0, |total, c| total * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))));
        }
    }
    let unsigned = text.strip_prefix(['+', '-']).unwrap_or(text);
    if unsigned == "Infinity" {
        return Some(if text.starts_with('-') { f64::NEG_INFINITY } else { f64::INFINITY });
    }
    let (mantissa, exponent) = unsigned.split_once(['e', 'E']).unwrap_or((unsigned, "0"));
    let exponent = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = |part: &str| part.chars().all(|c| c.is_ascii_digit());
    let valid = digits(whole) && digits(fraction) && !(whole.is_empty() && fraction.is_empty()) && !exponent.is_empty() && digits(exponent);
    valid.then(|| text.parse::<f64>().ok()).flatten()
}

/// The reply to one JSON-RPC line; a notification gets none.
fn answer(line: &str, engine: &Engine) -> Option<Value> {
    let Ok(message) = json::parse(line) else {
        return Some(failure(Value::Null, -32700, "parse error"));
    };
    let Some(object) = message.as_object() else {
        return Some(failure(Value::Null, -32600, "invalid request"));
    };
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(failure(Value::Null, -32600, "invalid request"));
    };
    let id = match object.get("id")? {
        id @ (Value::Str(_) | Value::Num(_)) => id.clone(),
        _ => Value::Null,
    };
    let empty = Object::new();
    let params = object.get("params").and_then(Value::as_object).unwrap_or(&empty);
    Some(match result_of(method, params, engine) {
        Some(result) => json::object(vec![("id", id), ("jsonrpc", Value::str("2.0")), ("result", result)]),
        None => failure(id, -32601, &format!("method not found: {method}")),
    })
}

fn result_of(method: &str, params: &Object, engine: &Engine) -> Option<Value> {
    match method {
        "initialize" => {
            let version = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSION);
            let info = json::object(vec![("name", Value::str("critique-packet-search")), ("version", Value::str("1"))]);
            Some(json::object(vec![("capabilities", json::object(vec![("tools", json::object(vec![]))])), ("protocolVersion", Value::str(version)), ("serverInfo", info)]))
        }
        "ping" => Some(json::object(vec![])),
        "tools/list" => Some(json::object(vec![("tools", Value::Arr(engine.tools()))])),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let empty = Object::new();
            let arguments = params.get("arguments").and_then(Value::as_object).unwrap_or(&empty);
            let (text, is_error) = match engine.call(name, arguments) {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            let mut pairs = vec![("content", Value::Arr(vec![json::object(vec![("text", Value::Str(text)), ("type", Value::str("text"))])]))];
            if is_error {
                pairs.push(("isError", Value::Bool(true)));
            }
            Some(json::object(pairs))
        }
        _ => None,
    }
}

fn failure(id: Value, code: i32, message: &str) -> Value {
    json::object(vec![("error", json::object(vec![("code", Value::Num(f64::from(code))), ("message", Value::str(message))])), ("id", id), ("jsonrpc", Value::str("2.0"))])
}

/// The search over one packet copy. Every path resolves inside the root: an absolute path or one that climbs out finds
/// nothing, no symlink is followed or listed, and a path whose real location is outside the root finds nothing either.
pub(crate) struct Engine {
    /// The packet root with its own symlinks resolved, so containment is checked on real paths.
    root: String,
    limits: SearchLimits,
    /// Whether `symbol` is offered: the packet holds a repo map that is a plain file.
    symbol: bool,
}

impl Engine {
    pub(crate) fn open(root: &str, limits: SearchLimits) -> Result<Engine, String> {
        let real = io(std::fs::canonicalize(root), "realpath", root)?;
        let real = real.to_string_lossy().into_owned();
        let kind = io(std::fs::symlink_metadata(&real), "lstat", &real)?;
        if !kind.is_dir() {
            return Err(format!("{root} is not a directory"));
        }
        let symbol = std::fs::symlink_metadata(join(&[&real, REPO_MAP_PATH])).is_ok_and(|map| map.is_file());
        Ok(Engine { root: real, limits, symbol })
    }

    fn tools(&self) -> Vec<Value> {
        let Ok(Value::Arr(tools)) = json::parse(TOOL_DEFINITIONS) else { unreachable!("the tool definitions are a JSON list") };
        tools.into_iter().take(if self.symbol { 3 } else { 2 }).collect()
    }

    /// One tool's answer, or the error text of a call that was wrong.
    pub(crate) fn call(&self, tool: &str, args: &Object) -> Result<String, String> {
        match tool {
            "grep" => self.grep(args),
            "list" => self.list(args),
            "symbol" if self.symbol => self.symbol(args),
            other => Err(format!("unknown tool {other}")),
        }
    }

    fn grep(&self, args: &Object) -> Result<String, String> {
        let flags = if args.get("ignore_case") == Some(&Value::Bool(true)) { "i" } else { "" };
        let source = required_string(args, "pattern")?;
        let pattern = Regex::with_flags(&source, flags).map_err(|message| format!("Invalid regular expression: /{source}/{flags}: {}", v8_message(&message)))?;
        let glob = optional_string(args, "glob")?;
        let start = match optional_string(args, "path")? {
            None => self.root.clone(),
            Some(scope) => match self.inside(&scope) {
                Some(start) => start,
                None => return Ok(OUTSIDE.to_string()),
            },
        };
        let mut hits = Vec::new();
        for path in self.files(&start)? {
            if let Some(glob) = &glob {
                if !matches_glob(&path, glob)? {
                    continue;
                }
            }
            let absolute = join(&[&self.root, &path]);
            let bytes = io(std::fs::read(&absolute), "open", &absolute)?;
            for (index, line) in String::from_utf8_lossy(&bytes).split('\n').enumerate() {
                let chars: Vec<char> = line.chars().collect();
                if pattern.find_at(&chars, 0).is_some() {
                    hits.push(format!("{path}:{}: {}", index + 1, clip_line(line)));
                }
            }
        }
        Ok(capped_answer(&hits, &self.limits, "matches", "No matches."))
    }

    fn list(&self, args: &Object) -> Result<String, String> {
        let glob = required_string(args, "glob")?;
        let mut paths = Vec::new();
        for path in self.files(&self.root)? {
            if matches_glob(&path, &glob)? {
                paths.push(path);
            }
        }
        Ok(capped_answer(&paths, &self.limits, "paths", "No paths match."))
    }

    fn symbol(&self, args: &Object) -> Result<String, String> {
        let path = join(&[&self.root, REPO_MAP_PATH]);
        let map = io(std::fs::read(&path), "open", &path)?;
        let name = required_string(args, "name")?;
        Ok(capped_answer(&symbol_matches(&String::from_utf8_lossy(&map), &name), &self.limits, "matches", "No symbol of that name."))
    }

    /// The absolute path `path` names inside the root, or `None` when it is absolute, climbs out, passes through a
    /// symlink or does not exist.
    fn inside(&self, path: &str) -> Option<String> {
        if path.starts_with('/') {
            return None;
        }
        let target = join(&[&self.root, path]);
        let within = target == self.root || target.starts_with(&format!("{}/", self.root.trim_end_matches('/')));
        let real = std::fs::canonicalize(&target).ok()?;
        (within && real.to_string_lossy() == target).then_some(target)
    }

    /// Every plain file under `dir`, relative to the root, sorted as JavaScript sorts strings; symlinks are skipped,
    /// never followed.
    fn files(&self, dir: &str) -> Result<Vec<String>, String> {
        let kind = io(std::fs::symlink_metadata(dir), "lstat", dir)?;
        if kind.is_file() {
            return Ok(vec![self.relative(dir)]);
        }
        let mut found = Vec::new();
        self.walk(dir, &mut found)?;
        found.sort_by_key(|path| path.encode_utf16().collect::<Vec<u16>>());
        Ok(found)
    }

    fn walk(&self, dir: &str, found: &mut Vec<String>) -> Result<(), String> {
        let entries = io(std::fs::read_dir(dir), "scandir", dir)?;
        for entry in entries.flatten() {
            let path = join(&[dir, &entry.file_name().to_string_lossy()]);
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => self.walk(&path, found)?,
                Ok(kind) if kind.is_file() => found.push(self.relative(&path)),
                _ => {}
            }
        }
        Ok(())
    }

    fn relative(&self, path: &str) -> String {
        path.strip_prefix(&self.root).map_or(path, |rest| rest.trim_start_matches('/')).to_string()
    }
}

/// A file system result, with a failure worded as node words it: `ENOENT: no such file or directory, open '<path>'`.
pub(crate) fn io<T>(result: std::io::Result<T>, syscall: &str, path: &str) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(fail(&error, syscall, Some(path)).message),
    }
}

/// What V8 says of a pattern this engine's parser refuses, where the two refuse the same pattern; the engine's own
/// words for syntax it does not implement.
fn v8_message(message: &str) -> &str {
    match message {
        "a group is not closed" => "Unterminated group",
        "a group is closed that was not opened" => "Unmatched ')'",
        "a class is not closed" => "Unterminated character class",
        "a pattern ends with a backslash" => "\\ at end of pattern",
        "nothing to repeat" => "Nothing to repeat",
        "range out of order in character class" => "Range out of order in character class",
        other => other,
    }
}

fn required_string(args: &Object, key: &str) -> Result<String, String> {
    match args.get(key) {
        Some(Value::Str(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(format!("{key} must be a non-empty string")),
    }
}

fn optional_string(args: &Object, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::Str(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("{key} must be a string")),
    }
}

fn clip_line(text: &str) -> String {
    if text.encode_utf16().count() as f64 <= LINE_MAX_CHARS {
        text.to_string()
    } else {
        format!("{}…", js_slice(text, 0.0, Some(LINE_MAX_CHARS)))
    }
}

/// The answer for these result lines, cut to the limits: the first lines that fit both caps, then a marker that says
/// how many were left out and asks for a narrower search.
pub(crate) fn capped_answer(lines: &[String], limits: &SearchLimits, unit: &str, empty: &str) -> String {
    if lines.is_empty() {
        return empty.to_string();
    }
    let mut kept = Vec::new();
    let mut size = 0.0;
    for line in lines.iter().take(limits.max_matches.min(lines.len() as f64) as usize) {
        let bytes = (line.len() + 1) as f64;
        if size + bytes > limits.max_bytes {
            break;
        }
        kept.push(line.clone());
        size += bytes;
    }
    let left = lines.len() - kept.len();
    if left > 0 {
        kept.push(format!("[{left} more {unit} left out; narrow the pattern, path or glob]"));
    }
    kept.join("\n")
}

/// The repo map's symbols named `name`: an exact match first, else every name that contains it, ignoring case. Each
/// reads `tree/<path>:<line>: <kind> <name>, <refs> refs: <signature>`.
pub(crate) fn symbol_matches(repo_map: &str, name: &str) -> Vec<String> {
    let heading = Regex::new(r"^## `(.+)` \(referenced by", false).expect("the heading pattern");
    let entry = Regex::new(r"^- `([^`]+)` \(([^,]+), line (\d+), (\d+) refs\): `(.*)`$", false).expect("the symbol pattern");
    let mut symbols: Vec<(String, String, String, String)> = Vec::new();
    let mut path: Option<String> = None;
    for line in repo_map.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        if let Some(hit) = heading.find_at(&chars, 0) {
            path = hit.group(&chars, 1);
            continue;
        }
        let (Some(hit), Some(path)) = (entry.find_at(&chars, 0), &path) else { continue };
        let group = |index| hit.group(&chars, index).unwrap_or_default();
        let symbol = group(1);
        let text = format!("{} {symbol}, {} refs: {}", group(2), group(4), group(5));
        symbols.push((path.clone(), symbol, group(3), text));
    }
    let exact: Vec<_> = symbols.iter().filter(|(_, symbol, _, _)| symbol == name).collect();
    let lower = name.to_lowercase();
    let found = if exact.is_empty() { symbols.iter().filter(|(_, symbol, _, _)| symbol.to_lowercase().contains(&lower)).collect() } else { exact };
    found.into_iter().map(|(path, _, line, text)| format!("tree/{path}:{line}: {}", clip_line(text))).collect()
}
