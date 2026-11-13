//! The mechanical evidence check fan-out runs on every finding, the index of the frozen packet it reads, and the
//! checks a packet without code skips. The node runner's `domain/evidence.ts`, `domain/line-references.ts`, and the
//! diff-window and tree-sample helpers it uses.

use crate::architecture::{has_verdict_word, is_layer_config_path, DEPENDENCY_CHECKS};
use crate::checks::{DeclaredCheck, NEEDS_TREE_REASON};
use crate::diff_filter::parse_diff;
use crate::fan_out_result::SkippedCheck;
use crate::findings::{Evidence, FindingBody, PreVerdict};
use crate::js_regex::Regex;
use crate::manifest::{held_ranges, LineRange, PacketWindow};
use crate::recovery::line_count;
use crate::refactoring_plan::{plan_step_ids, plan_target, PLAN_CHECK_PATH};
use crate::stage_table::{evidence_rule, is_code_stage, EvidenceRule};
use crate::tickets::ticket_target;
use model_gateway::json::number_text;
use model_gateway::text::{is_js_whitespace, js_trim};
use std::collections::HashMap;
use std::sync::OnceLock;

/// The packet folder a code packet keeps its tree in.
const TREE_DIR: &str = "tree";

/// One packet file's text, as it was frozen before any critic ran.
#[derive(Clone, Debug, PartialEq)]
pub struct PacketText {
    pub path: String,
    pub kind: String,
    pub text: String,
    /// Only for a tree file a repository packet cut short.
    pub window: Option<PacketWindow>,
}

/// A packet file with its text's white space folded.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedFile {
    pub path: String,
    pub kind: String,
    pub normalised: String,
}

/// The packet as the evidence check reads it, built once for every finding of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct PacketIndex {
    pub files: Vec<IndexedFile>,
    /// How many lines each packet file has, by packet path.
    pub lines: HashMap<String, usize>,
    /// The tree files a repository packet cut short, by packet path.
    pub windows: HashMap<String, PacketWindow>,
    /// The lines at the head the diff's hunks show, by the packet path under `tree/` they are cited as.
    pub shown: HashMap<String, Vec<LineRange>>,
    /// Whether the packet holds a tree, whose file and line citations are then checked at any stage.
    pub tree: bool,
    /// At the tickets stage, the ids of the spec's tickets, which a finding's location must name.
    pub tickets: Vec<String>,
    /// At the plan stage, the ids of the plan's steps, which a finding's location must name.
    pub steps: Vec<String>,
}

/// Runs of white space count as one space, so a quote may cross a line break.
fn normalise(text: &str) -> String {
    let mut folded = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if is_js_whitespace(c) {
            in_space = true;
            continue;
        }
        if in_space {
            folded.push(' ');
            in_space = false;
        }
        folded.push(c);
    }
    if in_space {
        folded.push(' ');
    }
    js_trim(&folded).to_string()
}

pub fn index_packet(packet: &[PacketText], tickets: &[String]) -> PacketIndex {
    let mut shown: HashMap<String, Vec<LineRange>> = HashMap::new();
    for file in packet.iter().filter(|file| file.kind == "diff") {
        for (path, ranges) in diff_shown_lines(&file.text) {
            shown.insert(format!("{TREE_DIR}/{path}"), ranges);
        }
    }
    let plan = packet.iter().find(|file| file.path == PLAN_CHECK_PATH).map_or("", |file| file.text.as_str());
    PacketIndex {
        files: packet.iter().map(|file| IndexedFile { path: file.path.clone(), kind: file.kind.clone(), normalised: normalise(&file.text) }).collect(),
        lines: packet.iter().map(|file| (file.path.clone(), line_count(&file.text))).collect(),
        shown,
        steps: plan_step_ids(plan),
        tickets: tickets.to_vec(),
        tree: packet.iter().any(|file| file.kind == "tree"),
        windows: packet.iter().filter_map(|file| file.window.clone().map(|window| (file.path.clone(), window))).collect(),
    }
}

/// Sorted, with overlapping and touching ranges merged.
pub fn merge_ranges(ranges: &[LineRange]) -> Vec<LineRange> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by(|a, b| a.first.total_cmp(&b.first).then(a.last.total_cmp(&b.last)));
    let mut merged: Vec<LineRange> = Vec::new();
    for range in sorted {
        match merged.last_mut() {
            Some(last) if range.first <= last.last + 1.0 => last.last = last.last.max(range.last),
            _ => merged.push(range),
        }
    }
    merged
}

