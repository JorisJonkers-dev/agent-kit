//! What of the repository's tree a packet sends, chosen by rule and never by a model: the files the artifact names,
//! then the top hotspots, then each module's entry file, then the files on each import cycle, each file once, in its
//! first tier. The node runner's `sampleRepository` in `domain/repository-packet.ts`.

use crate::code_facts::code_unit_order;
use crate::drop_summary::{cap_drop, drop_summary_bound, possible_drops, summarise_drops};
use crate::recovery::line_count;
use crate::repository::refusal;
use crate::repository_packet::{PacketLimits, TrackedFile};
use crate::tracked_tree::Excluded;
use crate::tree_sample::{admit_pieces, bytes, candidate_of, holds_all, line_bytes, sampled_file, sent_status, windows_of, Candidate, LineRange, Piece, SampledFile};
use model_gateway::json::{number_text, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// One file the runner computed into `facts/`, and what it holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactFile {
    pub name: String,
    pub holds: String,
    pub text: String,
}

/// What prepare knows about the repository beyond its tree. The scans that compute facts fill this in; until a scan
/// has run, its field is `None` and the sample tier that needs it is skipped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RepositoryFacts {
    pub files: Vec<FactFile>,
    /// Paths, hottest first.
    pub hotspots: Option<Vec<String>>,
    /// The files of each import cycle.
    pub cycles: Option<Vec<Vec<String>>>,
    /// Each path's rank by churn, 1 the most changed.
    pub churn_ranks: Option<HashMap<String, f64>>,
}

pub struct SampleRequest<'a> {
    pub commit: &'a str,
    pub limits: &'a PacketLimits,
    /// Every tracked path not excluded, sorted.
    pub tracked: &'a [TrackedFile],
    /// How many tracked paths each exclusion left out.
    pub excluded: Excluded,
    /// The repository paths already sent as documents, which the tree never repeats.
    pub documents: &'a [String],
    /// The bytes the documents take.
    pub document_bytes: i64,
    /// The text that names files: the artifact's.
    pub artifact_text: &'a str,
    pub facts: &'a RepositoryFacts,
    /// Paths sampled after every other tier, in this order.
    pub rest: Option<&'a [String]>,
    /// What code-nav's repository map did to the sample's order: a sentence for the facts index.
    pub ranking_note: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySample {
    pub manifest: String,
    pub index: String,
    pub facts: Vec<FactFile>,
    pub tree: Vec<SampledFile>,
    /// How many sampled files the cap left out in whole or in part.
    pub dropped: usize,
}

/// Why a file was sampled, in priority order.
const TIERS: [&str; 5] = ["named", "hotspot", "entry", "cycle", "rest"];
/// The tiers whose files the facts index lists by name; it counts the others' drops per directory.
const LISTED_TIERS: [&str; 2] = ["named", "hotspot"];

/// The file names that make a file its module's entry or interface.
const ENTRY_FILES: [&str; 17] = [
    "index.ts", "index.tsx", "index.js", "index.jsx", "index.mjs", "index.cjs", "main.ts", "__init__.py", "__main__.py", "main.py", "mod.rs", "lib.rs", "main.rs", "main.go", "Main.kt", "main.kt",
    "Main.java",
];

/// The packet path of the tree's manifest.
pub const MANIFEST_PATH: &str = "tree/MANIFEST.tsv";
pub const FACTS_INDEX_PATH: &str = "facts/INDEX.md";

const MANIFEST_HEADER: &str = "path\tlines\tchurn_rank";

fn listed(tier: &str) -> bool {
    LISTED_TIERS.contains(&tier)
}

