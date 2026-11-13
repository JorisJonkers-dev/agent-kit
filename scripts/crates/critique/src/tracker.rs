//! Where a repository keeps its specs and tickets, read the way Pocock's
//! `setup-matt-pocock-skills` records it, and how one spec reference names an
//! artifact there.

use crate::manifest::TRACKER_KINDS;
use model_gateway::text::{is_js_whitespace, js_trim};

/// The doc Pocock's setup writes to say where a repository's issues live.
pub const TRACKER_DOC: &str = "docs/agents/issue-tracker.md";

/// The tracker a repository's doc names, or the name the critique cannot read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackerChoice {
    Known { kind: &'static str },
    Unsupported { name: String },
}

/// Reads the tracker from the doc's `# Issue tracker: <name>` heading. A tracker the critique
/// cannot read (GitLab, or the free-form description Pocock's setup writes for any other) is
/// unsupported, and so is a doc with no such heading.
pub fn tracker_from_doc(text: &str) -> TrackerChoice {
    let name = tracker_name(text);
    let normalised = name.as_deref().map(str::to_lowercase);
    match normalised.as_deref() {
        Some("github") => TrackerChoice::Known { kind: TRACKER_KINDS[0] },
        Some("local markdown") => TrackerChoice::Known { kind: TRACKER_KINDS[1] },
        _ => TrackerChoice::Unsupported { name: name.unwrap_or_else(|| "no tracker".to_string()) },
    }
}

/// With no tracker doc, a repository whose allowlist key is on github.com tracks on GitHub; any
/// other on local markdown.
pub fn default_tracker(repository_key: &str) -> &'static str {
    if repository_key.starts_with("github.com/") {
        TRACKER_KINDS[0]
    } else {
        TRACKER_KINDS[1]
    }
}

/// The issue's repository as `host/owner/repo`, the form of a family allowlist key, and its number.
#[derive(Clone, Debug, PartialEq)]
pub struct IssueReference {
    pub repository: String,
    pub number: f64,
}

/// Reads `167`, `#167`, `owner/repo#167`, `host/owner/repo#167` (the form a manifest records) or an
/// issue URL. A bare number names an issue of `repository_key`. Anything else is not an issue
/// reference.
pub fn parse_issue_reference(reference: &str, repository_key: &str) -> Option<IssueReference> {
    let text = js_trim(reference);
    if let Some(number) = local_number(text) {
        return Some(IssueReference { repository: repository_key.to_string(), number });
    }
    if let Some((owner, repo, number)) = short_reference(text) {
        return Some(IssueReference { repository: key("github.com", &owner, &repo), number });
    }
    if let Some((host, owner, repo, number)) = full_reference(text) {
        return Some(IssueReference { repository: key(&host, &owner, &repo), number });
    }
    url_reference(text)
}

/// The issue as the packet holds it: its title as a heading, then its body, then each comment under
/// `## Comments`. Line endings become `\n`, so a quote a critic copies from the packet matches it.
pub fn render_issue(issue: &IssueText) -> String {
    let body = js_trim(&issue.body);
    let mut parts = vec![format!("# {}", issue.title)];
    if !body.is_empty() {
        parts.push(body.to_string());
    }
    if !issue.comments.is_empty() {
        parts.push("## Comments".to_string());
        for comment in &issue.comments {
            parts.push(format!(
                "### {}, {}\n\n{}",
                comment.author,
                comment.created_at,
                js_trim(&comment.body)
            ));
        }
    }
    let joined = parts.join("\n\n");
    let mut out = String::with_capacity(joined.len() + 1);
    let mut chars = joined.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out.push('\n');
    out
}

/// An issue's text as the packet holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueText {
    pub title: String,
    pub body: String,
    pub comments: Vec<IssueComment>,
}

/// One comment on an issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueComment {
    pub author: String,
    pub created_at: String,
    pub body: String,
}

/// The `# Issue tracker: <name>` heading's name, if the doc has one.
fn tracker_name(text: &str) -> Option<String> {
    let mut start = 0;
    loop {
        if let Some(name) = tracker_name_at(text, start) {
            return Some(name);
        }
        start += line_terminator_len(&text[start..])?;
    }
}

/// The heading at a line start: `#\s+Issue tracker:\s*(.+?)\s*$`, with the engine's backtracking.
fn tracker_name_at(text: &str, start: usize) -> Option<String> {
    let rest = &text[start..];
    if !rest.starts_with('#') {
        return None;
    }
    let after_hash = skip_whitespace(rest, 1);
    if after_hash == 1 {
        return None;
    }
    let after_literal = match_literal(rest, after_hash, "Issue tracker:")?;
    let run_start = after_literal;
    let run_end = skip_whitespace(rest, run_start);
    let mut capture_start = run_end;
    loop {
        if let Some(name) = capture_from(rest, capture_start) {
            return Some(name);
        }
        if capture_start == run_start {
            return None;
        }
        capture_start = previous_boundary(rest, capture_start);
    }
}

/// `(.+?)\s*$` from `at`: the shortest capture whose tail reaches a line end.
fn capture_from(rest: &str, at: usize) -> Option<String> {
    let mut end = at;
    for c in rest[at..].chars() {
        if is_line_terminator(c) {
            break;
        }
        end += c.len_utf8();
        if ends_line(rest, end) {
            return Some(rest[at..end].to_string());
        }
    }
    None
}

