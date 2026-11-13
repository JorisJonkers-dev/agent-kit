//! How a diff that does not fit a code packet's cap is sent: in windows by file, by rule and never by a model. Files
//! go in this order: the changed source files, then the tests, then the docs, each group by changed lines, most
//! first; each file's hunks in their own order. A hunk goes in with its file's header and its own `@@` line, so a
//! line it shows keeps its number; a hunk that does not fit is left out and listed by file and line, and a smaller
//! one after it may still go in. The node runner's `domain/diff-window.ts`.

use crate::code_facts::code_unit_order;
use crate::diff_filter::{parse_diff, section_text, DiffHunk, DiffSection};
use crate::js_regex::Regex;
use crate::triage::count_changed_lines;
use crate::tree_sample::bytes;
use model_gateway::json::number_text;

/// What a changed file is, in the order its hunks are offered to the cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Source,
    Test,
    Doc,
}

fn matches(pattern: &str, flags: &str, text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    Regex::with_flags(pattern, flags).expect("the path pattern compiles").find_at(&chars, 0).is_some()
}

pub fn file_kind(path: &str) -> FileKind {
    if matches(r"(^|\/)(tests?|__tests__|specs?)\/|[._-](test|spec)\.[^/]+$|(^|\/)test_[^/]+\.py$", "", path) {
        return FileKind::Test;
    }
    if matches(r"(^|\/)docs?\/|\.(md|mdx|markdown|rst|adoc|txt)$", "i", path) {
        FileKind::Doc
    } else {
        FileKind::Source
    }
}

/// The first line of a windowed diff.
pub const DIFF_WINDOW_NOTICE: &str = "The packet cap left hunks out of this diff; facts/INDEX.md lists each one by file and line.\n";

/// A file's hunks the cap left out of the diff.
#[derive(Clone, Debug, PartialEq)]
pub struct DiffDrop {
    pub path: String,
    /// Its hunks that were left out; none for a file with no hunks, whose whole section was.
    pub hunks: Vec<DiffHunk>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowedDiff {
    pub diff: String,
    pub dropped: Vec<DiffDrop>,
}

/// The diff with only the hunks that fit in `budget` bytes, each costing its bytes and, for its file's first, the
/// file's header. The lines that stand in for left-out files always go in, after the windowed files. With a budget
/// of 0 every file's hunks are left out: the most the facts index can list.
pub fn window_diff(diff: &str, budget: i64) -> WindowedDiff {
    let sections = parse_diff(diff);
    let files = rank_files(sections.iter().filter(|section| section.path.is_some()).collect());
    let standing: String = sections.iter().filter(|section| section.path.is_none()).map(section_text).collect();
    let mut remaining = budget;
    let mut kept = String::new();
    let mut dropped = Vec::new();
    for file in files {
        let path = file.path.clone().unwrap_or_default();
        let pieces: Vec<Option<&DiffHunk>> = if file.hunks.is_empty() { vec![None] } else { file.hunks.iter().map(Some).collect() };
        let (mut sent, mut left): (Vec<&DiffHunk>, Vec<&DiffHunk>) = (Vec::new(), Vec::new());
        for hunk in pieces {
            let cost = bytes(hunk.map_or("", |hunk| hunk.text.as_str())) + if sent.is_empty() { bytes(&file.header) } else { 0 };
            let fits = cost <= remaining;
            if fits {
                remaining -= cost;
            }
            match hunk {
                None if fits => kept.push_str(&file.header),
                None => dropped.push(DiffDrop { hunks: Vec::new(), path: path.clone() }),
                Some(hunk) if fits => sent.push(hunk),
                Some(hunk) => left.push(hunk),
            }
        }
        if !sent.is_empty() {
            kept.push_str(&file.header);
            kept.extend(sent.iter().map(|hunk| hunk.text.as_str()));
        }
        if !left.is_empty() {
            dropped.push(DiffDrop { hunks: left.into_iter().cloned().collect(), path });
        }
    }
    WindowedDiff { diff: format!("{DIFF_WINDOW_NOTICE}{kept}{standing}"), dropped }
}

/// Source files, then tests, then docs; each by changed lines, most first, then by path.
fn rank_files(mut files: Vec<&DiffSection>) -> Vec<&DiffSection> {
    let changed = |file: &DiffSection| file.hunks.iter().map(|hunk| count_changed_lines(&hunk.text)).sum::<f64>();
    let path = |file: &DiffSection| file.path.clone().unwrap_or_default();
    files.sort_by(|a, b| {
        (file_kind(&path(a)) as usize)
            .cmp(&(file_kind(&path(b)) as usize))
            .then_with(|| changed(b).partial_cmp(&changed(a)).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| code_unit_order(&path(a), &path(b)))
    });
    files
}

/// A dropped file's line in the facts index: its hunks at their lines at the head, a hunk that only removes lines by
/// its lines at the base.
pub fn diff_drop_line(drop: &DiffDrop) -> String {
    if drop.hunks.is_empty() {
        return format!("- `{}`: its whole section, which has no hunks", drop.path);
    }
    let head: Vec<String> = drop.hunks.iter().filter(|hunk| hunk.new.count > 0.0).map(|hunk| span(hunk.new.start, hunk.new.count)).collect();
    let base: Vec<String> = drop.hunks.iter().filter(|hunk| hunk.new.count == 0.0).map(|hunk| span(hunk.old.start, hunk.old.count)).collect();
    let mut parts = Vec::new();
    if !head.is_empty() {
        parts.push(format!("lines {}", head.join(", ")));
    }
    if !base.is_empty() {
        parts.push(format!("base lines {}", base.join(", ")));
    }
    format!("- `{}`: {}", drop.path, parts.join("; "))
}

fn span(start: f64, count: f64) -> String {
    if count == 1.0 {
        number_text(start)
    } else {
        format!("{}-{}", number_text(start), number_text(start + count - 1.0))
    }
}