/// Chooses what of the repository's tree a packet sends. Files are cut into windows of the routing's window size; the
/// windows go in tier by tier, each tier's first windows before its second, while they fit under the cap beside the
/// documents, the manifest, the facts and the facts index. A file's windows always run from its first line, so each
/// line keeps its number, and a window that does not fit drops the rest of its file. Every drop is recorded in the
/// facts index. Refused (`Err`) when the documents, the manifest and the facts alone pass the cap.
pub fn sample_repository(request: &SampleRequest) -> Result<RepositorySample, Value> {
    let limits = request.limits;
    let window = limits.window_lines as usize;
    let facts = admit_facts(&request.facts.files, limits.facts_cap_bytes as i64);
    let candidates = sample_candidates(request);
    let sampled: HashSet<&str> = candidates.iter().map(|candidate| candidate.path.as_str()).collect();
    let manifest = render_manifest(request, &sampled);
    let counted: Vec<&Candidate> = candidates.iter().filter(|candidate| !listed(candidate.tier)).collect();
    let reserved = drop_summary_bound(&counted.iter().flat_map(|candidate| possible_drops(candidate, window)).collect::<Vec<_>>());
    let none: Vec<Vec<LineRange>> = vec![Vec::new(); candidates.len()];
    let fixed = request.document_bytes
        + bytes(&manifest.text)
        + facts.iter().map(|file| bytes(&file.text)).sum::<i64>()
        + bytes(&render_index(request, &manifest, &facts, &candidates, &none, &[]))
        + reserved;
    if fixed > limits.cap_bytes as i64 {
        return Err(refusal(
            "packet-over-cap",
            &format!(
                "the documents, {MANIFEST_PATH} and facts/ come to {fixed} bytes, past the packet cap of {}; review fewer records or raise packet.cap_bytes in routing",
                number_text(limits.cap_bytes)
            ),
        ));
    }
    let admission = admit_pieces(&candidates, &window_order(&candidates, window), limits.cap_bytes as i64 - fixed, Some(&sample_line_if_listed));
    let held = admission.held;
    let drops = summarise_drops(&counted.iter().filter_map(|candidate| cap_drop(candidate, &held[index_of(&candidates, candidate)], window)).collect::<Vec<_>>());
    let tree = candidates.iter().enumerate().filter_map(|(index, candidate)| sampled_file(candidate, &held[index])).collect();
    Ok(RepositorySample {
        dropped: candidates.iter().enumerate().filter(|(index, candidate)| !holds_all(candidate, &held[*index])).count(),
        index: render_index(request, &manifest, &facts, &candidates, &held, &drops),
        facts,
        manifest: manifest.text,
        tree,
    })
}

fn index_of(candidates: &[Candidate], candidate: &Candidate) -> usize {
    candidates.iter().position(|other| std::ptr::eq(other, candidate)).unwrap_or_default()
}

fn sample_line_if_listed(candidate: &Candidate, held: &[LineRange]) -> Option<String> {
    listed(candidate.tier).then(|| sample_line(candidate, held))
}

/// Every candidate's windows, tier by tier, each tier's first windows before its second.
fn window_order(candidates: &[Candidate], size: usize) -> Vec<Piece> {
    let mut order: Vec<(usize, usize, usize, Piece)> = Vec::new();
    for (position, candidate) in candidates.iter().enumerate() {
        for (window, piece) in windows_of(position, candidate, size).into_iter().enumerate() {
            order.push((tier_index(candidate.tier), window, position, piece));
        }
    }
    order.sort_by_key(|entry| (entry.0, entry.1, entry.2));
    order.into_iter().map(|entry| entry.3).collect()
}

fn tier_index(tier: &str) -> usize {
    TIERS.iter().position(|candidate| *candidate == tier).unwrap_or_default()
}

/// The fact files, in order, that fit under the facts cap.
fn admit_facts(files: &[FactFile], cap: i64) -> Vec<FactFile> {
    let mut remaining = cap;
    let mut admitted = Vec::new();
    for file in files {
        let size = bytes(&file.text);
        if size <= remaining {
            remaining -= size;
            admitted.push(file.clone());
        }
    }
    admitted
}

/// The files each tier samples, each file once, in its first tier.
fn sample_candidates(request: &SampleRequest) -> Vec<Candidate> {
    let documents: HashSet<&str> = request.documents.iter().map(String::as_str).collect();
    let mut texts: Vec<(&str, &str)> = Vec::new();
    for file in request.tracked {
        let Some(text) = file.text.as_deref() else { continue };
        if documents.contains(file.path.as_str()) || file.path == "MANIFEST.tsv" {
            continue;
        }
        match texts.iter_mut().find(|entry| entry.0 == file.path) {
            Some(entry) => entry.1 = text,
            None => texts.push((&file.path, text)),
        }
    }
    let facts = request.facts;
    let tiers: [Vec<String>; 5] = [
        artifact_named_paths(request.artifact_text),
        facts.hotspots.clone().unwrap_or_default(),
        texts.iter().filter(|entry| ENTRY_FILES.contains(&entry.0.rsplit('/').next().unwrap_or_default())).map(|entry| entry.0.to_string()).collect(),
        facts.cycles.iter().flatten().flatten().cloned().collect(),
        request.rest.map(<[String]>::to_vec).unwrap_or_default(),
    ];
    let mut seen: HashSet<&str> = HashSet::new();
    let mut candidates = Vec::new();
    for (tier, paths) in TIERS.iter().zip(&tiers) {
        for path in paths {
            let Some(entry) = texts.iter().find(|entry| entry.0 == path) else { continue };
            if seen.insert(entry.0) {
                candidates.push(candidate_of(path, tier, entry.1));
            }
        }
    }
    candidates
}

