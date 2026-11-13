//! How a packet's `tree/` is filled under a cap, by rule and never by a model. Each file is a candidate, sent as the
//! pieces of it that fit: its windows (runs of lines from its first) or, for a code packet, the slices around a
//! change. A file holds every line of the pieces that went in, at its own line numbers; a run of lines it does not
//! hold before its last held line is blank but for a marker on its first line, and the file ends at its last held
//! line. The node runner's `domain/tree-sample.ts`.

use crate::recovery::line_count;

/// Lines `first` to `last` of a file, both included, counted from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineRange {
    pub first: usize,
    pub last: usize,
}

/// A file that may go into the tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: String,
    /// Why it was chosen: the first tier it was chosen in.
    pub tier: &'static str,
    pub text: String,
    pub line_count: usize,
}

/// Some lines of a candidate, offered to the cap in the order pieces are listed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    /// The index of the candidate in the list the pieces are offered with.
    pub candidate: usize,
    pub ranges: Vec<LineRange>,
    /// A window runs from where its file's last window stopped: one that does not fit drops the rest of its file's
    /// windows. A slice stands alone.
    pub window: bool,
    /// The piece's own line in the facts index when it went in, and when it did not.
    pub note: Option<(String, String)>,
}

/// What went in: each candidate's held lines, merged, and which of the pieces were admitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Admission {
    pub held: Vec<Vec<LineRange>>,
    pub admitted: Vec<bool>,
}

/// A tree file as frozen, and only when it was cut short, which of its lines it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampledFile {
    pub path: String,
    pub text: String,
    pub window: Option<SampledWindow>,
}

/// Which lines a file cut short holds: `ranges` (or lines 1 to `last`) of its `lines`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampledWindow {
    pub last: usize,
    pub lines: usize,
    pub ranges: Option<Vec<LineRange>>,
}

/// The facts index's line about one candidate and the lines it holds; `None` when the index has no line for it.
pub type Describe<'a> = Option<&'a dyn Fn(&Candidate, &[LineRange]) -> Option<String>>;

pub fn bytes(text: &str) -> i64 {
    text.len() as i64
}

/// A candidate for a file's text.
pub fn candidate_of(path: &str, tier: &'static str, text: &str) -> Candidate {
    Candidate { line_count: line_count(text), path: path.to_string(), text: text.to_string(), tier }
}

/// A candidate's windows of `size` lines, from its first line; an empty file is one empty window.
pub fn windows_of(index: usize, candidate: &Candidate, size: usize) -> Vec<Piece> {
    let count = candidate.line_count.div_ceil(size).max(1);
    (0..count)
        .map(|i| Piece {
            candidate: index,
            ranges: vec![LineRange { first: i * size + 1, last: (i * size + 1).max(candidate.line_count.min((i + 1) * size)) }],
            window: true,
            note: None,
        })
        .collect()
}

/// Offers each piece to the budget in order. A piece costs the bytes it adds to its file, and the change it makes to
/// the facts index: its file's line there, as `describe` words it (none when the line is `None`), and the piece's own
/// note. With no `describe`, the facts index has a cap of its own, so a piece costs only its bytes. A piece that does
/// not fit is left out, and a window that does not fit drops its file's later windows.
pub fn admit_pieces(candidates: &[Candidate], pieces: &[Piece], budget: i64, describe: Describe) -> Admission {
    let mut held: Vec<Vec<LineRange>> = vec![Vec::new(); candidates.len()];
    let mut admitted = vec![false; pieces.len()];
    let mut blocked = vec![false; candidates.len()];
    let mut remaining = budget;
    for (place, piece) in pieces.iter().enumerate() {
        let index = piece.candidate;
        let candidate = &candidates[index];
        if piece.window && blocked[index] {
            continue;
        }
        let before = held[index].clone();
        let mut joined = before.clone();
        joined.extend(piece.ranges.iter().copied());
        let after = merge_ranges(&joined);
        let listed = match describe {
            None => 0,
            Some(describe) => {
                let note = piece.note.as_ref();
                line_bytes(describe(candidate, &after).as_deref()) - line_bytes(describe(candidate, &before).as_deref()) + line_bytes(note.map(|note| note.0.as_str()))
                    - line_bytes(note.map(|note| note.1.as_str()))
            }
        };
        let cost = bytes(&held_text(candidate, &after)) - bytes(&held_text(candidate, &before)) + listed;
        if cost > remaining {
            if piece.window {
                blocked[index] = true;
            }
            continue;
        }
        remaining -= cost;
        held[index] = after;
        admitted[place] = true;
    }
    Admission { admitted, held }
}

