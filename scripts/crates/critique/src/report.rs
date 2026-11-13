//! The two documents a recorded run leaves: the full report, and the compact summary the orchestrator reads. The node
//! runner's `domain/report.ts`.

use crate::adjudication::{families_of, is_dissent, AdjudicatedRun};
use crate::critic::CriticRun;
use crate::findings::RunFinding;
use crate::gateway_ledger::gateway_ledger_warning;
use crate::manifest::DepthSignal;
use crate::refute::RefuteRecord;
use crate::roster::DroppedCritic;
use crate::stage_table::{adrs_packet, architecture_evidence, do_not_start};
use crate::verdicts::{
    candidate_questions, count_verdicts, degraded_ticket_review, open_critical_questions, unfixed_critical, AdjudicatedFinding, Verdict, VERDICTS,
};
use model_gateway::caps::to_fixed;
use model_gateway::json::number_text;
use model_gateway::text::js_slice;

/// About 1.5k tokens: the most a summary may add to the planning conversation.
pub const SUMMARY_MAX_CHARS: usize = 6000;
const CLAIM_MAX_CHARS: usize = 160;
/// What a critic that found nothing says it checked; the report keeps all of it.
const CHECKED_MAX_CHARS: usize = 300;
/// One critic's minor findings, by id; the report keeps each in full.
const MINOR_MAX_CHARS: usize = 300;
/// How many dangling blockers the ticket check line names; the checker's report in the packet has all of them.
const DANGLING_SHOWN: usize = 5;

type Lines = Vec<String>;

fn lines<const N: usize>(items: [&str; N]) -> Lines {
    items.iter().map(|item| (*item).to_string()).collect()
}

/// The compact summary, always in the same shape: header, critics, accepted findings, questions for the owner, minor
/// findings, dissent, and the report path. When it would exceed its budget, the longest list loses items from its end
/// and says how many more the report holds.
pub fn render_summary(run: &AdjudicatedRun, report_path: &str) -> String {
    let serious: Vec<&AdjudicatedFinding> = run.findings.iter().filter(|f| f.finding.body.severity != "minor").collect();
    let candidates = candidate_questions(&run.stage, &run.findings);
    let line_of = |finding: &AdjudicatedFinding, tail: Option<String>| finding_line(run, &finding.finding, tail);
    // The order of the lists is the order a tie between the longest is broken in: accepted, dissent, deferred, questions.
    let mut lists: [Vec<String>; 4] = [
        serious.iter().filter(|f| f.verdict.verdict == "accepted").map(|f| line_of(f, accepted_tail(&f.verdict))).collect(),
        run.findings.iter().filter(|f| is_dissent(&run.critics, f)).map(|f| line_of(f, Some(format!("Rejected: {}", f.verdict.reason.as_deref().unwrap_or(""))))).collect(),
        candidates.iter().flat_map(|c| &c.deferred).map(|position| line_of(&run.findings[*position], None)).collect(),
        match &candidates {
            Some(c) => c.asked.iter().map(|position| &run.findings[*position]).map(|f| line_of(f, question_tail(&f.verdict))).collect(),
            None => serious.iter().filter(|f| f.verdict.verdict == "needs-human").map(|f| line_of(f, question_tail(&f.verdict))).collect(),
        },
    ];
    let mut hidden = [0usize; 4];
    let render = |lists: &[Vec<String>; 4], hidden: &[usize; 4]| -> String {
        let mut out: Lines = Vec::new();
        out.extend(gate_lines(run));
        out.extend(violation_lines(run));
        out.push(format!("# Critique: {}, run {}", run.stage, run.run_id));
        out.push(String::new());
        out.extend(degraded_lines(run));
        out.push(depth_line(run));
        out.extend(round_lines(run));
        out.push(verdict_line(run));
        out.extend(skipped_line(run));
        out.extend(ticket_check_lines(run));
        out.extend(plan_warning_lines(run));
        out.extend(ledger_warning_lines(run));
        out.extend(do_not_start_lines(run));
        out.push(String::new());
        out.extend(lines(["## Critics", ""]));
        out.extend(run.critics.iter().map(|critic| format!("- {}", critic_line(run, critic, CHECKED_MAX_CHARS))));
        out.extend(run.dropped.iter().map(dropped_line));
        out.extend(lines(["", "## Accepted", ""]));
        out.extend(bullets(&lists[0], hidden[0]));
        out.extend(lines(["", "## Questions for you", ""]));
        out.extend(bullets(&lists[3], hidden[3]));
        out.push(String::new());
        if candidates.is_some() {
            out.extend(lines(["## Deferred", ""]));
            out.extend(bullets(&lists[2], hidden[2]));
            out.push(String::new());
        }
        out.extend(lines(["## Minor", ""]));
        out.extend(bullets(&minor_lines(run), 0));
        out.extend(lines(["", "## Dissent", ""]));
        out.extend(bullets(&lists[1], hidden[1]));
        out.extend(lines(["", "## Report", ""]));
        out.push(report_path.to_string());
        out.extend(do_not_commit_lines(run));
        out.push(String::new());
        out.join("\n")
    };
    let mut summary = render(&lists, &hidden);
    while summary.encode_utf16().count() > SUMMARY_MAX_CHARS {
        // The first of the longest non-empty lists, in the lists' order.
        let mut longest: Option<usize> = None;
        for (index, list) in lists.iter().enumerate() {
            if !list.is_empty() && longest.is_none_or(|best| list.len() > lists[best].len()) {
                longest = Some(index);
            }
        }
        let Some(longest) = longest else { break };
        lists[longest].pop();
        hidden[longest] += 1;
        summary = render(&lists, &hidden);
    }
    summary
}

