//! What of the tree a code packet sends, chosen by rule and never by a model: first every context slice, then the
//! files the change touched, then the files that import them, then the rest of the tree, in windows from each file's
//! first line, while they fit under the cap beside the diff, the tickets, the spec, the repo map and the facts index.
//! The node runner's `domain/code-sample.ts`.

use crate::code_facts::code_unit_order;
use crate::context_slices::ContextSlice;
use crate::diff_filter::LeftOutFile;
use crate::diff_window::{diff_drop_line, window_diff, DiffDrop, WindowedDiff};
use crate::drop_summary::{cap_drop, drop_summary_bound, possible_drops, summarise_drops};
use crate::ranking::rank_index;
use crate::repo_map::{RenderedRepoMap, REPO_MAP_PATH};
use crate::repository::refusal;
use crate::repository_packet::{PacketLimits, TrackedFile};
use crate::repository_sample::{artifact_named_paths, FACTS_INDEX_PATH};
use crate::tracked_tree::Excluded;
use crate::tree_sample::{admit_pieces, bytes, candidate_of, holds_all, line_bytes, sampled_file, sent_status, windows_of, Candidate, LineRange, Piece, SampledFile};
use model_gateway::json::{number_text, Value};
use std::collections::HashSet;

/// Why a file is in a code packet's tree, in priority order: the change touched it, it imports a file the change
/// touched, it holds a slice (a caller or a callee), or it is the rest of the tree.
const CODE_TIERS: [&str; 4] = ["changed", "importer", "related", "rest"];

/// How much of the room the documents leave a windowed diff may take; the tree keeps the rest.
const DIFF_SHARE: f64 = 0.5;
/// How much of the room the documents leave the facts index's lists of names may take when, listing every drop it
/// names, it would not fit: each list then ends with a line that counts what it leaves unlisted.
const INDEX_SHARE: f64 = 0.25;

const DIFF_WINDOW_NOTE: &str = "`diff/change.diff` did not fit beside the rest of the packet, so it holds the hunks that fit, in windows by file: the changed source files, then the tests, then the docs, each group by changed lines, most first. Each hunk keeps its `@@` line, so a line it shows is cited at its own number as `tree/<path>:<line>`. A line only a left-out hunk shows can be read and cited only where `tree/` holds it, as the Tree section says. The cap left out these hunks, at their lines at the commit under review, or as `base lines` for a hunk that only removes lines:";

pub struct CodeSampleRequest<'a> {
    pub commit: &'a str,
    pub limits: &'a PacketLimits,
    /// Every tracked path not excluded, sorted, with its text, or `None` for a binary file.
    pub tracked: &'a [TrackedFile],
    /// How many tracked paths each exclusion left out of the tree.
    pub excluded: Excluded,
    /// The files whose hunks the diff leaves out.
    pub left_out: &'a [LeftOutFile],
    /// The diff, without those hunks, before the cap windows it.
    pub diff: &'a str,
    /// The bytes the tickets and the spec take.
    pub document_bytes: i64,
    pub repo_map: &'a RenderedRepoMap,
    /// The repository paths the change touched that the tree holds.
    pub changed: &'a [String],
    /// The repository paths that import a changed file.
    pub importers: &'a [String],
    pub slices: &'a [ContextSlice],
    /// The text of the tickets and the spec, which, with the diff's, names files.
    pub named_text: &'a str,
    /// The paths code-nav's repository map ranked, most referenced first; the importers and the rest of the tree go in
    /// that order.
    pub ranked: Option<&'a [String]>,
    /// What the map did to the sample's order, for the facts index; absent when no map was asked for.
    pub ranking_note: Option<&'a str>,
}

pub struct CodeSample {
    /// The diff as sent: whole, or in windows when it alone does not fit.
    pub diff: String,
    pub index: String,
    pub tree: Vec<SampledFile>,
    /// The slices the cap let in, in the order they were chosen.
    pub slices: Vec<ContextSlice>,
    /// How many files of the tree the cap left out in whole or in part.
    pub dropped: usize,
}