/// A line's bytes in the facts index, its newline included; none for no line.
pub fn line_bytes(line: Option<&str>) -> i64 {
    line.map_or(0, |line| bytes(line) + 1)
}

/// Sorted, with overlapping and touching ranges merged.
pub fn merge_ranges(ranges: &[LineRange]) -> Vec<LineRange> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by_key(|range| (range.first, range.last));
    let mut merged: Vec<LineRange> = Vec::new();
    for range in sorted {
        match merged.last_mut() {
            Some(last) if range.first <= last.last + 1 => last.last = last.last.max(range.last),
            _ => merged.push(range),
        }
    }
    merged
}

/// Whether held ranges cover every line of a candidate.
pub fn holds_all(candidate: &Candidate, held: &[LineRange]) -> bool {
    held.len() == 1 && held[0].first == 1 && held[0].last >= candidate.line_count
}

/// The marker on the first line of a run of lines a file does not hold.
pub fn gap_marker(range: &LineRange) -> String {
    format!("… lines {}-{} are not in the packet; facts/INDEX.md says why", range.first, range.last)
}

/// The text a file holds: all of it once every line is held, nothing when no line is, and otherwise its held lines at
/// their own numbers up to its last held line, each run between them blank but for a marker on its first line.
pub fn held_text(candidate: &Candidate, held: &[LineRange]) -> String {
    if held.is_empty() {
        return String::new();
    }
    if holds_all(candidate, held) {
        return candidate.text.clone();
    }
    let lines: Vec<&str> = candidate.text.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut next = 1;
    for range in held {
        if range.first > next {
            out.push(gap_marker(&LineRange { first: next, last: range.first - 1 }));
            out.extend(vec![String::new(); range.first - next - 1]);
        }
        let from = (range.first - 1).min(lines.len());
        let to = range.last.min(lines.len()).max(from);
        out.extend(lines[from..to].iter().map(|line| line.to_string()));
        next = range.last + 1;
    }
    format!("{}\n", out.join("\n"))
}

/// The frozen tree file for a candidate's held lines, or nothing when it holds none.
pub fn sampled_file(candidate: &Candidate, held: &[LineRange]) -> Option<SampledFile> {
    if held.is_empty() {
        return None;
    }
    let text = held_text(candidate, held);
    if holds_all(candidate, held) {
        return Some(SampledFile { path: candidate.path.clone(), text, window: None });
    }
    let last = held[held.len() - 1].last;
    let prefix = held.len() == 1 && held[0].first == 1;
    Some(SampledFile { path: candidate.path.clone(), text, window: Some(SampledWindow { last, lines: candidate.line_count, ranges: if prefix { None } else { Some(held.to_vec()) } }) })
}

/// `1-400`, or `1-3, 10-20`.
pub fn format_ranges(ranges: &[LineRange]) -> String {
    ranges.iter().map(|range| format!("{}-{}", range.first, range.last)).collect::<Vec<_>>().join(", ")
}

/// The lines of a candidate its held ranges leave out.
pub fn missing_ranges(candidate: &Candidate, held: &[LineRange]) -> Vec<LineRange> {
    let mut missing = Vec::new();
    let mut next = 1;
    for range in held {
        if range.first > next {
            missing.push(LineRange { first: next, last: range.first - 1 });
        }
        next = range.last + 1;
    }
    if next <= candidate.line_count {
        missing.push(LineRange { first: next, last: candidate.line_count });
    }
    missing
}

/// What became of a file: dropped, sent in full, or which lines were sent and which the cap dropped.
pub fn sent_status(candidate: &Candidate, held: &[LineRange]) -> String {
    if held.is_empty() {
        return "the cap dropped it".to_string();
    }
    if holds_all(candidate, held) {
        return "sent in full".to_string();
    }
    format!("sent lines {}; the cap dropped lines {}", format_ranges(held), format_ranges(&missing_ranges(candidate, held)))
}
