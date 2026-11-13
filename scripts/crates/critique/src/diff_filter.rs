//! The diff filter: a unified diff cut into sections, and the hunks of every file
//! a caller gives a reason for replaced by one line that names it. The node
//! runner's `diff-filter.ts`.

use crate::triage::count_changed_lines;
use model_gateway::json::number_text;

/// A file of a diff whose hunks were left out, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct LeftOutFile {
    pub path: String,
    /// Why it is not shown, as `exclusionReason` words it.
    pub reason: String,
    /// How many added and removed lines it had.
    pub changed_lines: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilteredDiff {
    pub diff: String,
    pub left_out: Vec<LeftOutFile>,
}

/// Where a hunk sits on one side of the change: `count` lines from `start`.
#[derive(Clone, Debug, PartialEq)]
pub struct HunkSide {
    pub start: f64,
    pub count: f64,
}

/// One hunk: its `@@` line and the lines under it.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffHunk {
    pub text: String,
    pub old: HunkSide,
    pub new: HunkSide,
}

/// One part of a diff: a file's section, or a line that stands in for one.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffSection {
    /// The file's path at the head, or at the base for a deleted file; none for a
    /// line that stands in for a file's section, or text before the first one.
    pub path: Option<String>,
    /// Its lines before the first hunk: `diff --git` and the lines git adds under it.
    pub header: String,
    pub hunks: Vec<DiffHunk>,
}

/// The line that stands in a diff for a file whose hunks are left out.
const LEFT_OUT_PREFIX: &str = "Left out of this diff: ";

/// A unified diff with the hunks of every file `left_out` gives a reason for
/// replaced by one line that names the file and says why it is not shown. The line
/// starts with neither `+` nor `-`, so the changed-line count no longer counts the
/// file.
pub fn filter_diff(diff: &str, left_out: &dyn Fn(&str) -> Option<String>) -> FilteredDiff {
    let mut kept = String::new();
    let mut left = Vec::new();
    for section in parse_diff(diff) {
        let reason = section.path.as_deref().and_then(|path| left_out(path));
        match (section.path.as_deref(), reason) {
            (Some(path), Some(reason)) => {
                let changed = count_changed_lines(&section_text(&section));
                kept.push_str(&format!(
                    "{LEFT_OUT_PREFIX}{path}, {reason}; its {} changed lines are not shown.\n",
                    number_text(changed)
                ));
                left.push(LeftOutFile { path: path.to_string(), reason, changed_lines: changed });
            }
            _ => kept.push_str(&section_text(&section)),
        }
    }
    FilteredDiff { diff: kept, left_out: left }
}

/// A diff cut into sections at each `diff --git` line and at each line that stands
/// in for a file, each file's section cut into its header and hunks. Inside a hunk
/// every line starts with `+`, `-`, a space or `\`, so a line that starts with
/// `@@ -` there starts the next hunk.
pub fn parse_diff(diff: &str) -> Vec<DiffSection> {
    let mut sections: Vec<DiffSection> = Vec::new();
    for line in split_after_newlines(diff) {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if bare.starts_with("diff --git ") || bare.starts_with(LEFT_OUT_PREFIX) || sections.is_empty() {
            sections.push(DiffSection { path: None, header: line.to_string(), hunks: Vec::new() });
            continue;
        }
        let current = sections.last_mut().expect("a section was pushed above");
        match hunk_header(bare) {
            Some((old, new)) => current.hunks.push(DiffHunk { text: line.to_string(), old, new }),
            None => match current.hunks.last_mut() {
                Some(last) => last.text.push_str(line),
                None => current.header.push_str(line),
            },
        }
    }
    for section in &mut sections {
        section.path = if section.header.starts_with("diff --git ") { header_path(&section.header) } else { None };
    }
    sections
}

/// A section as the diff prints it.
pub fn section_text(section: &DiffSection) -> String {
    let mut text = section.header.clone();
    for hunk in &section.hunks {
        text.push_str(&hunk.text);
    }
    text
}

/// `diff.split(/(?<=\n)/)`: cut after each newline, keeping it, with no empty piece
/// after a final one.
fn split_after_newlines(diff: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (index, byte) in diff.bytes().enumerate() {
        if byte == b'\n' {
            pieces.push(&diff[start..=index]);
            start = index + 1;
        }
    }
    if start < diff.len() || pieces.is_empty() {
        pieces.push(&diff[start..]);
    }
    pieces
}

/// The sides of a `^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@` line, when `bare` is one.
fn hunk_header(bare: &str) -> Option<(HunkSide, HunkSide)> {
    let rest = bare.strip_prefix("@@ -")?;
    let (old_start, rest) = digits(rest)?;
    let (old_count, rest) = optional_count(rest)?;
    let rest = rest.strip_prefix(" +")?;
    let (new_start, rest) = digits(rest)?;
    let (new_count, rest) = optional_count(rest)?;
    rest.strip_prefix(" @@")?;
    Some((HunkSide { start: old_start, count: old_count }, HunkSide { start: new_start, count: new_count }))
}

/// The leading ASCII digits of `text` as a number, and the rest.
fn digits(text: &str) -> Option<(f64, &str)> {
    let end = text.bytes().position(|byte| !byte.is_ascii_digit()).unwrap_or(text.len());
    if end == 0 {
        return None;
    }
    text[..end].parse::<f64>().ok().map(|number| (number, &text[end..]))
}

