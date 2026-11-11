//! Transcript tool-mix analyser (issue #516, spec #392 story 149).
//!
//! Reads agent transcripts under `<root>/<agent>/**/*.jsonl`, keeps the tasks
//! that started inside a time window, and prints per agent which tools were
//! used to find code and how many tokens each task spent exploring.
//!
//! A transcript is one task. Each line is a JSON object; the analyser reads
//! the `timestamp` of a line and, for `"type":"assistant"` lines, the
//! `message.content` blocks of `"type":"tool_use"` (their `name` and `input`)
//! and `message.usage.output_tokens`.
//!
//! Exploration tokens are the output tokens of the assistant turns that call
//! tools, counted from the start of the task up to, and not including, the
//! first turn that calls an editing tool ([`EDIT_TOOLS`]). A task that never
//! edits counts every tool-calling turn.

use code_nav_eval::{parse_json, Json};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "usage: transcript-mix --root DIR --since DATE [--until DATE]\n\
\n\
Reads DIR/<agent>/**/*.jsonl, one transcript per task, and prints per agent the\n\
code-finding tools used and the tokens spent exploring before the first edit.\n\
DATE is YYYY-MM-DD or an RFC 3339 timestamp; a task is in the window when its\n\
first timestamp is at or after --since and before --until.\n";

/// Tools that edit files: the first one ends a task's exploration.
pub const EDIT_TOOLS: &[&str] = &["edit", "write", "multiedit", "notebookedit", "apply_patch", "patch"];

/// How a tool is used to find code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Search,
    Read,
    Lsp,
    Knowledge,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Search => "search",
            Kind::Read => "read",
            Kind::Lsp => "lsp",
            Kind::Knowledge => "knowledge",
        }
    }
}

/// Which code-finding kind a tool call is, or `None` when it does not find code.
pub fn classify(name: &str, input: Option<&Json>) -> Option<Kind> {
    let lower = name.to_ascii_lowercase();
    if lower.starts_with("mcp__") {
        if lower.contains("lsp") {
            return Some(Kind::Lsp);
        }
        if lower.contains("hindsight") || lower.contains("memory") || lower.contains("knowledge") {
            return Some(Kind::Knowledge);
        }
        return None;
    }
    match lower.as_str() {
        "grep" | "glob" | "search" | "find" | "codesearch" => Some(Kind::Search),
        "read" | "view" | "cat" => Some(Kind::Read),
        "lsp" => Some(Kind::Lsp),
        "bash" | "shell" => {
            let cmd = input.and_then(|i| i.get("command")).and_then(Json::as_str).unwrap_or("");
            match cmd.split_whitespace().next() {
                Some("rg" | "grep" | "fd" | "find" | "ag" | "ack") => Some(Kind::Search),
                _ => None,
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- time

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn num(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Seconds since the Unix epoch for `YYYY-MM-DD` (midnight UTC) or an RFC 3339
/// timestamp (fractions of a second are dropped; `Z`, `+HH:MM` or `-HH:MM`).
pub fn parse_time(text: &str) -> Result<i64, String> {
    let bad = || format!("invalid time '{text}' (want YYYY-MM-DD or RFC 3339)");
    let (date, rest) = match text.split_once('T') {
        Some((d, r)) => (d, Some(r)),
        None => (text, None),
    };
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return Err(bad());
    }
    let (y, m, d) = (
        num(parts[0]).ok_or_else(bad)?,
        num(parts[1]).ok_or_else(bad)?,
        num(parts[2]).ok_or_else(bad)?,
    );
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=12).contains(&m) || d < 1 || d > month_days[m as usize - 1] {
        return Err(bad());
    }
    let mut secs = days_from_civil(y, m, d) * 86400;
    let Some(rest) = rest else { return Ok(secs) };
    let split = rest.find(['Z', '+', '-']).ok_or_else(bad)?;
    let (clock, zone) = rest.split_at(split);
    let clock = clock.split('.').next().unwrap_or("");
    let c: Vec<&str> = clock.split(':').collect();
    if c.len() != 3 {
        return Err(bad());
    }
    let (h, mi, s) = (num(c[0]).ok_or_else(bad)?, num(c[1]).ok_or_else(bad)?, num(c[2]).ok_or_else(bad)?);
    if h > 23 || mi > 59 || s > 60 {
        return Err(bad());
    }
    secs += h * 3600 + mi * 60 + s;
    if zone != "Z" {
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let (zh, zm) = zone[1..].split_once(':').ok_or_else(bad)?;
        secs -= sign * (num(zh).ok_or_else(bad)? * 3600 + num(zm).ok_or_else(bad)? * 60);
    }
    Ok(secs)
}

// ---------------------------------------------------------------- analysis

/// What one transcript shows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Task {
    /// Seconds since the epoch of the first line with a readable timestamp.
    pub started: Option<i64>,
    /// Code-finding tool calls by tool name.
    pub tools: BTreeMap<String, (Kind, u64)>,
    /// Tokens spent in tool-calling turns before the first edit.
    pub exploration_tokens: u64,
    pub edited: bool,
}

/// Read one transcript. Lines that are blank or not JSON are skipped.
pub fn analyse_transcript(text: &str) -> Task {
    let mut task = Task::default();
    for line in text.lines() {
        let Ok(entry) = parse_json(line) else { continue };
        if task.started.is_none() {
            task.started = entry.get("timestamp").and_then(Json::as_str).and_then(|t| parse_time(t).ok());
        }
        if entry.get("type").and_then(Json::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = entry.get("message") else { continue };
        let blocks = message.get("content").and_then(Json::as_arr).unwrap_or(&[]);
        let calls: Vec<(&str, Option<&Json>)> = blocks
            .iter()
            .filter(|b| b.get("type").and_then(Json::as_str) == Some("tool_use"))
            .filter_map(|b| Some((b.get("name")?.as_str()?, b.get("input"))))
            .collect();
        if calls.is_empty() {
            continue;
        }
        let editing = calls.iter().any(|(n, _)| EDIT_TOOLS.contains(&n.to_ascii_lowercase().as_str()));
        if !task.edited {
            if editing {
                task.edited = true;
            } else {
                let tokens = message.get("usage").and_then(|u| u.get("output_tokens")).and_then(Json::as_u64);
                task.exploration_tokens += tokens.unwrap_or(0);
            }
        }
        for (name, input) in calls {
            if let Some(kind) = classify(name, input) {
                task.tools.entry(name.to_string()).or_insert((kind, 0)).1 += 1;
            }
        }
    }
    task
}

/// Every `*.jsonl` file under `dir`, sorted.
fn jsonl_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            jsonl_files(&path, out)?;
        } else if path.extension().is_some_and(|x| x == "jsonl") {
            out.push(path);
        }
    }
    out.sort();
    Ok(())
}

