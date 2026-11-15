//! `estate verdict [--signals FILE]`: applies the estate delivery verdict to
//! the signals SKILL.md gathers.
//!
//! Reads one JSON object from stdin, or from `--signals FILE`. Prints a
//! `VERDICT:` line, then one line per signal. Exits 0 for any well-formed
//! input and 2 for malformed input (message on stderr).

use crate::args::{self, Spec};
use crate::text::repr_list;
use crate::Ctx;
use serde_json::{Map, Value};

const SPEC: Spec = Spec {
    cmd: "verdict",
    usage: "usage: estate verdict [--signals FILE]",
    values: &["--signals"],
    required: &[],
    flags: &[],
    min_positional: 0,
    max_positional: 0,
};

const PUBLISH_STATES: [&str; 3] = ["failure", "missing", "success"];
const COMPOSE_STATES: [&str; 3] = ["clean", "errors", "missing"];
const STATUS_STATES: [&str; 4] = ["failure", "missing", "pending", "success"];
const DEPLOYMENT_STATES: [&str; 4] = ["failure", "in_progress", "missing", "success"];

fn require(condition: bool, message: impl FnOnce() -> String) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message())
    }
}

fn full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn read_text<'a>(obj: &'a Map<String, Value>, key: &str, place: &str) -> Result<&'a str, String> {
    let value = obj
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty());
    value.ok_or_else(|| format!("{place}.{key} must be a non-empty string"))
}

struct Signal<'a> {
    name: &'static str,
    state: &'a str,
    url: &'a str,
    head_sha: Option<&'a str>,
    object: &'a Map<String, Value>,
}

fn read_signal<'a>(
    signals: &'a Map<String, Value>,
    name: &'static str,
    states: &[&str],
) -> Result<Signal<'a>, String> {
    let object = signals
        .get(name)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("signals.{name} must be an object"))?;
    let state = object
        .get("state")
        .and_then(Value::as_str)
        .filter(|s| states.contains(s))
        .ok_or_else(|| format!("signals.{name}.state must be one of {}", repr_list(states)))?;
    let url = match object.get("url") {
        None => "",
        Some(Value::String(u)) => u,
        Some(_) => return Err(format!("signals.{name}.url must be a string")),
    };
    let head_sha = match object.get("headSha") {
        None | Some(Value::Null) => None,
        Some(Value::String(h)) if h.is_empty() => None,
        Some(Value::String(h)) if full_sha(h) => Some(h.as_str()),
        Some(_) => {
            return Err(format!(
                "signals.{name}.headSha must be a 40-hex commit, an empty string or null"
            ))
        }
    };
    Ok(Signal {
        name,
        state,
        url,
        head_sha,
        object,
    })
}

struct Issue<'a> {
    title: &'a str,
    url: &'a str,
}

fn read_issues(signals: &Map<String, Value>) -> Result<Vec<Issue<'_>>, String> {
    let list = match signals.get("issues") {
        None => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err("signals.issues must be a list".into()),
    };
    list.iter()
        .map(|issue| {
            let field = |key: &str| issue.get(key).and_then(Value::as_str);
            match (field("title"), field("url")) {
                (Some(title), Some(url)) => Ok(Issue { title, url }),
                _ => Err("each signals.issues entry needs string title and url".to_string()),
            }
        })
        .collect()
}

/// A signal for another commit than the release counts as missing.
fn effective_state<'a>(signal: &Signal<'a>, sha: &str) -> (&'a str, Option<String>) {
    match signal.head_sha {
        Some(head) if !head.eq_ignore_ascii_case(sha) => (
            "missing",
            Some(format!("headSha {head} differs from {sha}")),
        ),
        _ => (signal.state, None),
    }
}

/// landed: both good; not landed: neither good; mixed: exactly one. The
/// Deployment and the publish run are reported but never decide.
pub fn verdict(compose: &str, status: &str) -> &'static str {
    match (compose == "clean", status == "success") {
        (true, true) => "landed",
        (false, false) => "not landed",
        _ => "mixed",
    }
}

/// Validates the input and renders the whole report.
pub fn evaluate(data: &Value) -> Result<String, String> {
    let data = data.as_object().ok_or("input must be a JSON object")?;
    read_text(data, "project", "input")?;
    read_text(data, "release", "input")?;
    let sha = read_text(data, "sha", "input")?;
    require(full_sha(sha), || "input.sha must be a 40-hex commit".into())?;
    let signals = data
        .get("signals")
        .and_then(Value::as_object)
        .ok_or("signals must be an object")?;

    let publish = read_signal(signals, "publish", &PUBLISH_STATES)?;
    let compose = read_signal(signals, "compose", &COMPOSE_STATES)?;
    let codes: Vec<&str> = match compose.object.get("codes") {
        None => Vec::new(),
        Some(Value::Array(list)) => list
            .iter()
            .map(Value::as_str)
            .collect::<Option<_>>()
            .ok_or("signals.compose.codes must be a list of strings")?,
        Some(_) => return Err("signals.compose.codes must be a list of strings".into()),
    };
    let status = read_signal(signals, "status", &STATUS_STATES)?;
    let deployment = read_signal(signals, "deployment", &DEPLOYMENT_STATES)?;
    let issues = read_issues(signals)?;

    let rows = [publish, compose, status, deployment];
    let effective: Vec<(&str, Option<String>)> =
        rows.iter().map(|s| effective_state(s, sha)).collect();
    let mut lines = vec![format!(
        "VERDICT: {}",
        verdict(effective[1].0, effective[2].0)
    )];
    for (signal, (state, note)) in rows.iter().zip(&effective) {
        let mut line = format!("{}: {state}  {}", signal.name, signal.url);
        if let Some(note) = note {
            line.push_str(&format!("  (counted as missing: {note})"));
        }
        lines.push(line);
        if signal.name == "compose" {
            let shown = if codes.is_empty() {
                "none".to_string()
            } else {
                codes.join(", ")
            };
            lines.push(format!("  codes: {shown}"));
        }
    }
    if issues.is_empty() {
        lines.push("issues: none open".into());
    } else {
        lines.push(format!("issues: {} open", issues.len()));
        lines.extend(issues.iter().map(|i| format!("  {}  {}", i.title, i.url)));
    }
    Ok(lines.join("\n"))
}

fn read_input(file: Option<&str>, ctx: &mut Ctx) -> Result<Value, String> {
    let mut bytes = Vec::new();
    match file {
        Some(path) => bytes = std::fs::read(path).map_err(|e| e.to_string())?,
        None => {
            ctx.input
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
        }
    }
    let text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn run(argv: &[String], ctx: &mut Ctx) -> i32 {
    let parsed = match args::parse(&SPEC, argv, ctx) {
        Ok(p) => p,
        Err(status) => return status,
    };
    match read_input(parsed.value("--signals"), ctx).and_then(|data| evaluate(&data)) {
        Ok(report) => {
            ctx.say(&report);
            0
        }
        Err(error) => {
            ctx.warn(&format!("estate verdict: malformed input: {error}"));
            2
        }
    }
}

#[cfg(test)]
mod tests;
