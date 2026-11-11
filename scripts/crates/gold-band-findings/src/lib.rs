//! The finding check shim (#450, spec #392 story 91): only a finding with
//! checkable evidence may drive a repair.
//!
//! Workflow B's critics answer in the critique runner's finding schema
//! (`skills/workstation/critique/schemas/finding.schema.json`). This command
//! is the gate node that runs between the critics and the repair nodes. It
//! reads each critic's answer, the diff under review and the ticket, and
//! sorts every finding:
//!
//! * **kept** when it validates against the schema and carries at least one
//!   `quote` evidence item, every quote of which is found in the diff or the
//!   ticket (whitespace runs count as one space);
//! * a **hint** otherwise, with the reason: `schema` (it does not validate),
//!   `no-quote` (nothing that can be checked), `quote-too-short` (a quote
//!   shorter than [`MIN_QUOTE_CHARS`] matches anything) or `quote-not-found`
//!   (a fabricated quote). A hint is reported and never sent back to the
//!   writer.
//!
//! ```text
//! gold-band-findings check --run-id ID --attempt N --diff FILE
//!     --ticket FILE --report-dir DIR FINDINGS_FILE...
//! ```
//!
//! It writes `DIR/findings-check.json` (atomically) and prints the same JSON
//! on stdout: `run_id`, `attempt`, `actionable` (the number kept),
//! `hint_count`, `kept` and `hints`. A workflow routes back to the writer
//! only when `actionable` is above zero, so a run with hints alone does not.
//! The node runs under the gate wrapper (`gold-band-gate run`), so a green
//! node means the check ran.
//!
//! Exit 0: the check ran (whatever it found). Exit 1: a critic's answer is
//! not a finding document (not JSON, or it breaks the document's schema), so
//! the run must not read it as "no findings". Exit 2: bad usage or an input
//! that cannot be read.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;

use serde_json::{json, Map, Value};

/// The crate's name, which prefixes every message line.
pub const NAME: &str = "gold-band-findings";
/// The file the check writes inside the report directory.
pub const REPORT_FILE: &str = "findings-check.json";
/// The most bytes read from any one input.
pub const MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;
/// The shortest quote, in characters after whitespace is collapsed, that can
/// count as evidence.
pub const MIN_QUOTE_CHARS: usize = 8;

const SEVERITIES: [&str; 3] = ["critical", "major", "minor"];
const CATEGORIES: [&str; 13] = [
    "unasked-question",
    "contradiction",
    "omission",
    "invention",
    "untestable",
    "failure-mode",
    "security",
    "structure",
    "bug",
    "integration",
    "drift",
    "stale",
    "undocumented-decision",
];
const EVIDENCE_KINDS: [&str; 4] = ["quote", "input", "failing-test", "trace"];
const FINDING_KEYS: [&str; 8] = [
    "id",
    "severity",
    "category",
    "check",
    "location",
    "claim",
    "evidence",
    "suggestion",
];
const REQUIRED_KEYS: [&str; 7] = [
    "id", "severity", "category", "check", "location", "claim", "evidence",
];

/// Why a finding is only a hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hint {
    /// It does not validate against the finding schema; the reason.
    Schema(String),
    /// It has no `quote` evidence, so nothing can be checked.
    NoQuote,
    /// A quote is too short to be evidence.
    QuoteTooShort,
    /// A quote is found in neither the diff nor the ticket.
    QuoteNotFound,
}

impl Hint {
    /// The stable reason code written to the report.
    pub fn code(&self) -> &'static str {
        match self {
            Hint::Schema(_) => "schema",
            Hint::NoQuote => "no-quote",
            Hint::QuoteTooShort => "quote-too-short",
            Hint::QuoteNotFound => "quote-not-found",
        }
    }

    fn detail(&self) -> String {
        match self {
            Hint::Schema(why) => why.clone(),
            Hint::NoQuote => "no quote evidence to check".to_string(),
            Hint::QuoteTooShort => {
                format!("a quote shorter than {MIN_QUOTE_CHARS} characters")
            }
            Hint::QuoteNotFound => "a quote found in neither the diff nor the ticket".to_string(),
        }
    }
}

/// The sorted findings of one or more critics.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    /// Findings with checkable evidence, each with its `source` index.
    pub kept: Vec<Value>,
    /// Findings without, each with its `source`, `reason` and `detail`.
    pub hints: Vec<Value>,
}

