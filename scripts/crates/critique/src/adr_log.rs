//! The decision log's structure: its numbering, the statuses its records state,
//! whether each link and ADR number resolves, and whether a record in force
//! still cites one that is superseded. The node checker's `adr-log.ts`.

use crate::decision_docs::{in_force, record_status, without_fences};
use model_gateway::text::{basename, is_js_whitespace};
use std::collections::HashSet;

/// One record of the decision log, at its repository path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdrSource {
    pub path: String,
    pub text: String,
}

/// What the checker reads off one record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdrEntry {
    pub path: String,
    /// The number its file name starts with; `None` for a file such as a README with none.
    pub number: Option<u32>,
    pub title: String,
    /// The status the record states, lowercased; `None` when it states none, which counts as accepted.
    pub status: Option<String>,
    /// False when its status takes it out of force, or a record in force names it on a Supersedes or Replaces line.
    pub in_force: bool,
}

/// A number more than one record claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Duplicate {
    pub number: u32,
    pub records: Vec<String>,
}

/// The numbers the log uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Numbering {
    /// Numbers missing between the lowest and the highest the log uses.
    pub gaps: Vec<u32>,
    pub duplicates: Vec<Duplicate>,
    /// Files whose name starts with no number; reported, never an error.
    pub unnumbered: Vec<String>,
}

/// One target a record names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdrLink {
    pub record: String,
    /// The target as the record writes it: a link's path, or an ADR number such as `ADR-0004`.
    pub target: String,
    /// `supersede` for a target on a Supersedes, Superseded by, Amends or Replaces line; `link` for any other.
    pub kind: &'static str,
}

/// A status a record states that is none of the five the log knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownStatus {
    pub record: String,
    pub status: String,
}

/// A record in force that cites one the log supersedes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cites {
    pub record: String,
    pub cites: String,
}

/// What the checker reads off the log's text and its links alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdrStructure {
    pub records: Vec<AdrEntry>,
    pub numbering: Numbering,
    pub unknown_statuses: Vec<UnknownStatus>,
    pub unresolved_links: Vec<AdrLink>,
    pub cites_superseded: Vec<Cites>,
    /// The paths of the records the log supersedes: marked so, replaced by a record in force, or naming a replacement.
    pub superseded: Vec<String>,
}

/// The status values a record may state, as Nygard and MADR name them.
const STATUSES: [&str; 5] = ["proposed", "accepted", "rejected", "deprecated", "superseded"];

/// The phrases that link a record to the one it replaces or amends, or that replace or amend it.
const SUPERSEDE: [&[&str]; 6] = [&["supersedes"], &["superseded", "by"], &["amends"], &["amended", "by"], &["replaces"], &["replaced", "by"]];

/// The phrases on which a record says it replaces the records it names.
const REPLACING: [&[&str]; 2] = [&["supersedes"], &["replaces"]];

/// The phrases on which a record says another record replaced it.
const REPLACED: [&[&str]; 2] = [&["superseded", "by"], &["replaced", "by"]];

/// The repository paths the log's links name, so the caller can say which exist before the log is checked.
pub fn linked_paths(sources: &[AdrSource]) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for source in sources {
        for reference in references(source) {
            if let Some(path) = reference.path {
                push_unique(&mut paths, path);
            }
        }
    }
    paths
}