/// The lines at the head each file's hunks in a diff show, by repository path, so a citation of one resolves although
/// the tree does not hold it. A file with no lines at the head (deleted or emptied, `+0,0`) has none to clash with, so
/// its hunks show their lines at the base. Any other hunk that only removes lines shows none.
pub fn diff_shown_lines(diff: &str) -> Vec<(String, Vec<LineRange>)> {
    let mut shown: Vec<(String, Vec<LineRange>)> = Vec::new();
    for section in parse_diff(diff) {
        let Some(path) = section.path else { continue };
        let ranges = section.hunks.iter().filter_map(|hunk| {
            let side = if hunk.new.count > 0.0 {
                &hunk.new
            } else if hunk.new.start == 0.0 {
                &hunk.old
            } else {
                return None;
            };
            (side.count != 0.0).then(|| LineRange { first: side.start, last: side.start + side.count - 1.0 })
        });
        match shown.iter_mut().find(|(known, _)| *known == path) {
            Some((_, held)) => held.extend(ranges),
            None => shown.push((path, ranges.collect())),
        }
    }
    shown.into_iter().map(|(path, ranges)| (path, merge_ranges(&ranges))).collect()
}

/// A file and line, or range of lines, that a finding cites, as `path:line` or `path:first-last`.
#[derive(Clone, Debug, PartialEq)]
pub struct LineReference {
    /// The path as the critic wrote it, without a leading `./`.
    pub path: String,
    pub first: f64,
    pub last: f64,
}

fn pattern(cell: &'static OnceLock<Regex>, source: &str, flags: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::with_flags(source, flags).expect("the evidence patterns are valid"))
}

/// Every `path:line` and `path:first-last` in a location or a trace step. A URL is not a file, so
/// `https://example.com:443` cites nothing.
pub fn line_references(text: &str) -> Vec<LineReference> {
    static URL: OnceLock<Regex> = OnceLock::new();
    static REFERENCE: OnceLock<Regex> = OnceLock::new();
    static FILE: OnceLock<Regex> = OnceLock::new();
    let chars: Vec<char> = text.chars().collect();
    let mut cleaned: Vec<char> = Vec::new();
    let mut from = 0;
    for hit in pattern(&URL, r"\b[a-z][a-z0-9+.-]*:\/\/[^\s]*", "gi").find_all(&chars) {
        cleaned.extend(&chars[from..hit.start]);
        cleaned.push(' ');
        from = hit.end;
    }
    cleaned.extend(&chars[from..]);
    let reference = pattern(&REFERENCE, r"(?<![\w./@+-])((?:\.\/)?(?:[\w@+.-]+\/)*[\w@+.-]+):(\d+)(?:\s*[-–]\s*(\d+))?", "g");
    let file = pattern(&FILE, r"\/|\.[A-Za-z]\w*$", "");
    let mut found = Vec::new();
    for hit in reference.find_all(&cleaned) {
        let whole = hit.group(&cleaned, 1).unwrap_or_default();
        let path = whole.strip_prefix("./").unwrap_or(&whole).to_string();
        if file.find_at(&path.chars().collect::<Vec<_>>(), 0).is_none() {
            continue;
        }
        let first = hit.group(&cleaned, 2).and_then(|digits| digits.parse::<f64>().ok()).unwrap_or(0.0);
        let last = hit.group(&cleaned, 3).and_then(|digits| digits.parse::<f64>().ok()).unwrap_or(first);
        found.push(LineReference { first, last, path });
    }
    found
}

/// The packet paths a cited path may mean, most literal first: the packet path itself, the repository path under
/// `tree/`, a path with a diff's `a/` or `b/` prefix under `tree/`, and a document's repository path under
/// `artifact/` or `references/`, where a repository packet keeps its records.
pub fn packet_candidates(path: &str, tree_dir: &str) -> Vec<String> {
    let mut candidates = vec![path.to_string(), format!("{tree_dir}/{path}")];
    if let Some(rest) = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/")) {
        if !rest.is_empty() {
            candidates.push(format!("{tree_dir}/{rest}"));
        }
    }
    candidates.push(format!("artifact/{path}"));
    candidates.push(format!("references/{path}"));
    candidates
}