/// One agent's totals over the window.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AgentMix {
    pub tasks: u64,
    pub edited: u64,
    pub tools: BTreeMap<String, (Kind, u64)>,
    pub exploration_tokens: u64,
}

/// Analyse every agent directory under `root`; tasks starting in
/// `[since, until)` count. A transcript with no readable timestamp is outside
/// every window.
pub fn analyse_root(root: &Path, since: i64, until: i64) -> Result<BTreeMap<String, AgentMix>, String> {
    let entries = std::fs::read_dir(root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    let mut mixes = BTreeMap::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let agent = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let mut files = Vec::new();
        jsonl_files(&dir, &mut files)?;
        let mut mix = AgentMix::default();
        for file in files {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
            let task = analyse_transcript(&text);
            if !task.started.is_some_and(|t| t >= since && t < until) {
                continue;
            }
            mix.tasks += 1;
            mix.edited += u64::from(task.edited);
            mix.exploration_tokens += task.exploration_tokens;
            for (name, (kind, n)) in task.tools {
                mix.tools.entry(name).or_insert((kind, 0)).1 += n;
            }
        }
        if mix.tasks > 0 {
            mixes.insert(agent, mix);
        }
    }
    Ok(mixes)
}

/// The report: one block per agent.
pub fn render(mixes: &BTreeMap<String, AgentMix>) -> String {
    if mixes.is_empty() {
        return "no transcripts in the window\n".to_string();
    }
    let mut out = String::new();
    for (agent, mix) in mixes {
        out.push_str(&format!("agent {agent}\n"));
        out.push_str(&format!("  tasks {} (edited {})\n", mix.tasks, mix.edited));
        for (name, (kind, n)) in &mix.tools {
            out.push_str(&format!("  tool {name} ({}) {n}\n", kind.label()));
        }
        out.push_str(&format!("  exploration_tokens {}\n", mix.exploration_tokens));
    }
    out
}

fn option<'a>(args: &'a [String], flag: &str) -> Result<Option<&'a str>, String> {
    match args.iter().position(|a| a == flag) {
        None => Ok(None),
        Some(i) => args.get(i + 1).map(|v| Some(v.as_str())).ok_or_else(|| format!("{flag} needs a value")),
    }
}

fn parse_args(args: &[String]) -> Result<(PathBuf, i64, i64), String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" | "--since" | "--until" => i += 2,
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    let root = option(args, "--root")?.ok_or("--root is required")?;
    let since = parse_time(option(args, "--since")?.ok_or("--since is required")?)?;
    let until = match option(args, "--until")? {
        Some(u) => parse_time(u)?,
        None => i64::MAX,
    };
    Ok((PathBuf::from(root), since, until))
}

/// Run one command line; output and errors go to the given writers.
pub fn cli(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        let _ = write!(out, "{USAGE}");
        return 0;
    }
    let result = parse_args(args).and_then(|(root, since, until)| analyse_root(&root, since, until));
    match result {
        Ok(mixes) => {
            let _ = write!(out, "{}", render(&mixes));
            0
        }
        Err(e) => {
            let _ = writeln!(err, "error: {e}\n{USAGE}");
            2
        }
    }
}

/// The process entry: the real stdout and stderr.
pub fn run(args: impl IntoIterator<Item = OsString>) -> i32 {
    let args: Vec<String> = args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect();
    cli(&args, &mut std::io::stdout(), &mut std::io::stderr())
}

#[cfg(test)]
mod tests;
