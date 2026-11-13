//! The code a decision record names in backticks, and whether the tree still
//! holds it: the node runner's `namedCode`, `namedBy` and `driftAndStale`, with
//! their regular expressions written out by hand.

use crate::decision_docs::{decision_date, without_fences};
use model_gateway::text::js_trim;
use std::collections::{HashMap, HashSet};

/// The extensions that mark a backticked word as a path: `\.(?:ts|tsx|...)$` under `/i`.
const KNOWN_EXTENSIONS: [&str; 20] =
    ["ts", "tsx", "js", "mjs", "cjs", "json", "yaml", "yml", "toml", "md", "py", "sh", "css", "html", "sql", "lock", "kt", "java", "go", "rs"];

/// The code, configuration and commands a record names in backticks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NamedCode {
    /// Repository paths, without a leading `./` or a trailing `/`.
    pub paths: Vec<String>,
    pub symbols: Vec<String>,
}

/// A record names code that is gone: a path missing from the working tree, or a symbol no file of it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drift {
    pub record: String,
    /// The path or the symbol, as the record writes it.
    pub named: String,
    pub reason: &'static str,
}

/// A record's decision is older than the last change to a file it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stale {
    pub record: String,
    pub path: String,
    /// The day the record states its decision was made, `YYYY-MM-DD`.
    pub decision_date: String,
    /// The date of the last commit that changed the path, ISO 8601.
    pub last_change: String,
    pub last_commit: String,
}

/// The last commit that changed a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub commit: String,
    pub date: String,
}

/// What the working tree holds, as the drift and stale checks ask for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeFacts {
    /// The repository paths that exist in the working tree.
    pub existing: HashSet<String>,
    /// The last commit that changed an existing path; absent for a path no commit touched.
    pub changes: HashMap<String, Change>,
    /// The symbols that appear as a whole word in a file of the tree outside the decision log.
    pub present: HashSet<String>,
    /// Whether the search could not read every file, so an absent symbol proves nothing.
    pub incomplete: bool,
}

impl CodeFacts {
    pub fn path_exists(&self, path: &str) -> bool {
        self.existing.contains(path)
    }

    pub fn symbol_exists(&self, symbol: &str) -> bool {
        self.incomplete || self.present.contains(symbol)
    }

    pub fn last_change(&self, path: &str) -> Option<&Change> {
        self.changes.get(path)
    }
}

/// One record to check: its path, its text, and the code it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeChecked {
    pub path: String,
    pub text: String,
    pub code: NamedCode,
}

/// The paths and symbols a record names inside backticks, outside fenced blocks, in order and without repeats.
pub fn named_code(markdown: &str) -> NamedCode {
    let prose = without_fences(markdown);
    let mut paths: Vec<String> = Vec::new();
    let mut symbols: Vec<String> = Vec::new();
    for span in backtick_spans(&prose) {
        let (path_part, symbol_part) = split_anchor(js_trim(span));
        if is_path(path_part) {
            push_unique(&mut paths, strip_trailing_slashes(strip_dot_slash(path_part)));
            if let Some(anchor) = symbol_part.map(strip_call).filter(|anchor| is_identifier(*anchor)) {
                push_unique(&mut symbols, anchor);
            }
        } else if symbol_part.is_none() && is_symbol(path_part) {
            push_unique(&mut symbols, strip_call(path_part));
        }
    }
    NamedCode { paths, symbols }
}

/// The code each record names, which the caller resolves against the tree before `drift_and_stale` judges it.
pub fn named_by(records: &[(String, String)]) -> Vec<CodeChecked> {
    records.iter().map(|(path, text)| CodeChecked { path: path.clone(), text: text.clone(), code: named_code(text) }).collect()
}