/// What a finding's evidence holds, once each quote is found in the packet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvidenceFound {
    pub items: usize,
    /// A quote found in the artifact.
    pub spec: bool,
    /// A quote found in a reference: the decision source.
    pub decision: bool,
    /// A quote found in `history/`: a commit message or an issue.
    pub history: bool,
    /// A quote found in a fact the runner computed.
    pub fact: bool,
    /// An input, failing test or trace: a concrete scenario.
    pub scenario: bool,
}

/// Where each quote of a finding was found. Returns the index of the first quote no packet file holds.
pub fn evidence_found(finding: &FindingBody, packet: &PacketIndex) -> Result<EvidenceFound, usize> {
    let mut found = EvidenceFound { items: finding.evidence.len(), ..EvidenceFound::default() };
    for (index, evidence) in finding.evidence.iter().enumerate() {
        if evidence.kind != "quote" {
            found.scenario = true;
            continue;
        }
        let quote = normalise(&evidence.value);
        let holders: Vec<&IndexedFile> = packet.files.iter().filter(|file| file.normalised.contains(&quote)).collect();
        if holders.is_empty() {
            return Err(index);
        }
        let held_by = |kind: &str| holders.iter().any(|file| file.kind == kind);
        found.spec |= held_by("artifact");
        found.decision |= held_by("reference");
        found.fact |= held_by("fact");
        found.history |= held_by("history");
    }
    Ok(found)
}

fn verdict(reason: &str, code: &str, kind: &str) -> PreVerdict {
    PreVerdict { reason: reason.to_string(), reason_code: code.to_string(), verdict: kind.to_string() }
}

fn fabricated(reason: &str) -> PreVerdict {
    verdict(reason, "fabricated-evidence", "rejected")
}

/// The mechanical check fan-out runs on every finding before the orchestrator sees it. Every quote must occur in the
/// packet, and at a code stage every file and line the location or a trace step cites must be in the packet's frozen
/// copy, or the finding is rejected as fabricated evidence. Then the stage's evidence rule, or the one the role of the
/// critic that raised the finding takes at that stage, decides whether the finding has anything checkable; if not, it
/// is a hint. Returns nothing when the orchestrator must adjudicate the finding itself.
pub fn check_evidence(finding: &FindingBody, stage: &str, packet: &PacketIndex, role: Option<&str>) -> Option<PreVerdict> {
    let found = match evidence_found(finding, packet) {
        Ok(found) => found,
        Err(index) => return Some(fabricated(&format!("evidence {} quotes text that is not in the packet", index + 1))),
    };
    if is_code_stage(stage) || packet.tree {
        if let Some(missing) = missing_line(finding, packet) {
            return Some(fabricated(&missing));
        }
    }
    let rule = evidence_rule(stage, role);
    // A candidate's rejections are mechanical, not hints.
    if rule == EvidenceRule::Recovery {
        return recovery_verdict(finding, &found, packet);
    }
    let hint = if found.items == 0 { Some("no-evidence") } else { apply_rule(rule, finding, &found, packet) };
    hint.map(|code| verdict(hint_reason(code), code, "hint"))
}

fn apply_rule(rule: EvidenceRule, finding: &FindingBody, found: &EvidenceFound, packet: &PacketIndex) -> Option<&'static str> {
    match rule {
        EvidenceRule::Adrs => adr_rule(finding, found, packet),
        EvidenceRule::Architecture => architecture_rule(finding, found, packet),
        // A recovery finding never reaches here: `check_evidence` judged it before.
        EvidenceRule::Code | EvidenceRule::Recovery => code_rule(&finding.evidence),
        EvidenceRule::Decisions => decisions_rule(&finding.category, found),
        EvidenceRule::Plan => plan_rule(finding, found, packet),
        EvidenceRule::Records => (!found.spec).then_some("no-record-quote"),
        EvidenceRule::Spec => spec_rule(&finding.category, found),
        EvidenceRule::Tickets => ticket_rule(&finding.location, &packet.tickets),
    }
}