/// The path-like words of a text, in the order they first appear, without a `./` or `tree/` prefix or a sentence's
/// full stop: `text.match(/[\w@+.\-/]+/g)` with each word's prefix and trailing dots taken off.
pub fn artifact_named_paths(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '@' | '+' | '.' | '-' | '/')))
        .filter(|word| !word.is_empty())
        .map(|word| word.strip_prefix("./").or_else(|| word.strip_prefix("tree/")).unwrap_or(word).trim_end_matches('.').to_string())
        .collect()
}

/// The manifest's text, and how many of its least-ranked paths it counts per directory instead of listing.
struct RenderedManifest {
    text: String,
    collapsed: usize,
    /// Whether it counts the collapsed paths per directory; false when even the counts would pass its cap.
    counted: bool,
}

/// The order a manifest drops paths in: a path the sample chose last, then those without a churn rank before the
/// least churned, then the deepest, then the last in path order.
fn drop_order(sampled: &HashSet<&str>, churn: Option<&HashMap<String, f64>>, a: &str, b: &str) -> Ordering {
    let rank = |path: &str| (usize::from(sampled.contains(path)), -churn.and_then(|ranks| ranks.get(path)).copied().unwrap_or(f64::INFINITY), -(path.split('/').count() as i64));
    let (x, y) = (rank(a), rank(b));
    x.0.cmp(&y.0).then_with(|| x.1.partial_cmp(&y.1).unwrap_or(Ordering::Equal)).then_with(|| x.2.cmp(&y.2)).then_with(|| code_unit_order(b, a))
}

/// Every tracked path with its line count and churn rank, in path order. Past the manifest cap, its least-ranked
/// paths are counted per directory at its end instead, one at a time until it fits; when even every path counted does
/// not fit, the counts are left out too and the manifest says so.
fn render_manifest(request: &SampleRequest, sampled: &HashSet<&str>) -> RenderedManifest {
    let churn = request.facts.churn_ranks.as_ref();
    let mut rows: Vec<(String, String)> = Vec::new();
    for file in request.tracked {
        let lines = file.text.as_deref().map_or_else(|| "-".to_string(), |text| line_count(text).to_string());
        let rank = churn.and_then(|ranks| ranks.get(&file.path)).map_or_else(|| "-".to_string(), |rank| number_text(*rank));
        let row = format!("{}\t{lines}\t{rank}", file.path);
        match rows.iter_mut().find(|entry| entry.0 == file.path) {
            Some(entry) => entry.1 = row,
            None => rows.push((file.path.clone(), row)),
        }
    }
    let cap = request.limits.manifest_cap_bytes as i64;
    let mut row_bytes = line_bytes(Some(MANIFEST_HEADER)) + rows.iter().map(|row| line_bytes(Some(&row.1))).sum::<i64>();
    let mut count_bytes = 0;
    let mut collapsed = 0;
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut order: Vec<String> = rows.iter().map(|row| row.0.clone()).collect();
    order.sort_by(|a, b| drop_order(sampled, churn, a, b));
    let note = |collapsed: usize| if collapsed == 0 { 0 } else { line_bytes(Some(&collapsed_line(collapsed, cap))) };
    for path in &order {
        if row_bytes + count_bytes + note(collapsed) <= cap {
            break;
        }
        let directory = path.rfind('/').map_or_else(|| "./".to_string(), |at| format!("{}/", &path[..at]));
        let held = counts.iter().position(|entry| entry.0 == directory);
        let count = held.map_or(0, |at| counts[at].1);
        count_bytes += line_bytes(Some(&count_line(&directory, count + 1))) - if count == 0 { 0 } else { line_bytes(Some(&count_line(&directory, count))) };
        let row = rows.iter().position(|row| &row.0 == path).unwrap_or_default();
        row_bytes -= line_bytes(Some(&rows[row].1));
        match held {
            Some(at) => counts[at].1 += 1,
            None => counts.push((directory, 1)),
        }
        rows.remove(row);
        collapsed += 1;
    }
    let counted = row_bytes + count_bytes + note(collapsed) <= cap;
    let tail: Vec<String> = if collapsed == 0 {
        Vec::new()
    } else if counted {
        counts.sort_by(|a, b| code_unit_order(&a.0, &b.0));
        std::iter::once(collapsed_line(collapsed, cap)).chain(counts.iter().map(|(directory, count)| count_line(directory, *count))).collect()
    } else {
        vec![uncounted_line(collapsed, cap)]
    };
    let lines: Vec<&str> = std::iter::once(MANIFEST_HEADER).chain(rows.iter().map(|row| row.1.as_str())).chain(tail.iter().map(String::as_str)).collect();
    RenderedManifest { collapsed, counted, text: format!("{}\n", lines.join("\n")) }
}