/// Chooses what of the tree a code packet sends. A diff that does not fit beside the rest is sent in windows by file,
/// taking at most half the room the documents leave, so the tree keeps room for the slices. Every drop is recorded in
/// the facts index. Refused (`Err`) only when the tickets, the spec, the repo map and the facts index's own text pass
/// the cap.
pub fn sample_code_tree(request: &CodeSampleRequest) -> Result<CodeSample, Value> {
    let limits = request.limits;
    let window = limits.window_lines as usize;
    let cap = limits.cap_bytes as i64;
    let candidates = code_candidates(request);
    let named: HashSet<String> = artifact_named_paths(&format!("{}\n{}", request.diff, request.named_text)).into_iter().collect();
    let counted: Vec<&Candidate> = candidates.iter().filter(|candidate| candidate.tier == "rest" && !named.contains(&candidate.path)).collect();
    let counted_paths: HashSet<&str> = counted.iter().map(|candidate| candidate.path.as_str()).collect();
    let tree_line = |candidate: &Candidate, held: &[LineRange]| -> Option<String> {
        if counted_paths.contains(candidate.path.as_str()) || (candidate.tier == "rest" && holds_all(candidate, held)) {
            None
        } else {
            Some(file_line(candidate, held))
        }
    };
    let reserved = drop_summary_bound(&counted.iter().flat_map(|candidate| possible_drops(candidate, window)).collect::<Vec<_>>());
    let unsent: Vec<String> = candidates.iter().filter_map(|candidate| tree_line(candidate, &[])).collect();
    let mut sliced: Vec<(Piece, &ContextSlice)> = Vec::new();
    for (origin, slice) in request.slices.iter().enumerate() {
        if let Some(index) = candidates.iter().rposition(|candidate| candidate.path == slice.path) {
            let note = Some((slice_line(slice, true), slice_line(slice, false)));
            sliced.push((Piece { candidate: index, ranges: vec![LineRange { first: slice.first, last: slice.last, origin: Some(origin) }], window: false, note }, slice));
        }
    }
    let slice_pieces: Vec<Piece> = sliced.iter().map(|entry| entry.0.clone()).collect();
    let mut pieces = slice_pieces.clone();
    pieces.extend(window_order(&candidates, window));
    let none = vec![false; slice_pieces.len()];
    let documents = request.document_bytes + bytes(&request.repo_map.text);
    let fixed_with = |diff: &WindowedDiff| documents + bytes(&diff.diff) + bytes(&render_index(request, &unsent, &[], &slice_pieces, &none, &diff.dropped, None, true)) + reserved;
    let full = WindowedDiff { diff: request.diff.to_string(), dropped: Vec::new() };
    let mut diff = full.clone();
    // Set only when the facts index, listing every drop it names, would not fit: it is then capped on its own.
    let mut index_cap: Option<i64> = None;
    if fixed_with(&full) > cap {
        let empty = window_diff(request.diff, 0);
        let fixed_before = fixed_with(&empty);
        if fixed_before <= cap {
            diff = window_diff(request.diff, share(cap - fixed_before, DIFF_SHARE));
        } else {
            // Every list collapsed, each count line naming a cap no shorter than the one the index gets.
            let collapsed = bytes(&render_index(request, &unsent, &[], &slice_pieces, &none, &empty.dropped, Some(cap), false));
            let floor = documents + bytes(&empty.diff) + collapsed + reserved;
            if floor > cap {
                return Err(refusal(
                    "packet-over-cap",
                    &format!(
                        "the tickets, the spec, {REPO_MAP_PATH} and {FACTS_INDEX_PATH} come to {floor} bytes before any hunk of the diff, past the packet cap of {}; review a smaller change or raise packet.cap_bytes in routing",
                        number_text(limits.cap_bytes)
                    ),
                ));
            }
            // The lists get their share of the room beside the reserve for the counted drops.
            let room = cap - documents - bytes(&empty.diff) - reserved;
            let lists_cap = collapsed.max(share(room, INDEX_SHARE));
            index_cap = Some(lists_cap + reserved);
            let windowed = window_diff(request.diff, share(room - lists_cap, DIFF_SHARE));
            diff = if windowed.dropped.is_empty() { full } else { windowed };
        }
    }
    let admission = match index_cap {
        None => admit_pieces(&candidates, &pieces, cap - fixed_with(&diff), Some(&tree_line)),
        Some(index_cap) => admit_pieces(&candidates, &pieces, cap - documents - bytes(&diff.diff) - index_cap, None),
    };
    let held_of = |index: usize| admission.held[index].as_slice();
    let listed: Vec<String> = candidates.iter().enumerate().filter_map(|(index, candidate)| tree_line(candidate, held_of(index))).collect();
    let drops: Vec<_> = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| counted_paths.contains(candidate.path.as_str()))
        .filter_map(|(index, candidate)| cap_drop(candidate, held_of(index), window))
        .collect();
    let counts = summarise_drops(&drops);
    let tree = candidates.iter().enumerate().filter_map(|(index, candidate)| sampled_file(candidate, held_of(index))).collect();
    let admitted = &admission.admitted[..slice_pieces.len()];
    Ok(CodeSample {
        dropped: candidates.iter().enumerate().filter(|(index, candidate)| !holds_all(candidate, held_of(*index))).count(),
        index: render_index(request, &listed, &counts, &slice_pieces, admitted, &diff.dropped, index_cap, true),
        slices: sliced.iter().zip(admitted).filter(|(_, admitted)| **admitted).map(|(entry, _)| entry.1.clone()).collect(),
        diff: diff.diff,
        tree,
    })
}