/// The full report: every critic, every finding with its evidence and verdict, and dissent.
pub fn render_report(run: &AdjudicatedRun) -> String {
    let dissent: Vec<&AdjudicatedFinding> = run.findings.iter().filter(|f| is_dissent(&run.critics, f)).collect();
    let mut out: Lines = Vec::new();
    out.extend(gate_lines(run));
    out.extend(violation_lines(run));
    out.push(format!("# Critique report: {}, run {}", run.stage, run.run_id));
    out.push(String::new());
    out.extend(degraded_lines(run));
    out.push(format!("Repository: {}", run.repository));
    out.push(depth_line(run));
    out.push(verdict_line(run));
    out.extend(ticket_check_lines(run));
    out.push(String::new());
    out.extend(gate_section(run));
    out.extend(skipped_section(run));
    out.extend(lines(["## Critics", ""]));
    for critic in &run.critics {
        out.push(format!("### {}", critic.critic_id));
        out.push(String::new());
        out.push(format!("- {}", critic_line(run, critic, usize::MAX)));
        out.push(format!("- Role {}, lenses {}", critic.role, critic.lenses.join(", ")));
        out.push(format!("- Exit code {}, {} ms", critic.exit_code.map_or("null".to_string(), number_text), number_text(critic.duration_ms)));
        if let Some(detail) = &critic.detail {
            out.push(format!("- Detail: {detail}"));
        }
        if let Some(tools) = &critic.tools {
            out.push(format!("- Tools: {}", tools.join(", ")));
        }
        if let Some(failure) = &critic.search_failure {
            out.push(format!("- Search failure: {failure}"));
        }
        let prompts = [
            ("contract", &critic.prompt_hashes.contract),
            ("preamble", &critic.prompt_hashes.preamble),
            ("role", &critic.prompt_hashes.role),
            ("schema", &critic.prompt_hashes.schema),
        ];
        out.push(format!("- Prompt hashes: {}", hash_list(prompts.iter().map(|(name, hash)| (*name, hash.as_str())))));
        out.push(format!("- Lens hashes: {}", hash_list(critic.lens_hashes.iter().map(|(name, hash)| (name.as_str(), hash.as_str())))));
        out.push(String::new());
    }
    out.extend(run.dropped.iter().map(dropped_line));
    out.extend(lines(["## Findings", ""]));
    if run.findings.is_empty() {
        out.extend(lines(["none", ""]));
    } else {
        out.extend(run.findings.iter().flat_map(|f| finding_section(run, f)));
    }
    out.extend(architecture_section(run));
    out.extend(refute_section(run.refutes.as_deref()));
    out.extend(deferred_section(run));
    out.extend(lines(["## Dissent", ""]));
    if dissent.is_empty() {
        out.push("none".to_string());
    } else {
        out.extend(dissent.iter().map(|f| format!("- {}: {}", f.finding.id, f.verdict.reason.as_deref().unwrap_or(""))));
    }
    out.push(String::new());
    out.join("\n")
}

