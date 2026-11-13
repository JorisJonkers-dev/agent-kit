//! Which decision docs a spec is checked against, and what a record says about
//! itself: its status and the day it was decided. The node runner reads them
//! with regular expressions; the status and date readers are those expressions
//! written out by hand, and the path reader runs its expressions on the crate's
//! JavaScript-semantics engine.

use crate::js_regex::Regex;
use model_gateway::text::{is_js_whitespace, js_trim};

/// Where the ADRs live when there is no domain doc.
pub const ADR_DIRECTORY: &str = "docs/adr";

/// The statuses that take a record out of decision evidence. No status, or any other, counts as accepted.
const NOT_IN_FORCE: [&str; 4] = ["proposed", "rejected", "superseded", "deprecated"];

/// Whether JavaScript's `/m` counts the character as a line terminator: `^` and `$` match around it.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Every position a `/m` `^` matches at: the start of the text and the position after each line terminator.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.char_indices().filter(|(_, c)| is_line_terminator(*c)).map(|(at, c)| at + c.len_utf8()));
    starts
}

/// Markdown with its fenced blocks taken out: `/^(```|~~~)[^\n]*\n[\s\S]*?^\1[^\n]*$/gm`.
pub fn without_fences(markdown: &str) -> String {
    let starts = line_starts(markdown);
    let mut out = String::with_capacity(markdown.len());
    let mut copied = 0;
    let mut index = 0;
    while index < starts.len() {
        match fence_end(markdown, &starts, starts[index]) {
            Some(end) => {
                out.push_str(&markdown[copied..starts[index]]);
                copied = end;
                index = starts.partition_point(|&at| at < end);
            }
            None => index += 1,
        }
    }
    out.push_str(&markdown[copied..]);
    out
}

/// Where the fenced block that opens at the line start `start` ends, if it closes: the position of the
/// closing line's terminator, which the match leaves in place.
fn fence_end(text: &str, starts: &[usize], start: usize) -> Option<usize> {
    let rest = &text[start..];
    let marker = if rest.starts_with("```") {
        "```"
    } else if rest.starts_with("~~~") {
        "~~~"
    } else {
        return None;
    };
    // The opener line runs to the first newline; an unterminated opener never matches.
    let content = start + rest.find('\n')? + 1;
    // The lazy body ends at the first line start that opens with the same marker.
    let close = *starts[starts.partition_point(|&at| at < content)..].iter().find(|&&at| text[at..].starts_with(marker))?;
    let after = close + marker.len();
    // `[^\n]*` greedy then `$`: the rest of the closing line, up to its terminator.
    Some(after + text[after..].find('\n').unwrap_or(text.len() - after))
}

/// The YAML frontmatter's body: `/^---\r?\n([\s\S]*?)\r?\n---/`, anchored at the start of the text.
fn frontmatter(record: &str) -> &str {
    let Some(rest) = record.strip_prefix("---\r\n").or_else(|| record.strip_prefix("---\n")) else {
        return "";
    };
    match rest.find("\n---") {
        Some(at) if at > 0 && rest.as_bytes()[at - 1] == b'\r' => &rest[..at - 1],
        Some(at) => &rest[..at],
        None => "",
    }
}

/// The first word of the status a record states, lowercased; `None` when it states none.
pub fn record_status(record: &str) -> Option<String> {
    let prose = without_fences(record);
    status_key(frontmatter(record)).or_else(|| status_line(&prose)).or_else(|| status_heading(&prose))
}

/// True unless the record says it is proposed, rejected, superseded or deprecated.
pub fn in_force(record: &str) -> bool {
    record_status(record).is_none_or(|status| !NOT_IN_FORCE.contains(&status.as_str()))
}

/// The day a record states its decision was made, as `YYYY-MM-DD`; `None` when it states none.
pub fn decision_date(record: &str) -> Option<String> {
    let prose = without_fences(record);
    date_key(frontmatter(record)).or_else(|| date_line(&prose))
}

