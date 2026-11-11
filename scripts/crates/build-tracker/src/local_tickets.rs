//! Local markdown specs: the numbered ticket files beside the spec, read the
//! way `/to-tickets` writes them.

use crate::seed::SpecTicket;
use crate::state::{ExternalBlocker, Failure};
use crate::text::{is_space, is_terminator, is_word, parse_number, trim, utf16_cmp, without_md};

/// A local ticket file: its path under the repository root and its text.
pub struct LocalTicketFile {
    pub file: String,
    pub text: String,
}

const CLOSED_STATUSES: [&str; 5] = ["closed", "done", "resolved", "wontfix", "merged"];

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split(is_terminator)
}

/// The text of the file's first top-level `#` heading (`^#[ \t]+(.+?)[ \t]*$`, multiline).
fn first_heading(text: &str) -> Option<String> {
    lines(text).find_map(|line| {
        let rest = line.strip_prefix('#')?;
        let blank = |c: char| c == ' ' || c == '\t';
        let content = rest.trim_start_matches(blank);
        let leading = rest.len() - content.len();
        if leading == 0 {
            return None;
        }
        if content.is_empty() {
            // Only blanks follow: the last one is the text.
            return (leading >= 2).then(|| rest[leading - 1..].to_string());
        }
        Some(content.trim_end_matches(blank).to_string())
    })
}

/// A local spec's title: its first `#` heading, or its file name.
pub fn local_spec_title(text: &str, path: &str) -> String {
    first_heading(text).unwrap_or_else(|| path.rsplit_once('/').map_or(path, |(_, name)| name).to_string())
}

/// A heading without its leading `12:`, `#12 -` or `12.` (`^#?\d+\s*[:.\-–]\s*`).
fn strip_number(heading: &str) -> String {
    let rest = heading.strip_prefix('#').unwrap_or(heading);
    let after_digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let stripped = after_digits.trim_start_matches(is_space).strip_prefix([':', '.', '-', '–']);
    match stripped {
        Some(title) if after_digits.len() < rest.len() => title.trim_start_matches(is_space).to_string(),
        _ => heading.to_string(),
    }
}

fn is_closed(text: &str) -> bool {
    let value = lines(text).find_map(|line| {
        let rest = line.trim_start_matches([' ', '\t', '>']).trim_start_matches('*');
        let (word, rest) = rest.split_at_checked(6)?;
        if !word.eq_ignore_ascii_case("status") {
            return None;
        }
        let rest = rest.trim_start_matches('*').trim_start_matches([' ', '\t']).strip_prefix(':')?;
        Some(rest.to_string())
    });
    value.is_some_and(|value| {
        let plain: String = value.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect();
        CLOSED_STATUSES.contains(&trim(&plain).to_lowercase().as_str())
    })
}

/// `\b` after `at` bytes of `text`: a word character on exactly one side.
fn boundary_after(text: &str, at: usize) -> bool {
    let before = text[..at].chars().next_back().is_some_and(is_word);
    let after = text[at..].chars().next().is_some_and(is_word);
    before != after
}

/// `^(none|n\/a|nothing|-+)\b`, case-insensitive.
fn is_no_blocker(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let word = ["none", "n/a", "nothing"].iter().any(|word| lower.starts_with(word) && boundary_after(&lower, word.len()));
    let dashes = lower.len() - lower.trim_start_matches('-').len();
    word || (dashes > 0 && boundary_after(&lower, dashes))
}

/// `^(?:#?\d+\s*(?:and|&|\s)\s*)*#?\d+$`, case-insensitive: ticket numbers joined by spaces, `and` or `&`.
fn is_number_list(name: &str) -> bool {
    let mut rest = name;
    loop {
        let after_hash = rest.strip_prefix('#').unwrap_or(rest);
        let after_digits = after_hash.trim_start_matches(|c: char| c.is_ascii_digit());
        if after_digits.len() == after_hash.len() {
            return false;
        }
        if after_digits.is_empty() {
            return true;
        }
        let spaced = after_digits.trim_start_matches(is_space);
        let word = spaced.get(..3).filter(|word| word.eq_ignore_ascii_case("and")).map(|_| &spaced[3..]);
        let joined = word.or_else(|| spaced.strip_prefix('&')).map(|tail| tail.trim_start_matches(is_space));
        match joined {
            Some(tail) => rest = tail,
            None if spaced.len() < after_digits.len() => rest = spaced,
            None => return false,
        }
    }
}

