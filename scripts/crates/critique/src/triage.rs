//! What decides, without any model, whether a ticket's change is risky.

use crate::error::ShapeError;
use crate::fields::Fields;
use model_gateway::json::{self, Value};
use model_gateway::text::{is_js_whitespace, js_trim};

/// The cheap model asked, only when no rule fired, whether the change warrants a review.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageModel {
    /// A Vibe client of routing's `clients`.
    pub client: String,
    pub max_price_usd: f64,
    pub time_limit_s: f64,
}

/// Routing's `triage` as written: the risk rules and the triage model. The rules are repository paths that hold
/// risky code, a size above which any change is risky, and tracker labels that mark a ticket risky.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageRules {
    /// Globs over repository paths, as git prints them.
    pub risk_paths: Vec<String>,
    /// Changed lines (added plus removed) above which a change is risky.
    pub changed_lines_threshold: f64,
    /// Tracker labels, in any case, that make a ticket risky.
    pub labels: Vec<String>,
    pub model: TriageModel,
}

/// Reads routing's `triage`.
pub fn read_triage_rules(fields: &Fields<'_>) -> Result<TriageRules, ShapeError> {
    let model = fields.object("model")?;
    Ok(TriageRules {
        changed_lines_threshold: fields.number("changed_lines_threshold")?,
        labels: fields.strings("labels")?,
        model: TriageModel {
            client: model.string("client")?,
            max_price_usd: model.number("max_price_usd")?,
            time_limit_s: model.number("time_limit_s")?,
        },
        risk_paths: fields.strings("risk_paths")?,
    })
}

/// The risk rules a triage reads, borrowed from routing's `triage`.
#[derive(Clone, Copy, Debug)]
pub struct RiskRules<'a> {
    /// Globs over repository paths, as git prints them.
    pub risk_paths: &'a [String],
    /// Changed lines (added plus removed) above which a change is risky.
    pub changed_lines_threshold: f64,
    /// Tracker labels, in any case, that make a ticket risky.
    pub labels: &'a [String],
}

/// What a change amounts to, as the risk rules read it.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeChange {
    /// Every repository path the change touches, deleted files included.
    pub paths: Vec<String>,
    /// Lines added plus lines removed.
    pub changed_lines: f64,
}

/// The change a diff and its touched paths describe.
pub fn code_change(diff: &str, paths: &[String]) -> CodeChange {
    CodeChange { changed_lines: count_changed_lines(diff), paths: paths.to_vec() }
}

/// A rule that fired.
#[derive(Clone, Debug, PartialEq)]
pub enum RiskSignal {
    RiskPath { glob: String, path: String },
    Size { changed_lines: f64, threshold: f64 },
    Label { label: String },
}

/// The first of the globs the path matches, tried in order; a glob JavaScript cannot build a pattern from is an `Err`
/// when it is reached.
fn first_match<'a>(path: &str, globs: &'a [String]) -> Result<Option<&'a String>, String> {
    for glob in globs {
        if matches_glob(path, glob)? {
            return Ok(Some(glob));
        }
    }
    Ok(None)
}

/// Every rule the change and the ticket's labels fire, in a fixed order: each touched path that matches a risk
/// path (with the first glob it matches), then the size, then each risk label. A glob JavaScript cannot build a
/// pattern from is an `Err` with its message, as the node runner throws it.
pub fn risk_signals(change: &CodeChange, labels: &[String], rules: &RiskRules<'_>) -> Result<Vec<RiskSignal>, String> {
    let carried: Vec<String> = labels.iter().map(|label| label.to_lowercase()).collect();
    let mut signals: Vec<RiskSignal> = Vec::new();
    for path in &change.paths {
        if let Some(glob) = first_match(path, rules.risk_paths)? {
            signals.push(RiskSignal::RiskPath { glob: glob.clone(), path: path.clone() });
        }
    }
    if change.changed_lines > rules.changed_lines_threshold {
        signals.push(RiskSignal::Size { changed_lines: change.changed_lines, threshold: rules.changed_lines_threshold });
    }
    for label in rules.labels {
        if carried.contains(&label.to_lowercase()) {
            signals.push(RiskSignal::Label { label: label.clone() });
        }
    }
    Ok(signals)
}