/// `Math.floor(room * share)`.
fn share(room: i64, share: f64) -> i64 {
    (room as f64 * share).floor() as i64
}

/// Every text file of the tree, in tier order, each tier in path order.
fn code_candidates(request: &CodeSampleRequest) -> Vec<Candidate> {
    let changed: HashSet<&str> = request.changed.iter().map(String::as_str).collect();
    let importers: HashSet<&str> = request.importers.iter().map(String::as_str).collect();
    let related: HashSet<&str> = request.slices.iter().map(|slice| slice.path.as_str()).collect();
    let tier_of = |path: &str| -> &'static str {
        if changed.contains(path) {
            "changed"
        } else if importers.contains(path) {
            "importer"
        } else if related.contains(path) {
            "related"
        } else {
            "rest"
        }
    };
    let ranked: Vec<String> = request.ranked.map(<[String]>::to_vec).unwrap_or_default();
    let place = rank_index(&ranked);
    // The ranking orders the importers and the rest of the tree; the files the change touched keep path order.
    let rank_of = |candidate: &Candidate| -> f64 {
        if candidate.tier == "importer" || candidate.tier == "rest" {
            place(&candidate.path).map_or(f64::INFINITY, |at| at as f64)
        } else {
            0.0
        }
    };
    let mut candidates: Vec<Candidate> = request
        .tracked
        .iter()
        .filter_map(|file| {
            let text = file.text.as_deref()?;
            (format!("tree/{}", file.path) != REPO_MAP_PATH).then(|| candidate_of(&file.path, tier_of(&file.path), text))
        })
        .collect();
    candidates.sort_by(|a, b| {
        tier_index(a.tier)
            .cmp(&tier_index(b.tier))
            .then_with(|| rank_of(a).partial_cmp(&rank_of(b)).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| code_unit_order(&a.path, &b.path))
    });
    candidates
}

fn tier_index(tier: &str) -> usize {
    CODE_TIERS.iter().position(|candidate| *candidate == tier).unwrap_or_default()
}