/// Checks the decision log's structure: numbering, the status values records state, whether each link and ADR
/// number resolves, and whether a record in force still cites one that is superseded. `existing` holds the linked
/// repository paths that exist.
pub fn check_adr_log(sources: &[AdrSource], existing: &HashSet<String>) -> AdrStructure {
    let stated: Vec<AdrEntry> = sources.iter().map(entry).collect();
    let named: Vec<Vec<Reference>> = sources.iter().map(references).collect();
    let numbers: HashSet<u32> = stated.iter().filter_map(|record| record.number).collect();
    let resolves = |reference: &Reference| match reference.number {
        Some(number) => numbers.contains(&number),
        None => reference.path.as_ref().is_some_and(|path| existing.contains(path)),
    };
    let records_of = |reference: &Reference| -> Vec<String> {
        stated
            .iter()
            .filter(|record| match reference.number {
                Some(number) => record.number == Some(number),
                None => reference.path.as_deref() == Some(record.path.as_str()),
            })
            .map(|record| record.path.clone())
            .collect()
    };
    let mut replaced: Vec<String> = Vec::new();
    for (index, record) in stated.iter().enumerate() {
        if !record.in_force {
            continue;
        }
        for reference in named[index].iter().filter(|reference| reference.replacing) {
            for path in records_of(reference) {
                push_unique(&mut replaced, path);
            }
        }
    }
    let mut replaced_by: Vec<String> = Vec::new();
    for (index, record) in stated.iter().enumerate() {
        if named[index].iter().any(|reference| reference.replaced) {
            push_unique(&mut replaced_by, record.path.clone());
        }
    }
    let records: Vec<AdrEntry> = stated
        .iter()
        .map(|record| AdrEntry {
            in_force: record.in_force && !replaced.contains(&record.path) && !replaced_by.contains(&record.path),
            ..record.clone()
        })
        .collect();
    let mut superseded: Vec<String> = Vec::new();
    for record in records.iter().filter(|record| record.status.as_deref() == Some("superseded")) {
        push_unique(&mut superseded, record.path.clone());
    }
    for path in replaced.iter().chain(&replaced_by) {
        push_unique(&mut superseded, path.clone());
    }
    let mut cites_superseded: Vec<Cites> = Vec::new();
    for (index, record) in records.iter().enumerate() {
        if !record.in_force || record.number.is_none() {
            continue;
        }
        let mut cited: Vec<String> = Vec::new();
        for reference in named[index].iter().filter(|reference| reference.kind == "link") {
            for path in records_of(reference) {
                push_unique(&mut cited, path);
            }
        }
        for cites in cited {
            if superseded.contains(&cites) && cites != record.path {
                cites_superseded.push(Cites { record: record.path.clone(), cites });
            }
        }
    }
    let unknown_statuses: Vec<UnknownStatus> = records
        .iter()
        .filter_map(|record| {
            let status = record.status.as_deref()?;
            (!STATUSES.contains(&status)).then(|| UnknownStatus { record: record.path.clone(), status: status.to_string() })
        })
        .collect();
    let unresolved_links: Vec<AdrLink> = named
        .iter()
        .flatten()
        .filter(|reference| !resolves(reference))
        .map(|reference| AdrLink { kind: reference.kind, record: reference.record.clone(), target: reference.target.clone() })
        .collect();
    AdrStructure {
        numbering: numbering(&records),
        records,
        superseded,
        unknown_statuses,
        unresolved_links,
        cites_superseded,
    }
}

/// `failed` for a structural error only. Drifted and stale records are listed in the report but never fail it: they
/// are questions for the owner.
pub fn adr_check_status(check: &AdrStructure) -> &'static str {
    let errors = [
        !check.numbering.gaps.is_empty(),
        !check.numbering.duplicates.is_empty(),
        !check.unknown_statuses.is_empty(),
        !check.unresolved_links.is_empty(),
        !check.cites_superseded.is_empty(),
    ];
    if errors.contains(&true) {
        "failed"
    } else {
        "passed"
    }
}

/// One target a record names, with what it points at.
struct Reference {
    record: String,
    target: String,
    kind: &'static str,
    /// The repository path a link names, or `None` when it climbs out of the repository; only for a link to a file.
    path: Option<String>,
    /// The number an ADR number names; only for an ADR number.
    number: Option<u32>,
    /// True on a line where the record says it replaces what it names.
    replacing: bool,
    /// True on a clause where the record says another record replaced it.
    replaced: bool,
}

/// What one match of the reference pattern names.
enum Target<'a> {
    /// An inline link's or a reference definition's target.
    File(&'a str),
    /// An ADR number.
    Number(u32),
}

/// What the checker reads off one record.
fn entry(source: &AdrSource) -> AdrEntry {
    AdrEntry {
        in_force: in_force(&source.text),
        number: record_number(basename(&source.path)),
        path: source.path.clone(),
        status: record_status(&source.text),
        title: title(&source.text),
    }
}