/// How many touched paths match a risk path.
pub fn risk_path_hits(change: &CodeChange, risk_paths: &[String]) -> Result<usize, String> {
    let mut hits = 0;
    for path in &change.paths {
        if first_match(path, risk_paths)?.is_some() {
            hits += 1;
        }
    }
    Ok(hits)
}

/// Whether a repository path matches a glob. `**` stands for any number of whole path segments (none
/// included), `*` for any characters but `/`, `?` for one character but `/`, and `{a,b}` for either
/// alternative. The glob must match the whole path, case-sensitively, as git prints it.
pub fn matches_glob(path: &str, glob: &str) -> Result<bool, String> {
    let (_, tokens) = glob_tokens(glob)?;
    let units: Vec<u16> = path.encode_utf16().collect();
    let mut starts = vec![false; units.len() + 1];
    starts[0] = true;
    Ok(match_tokens(&tokens, &units, &starts)[units.len()])
}

/// One piece of a glob's pattern, as the matcher walks it.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    /// One UTF-16 unit of a literal character.
    Literal(u16),
    /// `.*`: any run without a line terminator.
    Any,
    /// `[^/]*`: any run without a slash.
    AnySegment,
    /// `[^/]`: one unit that is not a slash.
    OneSegment,
    /// `(?:.*/)?`: an optional run of whole segments.
    OptionalDirPrefix,
    /// `(?:/.*)?`: an optional trailing run of segments.
    OptionalDirSuffix,
    /// `(?:a|b)`: one of the alternatives.
    Group(Vec<Vec<Token>>),
}

/// A `{a,b}` group being read: the alternatives closed so far and the one still open.
#[derive(Default)]
struct GroupBuilder {
    done: Vec<Vec<Token>>,
    current: Vec<Token>,
}

impl GroupBuilder {
    /// Closes the open alternative and starts the next.
    fn split(&mut self) {
        self.done.push(std::mem::take(&mut self.current));
    }

    /// The group's alternatives, the open one last.
    fn finish(mut self) -> Vec<Vec<Token>> {
        self.split();
        self.done
    }
}

/// The glob as the pattern the TypeScript builds, and the tokens that match it. The pattern is only
/// needed for the error message an unclosed `{` produces.
fn glob_tokens(glob: &str) -> Result<(String, Vec<Token>), String> {
    let chars: Vec<char> = glob.chars().collect();
    let mut pattern = String::new();
    let mut tokens: Vec<Token> = Vec::new();
    let mut groups: Vec<GroupBuilder> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let rest = &chars[i..];
        if rest.starts_with(&['*', '*', '/']) {
            pattern.push_str("(?:.*/)?");
            push_token(&mut groups, &mut tokens, Token::OptionalDirPrefix);
            i += 3;
        } else if rest.starts_with(&['/', '*', '*']) && i + 3 == chars.len() {
            pattern.push_str("(?:/.*)?");
            push_token(&mut groups, &mut tokens, Token::OptionalDirSuffix);
            i += 3;
        } else if rest.starts_with(&['*', '*']) {
            pattern.push_str(".*");
            push_token(&mut groups, &mut tokens, Token::Any);
            i += 2;
        } else if chars[i] == '*' {
            pattern.push_str("[^/]*");
            push_token(&mut groups, &mut tokens, Token::AnySegment);
            i += 1;
        } else if chars[i] == '?' {
            pattern.push_str("[^/]");
            push_token(&mut groups, &mut tokens, Token::OneSegment);
            i += 1;
        } else if chars[i] == '{' {
            pattern.push_str("(?:");
            groups.push(GroupBuilder::default());
            i += 1;
        } else if chars[i] == '}' && !groups.is_empty() {
            pattern.push(')');
            if let Some(group) = groups.pop() {
                push_token(&mut groups, &mut tokens, Token::Group(group.finish()));
            }
            i += 1;
        } else if chars[i] == ',' && !groups.is_empty() {
            pattern.push('|');
            if let Some(group) = groups.last_mut() {
                group.split();
            }
            i += 1;
        } else {
            if is_pattern_special(chars[i]) {
                pattern.push('\\');
            }
            pattern.push(chars[i]);
            let mut units = [0u16; 2];
            for unit in chars[i].encode_utf16(&mut units) {
                push_token(&mut groups, &mut tokens, Token::Literal(*unit));
            }
            i += 1;
        }
    }
    if groups.is_empty() {
        Ok((pattern, tokens))
    } else {
        Err(format!("Invalid regular expression: /^{pattern}$/: Unterminated group"))
    }
}