/// `/^status[ \t]*:[ \t]*["']?([A-Za-z]+)/im`: a `status:` key at the start of a line.
fn status_key(text: &str) -> Option<String> {
    line_starts(text).into_iter().find_map(|start| {
        let rest = strip_word_ci(&text[start..], "status")?;
        let rest = skip_blanks(rest).strip_prefix(':')?;
        word(strip_quote(skip_blanks(rest)))
    })
}

/// `/^(?:[-*+][ \t]+)?(?:\*\*|__)?status(?:\*\*|__)?[ \t]*:[ \t]*(?:\*\*|__)?[ \t]*([A-Za-z]+)/im`.
fn status_line(text: &str) -> Option<String> {
    line_starts(text).into_iter().find_map(|start| {
        let rest = strip_bold(strip_list_marker(&text[start..]));
        let rest = strip_word_ci(rest, "status")?;
        let rest = skip_blanks(strip_bold(rest));
        let rest = rest.strip_prefix(':')?;
        word(skip_blanks(strip_bold(skip_blanks(rest))))
    })
}

/// `/^#{1,6}[ \t]+status[ \t]*\r?\n\s*[^A-Za-z\n]*([A-Za-z]+)/im`: the first line under a `## Status` heading.
fn status_heading(text: &str) -> Option<String> {
    line_starts(text).into_iter().find_map(|start| {
        let rest = &text[start..];
        let hashes = rest.chars().take_while(|c| *c == '#').count();
        if !(1..=6).contains(&hashes) {
            return None;
        }
        let after = &rest[hashes..];
        let rest = skip_blanks(after);
        if rest.len() == after.len() {
            return None;
        }
        let rest = strip_word_ci(rest, "status")?;
        let rest = skip_blanks(rest);
        let rest = rest.strip_prefix("\r\n").or_else(|| rest.strip_prefix('\n'))?;
        // `\s*[^A-Za-z\n]*` backtracks over the white space it took, and a letter must follow.
        let mut ends = vec![0];
        ends.extend(rest.char_indices().take_while(|&(_, c)| is_js_whitespace(c)).map(|(at, c)| at + c.len_utf8()));
        ends.into_iter().rev().find_map(|taken| word(skip_non_letters(&rest[taken..])))
    })
}

/// `/^date[ \t]*:[ \t]*["']?(\d{4}-\d{2}-\d{2})/im`: a `date:` key at the start of a line.
fn date_key(text: &str) -> Option<String> {
    line_starts(text).into_iter().find_map(|start| {
        let rest = strip_word_ci(&text[start..], "date")?;
        let rest = skip_blanks(rest).strip_prefix(':')?;
        iso_date(strip_quote(skip_blanks(rest)))
    })
}

/// `/^[ \t]*(?:[-*+][ \t]+)?(?:\*\*|__)?date(?:[ \t]*:[ \t]*(?:\*\*|__)?|(?:\*\*|__)[ \t]*:?)[ \t]*(\d{4}-\d{2}-\d{2})/im`.
fn date_line(text: &str) -> Option<String> {
    line_starts(text).into_iter().find_map(|start| {
        let rest = strip_bold(strip_list_marker(skip_blanks(&text[start..])));
        let rest = strip_word_ci(rest, "date")?;
        iso_date(skip_blanks(date_separator(rest)?))
    })
}

/// `(?:[ \t]*:[ \t]*(?:\*\*|__)?|(?:\*\*|__)[ \t]*:?)`: the colon, or the bold markers, that close a `date` key.
fn date_separator(text: &str) -> Option<&str> {
    if let Some(rest) = skip_blanks(text).strip_prefix(':') {
        return Some(strip_bold(skip_blanks(rest)));
    }
    let rest = strip_bold(text);
    if rest.len() == text.len() {
        return None;
    }
    let rest = skip_blanks(rest);
    Some(rest.strip_prefix(':').unwrap_or(rest))
}

/// `(\d{4}-\d{2}-\d{2})`: the day at the start of the text.
fn iso_date(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let digits = |from: usize| bytes.get(from..from + 2).is_some_and(|pair| pair.iter().all(u8::is_ascii_digit));
    let shaped = bytes.get(..4).is_some_and(|year| year.iter().all(u8::is_ascii_digit))
        && bytes.get(4) == Some(&b'-')
        && digits(5)
        && bytes.get(7) == Some(&b'-')
        && digits(8);
    shaped.then(|| text[..10].to_string())
}