/// Each tier's first windows before its second; a file that only holds a slice is the rest of the tree here.
fn window_order(candidates: &[Candidate], size: usize) -> Vec<Piece> {
    let mut order: Vec<(usize, usize, usize, Piece)> = Vec::new();
    for (position, candidate) in candidates.iter().enumerate() {
        for (window, piece) in windows_of(position, candidate, size).into_iter().enumerate() {
            order.push((tier_index(candidate.tier).min(tier_index("related")), window, position, piece));
        }
    }
    order.sort_by_key(|entry| (entry.0, entry.1, entry.2));
    order.into_iter().map(|entry| entry.3).collect()
}

/// A file's own line in the facts index.
fn file_line(candidate: &Candidate, held: &[LineRange]) -> String {
    format!("- `{}` ({}, {} lines): {}", candidate.path, candidate.tier, candidate.line_count, sent_status(candidate, held))
}

/// What a slice holds, as its line in the prompt and the facts index say it.
pub fn slice_reason(slice: &ContextSlice) -> String {
    let symbol = slice.symbol.as_ref().map(|symbol| format!("`{symbol}`"));
    let of = format!("`{}`", slice.of.as_deref().unwrap_or_default());
    let named = symbol.clone().unwrap_or_default();
    match slice.reason {
        "callee" => format!("{named}, which {of} calls"),
        "caller" => format!("{named}, which calls {of}"),
        _ => match symbol {
            None => "lines the change touches".to_string(),
            Some(symbol) => format!("{symbol}, which the change touches"),
        },
    }
}

/// The packet citation of a slice: `tree/<path>:<first>-<last>`.
pub fn slice_citation(slice: &ContextSlice) -> String {
    format!("tree/{}:{}-{}", slice.path, slice.first, slice.last)
}

fn slice_line(slice: &ContextSlice, admitted: bool) -> String {
    format!("- `{}`: {}: {}", slice_citation(slice), slice_reason(slice), if admitted { "sent" } else { "the cap dropped it" })
}

/// The facts index. `tree` holds the tree's files listed by name, which a cap on the index may collapse like its other
/// lists; `counts` holds the per-directory counts of the other drops, which it always keeps, as the cap reserved room
/// for them. With a cap and no `fill`, every list is collapsed to its count line: the least the index can take.
#[allow(clippy::too_many_arguments)]
fn render_index(
    request: &CodeSampleRequest,
    tree: &[String],
    counts: &[String],
    slice_pieces: &[Piece],
    admitted: &[bool],
    diff_dropped: &[DiffDrop],
    cap_bytes: Option<i64>,
    fill: bool,
) -> String {
    let left_out: Vec<String> = request.left_out.iter().map(|file| format!("- `{}`: {}; {} changed lines", file.path, file.reason, number_text(file.changed_lines))).collect();
    let dropped: Vec<String> = diff_dropped.iter().map(diff_drop_line).collect();
    let slices: Vec<String> = slice_pieces.iter().zip(admitted).map(|(piece, admitted)| piece.note.as_ref().map_or(String::new(), |note| if *admitted { note.0.clone() } else { note.1.clone() })).collect();
    let lists = [left_out, dropped, slices, tree.to_vec()];
    let Some(cap) = cap_bytes else { return index_text(request, &lists, counts) };
    let mut kept = [0usize; 4];
    let cut = |kept: &[usize; 4]| -> [Vec<String>; 4] { std::array::from_fn(|i| collapse(&lists[i], kept[i], cap)) };
    let mut remaining = if fill { cap - bytes(&index_text(request, &cut(&kept), counts)) } else { 0 };
    for (i, list) in lists.iter().enumerate() {
        for line in list {
            if line_bytes(Some(line)) > remaining {
                break;
            }
            remaining -= line_bytes(Some(line));
            kept[i] += 1;
        }
    }
    index_text(request, &cut(&kept), counts)
}