/// Adds a token to the innermost open alternative, or to the pattern when no group is open.
fn push_token(groups: &mut [GroupBuilder], tokens: &mut Vec<Token>, token: Token) {
    match groups.last_mut() {
        Some(group) => group.current.push(token),
        None => tokens.push(token),
    }
}

/// Whether the TypeScript escapes the character in the pattern: `/[.+^${}()|[\]\\]/`.
fn is_pattern_special(c: char) -> bool {
    matches!(c, '.' | '+' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\')
}

/// Whether a UTF-16 unit is one of JavaScript's line terminators, which `.` does not match.
fn is_line_terminator(unit: u16) -> bool {
    matches!(unit, 0x0a | 0x0d | 0x2028 | 0x2029)
}

const SLASH: u16 = b'/' as u16;

/// The positions `token` can reach from `from`, marked in `out`.
fn advance(token: &Token, units: &[u16], from: usize, out: &mut [bool]) {
    let length = units.len();
    match token {
        Token::Literal(unit) => {
            if from < length && units[from] == *unit {
                out[from + 1] = true;
            }
        }
        Token::OneSegment => {
            if from < length && units[from] != SLASH {
                out[from + 1] = true;
            }
        }
        Token::Any => {
            out[from] = true;
            let mut at = from;
            while at < length && !is_line_terminator(units[at]) {
                at += 1;
                out[at] = true;
            }
        }
        Token::AnySegment => {
            out[from] = true;
            let mut at = from;
            while at < length && units[at] != SLASH {
                at += 1;
                out[at] = true;
            }
        }
        Token::OptionalDirPrefix => {
            out[from] = true;
            let mut at = from;
            while at < length && !is_line_terminator(units[at]) {
                if units[at] == SLASH {
                    out[at + 1] = true;
                }
                at += 1;
            }
        }
        Token::OptionalDirSuffix => {
            out[from] = true;
            if from < length && units[from] == SLASH {
                let mut at = from + 1;
                out[at] = true;
                while at < length && !is_line_terminator(units[at]) {
                    at += 1;
                    out[at] = true;
                }
            }
        }
        Token::Group(alternatives) => {
            for alternative in alternatives {
                let mut starts = vec![false; length + 1];
                starts[from] = true;
                for (index, reached) in match_tokens(alternative, units, &starts).iter().enumerate() {
                    if *reached {
                        out[index] = true;
                    }
                }
            }
        }
    }
}

/// The positions the token sequence can end at, from the positions `starts` marks.
fn match_tokens(tokens: &[Token], units: &[u16], starts: &[bool]) -> Vec<bool> {
    let mut current = starts.to_vec();
    for token in tokens {
        let mut next = vec![false; units.len() + 1];
        for (from, reached) in current.iter().enumerate() {
            if *reached {
                advance(token, units, from, &mut next);
            }
        }
        current = next;
    }
    current
}

/// Added plus removed lines of a unified diff. A file's header (from its `diff --git` line to its `+++`
/// line or first hunk) is not counted, so a removed line that reads `-- x` is still one line.
pub fn count_changed_lines(diff: &str) -> f64 {
    let mut header = false;
    let mut count = 0.0;
    for line in diff.split('\n') {
        if line.starts_with("diff --git ") {
            header = true;
        } else if header && (line.starts_with("+++ ") || line.starts_with("@@")) {
            header = false;
        } else if !header && (line.starts_with('+') || line.starts_with('-')) {
            count += 1.0;
        }
    }
    count
}

/// The triage model's answer, read as yes or no: one word, in any case, with surrounding markup or a
/// full stop allowed. Anything else is no answer.
pub fn read_triage_answer(text: &str) -> Option<&'static str> {
    let lowered = js_trim(text).to_lowercase();
    let word = lowered.trim_start_matches(is_answer_lead).trim_end_matches(is_answer_tail);
    match word {
        "yes" => Some("yes"),
        "no" => Some("no"),
        _ => None,
    }
}