/// `status` under `/i`: the text after it when it starts with the word in any ASCII case.
fn strip_word_ci<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let head = text.get(..word.len())?;
    head.eq_ignore_ascii_case(word).then(|| &text[word.len()..])
}

/// `([A-Za-z]+)`: the run of ASCII letters at the start, lowercased.
fn word(text: &str) -> Option<String> {
    let end = text.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(text.len());
    (end > 0).then(|| text[..end].to_ascii_lowercase())
}

/// `[ \t]*`.
fn skip_blanks(text: &str) -> &str {
    text.trim_start_matches(|c: char| c == ' ' || c == '\t')
}

/// `[^A-Za-z\n]*`: the text from the first ASCII letter or newline on.
fn skip_non_letters(text: &str) -> &str {
    &text[text.find(|c: char| c.is_ascii_alphabetic() || c == '\n').unwrap_or(text.len())..]
}

/// `(?:\*\*|__)?`: the bold markers around a key.
fn strip_bold(text: &str) -> &str {
    text.strip_prefix("**").or_else(|| text.strip_prefix("__")).unwrap_or(text)
}

/// `["']?`: one quote before a value.
fn strip_quote(text: &str) -> &str {
    text.strip_prefix('"').or_else(|| text.strip_prefix('\'')).unwrap_or(text)
}

/// `(?:[-*+][ \t]+)?`: a list item's marker and the blanks after it.
fn strip_list_marker(text: &str) -> &str {
    let Some(rest) = ["-", "*", "+"].into_iter().find_map(|marker| text.strip_prefix(marker)) else {
        return text;
    };
    let after = skip_blanks(rest);
    if after.len() < rest.len() {
        after
    } else {
        text
    }
}

/// The doc Pocock's setup writes to say where a repository's domain docs live.
pub const DOMAIN_DOC: &str = "docs/agents/domain.md";

/// Tried in order when there is no domain doc; the first that exists is the glossary.
pub const GLOSSARIES: [&str; 2] = ["GLOSSARY.md", "CONTEXT.md"];

/// The file Pocock's multi-context layout uses to point at one `CONTEXT.md` per context.
pub const CONTEXT_MAP: &str = "CONTEXT-MAP.md";

/// The docs that define words rather than record decisions, so no status can leave them out.
pub const NOT_RECORDS: [&str; 3] = ["GLOSSARY.md", "CONTEXT.md", "CONTEXT-MAP.md"];

/// Whether a decision doc, at its repository path, is a decision record: a markdown file directly in an `adr`
/// directory, the repository's `docs/adr/` or a context's own.
pub fn is_decision_record(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    path.ends_with(".md") && parts.len() >= 2 && parts[parts.len() - 2] == "adr"
}

/// The repository paths a markdown doc names, in order and without repeats: every inline code span and link target
/// outside fenced blocks that reads as a relative path. Placeholders, absolute paths, `..` and skill names are left out.
pub fn named_paths(markdown: &str) -> Vec<String> {
    let prose: Vec<char> = without_fences(markdown).chars().collect();
    let spans = Regex::new(r"`([^`\n]+)`|\]\(([^)\s]+)\)", false).expect("the span pattern compiles");
    let shape = Regex::new(r"^[\w.-]+(\/[\w.-]+)*$", false).expect("the path pattern compiles");
    let mut paths: Vec<String> = Vec::new();
    for hit in spans.find_all(&prose) {
        let candidate = hit.group(&prose, 1).or_else(|| hit.group(&prose, 2)).unwrap_or_default();
        let trimmed = js_trim(&candidate);
        let candidate = trimmed.strip_prefix("./").unwrap_or(trimmed).trim_end_matches('/');
        let chars: Vec<char> = candidate.chars().collect();
        if shape.find_at(&chars, 0).is_some() && !candidate.split('/').any(|part| part == "..") && !paths.iter().any(|seen| seen == candidate) {
            paths.push(candidate.to_string());
        }
    }
    paths
}