/// Why the first file-and-line citation of the location, or of a trace step, names a file the packet does not hold or
/// a line its file does not have. A line a hunk of the diff shows is held, whether or not the tree holds it.
fn missing_line(finding: &FindingBody, packet: &PacketIndex) -> Option<String> {
    let mut sources = vec![(finding.location.clone(), "the location".to_string())];
    for (index, evidence) in finding.evidence.iter().enumerate() {
        if evidence.kind == "trace" {
            sources.push((evidence.value.clone(), format!("evidence {}", index + 1)));
        }
    }
    for (text, wher) in sources {
        for reference in line_references(&text) {
            let held_path = packet_candidates(&reference.path, TREE_DIR).into_iter().find(|candidate| packet.lines.contains_key(candidate) || packet.shown.contains_key(candidate));
            let Some(path) = held_path else { return Some(format!("{wher} cites {}, which is not in the packet", reference.path)) };
            let lines = packet.lines.get(&path).map(|count| *count as f64);
            let window = packet.windows.get(&path);
            let in_tree = match (lines, window) {
                (None, _) => Vec::new(),
                (Some(count), None) => vec![LineRange { first: 1.0, last: count }],
                (Some(_), Some(window)) => held_ranges(window),
            };
            let mut all = in_tree;
            all.extend(packet.shown.get(&path).cloned().unwrap_or_default());
            let held = merge_ranges(&all);
            let Some(outside) = first_unheld(&reference, &held) else { continue };
            return Some(match (lines, window) {
                (Some(count), None) => format!("{wher} cites line {} of {path}, which has {} lines", number_text(outside), number_text(count)),
                _ => {
                    let spans: Vec<String> = held.iter().map(|range| format!("{}-{}", number_text(range.first), number_text(range.last))).collect();
                    let of = window.map_or(String::new(), |window| format!(" of {}", number_text(window.lines)));
                    format!("{wher} cites line {} of {path}, outside the lines the packet holds ({}{of})", number_text(outside), spans.join(", "))
                }
            });
        }
    }
    None
}

/// The first line a citation names that the file does not hold: its first line, its last, or for a range across a run
/// of lines left out, the first line of that run.
fn first_unheld(reference: &LineReference, held: &[LineRange]) -> Option<f64> {
    let holding = |line: f64| held.iter().position(|range| range.first <= line && line <= range.last);
    let Some(first) = holding(reference.first) else { return Some(reference.first) };
    let Some(last) = holding(reference.last) else { return Some(reference.last) };
    (first != last).then(|| held[first].last + 1.0)
}

/// The decision-recovery stage's rule: every finding is a candidate, an `undocumented-decision`, and is rejected
/// mechanically when it cites no file of `tree/`, cites only one, or gives a "because" with neither a quote of
/// `history/` nor the plain statement "no rationale found".
fn recovery_verdict(finding: &FindingBody, found: &EvidenceFound, packet: &PacketIndex) -> Option<PreVerdict> {
    if finding.category != "undocumented-decision" {
        return Some(verdict(hint_reason("not-a-candidate"), "not-a-candidate", "hint"));
    }
    let sites = trace_sites(&finding.evidence, packet);
    if sites.is_empty() {
        return Some(verdict("cites no file of tree/ in a trace: the decision is not traced to the code", "no-code-trace", "rejected"));
    }
    if sites.len() == 1 {
        return Some(verdict("traces to one site only: one site may be an accident, so it needs two", "single-site", "rejected"));
    }
    if !found.history && !says_no_rationale(&finding.claim) {
        return Some(verdict("gives no reason from history/ and does not say that no rationale was found", "invented-rationale", "rejected"));
    }
    None
}

fn says_no_rationale(claim: &str) -> bool {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(&CELL, r"\bno rationale found\b", "i").find_at(&claim.chars().collect::<Vec<_>>(), 0).is_some()
}

/// The distinct files of `tree/` the trace steps of a finding cite, by packet path.
fn trace_sites(evidence: &[Evidence], packet: &PacketIndex) -> Vec<String> {
    let mut sites: Vec<String> = Vec::new();
    for item in evidence.iter().filter(|item| item.kind == "trace") {
        for reference in line_references(&item.value) {
            let found = packet_candidates(&reference.path, TREE_DIR).into_iter().find(|candidate| packet.lines.contains_key(candidate));
            if let Some(path) = found.filter(|path| path.starts_with("tree/")) {
                if !sites.contains(&path) {
                    sites.push(path);
                }
            }
        }
    }
    sites
}