/// A hunk side's count: the digits after a comma, or one when the comma is absent.
fn optional_count(text: &str) -> Option<(f64, &str)> {
    match text.strip_prefix(',') {
        Some(rest) => digits(rest),
        None => Some((1.0, text)),
    }
}

/// The path a file's header names: its `+++` line, else its `rename to` or `copy to`
/// line, else its `---` line (a deleted file), else its `diff --git` line when that
/// names one path twice (a mode change or a binary file). The `diff --git` line
/// alone cannot say where its first path ends when a path holds ` b/`, so it is read
/// last.
fn header_path(header: &str) -> Option<String> {
    let trimmed = header.strip_suffix('\n').unwrap_or(header);
    let lines: Vec<&str> = trimmed.split('\n').collect();
    let field = |prefix: &str, strip: &str| -> Option<String> {
        let line = *lines.iter().find(|entry| entry.starts_with(prefix))?;
        let path = unquote(strip_trailing_tab(&line[prefix.len()..]));
        if path == "/dev/null" || !path.starts_with(strip) {
            return None;
        }
        Some(path[strip.len()..].to_string())
    };
    field("+++ ", "b/")
        .or_else(|| field("rename to ", ""))
        .or_else(|| field("copy to ", ""))
        .or_else(|| field("--- ", "a/"))
        .or_else(|| git_line_path(lines.first().copied().unwrap_or("")))
}

/// `text` with one trailing tab removed.
fn strip_trailing_tab(text: &str) -> &str {
    text.strip_suffix('\t').unwrap_or(text)
}

/// The path a `diff --git a/<path> b/<path>` line names twice, quoted or not.
fn git_line_path(line: &str) -> Option<String> {
    let rest: Vec<u16> = line.get("diff --git ".len()..).unwrap_or("").encode_utf16().collect();
    if rest.first() == Some(&u16::from(b'"')) {
        let second = String::from_utf16_lossy(rest.get(quoted_length(&rest) + 1..).unwrap_or(&[]));
        return unquote(&second).strip_prefix("b/").map(str::to_string);
    }
    let length = (rest.len() as f64 - 5.0) / 2.0;
    let from = slice_index(2.0, rest.len());
    let to = slice_index(2.0 + length, rest.len()).max(from);
    let mut expected: Vec<u16> = "a/".encode_utf16().collect();
    expected.extend_from_slice(&rest[from..to]);
    expected.extend(" b/".encode_utf16());
    expected.extend_from_slice(&rest[from..to]);
    (rest == expected).then(|| String::from_utf16_lossy(&rest[from..to]))
}

/// A `String.prototype.slice` index: a negative one counts from the end, and the
/// result is clamped to the string.
fn slice_index(value: f64, len: usize) -> usize {
    let len = len as f64;
    let index = if value < 0.0 { (len + value).max(0.0) } else { value.min(len) };
    index as usize
}

/// How many UTF-16 units a C-quoted string at the start of `units` takes, its quotes
/// included.
fn quoted_length(units: &[u16]) -> usize {
    let mut index = 1;
    while index < units.len() && units[index] != u16::from(b'"') {
        index += if units[index] == u16::from(b'\\') { 2 } else { 1 };
    }
    index + 1
}

/// A path as git prints it, without the C quoting git adds to a path holding a
/// quote, a backslash or a control character.
fn unquote(text: &str) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let quote = u16::from(b'"');
    if units.len() < 2 || units.first() != Some(&quote) || units.last() != Some(&quote) {
        return text.to_string();
    }
    let body = &units[1..units.len() - 1];
    let mut bytes: Vec<u8> = Vec::new();
    let mut index = 0;
    while index < body.len() {
        if body[index] != u16::from(b'\\') {
            push_unit(&mut bytes, body[index]);
            index += 1;
            continue;
        }
        match octal(body.get(index + 1..).unwrap_or(&[])) {
            Some(value) => {
                bytes.push(value);
                index += 4;
            }
            None => {
                bytes.push(escape(body.get(index + 1).copied().unwrap_or(0)));
                index += 2;
            }
        }
    }
    // `new TextDecoder().decode` drops a leading byte order mark.
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// One UTF-16 unit as `TextEncoder` writes it: a lone surrogate becomes U+FFFD.
fn push_unit(bytes: &mut Vec<u8>, unit: u16) {
    if (0xd800..=0xdfff).contains(&unit) {
        bytes.extend_from_slice(&[0xef, 0xbf, 0xbd]);
        return;
    }
    let mut buffer = [0u8; 3];
    let text = char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}');
    bytes.extend_from_slice(text.encode_utf8(&mut buffer).as_bytes());
}

/// The byte a backslash escape stands for.
fn escape(unit: u16) -> u8 {
    match unit {
        0x22 | 0x5c => unit as u8,
        0x61 => 0x07,
        0x62 => 0x08,
        0x66 => 0x0c,
        0x6e => 0x0a,
        0x72 => 0x0d,
        0x74 => 0x09,
        0x76 => 0x0b,
        other => (other & 0xff) as u8,
    }
}

/// The value of three octal digits at the start of `units`, when there are three.
fn octal(units: &[u16]) -> Option<u8> {
    let mut value = 0u16;
    for &unit in units.get(..3)? {
        let digit = match unit {
            0x30..=0x37 => unit - 0x30,
            _ => return None,
        };
        value = value * 8 + digit;
    }
    Some((value & 0xff) as u8)
}