/// The numbers the log uses, in the order the records first claim them.
fn numbering(records: &[AdrEntry]) -> Numbering {
    let mut by_number: Vec<(u32, Vec<String>)> = Vec::new();
    for record in records {
        let Some(number) = record.number else { continue };
        match by_number.iter_mut().find(|(used, _)| *used == number) {
            Some((_, paths)) => paths.push(record.path.clone()),
            None => by_number.push((number, vec![record.path.clone()])),
        }
    }
    by_number.sort_by_key(|(number, _)| *number);
    let mut gaps: Vec<u32> = Vec::new();
    if let (Some((first, _)), Some((last, _))) = (by_number.first(), by_number.last()) {
        for number in *first..=*last {
            if !by_number.iter().any(|(used, _)| *used == number) {
                gaps.push(number);
            }
        }
    }
    let duplicates: Vec<Duplicate> = by_number
        .iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(number, paths)| Duplicate { number: *number, records: sorted(paths) })
        .collect();
    Numbering {
        gaps,
        duplicates,
        unnumbered: records.iter().filter(|record| record.number.is_none()).map(|record| record.path.clone()).collect(),
    }
}

/// Whether one record marks itself superseded by another: it names the other in a clause saying it was superseded or
/// replaced, by link or ADR number.
pub fn marked_superseded_by(record: &AdrSource, target: &str) -> bool {
    references(record).iter().any(|reference| reference.replaced && names_record(reference, target))
}

/// Whether a target a record names is the record at `path`, by repository path or by the number its file name starts with.
fn names_record(reference: &Reference, path: &str) -> bool {
    if reference.path.as_deref() == Some(path) {
        return true;
    }
    let name = model_gateway::text::basename(path);
    matches!((record_number(name), reference.number), (Some(number), Some(named)) if number == named)
}

/// Every target a record names outside fenced blocks, in text order.
fn references(source: &AdrSource) -> Vec<Reference> {
    without_fences(&source.text).split('\n').flat_map(|line| line_references(&source.path, line)).collect()
}

/// Every target one line names, in text order, with the clause that governs it.
fn line_references(record: &str, line: &str) -> Vec<Reference> {
    let text = without_urls(line);
    let breaks = clause_breaks(&text);
    let mut found: Vec<Reference> = Vec::new();
    let mut next = 0;
    for (at, _) in text.char_indices() {
        if at < next {
            continue;
        }
        let Some((end, target)) = reference_at(&text, at) else { continue };
        next = end;
        let before = breaks.iter().rev().find(|break_at| **break_at < at).copied().unwrap_or(0);
        let after = breaks.iter().find(|break_at| **break_at >= at).copied().unwrap_or(text.len());
        let clause = &text[before..after];
        let kind = if holds_phrase(clause, &SUPERSEDE) { "supersede" } else { "link" };
        let replaced = holds_phrase(clause, &REPLACED);
        let replacing = holds_phrase(clause, &REPLACING);
        match target {
            Target::Number(number) => found.push(Reference {
                kind,
                number: Some(number),
                path: None,
                record: record.to_string(),
                replaced,
                replacing,
                target: text[at..end].to_string(),
            }),
            Target::File(target) if !not_a_file(target) => found.push(Reference {
                kind,
                number: None,
                path: repository_path(record, target),
                record: record.to_string(),
                replaced,
                replacing,
                target: target.to_string(),
            }),
            Target::File(_) => {}
        }
    }
    found
}

/// The first target the reference pattern matches at `at`, its alternatives tried in order.
fn reference_at(text: &str, at: usize) -> Option<(usize, Target<'_>)> {
    inline_link(text, at).or_else(|| reference_definition(text, at)).or_else(|| adr_number(text, at))
}

