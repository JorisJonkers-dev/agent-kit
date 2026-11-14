//! `fetch`: Workflow A's fetch shim. It downloads the public sources the
//! planning node listed and stores each as text, so the fetchers read a page
//! that a pinned wrapper fetched and the verifier checks every quote against
//! that same page (issue #456).
//!
//! The plan is a JSON object whose `sources` array holds `{id, url}`
//! objects (other keys are ignored). The width limit is enforced here, not
//! trusted to the planner: a plan with more than `--max-sources` sources,
//! an id that is not `[a-z0-9][a-z0-9-]*` (at most 64 bytes), a duplicate
//! id, or a url that is not `https://` on a public host name (no IP literal,
//! no `localhost`, `.local`, `.internal` or dotless host, no user info) is
//! refused whole and nothing is fetched.
//!
//! Each source is fetched with `curl` (https only, redirects too, at most
//! five, `--max-time` and `--max-filesize` set), its text extracted
//! ([crate::html::page_text]) and stored as `<out>/<id>.txt`. A source that
//! fails is recorded with its error and the others go on. The log,
//! `<out>/fetch-log.json`, lists every source with either `file`, `sha256`
//! (of the stored text) and `bytes`, or `error`.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::html::page_text;

/// The fetch log's file name inside the output directory.
pub const LOG: &str = "fetch-log.json";

/// How `fetch` runs: the width limit, the per-page limits and the `curl`
/// program (a flag, so a test or a live wiring can pin it).
pub struct Limits {
    pub max_sources: usize,
    pub max_bytes: u64,
    pub timeout_secs: u64,
    pub curl: String,
}

/// One source of the plan.
pub(crate) struct Source {
    pub(crate) id: String,
    pub(crate) url: String,
}

/// Why `fetch` stopped: `Usage` for an unreadable plan or output directory
/// (exit 2), `Refused` for plan rule problems or no page fetched (exit 1).
#[derive(Debug, PartialEq)]
pub enum Failure {
    Usage(String),
    Refused(Vec<String>),
}

/// Whether `id` is a safe file stem: `[a-z0-9][a-z0-9-]*`, at most 64 bytes.
fn good_id(id: &str) -> bool {
    let mut chars = id.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    first_ok
        && id.len() <= 64
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The problem with `url`, or `None` when it is https on a public host name.
fn url_problem(url: &str) -> Option<&'static str> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Some("not-https");
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Some("not-public");
    }
    let host = authority
        .rsplit_once(':')
        .map_or(authority, |(host, _port)| host)
        .to_ascii_lowercase();
    let literal = host.starts_with('[') || host.chars().all(|c| c.is_ascii_digit() || c == '.');
    let local = host == "localhost"
        || [".localhost", ".local", ".internal"]
            .iter()
            .any(|suffix| host.ends_with(suffix));
    if literal || local || !host.contains('.') {
        return Some("not-public");
    }
    None
}

/// Reads and checks the plan: its sources, or every problem with them.
pub(crate) fn read_plan(plan: &Path, max_sources: usize) -> Result<Vec<Source>, Failure> {
    let text = fs::read_to_string(plan)
        .map_err(|e| Failure::Usage(format!("{}: cannot read: {e}", plan.display())))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::Usage(format!("{}: not valid JSON: {e}", plan.display())))?;
    let Some(items) = value.get("sources").and_then(Value::as_array) else {
        return Err(Failure::Usage(format!(
            "{}: no sources array",
            plan.display()
        )));
    };
    let mut problems = Vec::new();
    if items.len() > max_sources {
        problems.push(format!(
            "plan: too-many-sources: {} > {max_sources}",
            items.len()
        ));
    }
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        let url = item.get("url").and_then(Value::as_str).unwrap_or_default();
        let shown = if id.is_empty() {
            format!("sources[{index}]")
        } else {
            id.to_string()
        };
        if !good_id(id) {
            problems.push(format!("{shown}: bad-id"));
        } else if !seen.insert(id.to_string()) {
            problems.push(format!("{shown}: duplicate-id"));
        }
        if let Some(problem) = url_problem(url) {
            problems.push(format!("{shown}: {problem}"));
        }
        sources.push(Source {
            id: id.to_string(),
            url: url.to_string(),
        });
    }
    if problems.is_empty() {
        Ok(sources)
    } else {
        Err(Failure::Refused(problems))
    }
}

/// One stored page: its file name in the output directory, the SHA-256 of
/// its text and the text's length in bytes.
struct Stored {
    file: String,
    sha256: String,
    bytes: usize,
}