/// Whether the answer's leading markup may be stripped: JavaScript white space and `* _ ` " '`.
fn is_answer_lead(c: char) -> bool {
    is_js_whitespace(c) || matches!(c, '*' | '_' | '`' | '"' | '\'')
}

/// Whether the answer's trailing markup may be stripped: the leading set and a full stop or bang.
fn is_answer_tail(c: char) -> bool {
    is_answer_lead(c) || matches!(c, '.' | '!')
}

/// What triage decides: a risky change is reviewed, a low-risk one skips the review.
pub const RISKY: &str = "risky";
pub const LOW_RISK: &str = "low-risk";

/// Why the triage model gave no answer, so the change stays risky.
pub const NOT_OPTED_IN: &str = "not-opted-in";
pub const CODE_NOT_ALLOWED: &str = "code-not-allowed";
pub const FAMILY_NOT_ALLOWED: &str = "family-not-allowed";
pub const BUDGET: &str = "budget";
pub const UNAVAILABLE: &str = "unavailable";
pub const FAILED: &str = "failed";
pub const UNCLEAR_ANSWER: &str = "unclear-answer";

/// The signal that decided a triage: a risk rule, the model's answer, or the model's absence.
#[derive(Clone, Debug, PartialEq)]
pub enum TriageSignal {
    Risk(RiskSignal),
    Model { answer: &'static str },
    ModelUnavailable { reason: &'static str, detail: String },
}

/// One line naming the decision and the signal behind it.
pub fn triage_summary(decision: &str, signal: &TriageSignal, client: &str) -> String {
    let model = format!("no risk rule fired, and the triage model ({client})");
    match signal {
        TriageSignal::Risk(RiskSignal::RiskPath { glob, path }) => {
            format!("Triage: risky: {path} matches the risk path {glob}.")
        }
        TriageSignal::Risk(RiskSignal::Size { changed_lines, threshold }) => format!(
            "Triage: risky: {} changed lines, above the threshold of {}.",
            json::number_text(*changed_lines),
            json::number_text(*threshold)
        ),
        TriageSignal::Risk(RiskSignal::Label { label }) => {
            format!("Triage: risky: the ticket carries the label {label}.")
        }
        TriageSignal::Model { .. } => {
            if decision == RISKY {
                format!("Triage: risky: {model} answered yes.")
            } else {
                format!("Triage: low-risk: {model} answered no, so the review is skipped.")
            }
        }
        TriageSignal::ModelUnavailable { detail, .. } => {
            format!("Triage: risky: {model} could not answer: {detail}.")
        }
    }
}

/// The triage model a record names, and what it answered.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordModel {
    pub client: String,
    pub model: String,
    pub cost_usd: Option<f64>,
    pub charged_usd: f64,
    pub answer: Option<&'static str>,
}

/// What a triage's ledger line is made from.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageRecord {
    pub status: String,
    pub recorded_at: String,
    pub repository: String,
    pub flow: String,
    pub run_id: String,
    /// The ticket's reference.
    pub ticket: String,
    pub signal: TriageSignal,
    pub signals: Vec<RiskSignal>,
    pub changed_files: f64,
    pub changed_lines: f64,
    pub changed_lines_threshold: f64,
    pub model: Option<RecordModel>,
    /// The hash of the triage prompt, when the model was asked.
    pub prompt_hash: Option<String>,
}

