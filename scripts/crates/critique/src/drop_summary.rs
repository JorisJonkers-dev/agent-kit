//! How the facts index records the files a packet leaves out that no critic is likely to look for by name: counted
//! per directory and reason, with the bytes the packet does not hold, so the index stays small however many files are
//! left out. The node runner's `domain/drop-summary.ts`.

use crate::code_facts::code_unit_order;
use crate::tree_sample::{bytes, held_text, holds_all, line_bytes, Candidate, LineRange};
use std::collections::HashSet;

/// One file left out, in whole or in part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drop {
    pub path: String,
    /// What became of it, worded for many files: `the cap dropped them`.
    pub reason: String,
    /// The bytes of it the packet does not hold.
    pub bytes: i64,
}

/// The most lines a summary takes; past it, directories are counted by a shorter prefix.
pub const DROP_SUMMARY_LINES: usize = 24;

pub const CAP_DROPPED: &str = "the cap dropped them";

/// The reason of a file the cap cut short, which holds whole windows from its first line.
pub fn cap_cut_short(window_lines: usize) -> String {
    format!("the cap cut them short: each holds only its first lines, in windows of {window_lines} lines")
}

/// What the cap left out of a candidate holding these lines, or nothing when it holds every line.
pub fn cap_drop(candidate: &Candidate, held: &[LineRange], window_lines: usize) -> Option<Drop> {
    if holds_all(candidate, held) {
        return None;
    }
    Some(Drop {
        bytes: bytes(&candidate.text) - bytes(&held_text(candidate, held)),
        path: candidate.path.clone(),
        reason: if held.is_empty() { CAP_DROPPED.to_string() } else { cap_cut_short(window_lines) },
    })
}

/// Every way the cap may leave a candidate out, at the most bytes each can take: dropped, or cut short when it has
/// more than one window.
pub fn possible_drops(candidate: &Candidate, window_lines: usize) -> Vec<Drop> {
    let whole = Drop { bytes: bytes(&candidate.text), path: candidate.path.clone(), reason: CAP_DROPPED.to_string() };
    if candidate.line_count > window_lines {
        vec![whole.clone(), Drop { reason: cap_cut_short(window_lines), ..whole }]
    } else {
        vec![whole]
    }
}

fn folders_of(path: &str) -> Vec<&str> {
    let mut parts: Vec<&str> = path.split('/').collect();
    parts.pop();
    parts
}

/// The facts index lines for these drops: one per directory and reason, with the count and the bytes, in directory
/// order. When that passes `DROP_SUMMARY_LINES`, a file more than `k` folders deep is counted under its first `k`
/// folders as `a/b/**`, for the largest `k` that fits.
pub fn summarise_drops(drops: &[Drop]) -> Vec<String> {
    let deepest = drops.iter().map(|drop| folders_of(&drop.path).len()).max().unwrap_or(0);
    for k in (1..=deepest + 1).rev() {
        let lines = group_lines(drops, k);
        if lines.len() <= DROP_SUMMARY_LINES {
            return lines;
        }
    }
    group_lines(drops, 0)
}

/// The most bytes `summarise_drops` can take, newlines included, for any of these possible drops: each file once,
/// with one of the reasons it may get, its bytes at most those given. A packet reserves it under its cap before it
/// knows what it drops.
pub fn drop_summary_bound(possible: &[Drop]) -> i64 {
    let mut exact: Vec<(String, String, HashSet<&str>, i64)> = Vec::new();
    for drop in possible {
        let key = key_of(&drop.path, usize::MAX);
        match exact.iter_mut().find(|group| group.0 == key && group.1 == drop.reason) {
            Some(group) => {
                group.2.insert(&drop.path);
                group.3 += drop.bytes;
            }
            None => exact.push((key, drop.reason.clone(), HashSet::from([drop.path.as_str()]), drop.bytes)),
        }
    }
    if exact.len() <= DROP_SUMMARY_LINES {
        return exact.iter().map(|group| line_bytes(Some(&line(&group.0, group.2.len(), group.3, &group.1)))).sum();
    }
    let mut most: Vec<(&str, i64)> = Vec::new();
    for drop in possible {
        match most.iter_mut().find(|entry| entry.0 == drop.path) {
            Some(entry) => entry.1 = entry.1.max(drop.bytes),
            None => most.push((&drop.path, drop.bytes)),
        }
    }
    let total: i64 = most.iter().map(|entry| entry.1).sum();
    let key = exact.iter().map(|group| bytes(&group.0) + 2).max().unwrap_or(0);
    let reasons: HashSet<&str> = possible.iter().map(|drop| drop.reason.as_str()).collect();
    let reason = reasons.iter().map(|reason| bytes(reason)).max().unwrap_or(0);
    let widest = line_bytes(Some(&line(&"k".repeat(key as usize), most.len(), total, &"r".repeat(reason as usize))));
    DROP_SUMMARY_LINES.max(reasons.len()) as i64 * widest
}

/// The directory a file is counted under when files more than `k` folders deep are counted by their first `k`.
fn key_of(path: &str, k: usize) -> String {
    let folders = folders_of(path);
    if folders.len() < k {
        return if folders.is_empty() { "./".to_string() } else { format!("{}/", folders.join("/")) };
    }
    if k == 0 {
        "**".to_string()
    } else {
        format!("{}/**", folders[..k].join("/"))
    }
}

fn group_lines(drops: &[Drop], k: usize) -> Vec<String> {
    let mut groups: Vec<(String, String, usize, i64)> = Vec::new();
    for drop in drops {
        let key = key_of(&drop.path, k);
        match groups.iter_mut().find(|group| group.0 == key && group.1 == drop.reason) {
            Some(group) => {
                group.2 += 1;
                group.3 += drop.bytes;
            }
            None => groups.push((key, drop.reason.clone(), 1, drop.bytes)),
        }
    }
    groups.sort_by(|a, b| code_unit_order(&a.0, &b.0).then_with(|| code_unit_order(&a.1, &b.1)));
    groups.iter().map(|group| line(&group.0, group.2, group.3, &group.1)).collect()
}

fn line(key: &str, count: usize, size: i64, reason: &str) -> String {
    format!("- `{key}` ({count} {}, {size} bytes): {reason}", if count == 1 { "file" } else { "files" })
}