/// Collapses every run of whitespace to one space and trims the ends.
pub fn normalise(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Validates one finding against the finding schema.
pub fn validate_finding(finding: &Value) -> Result<(), String> {
    let obj = finding.as_object().ok_or("a finding is not an object")?;
    if let Some(key) = obj.keys().find(|k| !FINDING_KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }
    if let Some(key) = REQUIRED_KEYS.iter().find(|k| !obj.contains_key(**k)) {
        return Err(format!("missing key `{key}`"));
    }
    for key in ["id", "location", "claim"] {
        non_empty_string(obj, key)?;
    }
    one_of(obj, "severity", &SEVERITIES)?;
    one_of(obj, "category", &CATEGORIES)?;
    let check = non_empty_string(obj, "check")?;
    if !is_check_id(check) {
        return Err("`check` is not a kebab-case check id".to_string());
    }
    if let Some(s) = obj.get("suggestion") {
        if !s.is_string() {
            return Err("`suggestion` is not a string".to_string());
        }
    }
    let evidence = obj["evidence"]
        .as_array()
        .ok_or("`evidence` is not an array")?;
    evidence.iter().try_for_each(validate_evidence)
}

fn non_empty_string<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    match obj[key].as_str() {
        Some(s) if !s.is_empty() => Ok(s),
        _ => Err(format!("`{key}` is not a non-empty string")),
    }
}

fn one_of(obj: &Map<String, Value>, key: &str, allowed: &[&str]) -> Result<(), String> {
    match obj[key].as_str() {
        Some(s) if allowed.contains(&s) => Ok(()),
        _ => Err(format!("`{key}` is not one of {}", allowed.join(", "))),
    }
}

/// `^[a-z0-9]+(-[a-z0-9]+)*$`.
fn is_check_id(text: &str) -> bool {
    text.split('-').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

fn validate_evidence(item: &Value) -> Result<(), String> {
    let obj = item
        .as_object()
        .ok_or("an evidence item is not an object")?;
    if let Some(key) = obj
        .keys()
        .find(|k| !["kind", "value", "file"].contains(&k.as_str()))
    {
        return Err(format!("unknown evidence key `{key}`"));
    }
    match obj.get("kind").and_then(Value::as_str) {
        Some(kind) if EVIDENCE_KINDS.contains(&kind) => {}
        _ => {
            return Err(format!(
                "evidence `kind` is not one of {}",
                EVIDENCE_KINDS.join(", ")
            ))
        }
    }
    match obj.get("value").and_then(Value::as_str) {
        Some(s) if !s.is_empty() => {}
        _ => return Err("evidence `value` is not a non-empty string".to_string()),
    }
    match obj.get("file") {
        None => Ok(()),
        Some(Value::String(s)) if !s.is_empty() => Ok(()),
        Some(_) => Err("evidence `file` is not a non-empty string".to_string()),
    }
}

/// Decides one finding against the normalised diff and ticket.
pub fn judge(finding: &Value, diff: &str, ticket: &str) -> Result<(), Hint> {
    validate_finding(finding).map_err(Hint::Schema)?;
    let quotes: Vec<String> = finding["evidence"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["kind"] == "quote")
        .filter_map(|e| e["value"].as_str())
        .map(normalise)
        .collect();
    if quotes.is_empty() {
        return Err(Hint::NoQuote);
    }
    if quotes.iter().any(|q| q.chars().count() < MIN_QUOTE_CHARS) {
        return Err(Hint::QuoteTooShort);
    }
    if quotes
        .iter()
        .any(|q| !diff.contains(q) && !ticket.contains(q))
    {
        return Err(Hint::QuoteNotFound);
    }
    Ok(())
}

/// Validates a critic's whole answer and returns its findings.
pub fn parse_document(text: &str) -> Result<Vec<Value>, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    let obj = doc.as_object().ok_or("the answer is not an object")?;
    if let Some(key) = obj
        .keys()
        .find(|k| !["findings", "no_findings_reason"].contains(&k.as_str()))
    {
        return Err(format!("unknown key `{key}`"));
    }
    let findings = obj
        .get("findings")
        .and_then(Value::as_array)
        .ok_or("`findings` is not an array")?;
    if findings.is_empty() {
        match obj.get("no_findings_reason").and_then(Value::as_str) {
            Some(r) if !r.is_empty() => {}
            _ => return Err("no findings and no `no_findings_reason`".to_string()),
        }
    } else if obj
        .get("no_findings_reason")
        .is_some_and(|r| !r.is_string())
    {
        return Err("`no_findings_reason` is not a string".to_string());
    }
    Ok(findings.clone())
}