/// An architecture review's accepted findings as cited candidates, in the order raised: each with its lines, its
/// evidence and the observable that would show a fix worked. A recovered decision is never among them.
fn architecture_section(run: &AdjudicatedRun) -> Lines {
    if !architecture_evidence(&run.stage) {
        return Vec::new();
    }
    let accepted: Vec<&RunFinding> =
        run.findings.iter().filter(|f| f.verdict.verdict == "accepted" && f.finding.body.category != "undocumented-decision").map(|f| &f.finding).collect();
    let mut out = lines(["## Architecture candidates", ""]);
    if accepted.is_empty() {
        out.extend(lines(["none", ""]));
        return out;
    }
    for finding in accepted {
        let body = &finding.body;
        out.push(format!("### {} [{}] {}", finding.id, body.severity, body.location));
        out.push(String::new());
        out.push(body.claim.clone());
        out.push(String::new());
        out.extend(evidence_lines(finding));
        out.push(format!("- {}", body.suggestion.as_deref().unwrap_or("Observable: none named")));
        out.push(String::new());
    }
    out
}

fn evidence_lines(finding: &RunFinding) -> Lines {
    finding
        .body
        .evidence
        .iter()
        .map(|e| format!("- {}{}: {}", e.kind, e.file.as_ref().map_or(String::new(), |file| format!(" ({file})")), e.value))
        .collect()
}

/// The candidates a recovery run did not ask about, past its cap: each stays in this report and in the run directory
/// until the owner is asked, and writes nothing to the decision log.
fn deferred_section(run: &AdjudicatedRun) -> Lines {
    let Some(candidates) = candidate_questions(&run.stage, &run.findings) else { return Vec::new() };
    let mut out = lines(["## Deferred candidates", ""]);
    if candidates.deferred.is_empty() {
        out.push("none".to_string());
    }
    for position in &candidates.deferred {
        let f = &run.findings[*position].finding;
        out.push(format!("- {} [{}] {}: {}", f.id, f.body.severity, f.body.location, clip(&f.body.claim, CLAIM_MAX_CHARS)));
    }
    out.push(String::new());
    out
}

/// The refute round, when the run had one: each accepted critical finding's outcome and who refuted it.
fn refute_section(refutes: Option<&[RefuteRecord]>) -> Lines {
    let Some(refutes) = refutes else { return Vec::new() };
    let mut out = lines(["## Refute round", ""]);
    if refutes.is_empty() {
        out.push("no accepted critical finding".to_string());
    }
    out.extend(refutes.iter().map(refute_line));
    out.push(String::new());
    out
}

fn refute_line(result: &RefuteRecord) -> String {
    let by = result.refuter.as_ref().map_or(String::new(), |refuter| format!(" by {} ({}, {}, {})", refuter.critic_id, refuter.client, refuter.model, refuter.family));
    let reason = result.reason_code.as_ref().map_or(String::new(), |code| format!(" ({code})"));
    let answered = match (&result.answer, result.outcome.as_str()) {
        (Some(answer), "stands") => format!(", which answered {answer}"),
        _ => String::new(),
    };
    let text = result.reply.as_ref().map(|reply| &reply.claim).or(result.detail.as_ref());
    let severity = result.severity.as_ref().map_or(String::new(), |severity| format!(" to {severity}"));
    format!("- {}: {}{severity}{reason}{by}{answered}{}", result.finding_id, result.outcome, text.map_or(String::new(), |text| format!(": {text}")))
}

/// A feature review's summary and report open with the gate line, so a script can read it.
fn gate_lines(run: &AdjudicatedRun) -> Lines {
    run.gate.as_ref().map_or(Vec::new(), |gate| vec![gate.line.clone(), String::new()])
}