/// Drift: a record in force names a path or a symbol the tree no longer holds. Stale: it states a
/// decision date, and a path it names changed on a later day. A record that names no code is never flagged.
pub fn drift_and_stale(records: &[CodeChecked], facts: &CodeFacts) -> (Vec<Drift>, Vec<Stale>) {
    let mut drifted = Vec::new();
    let mut stale = Vec::new();
    for record in records {
        let missing: Vec<&str> = record.code.paths.iter().filter(|path| !facts.path_exists(path.as_str())).map(String::as_str).collect();
        for named in &missing {
            drifted.push(Drift { record: record.path.clone(), named: (*named).to_string(), reason: "missing-path" });
        }
        for named in &record.code.symbols {
            if !facts.symbol_exists(named) {
                drifted.push(Drift { record: record.path.clone(), named: named.clone(), reason: "missing-symbol" });
            }
        }
        let Some(decided) = decision_date(&record.text) else {
            continue;
        };
        let log_dir = record.path.rfind('/').map_or("", |at| &record.path[..=at]);
        // A record naming another decision (or itself) is not naming code: editing a record never makes it stale.
        for path in &record.code.paths {
            if missing.contains(&path.as_str()) || (!log_dir.is_empty() && path.starts_with(log_dir)) {
                continue;
            }
            let Some(change) = facts.last_change(path) else {
                continue;
            };
            if day(&change.date) > decided.as_str() {
                stale.push(Stale {
                    record: record.path.clone(),
                    path: path.clone(),
                    decision_date: decided.clone(),
                    last_change: change.date.clone(),
                    last_commit: change.commit.clone(),
                });
            }
        }
    }
    (drifted, stale)
}

/// `/`([^`\n]+)`/g`: the text inside each inline code span, left to right and without overlap.
fn backtick_spans(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(open) = text[at..].find('`') {
        let start = at + open + 1;
        let rest = &text[start..];
        match rest.find(|c: char| c == '`' || c == '\n') {
            Some(end) if end > 0 && rest.as_bytes()[end] == b'`' => {
                spans.push(&rest[..end]);
                at = start + end + 1;
            }
            _ => at = start,
        }
    }
    spans
}

/// `text.split('#')`'s first two pieces: the path part, and the symbol part when there is a `#`.
fn split_anchor(text: &str) -> (&str, Option<&str>) {
    let Some((path, rest)) = text.split_once('#') else {
        return (text, None);
    };
    (path, Some(rest.split_once('#').map_or(rest, |(head, _)| head)))
}

/// A path has a known extension, a trailing `/` or a leading `./`: a bare `yes/no` is prose, not a path.
fn is_path(text: &str) -> bool {
    let path = strip_dot_slash(text);
    let marked = path.len() != text.len() || path.ends_with('/') || has_known_extension(path);
    marked && path_shape(path) && !path.split('/').any(|segment| segment == "..")
}

/// `^[\w@.-]+(?:\/[\w@.-]+)*\/?$`: segments of word characters, `@`, `.` and `-`, with an optional trailing `/`.
fn path_shape(text: &str) -> bool {
    let trimmed = text.strip_suffix('/').unwrap_or(text);
    !trimmed.is_empty() && trimmed.split('/').all(|segment| !segment.is_empty() && segment.chars().all(is_path_char))
}

/// `[\w@.-]`: an ASCII word character, `@`, `.` or `-`.
fn is_path_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '.' | '-')
}

/// `\.(?:ts|tsx|...)$` under `/i`: the text ends with a known extension.
fn has_known_extension(text: &str) -> bool {
    text.rsplit_once('.').is_some_and(|(_, extension)| KNOWN_EXTENSIONS.iter().any(|known| extension.eq_ignore_ascii_case(known)))
}

/// An identifier with an internal hump, with or without `()`: a bare `fetch()` or `max_tokens` is prose.
fn is_symbol(text: &str) -> bool {
    let name = strip_call(text);
    is_identifier(name) && name.as_bytes().windows(2).any(|pair| (pair[0].is_ascii_lowercase() || pair[0].is_ascii_digit()) && pair[1].is_ascii_uppercase())
}

/// `^[A-Za-z_]\w*$`: an ASCII identifier.
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `/^\.\//`: one leading `./`.
fn strip_dot_slash(text: &str) -> &str {
    text.strip_prefix("./").unwrap_or(text)
}

/// `/\/+$/`: every trailing `/`.
fn strip_trailing_slashes(text: &str) -> &str {
    text.trim_end_matches('/')
}

/// `/\(\)$/`: one trailing `()`.
fn strip_call(text: &str) -> &str {
    text.strip_suffix("()").unwrap_or(text)
}

/// `date.slice(0, 10)`: the day part of an ISO 8601 date.
fn day(date: &str) -> &str {
    date.get(..10).unwrap_or(date)
}

/// Adds `value` unless it is already there: a `Set` keeps insertion order.
fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|seen| seen.as_str() == value) {
        values.push(value.to_string());
    }
}