/// The names a ticket's "Blocked by" lists: on the line itself (`**Blocked by:** 01, 02`), or as list items under a
/// `Blocked by` line or heading. "None (can start immediately)" lists nothing.
fn blocker_names(text: &str) -> Vec<String> {
    let rows: Vec<&str> = text.split('\n').map(|row| row.strip_suffix('\r').unwrap_or(row)).collect();
    let prefix = |row: &str| -> Option<usize> {
        let lead = row.len() - row.trim_start_matches(|c: char| is_space(c) || matches!(c, '>' | '*' | '_' | '#' | '-')).len();
        let phrase = row.get(lead..lead + 10)?;
        let end = lead + 10;
        (phrase.eq_ignore_ascii_case("blocked by") && boundary_after(row, end)).then_some(end)
    };
    let Some(at) = rows.iter().position(|row| prefix(row).is_some()) else {
        return Vec::new();
    };
    let end = prefix(rows[at]).unwrap_or(0);
    let rest = rows[at][end..].trim_start_matches(|c: char| is_space(c) || matches!(c, '*' | '_' | ':'));
    let mut items: Vec<&str> = Vec::new();
    if !trim(rest).is_empty() {
        items.push(rest);
    } else {
        for row in &rows[at + 1..] {
            match list_item(row) {
                Some(item) => items.push(item),
                None if !trim(row).is_empty() || !items.is_empty() => break,
                None => {}
            }
        }
    }
    let mut names = Vec::new();
    for item in items.iter().flat_map(|item| item.split([',', ';'])) {
        let cleaned: String = item.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '"' | '\'')).collect();
        let name = trim(&cleaned);
        if name.is_empty() || is_no_blocker(name) {
            continue;
        }
        if is_number_list(name) {
            names.extend(name.split(|c: char| !c.is_ascii_digit()).filter(|digits| !digits.is_empty()).map(str::to_string));
        } else {
            names.push(name.to_string());
        }
    }
    names
}

/// `^\s*(?:[-*+]|\d+[.)])\s+(.*)$`: the text of a list item, none for any other line.
fn list_item(row: &str) -> Option<&str> {
    let rest = row.trim_start_matches(is_space);
    let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    let after_marker = if digits.len() < rest.len() {
        digits.strip_prefix(['.', ')'])?
    } else {
        rest.strip_prefix(['-', '*', '+'])?
    };
    let item = after_marker.trim_start_matches(is_space);
    (item.len() < after_marker.len() && !item.chars().any(is_terminator)).then_some(item)
}

/// `^#?(\d+)(?=$|[\s\-:.(])`: the ticket number a blocker name starts with.
fn leading_number(name: &str) -> Option<u64> {
    let rest = name.strip_prefix('#').unwrap_or(name);
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let follows = rest[digits..].chars().next().is_none_or(|c| is_space(c) || matches!(c, '-' | ':' | '.' | '('));
    (digits > 0 && follows).then(|| parse_number(rest[..digits].trim_start_matches('0')).unwrap_or(0))
}

struct Parsed {
    blockers: Vec<String>,
    closed: bool,
    file: String,
    number: u64,
    title: String,
}

/// Reads local ticket files. A ticket's number is the number its file name starts with (`01-store.md` is ticket 1); a
/// file without one is refused. Its "Blocked by" names other tickets by number or by title; a name that matches no ticket
/// is a blocker outside the spec, kept as written. A `Status:` of done, closed, resolved, merged or wontfix closes it.
pub fn local_tickets(files: &[LocalTicketFile]) -> Result<Vec<SpecTicket>, Failure> {
    let mut parsed: Vec<Parsed> = Vec::new();
    for file in files {
        let name = file.file.rsplit_once('/').map_or(file.file.as_str(), |(_, name)| name);
        let stem = without_md(name);
        let digits = stem.len() - stem.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let number = parse_number(stem[..digits].trim_start_matches('0')).unwrap_or(0);
        if number == 0 {
            return Err(Failure::new(
                "invalid-ticket-file",
                format!("{} has no ticket number in front of its name; name each ticket file like 01-{stem}.md", file.file),
            ));
        }
        let title = first_heading(&file.text).map_or_else(|| stem.to_string(), |heading| strip_number(&heading));
        parsed.push(Parsed { blockers: blocker_names(&file.text), closed: is_closed(&file.text), file: file.file.clone(), number, title });
    }
    for (index, ticket) in parsed.iter().enumerate() {
        if let Some(other) = parsed[..index].iter().find(|other| other.number == ticket.number) {
            return Err(Failure::new("invalid-ticket-file", format!("{} and {} both have ticket number {}", other.file, ticket.file, ticket.number)));
        }
    }
    let resolve_name = |name: &str| -> Option<u64> {
        let found = match leading_number(name) {
            Some(number) => parsed.iter().find(|ticket| ticket.number == number),
            None => parsed.iter().find(|ticket| ticket.title.to_lowercase() == name.to_lowercase()),
        };
        found.map(|ticket| ticket.number)
    };
    let mut tickets = Vec::new();
    for ticket in &parsed {
        let (mut blocked_by, mut external) = (Vec::new(), Vec::new());
        for name in &ticket.blockers {
            match resolve_name(name) {
                None => external.push(ExternalBlocker { reference: name.clone(), state: None }),
                Some(number) if !blocked_by.contains(&number) => blocked_by.push(number),
                Some(_) => {}
            }
        }
        tickets.push(SpecTicket { blocked_by, closed: ticket.closed, external, file: Some(ticket.file.clone()), number: ticket.number, title: ticket.title.clone() });
    }
    tickets.sort_by_key(|ticket| ticket.number);
    Ok(tickets)
}

/// The `.md` names among a folder's files, in the order JavaScript's default sort puts them.
pub fn markdown_names(names: &[String]) -> Vec<String> {
    let mut markdown: Vec<String> = names.iter().filter(|name| name.to_lowercase().ends_with(".md")).cloned().collect();
    markdown.sort_by(|left, right| utf16_cmp(left, right));
    markdown
}