/// `\]\(<?([^)\s>]+)>?(?:[ \t]+"[^"]*")?\)` at `at`: the end of the match and the target it names.
fn inline_link(text: &str, at: usize) -> Option<(usize, Target<'_>)> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&b']') || bytes.get(at + 1) != Some(&b'(') {
        return None;
    }
    let start = at + 2;
    // `<?` is greedy: the `<` is taken first, then left for the target.
    let mut opens = [start, start];
    let open_count = if bytes.get(start) == Some(&b'<') {
        opens[0] = start + 1;
        2
    } else {
        1
    };
    for &open in &opens[..open_count] {
        let ends = run_ends(text, open, |c| c == ')' || c == '>');
        for &close in ends.iter().rev() {
            // `>?` is greedy too, and the title is tried before it is left out.
            let mut heads = [close, close];
            let head_count = if bytes.get(close) == Some(&b'>') {
                heads[0] = close + 1;
                2
            } else {
                1
            };
            for &head in &heads[..head_count] {
                if let Some(end) = title_then_paren(text, head) {
                    return Some((end, Target::File(&text[open..close])));
                }
                if bytes.get(head) == Some(&b')') {
                    return Some((head + 1, Target::File(&text[open..close])));
                }
            }
        }
    }
    None
}

/// `^[ \t]*\[(?!\^)[^\]\n]+\]:[ \t]*<?([^\s>]+)>?` at `at`: the end of the match and the target it names.
fn reference_definition(text: &str, at: usize) -> Option<(usize, Target<'_>)> {
    if !at_line_start(text, at) {
        return None;
    }
    let bytes = text.as_bytes();
    let mut cursor = at;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'[') || bytes.get(cursor + 1) == Some(&b'^') {
        return None;
    }
    let label = cursor + 1;
    let mut close = None;
    for (offset, c) in text[label..].char_indices() {
        if c == ']' {
            close = Some(label + offset);
            break;
        }
    }
    let close = close?;
    if close == label || bytes.get(close + 1) != Some(&b':') {
        return None;
    }
    let mut cursor = close + 2;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    let mut opens = [cursor, cursor];
    let open_count = if bytes.get(cursor) == Some(&b'<') {
        opens[0] = cursor + 1;
        2
    } else {
        1
    };
    for &open in &opens[..open_count] {
        let Some(&close) = run_ends(text, open, |c| c == '>').last() else { continue };
        let end = if bytes.get(close) == Some(&b'>') { close + 1 } else { close };
        return Some((end, Target::File(&text[open..close])));
    }
    None
}

/// `\bADR[- ]?(\d{1,5})\b` at `at`: the end of the match and the number it names.
fn adr_number(text: &str, at: usize) -> Option<(usize, Target<'_>)> {
    let bytes = text.as_bytes();
    if at > 0 && is_word(bytes[at - 1]) {
        return None;
    }
    if !bytes.get(at..at + 3).is_some_and(|head| head.eq_ignore_ascii_case(b"ADR")) {
        return None;
    }
    let mut cursor = at + 3;
    if matches!(bytes.get(cursor), Some(b'-' | b' ')) {
        cursor += 1;
    }
    let digits = bytes[cursor..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 || digits > 5 {
        return None;
    }
    let end = cursor + digits;
    if matches!(bytes.get(end), Some(byte) if is_word(*byte)) {
        return None;
    }
    Some((end, Target::Number(text[cursor..end].parse().ok()?)))
}

/// The end of each prefix of the run of characters from `from` that are neither white space nor `excluded`.
fn run_ends(text: &str, from: usize, excluded: impl Fn(char) -> bool) -> Vec<usize> {
    let mut ends = Vec::new();
    for (offset, c) in text[from..].char_indices() {
        if excluded(c) || is_js_whitespace(c) {
            break;
        }
        ends.push(from + offset + c.len_utf8());
    }
    ends
}

/// `[ \t]+"[^"]*"\)` at `at`: the position after the `)`.
fn title_then_paren(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut cursor = at;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    if cursor == at || bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    let close = cursor + 1 + text[cursor + 1..].find('"')?;
    (bytes.get(close + 1) == Some(&b')')).then_some(close + 2)
}

/// Whether a target names no repository file: it has a scheme such as `https:` or `mailto:`, or is only an anchor.
fn not_a_file(target: &str) -> bool {
    let bytes = target.as_bytes();
    if bytes.first() == Some(&b'#') {
        return true;
    }
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return false;
    }
    let mut at = 1;
    while bytes.get(at).is_some_and(|byte| is_scheme_char(*byte)) {
        at += 1;
    }
    bytes.get(at) == Some(&b':')
}