/// The head the gate holds for, and each open critical follow-up that holds it.
fn gate_section(run: &AdjudicatedRun) -> Lines {
    let Some(gate) = &run.gate else { return Vec::new() };
    let open: Lines = gate
        .open_critical
        .iter()
        .map(|follow_up| {
            let from = if follow_up.run_id == run.run_id { String::new() } else { format!(" of run {}", follow_up.run_id) };
            format!("- {} ({}), for {}{from}", follow_up.ticket, follow_up.reference, follow_up.finding_id)
        })
        .collect();
    let mut out = lines(["## Gate", ""]);
    out.push(format!("{} for head {} (base {})", gate.line, gate.head_commit, gate.base_commit));
    out.extend(lines(["", "Open critical follow-ups:", ""]));
    if open.is_empty() {
        out.push("none".to_string());
    }
    out.extend(open);
    out.push(String::new());
    out
}

/// A critic that changed its packet or the working tree makes its critic client unsafe, so the summary and the report
/// open with that, one line per such critic, before anything else.
fn violation_lines(run: &AdjudicatedRun) -> Lines {
    let violators: Vec<&CriticRun> = run.critics.iter().filter(|critic| critic.status == "violated-readonly").collect();
    if violators.is_empty() {
        return Vec::new();
    }
    let mut out: Lines = violators
        .iter()
        .map(|critic| {
            format!(
                "> **Warning: read-only violated** by critic client {} ({}). Its findings were discarded and the packet restored. Do not rely on this client until its read-only settings are fixed.",
                critic.client, critic.critic_id
            )
        })
        .collect();
    out.push(String::new());
    out
}

/// A stage where no critic reviewed anything says so first.
fn degraded_lines(run: &AdjudicatedRun) -> Lines {
    if run.degraded { lines(["degraded: no critique ran", ""]) } else { Vec::new() }
}

fn round_lines(run: &AdjudicatedRun) -> Lines {
    if run.round == 2.0 { vec![format!("Round 2, after run {}", run.previous_run_id.as_deref().unwrap_or(""))] } else { Vec::new() }
}

/// The model gateway's warning for each Vibe plan window at or above its warning ratio, in the gateway's own words.
fn plan_warning_lines(run: &AdjudicatedRun) -> Lines {
    run.plan_warnings.iter().flatten().map(|warning| format!("Vibe plan: {warning}")).collect()
}

/// A run the gateway's ledger would not take: the owner is told the run is in critique's own ledger only.
fn ledger_warning_lines(run: &AdjudicatedRun) -> Lines {
    run.ledger_warnings.iter().flatten().map(|warning| gateway_ledger_warning(warning)).collect()
}

fn do_not_start_lines(run: &AdjudicatedRun) -> Lines {
    let Some(next) = do_not_start(&run.stage) else { return Vec::new() };
    let open: Vec<&str> = open_critical_questions(&run.findings).iter().map(|f| f.finding.id.as_str()).collect();
    let which = if open.len() == 1 { format!("critical question {} is", open.join("")) } else { format!("critical questions {} are", open.join(", ")) };
    let mut out = Lines::new();
    if run.ticket_check.as_ref().is_some_and(|check| check.status == "failed") {
        out.push(format!("Do not start {next}: the tickets hold a dependency cycle."));
    }
    if !open.is_empty() {
        out.push(format!("Do not start {next}: {which} unanswered."));
    }
    out
}