/// `\s*$` from `at`: whether a run of white space reaches the end of the text or a line terminator.
fn ends_line(rest: &str, at: usize) -> bool {
    let mut end = skip_whitespace(rest, at);
    loop {
        if end == rest.len() || rest[end..].chars().next().map_or(false, is_line_terminator) {
            return true;
        }
        if end == at {
            return false;
        }
        end = previous_boundary(rest, end);
    }
}

/// The bytes from the start of `text` to just past its first line terminator, if it has one.
fn line_terminator_len(text: &str) -> Option<usize> {
    let mut offset = 0;
    for c in text.chars() {
        offset += c.len_utf8();
        if is_line_terminator(c) {
            return Some(offset);
        }
    }
    None
}

/// Whether JavaScript's `.` and `$` count the character as a line terminator.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// The position after the run of JavaScript white space that starts at `at`.
fn skip_whitespace(text: &str, at: usize) -> usize {
    let mut end = at;
    for c in text[at..].chars() {
        if !is_js_whitespace(c) {
            break;
        }
        end += c.len_utf8();
    }
    end
}

/// The start of the character before the one at `at`.
fn previous_boundary(text: &str, at: usize) -> usize {
    let mut index = at - 1;
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// The position after `literal` at `at`, folding ASCII case, if it is there.
fn match_literal(text: &str, at: usize, literal: &str) -> Option<usize> {
    let mut index = at;
    for expected in literal.chars() {
        let actual = text[index..].chars().next()?;
        if !actual.eq_ignore_ascii_case(&expected) {
            return None;
        }
        index += actual.len_utf8();
    }
    Some(index)
}

/// `^#?(\d+)$`.
fn local_number(text: &str) -> Option<f64> {
    let rest = text.strip_prefix('#').unwrap_or(text);
    let (number, at) = scan_digits(rest, 0)?;
    if at != rest.len() {
        return None;
    }
    Some(number)
}

/// `^([\w.-]+)\/([\w.-]+)#(\d+)$`.
fn short_reference(text: &str) -> Option<(String, String, f64)> {
    let (owner, at) = scan_word(text, 0)?;
    let at = match_literal(text, at, "/")?;
    let (repo, at) = scan_word(text, at)?;
    let at = match_literal(text, at, "#")?;
    let (number, at) = scan_digits(text, at)?;
    if at != text.len() {
        return None;
    }
    Some((owner, repo, number))
}

/// `^([\w.-]+)\/([\w.-]+)\/([\w.-]+)#(\d+)$`.
fn full_reference(text: &str) -> Option<(String, String, String, f64)> {
    let (host, at) = scan_word(text, 0)?;
    let at = match_literal(text, at, "/")?;
    let (owner, at) = scan_word(text, at)?;
    let at = match_literal(text, at, "/")?;
    let (repo, at) = scan_word(text, at)?;
    let at = match_literal(text, at, "#")?;
    let (number, at) = scan_digits(text, at)?;
    if at != text.len() {
        return None;
    }
    Some((host, owner, repo, number))
}

/// `^https?:\/\/([^/\s]+)\/([^/\s]+)\/([^/\s]+)\/issues\/(\d+)(?:[/?#]\S*)?$`, folding ASCII case.
fn url_reference(text: &str) -> Option<IssueReference> {
    let mut at = match_literal(text, 0, "http")?;
    if text[at..].chars().next().map_or(false, |c| c.eq_ignore_ascii_case(&'s')) {
        at += 1;
    }
    at = match_literal(text, at, "://")?;
    let (host, at) = scan_url_segment(text, at)?;
    let at = match_literal(text, at, "/")?;
    let (owner, at) = scan_url_segment(text, at)?;
    let at = match_literal(text, at, "/")?;
    let (repo, at) = scan_url_segment(text, at)?;
    let at = match_literal(text, at, "/issues/")?;
    let (number, at) = scan_digits(text, at)?;
    if !url_tail_ok(text, at) {
        return None;
    }
    Some(IssueReference { repository: key(&host, &owner, &repo), number })
}

/// `(?:[/?#]\S*)?$`: nothing, or a `/`, `?` or `#` and then non-white-space to the end.
fn url_tail_ok(text: &str, at: usize) -> bool {
    let mut chars = text[at..].chars();
    match chars.next() {
        None => true,
        Some(first) => matches!(first, '/' | '?' | '#') && chars.all(|c| !is_js_whitespace(c)),
    }
}

/// One or more `[\w.-]` characters, ASCII only.
fn scan_word(text: &str, at: usize) -> Option<(String, usize)> {
    let mut end = at;
    for c in text[at..].chars() {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-') {
            break;
        }
        end += 1;
    }
    if end == at {
        None
    } else {
        Some((text[at..end].to_string(), end))
    }
}

/// One or more characters that are neither `/` nor JavaScript white space.
fn scan_url_segment(text: &str, at: usize) -> Option<(String, usize)> {
    let mut end = at;
    for c in text[at..].chars() {
        if c == '/' || is_js_whitespace(c) {
            break;
        }
        end += c.len_utf8();
    }
    if end == at {
        None
    } else {
        Some((text[at..end].to_string(), end))
    }
}

/// One or more ASCII digits, as `Number` reads them.
fn scan_digits(text: &str, at: usize) -> Option<(f64, usize)> {
    let mut end = at;
    for c in text[at..].chars() {
        if !c.is_ascii_digit() {
            break;
        }
        end += 1;
    }
    if end == at {
        return None;
    }
    text[at..end].parse::<f64>().ok().map(|number| (number, end))
}

/// `${host}/${owner}/${repo}` lowercased.
fn key(host: &str, owner: &str, repo: &str) -> String {
    format!("{host}/{owner}/{repo}").to_lowercase()
}