/// Sorts the findings of each critic answer; `documents[i]` is source `i`.
pub fn check(documents: &[Vec<Value>], diff: &str, ticket: &str) -> Checked {
    let (diff, ticket) = (normalise(diff), normalise(ticket));
    let mut kept = Vec::new();
    let mut hints = Vec::new();
    for (source, findings) in documents.iter().enumerate() {
        for finding in findings {
            match judge(finding, &diff, &ticket) {
                Ok(()) => kept.push(json!({ "source": source, "finding": finding })),
                Err(hint) => hints.push(json!({
                    "source": source,
                    "id": finding.get("id").and_then(Value::as_str),
                    "reason": hint.code(),
                    "detail": hint.detail(),
                    "finding": finding,
                })),
            }
        }
    }
    Checked { kept, hints }
}

/// The report this command writes and prints.
pub fn report(run_id: &str, attempt: u32, checked: &Checked) -> Value {
    json!({
        "run_id": run_id,
        "attempt": attempt,
        "actionable": checked.kept.len(),
        "hint_count": checked.hints.len(),
        "kept": checked.kept,
        "hints": checked.hints,
    })
}

struct Args {
    run_id: String,
    attempt: u32,
    diff: String,
    ticket: String,
    report_dir: String,
    findings: Vec<String>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    if args.first().map(String::as_str) != Some("check") {
        return Err("the one subcommand is `check`".to_string());
    }
    let mut seen: Map<String, Value> = Map::new();
    let mut findings = Vec::new();
    let mut iter = args[1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            flag @ ("--run-id" | "--attempt" | "--diff" | "--ticket" | "--report-dir") => {
                let value = iter.next().ok_or(format!("{flag} needs a value"))?;
                if seen.insert(flag.to_string(), json!(value)).is_some() {
                    return Err(format!("{flag} given twice"));
                }
            }
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}")),
            file => findings.push(file.to_string()),
        }
    }
    let get = |flag: &str| {
        seen.get(flag)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or(format!("{flag} is required"))
    };
    let attempt = get("--attempt")?
        .parse::<u32>()
        .ok()
        .filter(|n| *n >= 1)
        .ok_or("--attempt is not a positive integer")?;
    if findings.is_empty() {
        return Err("at least one findings file is required".to_string());
    }
    let distinct: BTreeSet<&String> = findings.iter().collect();
    if distinct.len() != findings.len() {
        return Err("a findings file is named twice".to_string());
    }
    Ok(Args {
        run_id: get("--run-id").and_then(|s| {
            if s.is_empty() {
                Err("--run-id is empty".to_string())
            } else {
                Ok(s)
            }
        })?,
        attempt,
        diff: get("--diff")?,
        ticket: get("--ticket")?,
        report_dir: get("--report-dir")?,
        findings,
    })
}

fn read_bounded(path: &str) -> Result<String, String> {
    let file = File::open(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read {path}: {e}"))?;
    if bytes.len() as u64 > MAX_INPUT_BYTES {
        return Err(format!("{path} is over {MAX_INPUT_BYTES} bytes"));
    }
    String::from_utf8(bytes).map_err(|_| format!("{path} is not UTF-8"))
}

/// Writes `text` to `dir/REPORT_FILE` through a temp file and a rename.
fn write_report(dir: &Path, text: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{REPORT_FILE}.{}.tmp", std::process::id()));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, dir.join(REPORT_FILE)).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Runs the command line; the output streams are injected for the tests.
pub fn run_with(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let usage = |err: &mut dyn Write, why: &str| {
        let _ = writeln!(err, "{NAME}: usage: {why}");
        2
    };
    let parsed = match parse_args(args) {
        Ok(p) => p,
        Err(why) => return usage(err, &why),
    };
    let read = |path: &String| read_bounded(path);
    let (diff, ticket) = match (read(&parsed.diff), read(&parsed.ticket)) {
        (Ok(d), Ok(t)) => (d, t),
        (Err(why), _) | (_, Err(why)) => return usage(err, &why),
    };
    let mut documents = Vec::new();
    for path in &parsed.findings {
        let text = match read(path) {
            Ok(t) => t,
            Err(why) => return usage(err, &why),
        };
        match parse_document(&text) {
            Ok(findings) => documents.push(findings),
            Err(why) => {
                let _ = writeln!(err, "{NAME}: malformed: {path}: {why}");
                return 1;
            }
        }
    }
    let result = report(
        &parsed.run_id,
        parsed.attempt,
        &check(&documents, &diff, &ticket),
    );
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );
    if let Err(e) = write_report(Path::new(&parsed.report_dir), &text) {
        return usage(err, &format!("cannot write the report: {e}"));
    }
    let _ = out.write_all(text.as_bytes());
    0
}

/// The process entry: real arguments, real streams.
pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    run_with(&args, &mut io::stdout(), &mut io::stderr())
}

#[cfg(test)]
mod tests;