/// A triage's ledger line. A low-risk change is a ticket-review run that never ran, so it is a `run` line
/// with status `skipped-low-risk`; a risky one is a `triage` line, since the review that follows has run
/// lines of its own. Metadata only: a risk path hit keeps the glob, never the path, and an unavailable
/// model keeps its reason, never its detail.
pub fn triage_ledger_line(record: &TriageRecord) -> Value {
    let low_risk = record.status == LOW_RISK;
    let model = record.model.as_ref();
    json::object(vec![
        ("type", Value::str(if low_risk { "run" } else { "triage" })),
        ("recorded_at", Value::str(&record.recorded_at)),
        ("repository", Value::str(&record.repository)),
        ("flow", Value::str(&record.flow)),
        ("run_id", Value::str(&record.run_id)),
        ("stage", Value::str("ticket-review")),
        ("status", Value::str(if low_risk { "skipped-low-risk" } else { "risky" })),
        ("ticket", Value::str(&record.ticket)),
        ("triage_signal", ledger_signal(&record.signal)),
        ("risk_signals", Value::Arr(record.signals.iter().map(|signal| ledger_signal(&TriageSignal::Risk(signal.clone()))).collect())),
        ("changed_files", Value::Num(record.changed_files)),
        ("changed_lines", Value::Num(record.changed_lines)),
        ("changed_lines_threshold", Value::Num(record.changed_lines_threshold)),
        ("client", model.map_or(Value::Null, |named| Value::str(&named.client))),
        ("model", model.map_or(Value::Null, |named| Value::str(&named.model))),
        ("answer", model.and_then(|named| named.answer).map_or(Value::Null, Value::str)),
        ("cost_usd", model.and_then(|named| named.cost_usd).map_or(Value::Null, Value::Num)),
        ("charged_usd", model.map_or(Value::Num(0.0), |named| Value::Num(named.charged_usd))),
        ("prompt_hash", record.prompt_hash.as_deref().map_or(Value::Null, Value::str)),
    ])
}

/// The signal as a ledger line keeps it: a risk path hit loses its path, an unavailable model its detail.
fn ledger_signal(signal: &TriageSignal) -> Value {
    match signal {
        TriageSignal::Risk(RiskSignal::RiskPath { glob, .. }) => {
            json::object(vec![("glob", Value::str(glob)), ("kind", Value::str("risk-path"))])
        }
        TriageSignal::ModelUnavailable { reason, .. } => json::object(vec![
            ("kind", Value::str("model-unavailable")),
            ("reason", Value::str(reason)),
        ]),
        other => signal_json(other),
    }
}

/// The whole signal, as `triage.json` writes it: the object literals the TypeScript builds, in their order.
pub fn signal_json(signal: &TriageSignal) -> Value {
    match signal {
        TriageSignal::Risk(RiskSignal::RiskPath { glob, path }) => json::object(vec![
            ("glob", Value::str(glob)),
            ("kind", Value::str("risk-path")),
            ("path", Value::str(path)),
        ]),
        TriageSignal::Risk(RiskSignal::Size { changed_lines, threshold }) => json::object(vec![
            ("changed_lines", Value::Num(*changed_lines)),
            ("kind", Value::str("size")),
            ("threshold", Value::Num(*threshold)),
        ]),
        TriageSignal::Risk(RiskSignal::Label { label }) => {
            json::object(vec![("kind", Value::str("label")), ("label", Value::str(label))])
        }
        TriageSignal::Model { answer } => {
            json::object(vec![("answer", Value::str(answer)), ("kind", Value::str("model"))])
        }
        TriageSignal::ModelUnavailable { reason, detail } => json::object(vec![
            ("detail", Value::str(detail)),
            ("kind", Value::str("model-unavailable")),
            ("reason", Value::str(reason)),
        ]),
    }
}