/// The architecture-review stage's rule. An unasked question needs only a trace that cites a file and line. Any other
/// finding needs a rule the project declared or a fact the runner computed, or a trace showing the consequence; a
/// finding whose check assumes a layer rule needs one in the packet; a verdict word needs a fact or a quoted decision
/// beside it; and a fix is named by the observable that would show it worked.
fn architecture_rule(finding: &FindingBody, found: &EvidenceFound, packet: &PacketIndex) -> Option<&'static str> {
    let traced = code_rule(&finding.evidence).is_none() && finding.evidence.iter().any(|item| item.kind == "trace");
    if finding.category == "unasked-question" {
        return (!traced).then_some("no-trace");
    }
    let layered = finding.check.as_deref().is_some_and(|check| DEPENDENCY_CHECKS.contains(&check));
    if layered && !packet.files.iter().any(|file| is_layer_config_path(&file.path)) {
        return Some("no-layer-config");
    }
    if has_verdict_word(&finding.claim) && !found.fact && !found.decision {
        return Some("verdict-word");
    }
    if !found.decision && !found.fact && !traced {
        return Some("no-rule-or-consequence");
    }
    static CELL: OnceLock<Regex> = OnceLock::new();
    let observable = pattern(&CELL, r"\bobservable\b", "i").find_at(&finding.suggestion.clone().unwrap_or_default().chars().collect::<Vec<_>>(), 0).is_some();
    (!observable).then_some("no-observable")
}

/// The plan stage's rule: a finding breaks one named rule at one step, so its location names both; it quotes the step
/// in the plan, and beside the step either a quote of an ADR or a fact the runner computed, or a trace.
fn plan_rule(finding: &FindingBody, found: &EvidenceFound, packet: &PacketIndex) -> Option<&'static str> {
    let (rule, step) = plan_target(&finding.location, &packet.steps);
    if step.is_none() {
        return Some("no-plan-step");
    }
    if rule.is_none() {
        return Some("no-plan-rule");
    }
    if !found.spec {
        return Some("no-step-quote");
    }
    let anchored = found.decision || found.fact || finding.evidence.iter().any(|item| item.kind == "trace");
    (!anchored).then_some("no-plan-anchor")
}

/// The code-stage rule: a code finding needs a failing test, or a trace that cites a file and line.
fn code_rule(evidence: &[Evidence]) -> Option<&'static str> {
    let checkable = evidence.iter().any(|item| item.kind == "failing-test" || (item.kind == "trace" && !line_references(&item.value).is_empty()));
    (!checkable).then_some("no-test-or-trace")
}

/// The tickets-stage rule: a finding names the rule it breaks and the ticket it is about. An uncovered requirement is
/// about the spec, so it needs no ticket.
fn ticket_rule(location: &str, tickets: &[String]) -> Option<&'static str> {
    let (rule, ticket) = ticket_target(location, tickets);
    match rule {
        None => Some("no-ticket-rule"),
        Some("uncovered-requirement") => None,
        Some(_) => ticket.is_none().then_some("no-ticket-id"),
    }
}

/// The spec-stage rule: an omission or invention needs a quote from the spec and one from the decision source; any
/// other finding needs a spec quote and either a decision quote or a concrete scenario.
fn spec_rule(category: &str, found: &EvidenceFound) -> Option<&'static str> {
    if !found.spec {
        return Some("no-spec-quote");
    }
    if matches!(category, "omission" | "invention") {
        return (!found.decision).then_some("no-decision-quote");
    }
    (!(found.decision || found.scenario)).then_some("no-decision-or-scenario")
}

/// The decisions-stage rule: a gap is an unasked question or a contradiction, quotes the decisions under review or
/// the decision docs beside them, and gives a concrete scenario.
fn decisions_rule(category: &str, found: &EvidenceFound) -> Option<&'static str> {
    if !matches!(category, "unasked-question" | "contradiction") {
        return Some("not-a-decision-gap");
    }
    if !found.spec && !found.decision {
        return Some("no-decision-quote");
    }
    (!found.scenario).then_some("no-scenario")
}