/// A ticket review whose accepted critical finding is not fixed yet ends its summary with "do not commit". So does one
/// no critic reported in: nobody reviewed the change.
fn do_not_commit_lines(run: &AdjudicatedRun) -> Lines {
    if degraded_ticket_review(&run.stage, run.degraded) {
        return lines(["", "No critic reported, so this review is degraded and the commit stays blocked: do not commit."]);
    }
    let ids: Vec<&str> = unfixed_critical(&run.stage, &run.findings).iter().map(|f| f.finding.id.as_str()).collect();
    if ids.is_empty() {
        return Vec::new();
    }
    let which = if ids.len() == 1 { format!("Critical finding {} is", ids.join("")) } else { format!("Critical findings {} are", ids.join(", ")) };
    vec![String::new(), format!("{which} accepted and not fixed: do not commit.")]
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The ticket checker's findings in one line. A dangling blocker may be a ticket of another spec, so it is something to
/// check, not a defect.
fn ticket_check_lines(run: &AdjudicatedRun) -> Lines {
    let Some(check) = &run.ticket_check else { return Vec::new() };
    let tickets = plural(check.ticket_ids.len(), "ticket", "tickets");
    let cycles = if check.cycles.is_empty() {
        "no cycle".to_string()
    } else {
        let named: Vec<String> = check.cycles.iter().map(|cycle| cycle.iter().chain(cycle.first()).cloned().collect::<Vec<_>>().join(" → ")).collect();
        format!("{} {}", if check.cycles.len() == 1 { "cycle" } else { "cycles" }, named.join(", "))
    };
    let dangling = &check.dangling_blockers;
    let mut shown: Vec<String> = dangling.iter().take(DANGLING_SHOWN).map(|d| format!("{} blocked by {}", d.ticket, d.blocker)).collect();
    if dangling.len() > DANGLING_SHOWN {
        shown.push(format!("{} more in the packet's check/tickets.json", dangling.len() - DANGLING_SHOWN));
    }
    let blockers = if dangling.is_empty() {
        "no dangling blocker".to_string()
    } else {
        format!("{} to check ({})", plural(dangling.len(), "dangling blocker", "dangling blockers"), shown.join(", "))
    };
    let coverage = match &check.uncited_stories {
        None => format!("user story coverage not checkable ({})", check.coverage_reason.as_deref().unwrap_or("not checked")),
        Some(uncited) if uncited.is_empty() => "every user story cited".to_string(),
        Some(uncited) => {
            let named: Vec<String> = uncited.iter().map(|story| number_text(*story)).collect();
            format!("user stor{} {} cited by no ticket", if uncited.len() == 1 { "y" } else { "ies" }, named.join(", "))
        }
    };
    vec![format!("Ticket check: {}: {}", check.status, [tickets, cycles, blockers, coverage].join("; "))]
}

/// At the decisions stage an accepted finding is a question for the owner: the summary says whether the owner answered
/// it, or why it was left open.
fn accepted_tail(verdict: &Verdict) -> Option<String> {
    if verdict.answered == Some(true) {
        return Some("(answered)".to_string());
    }
    let mut parts: Vec<&str> = verdict.reason_code.iter().map(String::as_str).collect();
    if verdict.fixed == Some(true) {
        parts.push("fixed");
    }
    (!parts.is_empty()).then(|| format!("({})", parts.join(", ")))
}

fn question_tail(verdict: &Verdict) -> Option<String> {
    let mut parts: Lines = verdict.reason.iter().map(|reason| format!("Question: {reason}")).collect();
    if verdict.answered == Some(true) {
        parts.push("(answered)".to_string());
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The depth, whether the caller or the signals chose it, and the signals that fired (or, when none did, what was
/// measured), so the owner can tell whether the automatic choice is sound.
fn depth_line(run: &AdjudicatedRun) -> String {
    let choice = &run.depth_choice;
    let measured: Vec<String> = choice.measures.iter().map(|(measure, m)| format!("{measure} {} of {}", number_text(m.value), number_text(m.threshold))).collect();
    let signals = if choice.signals.is_empty() {
        let mut parts = vec!["none fired".to_string()];
        if !measured.is_empty() {
            parts.push(measured.join(", "));
        }
        parts.join(": ")
    } else {
        choice.signals.iter().map(signal_text).collect::<Vec<_>>().join("; ")
    };
    if choice.chosen_by == "argument" {
        let signalled = if choice.signals.is_empty() { "base" } else { "deep" };
        format!("Depth: {}, chosen by argument; the signals alone would choose {signalled} ({signals})", run.depth)
    } else {
        format!("Depth: {}, chosen by signals: {signals}", run.depth)
    }
}

fn signal_text(signal: &DepthSignal) -> String {
    match signal {
        DepthSignal::Threshold { measure, threshold, value } => format!("{measure} {} > {}", number_text(*value), number_text(*threshold)),
        DepthSignal::RiskTerm { term } => format!("risk term \"{term}\""),
        DepthSignal::Label { label } => format!("label {label}"),
    }
}

fn dropped_line(dropped: &DroppedCritic) -> String {
    let detail = dropped.detail.as_ref().map_or(String::new(), |detail| format!(" ({detail})"));
    let slot = match (&dropped.replaced_by, dropped.deep) {
        (Some(replacement), _) => format!("replaced by {replacement}"),
        (None, Some(true)) => "deep critic left out".to_string(),
        (None, _) => "no fallback left, so the stage runs with fewer critics".to_string(),
    };
    format!("- Not run: {} ({}): {}{detail}; {slot}", dropped.client, dropped.family, dropped.reason)
}

fn hash_list<'a>(hashes: impl Iterator<Item = (&'a str, &'a str)>) -> String {
    hashes.map(|(name, hash)| format!("{name} {hash}")).collect::<Vec<_>>().join(", ")
}

fn finding_section(run: &AdjudicatedRun, adjudicated: &AdjudicatedFinding) -> Lines {
    let finding = &adjudicated.finding;
    let body = &finding.body;
    let verdict = &adjudicated.verdict;
    let mut out = vec![
        format!("### {} [{} {}] {}", finding.id, body.severity, body.category, body.location),
        String::new(),
        format!("Raised by: {} ({})", finding.raised_by.join(", "), families_of(&run.critics, finding).join(", ")),
        format!("Verdict: {}", verdict_text(verdict)),
    ];
    if let Some(severity) = &verdict.severity {
        out.push(format!(
            "Severity: {severity}, overriding the check's {} ({})",
            finding.declared_severity.as_deref().unwrap_or("undefined"),
            verdict.severity_reason.as_deref().unwrap_or("undefined")
        ));
    }
    if let Some(pre) = finding.pre_verdict.as_ref().filter(|pre| pre.verdict != verdict.verdict) {
        out.push(format!("Pre-verdict: {} ({}): {}", pre.verdict, pre.reason_code, pre.reason));
    }
    out.push(String::new());
    out.push(body.claim.clone());
    out.extend(lines(["", "Evidence:", ""]));
    if body.evidence.is_empty() {
        out.push("- (no evidence)".to_string());
    }
    out.extend(evidence_lines(finding));
    if let Some(suggestion) = &body.suggestion {
        out.push(String::new());
        out.push(format!("Suggestion: {suggestion}"));
    }
    out.push(String::new());
    out
}

fn verdict_text(verdict: &Verdict) -> String {
    let code = verdict.reason_code.as_ref().map_or(String::new(), |code| format!(" ({code})"));
    let follow_up = verdict.follow_up.as_ref().map_or(String::new(), |follow_up| format!(", follow-up {follow_up}"));
    let crossed = verdict.trace_tickets.as_ref().map_or(String::new(), |tickets| format!(", trace across {}", tickets.join(", ")));
    let kept = verdict.regression_test.as_ref().map_or(String::new(), |test| format!(", regression test {test}"));
    let fixed = if verdict.fixed == Some(true) { ", fixed" } else { "" };
    let answered = if verdict.answered == Some(true) { ", answered" } else { "" };
    let reason = verdict.reason.as_ref().map_or(String::new(), |reason| format!(": {reason}"));
    format!("{}{answered}{code}{follow_up}{crossed}{kept}{fixed}{reason}", verdict.verdict)
}

/// The summary's one line naming what a documents-only run skipped.
fn skipped_line(run: &AdjudicatedRun) -> Lines {
    let skipped = run.skipped_checks.as_deref().unwrap_or(&[]);
    if skipped.is_empty() {
        return Vec::new();
    }
    let named: Vec<String> = skipped.iter().map(|check| format!("`{}`", check.check)).collect();
    vec![format!("Skipped, documents only: {} (the report says why)", named.join(", "))]
}

/// Every check the run could not run because its packet holds no code under `tree/`, and what each checks.
fn skipped_section(run: &AdjudicatedRun) -> Lines {
    let skipped = run.skipped_checks.as_deref().unwrap_or(&[]);
    if skipped.is_empty() {
        return Vec::new();
    }
    let why = if adrs_packet(&run.stage) { ": an ADR review sends documents only while the code permission is off" } else { "" };
    let mut out = lines(["## Skipped checks", ""]);
    out.push(format!("The packet holds no code under `tree/`{why}. These checks need the code, so they did not run:"));
    out.push(String::new());
    out.extend(skipped.iter().map(|check| format!("- `{}` ({}){}", check.check, check.from, check.about.as_ref().map_or(String::new(), |about| format!(": {about}")))));
    out.push(String::new());
    out
}

fn verdict_line(run: &AdjudicatedRun) -> String {
    let [accepted, hint, needs_human, rejected] = count_verdicts(&run.findings);
    format!("Verdicts: accepted {accepted}, rejected {rejected}, needs-human {needs_human}, hint {hint}")
}

fn critic_line(run: &AdjudicatedRun, critic: &CriticRun, checked_max_chars: usize) -> String {
    let who = format!("{} ({}, {})", critic.client, critic.model, critic.family);
    if critic.status == "budget-skip" {
        return format!("{who}: budget-skip: {}{}", critic.detail.as_deref().unwrap_or(""), retry_text(run, critic));
    }
    let cost = critic.cost_usd.map_or("cost unknown".to_string(), |cost| format!("${}", to_fixed(cost, 4)));
    let status = match critic.status.as_str() {
        "ok" => "ok".to_string(),
        "violated-readonly" => "violated read-only".to_string(),
        _ => format!("failed ({})", critic.failure.as_deref().unwrap_or("unknown")),
    };
    let checked = critic.no_findings_reason.as_ref().map_or(String::new(), |reason| format!(" (checked: {})", clip(reason, checked_max_chars)));
    let count = critic.finding_count;
    format!("{who}: {status}, {} finding{}, {cost}{}{checked}", number_text(count), if count == 1.0 { "" } else { "s" }, retry_text(run, critic))
}

/// A fallback retry names the critic it stands in for, and why that one failed.
fn retry_text(run: &AdjudicatedRun, critic: &CriticRun) -> String {
    let Some(retry_of) = &critic.retry_of else { return String::new() };
    let failed = run.critics.iter().find(|other| &other.critic_id == retry_of).and_then(|other| other.failure.as_deref()).unwrap_or("failed");
    format!(", fallback for {retry_of} ({failed})")
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.encode_utf16().count() > max_chars {
        format!("{}…", js_slice(text, 0.0, Some((max_chars - 1) as f64)))
    } else {
        text.to_string()
    }
}

/// One finding in the summary; agreement across families is named, so it stays visible.
fn finding_line(run: &AdjudicatedRun, finding: &RunFinding, tail: Option<String>) -> String {
    let body = &finding.body;
    let families = families_of(&run.critics, finding);
    let agreement = if families.len() < 2 { String::new() } else { format!(" (raised by {})", families.join(" and ")) };
    format!(
        "- {} [{} {}] {}: {}{agreement}{}",
        finding.id,
        body.severity,
        body.category,
        body.location,
        clip(&body.claim, CLAIM_MAX_CHARS),
        tail.map_or(String::new(), |tail| format!(" {tail}"))
    )
}

fn minor_lines(run: &AdjudicatedRun) -> Lines {
    run.critics
        .iter()
        .filter_map(|critic| {
            let minor: Vec<&AdjudicatedFinding> =
                run.findings.iter().filter(|f| f.finding.body.severity == "minor" && f.finding.raised_by.contains(&critic.critic_id)).collect();
            if minor.is_empty() {
                return None;
            }
            let by_verdict: Vec<String> = VERDICTS
                .iter()
                .filter_map(|verdict| {
                    let ids: Vec<&str> = minor.iter().filter(|f| f.verdict.verdict == *verdict).map(|f| f.finding.id.as_str()).collect();
                    (!ids.is_empty()).then(|| format!("{verdict} {}", ids.join(", ")))
                })
                .collect();
            Some(clip(&format!("- {}: {} minor: {}", critic.critic_id, minor.len(), by_verdict.join("; ")), MINOR_MAX_CHARS))
        })
        .collect()
}

fn bullets(items: &[String], hidden: usize) -> Lines {
    if items.is_empty() && hidden == 0 {
        return vec!["none".to_string()];
    }
    let mut out = items.to_vec();
    if hidden != 0 {
        out.push(format!("- … {hidden} more in the report"));
    }
    out
}