/// Whether the byte may be part of a URL scheme: an ASCII letter, digit, `+`, `.` or `-`.
fn is_scheme_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-')
}

/// The repository path a link in a record names, without its anchor; `None` when it climbs out of the repository.
fn repository_path(record: &str, target: &str) -> Option<String> {
    let file = match target.find('#') {
        Some(at) => &target[..at],
        None => target,
    };
    let path = match file.strip_prefix('/') {
        Some(rest) => normalize(rest),
        None => normalize(&join(dirname(record), file)),
    };
    (path != ".." && !path.starts_with("../")).then_some(path)
}

/// `path.posix.normalize`: `.` and `..` resolved, `//` collapsed, a trailing slash kept; an absolute path stays absolute and cannot climb above its root.
fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let trailing = path.ends_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if matches!(parts.last(), Some(&"..") | None) {
                    if !absolute {
                        parts.push("..");
                    }
                } else {
                    parts.pop();
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    match (absolute, joined.is_empty(), trailing) {
        (true, true, _) => "/".to_string(),
        (true, false, true) => format!("/{joined}/"),
        (true, false, false) => format!("/{joined}"),
        (false, true, true) => "./".to_string(),
        (false, true, false) => ".".to_string(),
        (false, false, true) => format!("{joined}/"),
        (false, false, false) => joined,
    }
}

/// `path.posix.join` for a record's directory and a link's file.
fn join(dir: &str, file: &str) -> String {
    normalize(&format!("{dir}/{file}"))
}

/// `path.posix.dirname` for a repository path.
fn dirname(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(at) => match trimmed[..at].trim_end_matches('/') {
            "" => "/",
            head => head,
        },
        None => ".",
    }
}

/// The number a record's file name starts with: up to five digits, never the year of a date such as `2024-01-15`.
fn record_number(name: &str) -> Option<u32> {
    let bytes = name.as_bytes();
    if date_like(bytes) {
        return None;
    }
    let digits = bytes.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 || digits > 5 || !matches!(bytes.get(digits), Some(b'-' | b'_' | b' ')) {
        return None;
    }
    name[..digits].parse().ok()
}