/// A list's first `kept` lines, then, when that leaves some out, one line that counts them.
fn collapse(list: &[String], kept: usize, cap_bytes: i64) -> Vec<String> {
    if kept >= list.len() {
        return list.to_vec();
    }
    let mut shown = list[..kept].to_vec();
    shown.push(format!("- … and {} more, which this index's cap of {cap_bytes} bytes leaves unlisted", list.len() - kept));
    shown
}

/// The facts index around its four lists (the files left out of the diff, its hunks the cap left out, the slices and
/// the tree's named files) and the tree's counted drops.
fn index_text(request: &CodeSampleRequest, [left_out, diff_dropped, slices, tree]: &[Vec<String>; 4], counts: &[String]) -> String {
    let limits = request.limits;
    let repo_map = request.repo_map;
    let Excluded { generated, lock, vendored } = request.excluded;
    let binary = request.tracked.iter().filter(|file| file.text.is_none()).count();
    let map_doc = if repo_map.source == "code-nav" {
        "the files and symbols code-nav's map ranks, by how often their symbols are referenced"
    } else {
        "every source file's top-level symbols and their signatures, ranked by how many other files reference them"
    };
    let map_dropped = if repo_map.dropped == 0 { String::new() } else { format!("; its cap left out the {} least referenced files", repo_map.dropped) };
    let mut lines: Vec<String> = vec![
        "# Facts".to_string(),
        String::new(),
        format!("What the runner computed from the repository at commit `{}`. Facts are settled: cite them, never recompute them. A quote from a fact is a fact, never a recorded decision.", request.commit),
        String::new(),
        "## Files".to_string(),
        String::new(),
        format!("- `{REPO_MAP_PATH}`: {map_doc}{map_dropped}."),
        String::new(),
        "## Limits".to_string(),
        String::new(),
        format!("Packet cap {} bytes; repo map cap {} bytes; windows of {} lines.", number_text(limits.cap_bytes), number_text(limits.repo_map_cap_bytes), number_text(limits.window_lines)),
        String::new(),
    ];
    if let Some(note) = request.ranking_note {
        lines.extend(["## Ranking".to_string(), String::new(), note.to_string(), String::new()]);
    }
    lines.extend([
        "## Excluded".to_string(),
        String::new(),
        format!("Never sent under `tree/`: {vendored} vendored, {generated} generated and {lock} lock files, and {binary} binary files."),
    ]);
    if !left_out.is_empty() {
        lines.extend([String::new(), "Left out of `diff/change.diff`, each with one line in its place:".to_string(), String::new()]);
        lines.extend(left_out.iter().cloned());
    }
    if !diff_dropped.is_empty() {
        lines.extend([String::new(), "## Diff".to_string(), String::new(), DIFF_WINDOW_NOTE.to_string(), String::new()]);
        lines.extend(diff_dropped.iter().cloned());
    }
    lines.extend([
        String::new(),
        "## Context slices".to_string(),
        String::new(),
        "The code around the change, at its own line numbers: each changed function or block, then the symbols it calls, then the functions that call it.".to_string(),
        String::new(),
    ]);
    if slices.is_empty() {
        lines.push("The change touches no text file the tree holds.".to_string());
    } else {
        lines.extend(slices.iter().cloned());
    }
    lines.extend([
        String::new(),
        "## Tree".to_string(),
        String::new(),
        format!("Files under `tree/`, in the order the cap let them in: the slices above, then the files the change touched, then the files that import them, then the rest of the tree. A file goes in windows of {} lines from its first line. A file cut short holds the lines listed here at their own numbers; a run of lines it does not hold is blank but for a marker on its first line. Of the rest of the tree, a file the diff, a ticket or the spec names is listed by name, the cap's other drops are counted per directory with the bytes left out (`dir/**` counts every file under it), and a file neither listed nor counted was sent in full.", number_text(limits.window_lines)),
        String::new(),
    ]);
    lines.extend(tree.iter().cloned());
    lines.extend(counts.iter().cloned());
    lines.push(String::new());
    lines.join("\n")
}