/// The ADR review's rule: every finding quotes a record under review; a contradiction quotes two distinct files; and
/// a drift or stale finding needs code under `tree/`, so a documents-only packet skips it.
fn adr_rule(finding: &FindingBody, found: &EvidenceFound, packet: &PacketIndex) -> Option<&'static str> {
    if !packet.tree && matches!(finding.category.as_str(), "drift" | "stale") {
        return Some("needs-tree");
    }
    if !found.spec {
        return Some("no-record-quote");
    }
    if finding.category == "contradiction" && !quotes_two_files(&finding.evidence, packet) {
        return Some("one-file-contradiction");
    }
    (finding.category == "unasked-question" && !found.scenario).then_some("no-scenario")
}

/// Whether two of the quotes can be told apart by file: some record or reference holds the first and not the second,
/// and another holds the second and not the first.
fn quotes_two_files(evidence: &[Evidence], packet: &PacketIndex) -> bool {
    let documents: Vec<&IndexedFile> = packet.files.iter().filter(|file| file.kind == "artifact" || file.kind == "reference").collect();
    let holders: Vec<Vec<&str>> = evidence
        .iter()
        .filter(|item| item.kind == "quote")
        .map(|item| {
            let quote = normalise(&item.value);
            documents.iter().filter(|file| file.normalised.contains(&quote)).map(|file| file.path.as_str()).collect()
        })
        .collect();
    let apart = |first: &[&str], second: &[&str]| first.iter().any(|path| !second.contains(path));
    holders.iter().enumerate().any(|(i, first)| holders.iter().enumerate().any(|(j, second)| i != j && apart(first, second) && apart(second, first)))
}

/// Every check a run's critics could not run because the packet holds no code under `tree/`: the categories the
/// stage's evidence rule takes only with code, then each check marked "Needs `tree/`" in the role and lens files the
/// critics' prompts declare, by the file that declares it, once each.
pub fn skipped_checks(stage: &str, tree: bool, sources: &[(String, Vec<DeclaredCheck>)]) -> Vec<SkippedCheck> {
    if tree {
        return Vec::new();
    }
    let mut skipped: Vec<SkippedCheck> = Vec::new();
    if evidence_rule(stage, None) == EvidenceRule::Adrs {
        skipped.push(SkippedCheck {
            about: Some("a decision that names code, config or a command, checked against the code".to_string()),
            check: "drift".to_string(),
            from: "category".to_string(),
        });
        skipped.push(SkippedCheck {
            about: Some("an assumption, revisit condition or promised consequence, checked against the code".to_string()),
            check: "stale".to_string(),
            from: "category".to_string(),
        });
    }
    for (from, checks) in sources {
        for check in checks {
            let known = skipped.iter().any(|entry| entry.check == check.id);
            if check.needs_tree && !known {
                skipped.push(SkippedCheck { about: None, check: check.id.clone(), from: from.clone() });
            }
        }
    }
    skipped
}

fn hint_reason(code: &str) -> &'static str {
    match code {
        "needs-tree" => NEEDS_TREE_REASON,
        "no-record-quote" => "no quote from a record under review",
        "one-file-contradiction" => "a contradiction quotes one file; it needs a quote from each of two files",
        "no-decision-or-scenario" => "no quote from a decision source and no concrete scenario",
        "no-decision-quote" => "no quote from a decision source",
        "no-evidence" => "no evidence to check",
        "no-scenario" => "no concrete scenario",
        "no-spec-quote" => "no quote from the spec",
        "no-test-or-trace" => "no failing test and no trace with a file and line",
        "no-ticket-id" => "the location names no ticket of the spec",
        "no-ticket-rule" => "the location names no ticket rule",
        "no-layer-config" => "the check assumes a layer rule and the packet holds no layer config: ask whether the layering is intended, as an unasked question with a trace",
        "no-trace" => "an unasked question needs a trace that cites a file and line",
        "no-plan-step" => "the location names no step of the plan",
        "no-plan-rule" => "the location names no plan rule",
        "no-step-quote" => "no quote of the step from the plan",
        "no-plan-anchor" => "beside the step quote, no ADR quote, no fact row and no trace",
        "no-observable" => "no observable named: say what would show a fix worked",
        "no-rule-or-consequence" => "taste: neither a declared rule, a fact nor a traced consequence backs it",
        "verdict-word" => "a verdict word with no fact and no quoted decision: state the number, or quote the decision that gives the threshold",
        "not-a-candidate" => "not an undocumented-decision candidate: this stage recovers decisions only",
        _ => "neither an unasked question nor a contradiction",
    }
}