fn collapsed_line(collapsed: usize, cap: i64) -> String {
    format!("# collapsed: past the manifest cap of {cap} bytes, its {collapsed} least-ranked paths are counted per directory below, as directory and paths")
}

/// The collapsed line when even the per-directory counts would pass the cap, so they are left out too.
fn uncounted_line(collapsed: usize, cap: i64) -> String {
    format!("# collapsed: past the manifest cap of {cap} bytes, its {collapsed} least-ranked paths are left out, and so are their per-directory counts, which would not fit")
}

fn count_line(directory: &str, count: usize) -> String {
    format!("# {directory}\t{count}")
}

/// What became of one sampled file, holding these lines.
fn sample_line(candidate: &Candidate, held: &[LineRange]) -> String {
    format!("- `{}` ({}, {} lines): {}", candidate.path, candidate.tier, candidate.line_count, sent_status(candidate, held))
}

fn render_index(request: &SampleRequest, manifest: &RenderedManifest, facts: &[FactFile], candidates: &[Candidate], held: &[Vec<LineRange>], drops: &[String]) -> String {
    let limits = request.limits;
    let fact_lines: Vec<String> = request
        .facts
        .files
        .iter()
        .map(|file| format!("- `facts/{}`: {}{}", file.name, file.holds, if facts.contains(file) { "" } else { "; the facts cap dropped it" }))
        .collect();
    let Excluded { generated, lock, vendored } = request.excluded;
    let mut skipped = Vec::new();
    if request.facts.hotspots.is_none() {
        skipped.push("No hotspot facts yet, so no hotspot was sampled.".to_string());
    }
    if request.facts.cycles.is_none() {
        skipped.push("No import facts yet, so no import cycle was sampled.".to_string());
    }
    let samples: Vec<String> = candidates.iter().enumerate().filter(|(_, candidate)| listed(candidate.tier)).map(|(index, candidate)| sample_line(candidate, &held[index])).collect();
    let collapsed = if manifest.collapsed == 0 {
        String::new()
    } else {
        format!(
            " Past its cap of {} bytes, it {} its {} least-ranked paths{}.",
            number_text(limits.manifest_cap_bytes),
            if manifest.counted { "counts" } else { "leaves out" },
            manifest.collapsed,
            if manifest.counted { " per directory at its end instead of listing them" } else { ", and their per-directory counts, which would not fit" }
        )
    };
    let mut lines: Vec<String> = vec![
        "# Facts".to_string(),
        String::new(),
        format!("What the runner computed from the repository at commit `{}`. Facts are settled: cite them, never recompute them. A quote from a fact is a fact, never a recorded decision.", request.commit),
        String::new(),
        "## Files".to_string(),
        String::new(),
        format!("- `{MANIFEST_PATH}`: every tracked path but the excluded ones, with its line count (`-` for a binary file) and its churn rank (`-` while no churn facts exist). Columns: `path`, `lines`, `churn_rank`.{collapsed}"),
    ];
    lines.extend(fact_lines.iter().cloned());
    if fact_lines.is_empty() {
        lines.extend([String::new(), "No other facts are computed yet.".to_string()]);
    }
    lines.extend([
        String::new(),
        "## Limits".to_string(),
        String::new(),
        format!("Packet cap {} bytes; facts cap {} bytes; windows of {} lines.", number_text(limits.cap_bytes), number_text(limits.facts_cap_bytes), number_text(limits.window_lines)),
        String::new(),
    ]);
    if let Some(note) = request.ranking_note {
        lines.extend(["## Ranking".to_string(), String::new(), note.to_string(), String::new()]);
    }
    lines.extend([
        "## Excluded".to_string(),
        String::new(),
        format!("Never sent, and not in `{MANIFEST_PATH}`: {vendored} vendored, {generated} generated and {lock} lock files."),
        String::new(),
        "## Sample".to_string(),
        String::new(),
        format!("Files sent under `tree/`, in the order they were chosen: the files the artifact names, the top hotspots, each module's entry file, then the files on each import cycle. A longer file is sent in windows of {} lines from its first line, at its own line numbers. The files the artifact names and the top hotspots are listed by name; the cap's drops of entry files and files on a cycle are counted per directory with the bytes left out (`dir/**` counts every file under it), and one neither listed nor counted was sent in full.", number_text(limits.window_lines)),
        String::new(),
    ]);
    if !skipped.is_empty() {
        lines.extend(skipped);
        lines.push(String::new());
    }
    if candidates.is_empty() {
        lines.push("Nothing was sampled.".to_string());
    } else {
        lines.extend(samples);
        lines.extend(drops.iter().cloned());
    }
    lines.push(String::new());
    lines.join("\n")
}