/// Whether the name starts with a date such as `2024-01-15` or `2024_1_5`, which is no record number.
fn date_like(bytes: &[u8]) -> bool {
    if !digits_at(bytes, 0, 4) || !matches!(bytes.get(4), Some(b'-' | b'_')) {
        return false;
    }
    for month in [2, 1] {
        let day_start = 5 + month;
        if !digits_at(bytes, 5, month) || !matches!(bytes.get(day_start), Some(b'-' | b'_')) {
            continue;
        }
        for day in [2, 1] {
            let end = day_start + 1 + day;
            if digits_at(bytes, day_start + 1, day) && !matches!(bytes.get(end), Some(byte) if byte.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// Whether `width` ASCII digits start at `at`.
fn digits_at(bytes: &[u8], at: usize, width: usize) -> bool {
    bytes.get(at..at + width).is_some_and(|run| run.iter().all(u8::is_ascii_digit))
}

/// The first `#` heading's text, as `/^#[ \t]+(.+?)[ \t]*$/m` reads it.
fn title(text: &str) -> String {
    let mut start = 0;
    loop {
        if let Some(found) = heading_at(text, start) {
            return found;
        }
        match text[start..].char_indices().find(|(_, c)| is_line_terminator(*c)) {
            Some((offset, terminator)) => start += offset + terminator.len_utf8(),
            None => return String::new(),
        }
    }
}

/// The heading's text when one starts at the line start `at`.
fn heading_at(text: &str, at: usize) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.get(at) != Some(&b'#') {
        return None;
    }
    let mut cursor = at + 1;
    while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
        cursor += 1;
    }
    if cursor == at + 1 {
        return None;
    }
    let end = text[cursor..].char_indices().find(|(_, c)| is_line_terminator(*c)).map_or(text.len(), |(offset, _)| cursor + offset);
    let rest = &text[cursor..end];
    if rest.is_empty() {
        // `(.+?)` needs a character, so the last space or tab of the run becomes the title.
        return (cursor - at > 2).then(|| text[cursor - 1..cursor].to_string());
    }
    Some(rest.trim_end_matches(|c| c == ' ' || c == '\t').to_string())
}

/// The line with every bare URL replaced by a space, as `line.replace(BARE_URL, ' ')` does.
fn without_urls(line: &str) -> String {
    let mut text = String::with_capacity(line.len());
    let mut next = 0;
    for (at, c) in line.char_indices() {
        if at < next {
            continue;
        }
        match bare_url(line, at) {
            Some(end) => {
                text.push(' ');
                next = end;
            }
            None => text.push(c),
        }
    }
    text
}

/// `\b[a-z][a-z0-9+.-]*:\/\/\S+` at `at`: the end of the URL, which names no record of this log.
fn bare_url(text: &str, at: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if at > 0 && is_word(bytes[at - 1]) {
        return None;
    }
    if !bytes.get(at).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut cursor = at + 1;
    while bytes.get(cursor).is_some_and(|byte| is_scheme_char(*byte)) {
        cursor += 1;
    }
    if !bytes[cursor..].starts_with(b"://") {
        return None;
    }
    let mut end = cursor + 3;
    let mut found = false;
    for (offset, c) in text[end..].char_indices() {
        if is_js_whitespace(c) {
            break;
        }
        found = true;
        end = cursor + 3 + offset + c.len_utf8();
    }
    found.then_some(end)
}

/// The clauses of a line: a keyword governs only the targets in its own clause, up to a semicolon or the end of a
/// sentence.
fn clause_breaks(text: &str) -> Vec<usize> {
    let mut breaks = Vec::new();
    for (at, c) in text.char_indices() {
        let sentence_end = c == '.' && text[at + 1..].chars().next().is_some_and(is_js_whitespace);
        if c == ';' || sentence_end {
            breaks.push(at);
        }
    }
    breaks
}

/// Whether `text` holds one of `phrases` as whole words, ignoring ASCII case; a phrase's words are separated by
/// spaces or tabs.
fn holds_phrase(text: &str, phrases: &[&[&str]]) -> bool {
    let bytes = text.as_bytes();
    (0..bytes.len()).any(|at| starts_word(bytes, at) && phrases.iter().any(|words| phrase_ends(bytes, at, words).is_some()))
}

/// Whether a word starts at `at`: a word character with no word character before it.
fn starts_word(bytes: &[u8], at: usize) -> bool {
    bytes.get(at).is_some_and(|byte| is_word(*byte)) && (at == 0 || !is_word(bytes[at - 1]))
}

/// Where `words` ends when it starts at `at`, its last word ending at a word boundary.
fn phrase_ends(bytes: &[u8], at: usize, words: &[&str]) -> Option<usize> {
    let mut cursor = at;
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            let start = cursor;
            while matches!(bytes.get(cursor), Some(b' ' | b'\t')) {
                cursor += 1;
            }
            if cursor == start {
                return None;
            }
        }
        let end = cursor + word.len();
        if !bytes.get(cursor..end).is_some_and(|found| found.eq_ignore_ascii_case(word.as_bytes())) {
            return None;
        }
        cursor = end;
    }
    (!bytes.get(cursor).is_some_and(|byte| is_word(*byte))).then_some(cursor)
}

/// Whether JavaScript's `\w` counts the byte: ASCII letters, digits and the underscore.
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether JavaScript counts the character as a line terminator, which `.` does not match and `/m` anchors on.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Whether `/m`'s `^` holds at `at`: the start of the text or just after a line terminator.
fn at_line_start(text: &str, at: usize) -> bool {
    text[..at].chars().next_back().is_none_or(is_line_terminator)
}

/// Adds `value` when it is not there yet, keeping the order of first insertion, as a `Set` does.
fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// The paths in the order JavaScript's default sort puts them: by UTF-16 code unit.
fn sorted(paths: &[String]) -> Vec<String> {
    let mut sorted = paths.to_vec();
    sorted.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    sorted
}