/// Fetches one source into `<out>/<id>.txt`: the stored page, or the error
/// that stopped it.
fn fetch_one(source: &Source, out: &Path, limits: &Limits) -> Result<Stored, String> {
    let raw = out.join(format!("{}.raw", source.id));
    let result = Command::new(&limits.curl)
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args([
            "--max-redirs",
            "5",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
        ])
        .arg("--max-time")
        .arg(limits.timeout_secs.to_string())
        .arg("--max-filesize")
        .arg(limits.max_bytes.to_string())
        .arg("--output")
        .arg(&raw)
        .arg("--")
        .arg(&source.url)
        .output()
        .map_err(|e| format!("cannot start {}: {e}", limits.curl))?;
    let bytes = fs::read(&raw).unwrap_or_default();
    let _ = fs::remove_file(&raw);
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let first = stderr.lines().next().unwrap_or_default().trim().to_string();
        return Err(format!(
            "curl exit {}: {first}",
            result.status.code().unwrap_or(-1)
        ));
    }
    if bytes.len() as u64 > limits.max_bytes {
        return Err(format!("too-large: over {} bytes", limits.max_bytes));
    }
    let text = page_text(&String::from_utf8_lossy(&bytes));
    if text.is_empty() {
        return Err("no-text".to_string());
    }
    let file = format!("{}.txt", source.id);
    fs::write(out.join(&file), &text).map_err(|e| format!("cannot write {file}: {e}"))?;
    Ok(Stored {
        sha256: gold_band_report::sha256_hex(text.as_bytes()),
        bytes: text.len(),
        file,
    })
}

/// Runs the fetch: the number of pages stored, or why it stopped. The log
/// is written whenever the plan passed, even when no page was fetched.
pub fn fetch(plan: &Path, out: &Path, limits: &Limits) -> Result<usize, Failure> {
    let sources = read_plan(plan, limits.max_sources)?;
    fs::create_dir_all(out)
        .map_err(|e| Failure::Usage(format!("{}: cannot create: {e}", out.display())))?;
    let _ = fs::remove_file(out.join(LOG));
    let mut records = Vec::new();
    let mut stored = 0;
    for source in &sources {
        let mut record = json!({"id": source.id, "url": source.url});
        match fetch_one(source, out, limits) {
            Ok(page) => {
                stored += 1;
                record["file"] = Value::String(page.file);
                record["sha256"] = Value::String(page.sha256);
                record["bytes"] = json!(page.bytes);
            }
            Err(error) => {
                record["error"] = Value::String(error);
            }
        }
        records.push(record);
    }
    let log = json!({"max_sources": limits.max_sources, "sources": records});
    let text = serde_json::to_string_pretty(&log).unwrap_or_default() + "\n";
    fs::write(out.join(LOG), text)
        .map_err(|e| Failure::Usage(format!("{LOG}: cannot write: {e}")))?;
    if stored == 0 {
        return Err(Failure::Refused(vec!["plan: no-page-fetched".to_string()]));
    }
    Ok(stored)
}

/// Whether `theme` is a theme folder name: one or two `/`-separated parts of
/// `[a-z0-9][a-z0-9-]*`.
fn good_theme(theme: &str) -> bool {
    let parts: Vec<&str> = theme.split('/').collect();
    parts.len() <= 2 && parts.iter().all(|part| good_id(part))
}

/// `adopt`: copies every page the fetch stored, byte for byte, into the
/// plan's theme folder of the papers directory as `<theme>/<id>.txt`, so the
/// merge worker never retypes a page. A file already there with other bytes is
/// refused (`exists`), never overwritten. Returns the number of pages copied.
pub fn adopt(plan: &Path, fetched: &Path, papers: &Path) -> Result<usize, Failure> {
    let text = fs::read_to_string(plan)
        .map_err(|e| Failure::Usage(format!("{}: cannot read: {e}", plan.display())))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| Failure::Usage(format!("{}: not valid JSON: {e}", plan.display())))?;
    let theme = value
        .get("theme")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !good_theme(theme) {
        return Err(Failure::Refused(vec![format!("plan: bad-theme: {theme}")]));
    }
    let log_path = fetched.join(LOG);
    let log: Value = fs::read_to_string(&log_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| Failure::Usage(format!("{}: cannot read", log_path.display())))?;
    let dir = papers.join(theme);
    fs::create_dir_all(&dir).map_err(|e| Failure::Usage(format!("{theme}: cannot create: {e}")))?;
    let mut problems = Vec::new();
    let mut copied = 0;
    let sources = log
        .get("sources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for source in &sources {
        let id = source.get("id").and_then(Value::as_str).unwrap_or_default();
        let Some(file) = source.get("file").and_then(Value::as_str) else {
            continue;
        };
        if !good_id(id) || file != format!("{id}.txt") {
            problems.push(format!("{id}: bad-log-entry"));
            continue;
        }
        let Ok(bytes) = fs::read(fetched.join(file)) else {
            problems.push(format!("{file}: missing-page"));
            continue;
        };
        let target = dir.join(file);
        match fs::read(&target) {
            Ok(existing) if existing == bytes => copied += 1,
            Ok(_) => problems.push(format!("{theme}/{file}: exists")),
            Err(_) => match fs::write(&target, &bytes) {
                Ok(()) => copied += 1,
                Err(e) => problems.push(format!("{theme}/{file}: cannot write: {e}")),
            },
        }
    }
    if problems.is_empty() {
        Ok(copied)
    } else {
        Err(Failure::Refused(problems))
    }
}
