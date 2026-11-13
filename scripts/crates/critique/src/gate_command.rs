//! `gate` and the `critique-commit-gate` and `critique-pr-gate` hooks (ADR-0002). Given a `PreToolUse` payload for
//! `Bash`, it finds the commits, pushes, merges and `gh pr create` calls the command runs and decides each from git
//! and the critique's local files alone, with no model and no network:
//!
//! - a commit is blocked while its staged change is risky by the deterministic triage rules and the last ticket review
//!   of exactly that staged change is missing, failed, or degraded;
//! - a merge commit is let through when every commit it brings in is vouched for and it adds nothing of its own beyond
//!   manifest pins and rebuilt generated files; otherwise it is judged like a commit;
//! - a `gh pr create`, or a push that updates a branch outside the feature branch patterns, is blocked unless the
//!   latest feature review of the commit it sends says `GATE: PASS`.
//!
//! It does nothing in a repository that is not opted in for code, has a `nedap` remote or none, has no active flow,
//! and under the kill switch. Anything that keeps it from deciding allows the command and is logged. The node runner's
//! `application/gate.ts`, `application/merge-gate.ts` and the results of `domain/gate.ts`. Like `session-start`, a
//! decision slower than the deadline is answered as the node runner's timer answers, once it is made.

use crate::feature_gate::gate_index_path;
use crate::feature_review::{parse_gate_record, GateRecord};
use crate::fields::Fields;
use crate::git::CommitLink;
use crate::locations::{critique_config_path, gate_log_path};
use crate::merge_commit::{diff_of_paths, differs_only_in_pins, is_manifest};
use crate::paths::join;
use crate::repository::{read_allowlist, resolve, Resolution, Resolved};
use crate::repository_packet::{exclusion_of, is_generated, PacketLimits};
use crate::routing::{Routing, TriageRouting};
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::shell_command::{gated_invocations, GatedInvocation, PushTarget, PushTargets, Widened};
use crate::staged_change::{read_staged, staged_diff_hash};
use crate::ticket_review::{ticket_review_index_path, CriticFailureLine, TicketReviewRecord};
use crate::triage::{code_change, matches_glob, risk_signals, RiskRules, RiskSignal};
use crate::triage_index::{triage_index_path, TriageIndexRecord};
use model_gateway::json::{self, Value};
use model_gateway::text::dirname;
use model_gateway::timestamps::to_iso;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The gate's own deadline. A hook must finish well under a second; past this the gate allows.
pub const GATE_DEADLINE_MS: u64 = 700;

/// The most commits a merge check reads one by one when routing has no `gate`.
const DEFAULT_MERGE_COMMITS: f64 = crate::gate::DEFAULT_MERGE_COMMITS;

/// What kept the gate from deciding: a state file it relies on that does not read, or anything else.
#[derive(Debug)]
pub(crate) enum Failure {
    Unreadable(String),
    Error(String),
}

impl From<crate::error::CritiqueError> for Failure {
    fn from(error: crate::error::CritiqueError) -> Self {
        Failure::Error(error.message)
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Error(message)
    }
}

type Decided = Result<Value, Failure>;

fn allowed(gate: Option<&str>, reason: &str) -> Value {
    json::object(vec![("gate", gate.map_or(Value::Null, Value::str)), ("line", Value::Null), ("reason", Value::str(reason)), ("status", Value::str("allowed"))])
}

fn failed_open(gate: Option<&str>, failure: &str, detail: &str) -> Value {
    json::object(vec![
        ("detail", Value::str(detail)),
        ("failure", Value::str(failure)),
        ("gate", gate.map_or(Value::Null, Value::str)),
        ("line", Value::Null),
        ("reason", Value::str("failed-open")),
        ("status", Value::str("allowed")),
    ])
}

fn blocked(gate: &str, reason: &str, line: String) -> Value {
    let deny = json::object(vec![("hookEventName", Value::str("PreToolUse")), ("permissionDecision", Value::str("deny")), ("permissionDecisionReason", Value::str(&line))]);
    json::object(vec![
        ("gate", Value::str(gate)),
        ("hook_output", json::object(vec![("hookSpecificOutput", deny)])),
        ("line", Value::Str(line)),
        ("reason", Value::str(reason)),
        ("status", Value::str("blocked")),
    ])
}

fn failure(gate: Option<&str>, error: Failure) -> Value {
    match error {
        Failure::Unreadable(detail) => failed_open(gate, "unreadable-state", &detail),
        Failure::Error(detail) => failed_open(gate, "error", &detail),
    }
}

fn field<'a>(result: &'a Value, key: &str) -> Option<&'a str> {
    result.as_object().and_then(|object| object.get(key)).and_then(Value::as_str)
}

/// Why a staged change is risky, in a few words: the deciding signal, and how many more fired.
pub fn risk_reason(signals: &[RiskSignal]) -> String {
    let why = match &signals[0] {
        RiskSignal::RiskPath { path, .. } => format!("{path} is a risk path"),
        RiskSignal::Size { changed_lines, threshold } => format!("{} changed lines, above {}", json::number_text(*changed_lines), json::number_text(*threshold)),
        RiskSignal::Label { label } => format!("the ticket carries {label}"),
    };
    match signals.len() - 1 {
        0 => why,
        1 => format!("{why}, and 1 more risk signal"),
        more => format!("{why}, and {more} more risk signals"),
    }
}

/// Runs `gate` on a hook payload; it never fails, and what kept it from deciding is logged.
pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, crate::error::CritiqueError> {
    Ok(decide_in_time(raw, runtime, Duration::from_millis(GATE_DEADLINE_MS)))
}

/// The decision, or the failed-open answer when it errs or takes longer than `deadline`.
pub(crate) fn decide_in_time(raw: &Value, runtime: &Runtime, deadline: Duration) -> Value {
    if runtime.env.get("CRITIQUE_OFF").map(String::as_str) == Some("1") {
        return allowed(None, "off");
    }
    let started = Instant::now();
    let decided = Gate { runtime, routing: None }.decide(raw).unwrap_or_else(|error| failure(None, error));
    let result = if started.elapsed() > deadline { failed_open(None, "deadline", &format!("the gate did not decide within {GATE_DEADLINE_MS} ms")) } else { decided };
    if field(&result, "reason") == Some("failed-open") {
        log(raw, runtime, &result);
    }
    result
}

/// What a hook prints: the gate's `hook_output` when it blocks, nothing otherwise; it always exits 0.
pub(crate) fn hook_output(hook: &str, input: Value, runtime: &Runtime) -> String {
    let payload = match input {
        Value::Obj(mut object) => {
            object.set("gate", Value::str(hook));
            Value::Obj(object)
        }
        other => other,
    };
    let result = decide_in_time(&payload, runtime, Duration::from_millis(GATE_DEADLINE_MS));
    match result.as_object().and_then(|object| object.get("hook_output")) {
        Some(output) if field(&result, "status") == Some("blocked") => format!("{}\n", json::compact(output)),
        _ => String::new(),
    }
}

/// Reads one state file or index line; what does not read is unreadable state, named by `where_`.
fn read_state<T>(where_: &str, parsed: Result<T, String>) -> Result<T, Failure> {
    parsed.map_err(|message| Failure::Unreadable(format!("{where_}: {message}")))
}

fn json_fields(text: &str) -> Result<Value, String> {
    json::parse(text)
}

/// This machine's critique configuration's kill switch.
fn config_disabled(text: &str) -> Result<bool, String> {
    let value = json_fields(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    if file.has("disabled") {
        file.boolean("disabled").map_err(|error| error.0)
    } else {
        Ok(false)
    }
}

/// A flow state file's phase and repository; a file that does not read is an error, one that does not parse is
/// unreadable state.
pub(crate) fn read_flow(runtime: &Runtime, path: &str) -> Result<(String, String), Failure> {
    read_state(path, flow_phase(&read_text(runtime, path)?))
}

/// A file's text; a file that does not read is an error, not unreadable state.
fn read_text(runtime: &Runtime, path: &str) -> Result<String, Failure> {
    match runtime.files.read_text(path) {
        Ok(text) => Ok(text),
        Err(error) => Err(Failure::Error(error.message)),
    }
}

/// The phase and repository of a flow state file.
fn flow_phase(text: &str) -> Result<(String, String), String> {
    let value = json_fields(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    let phase = file.one_of("phase", &["planning", "implementation", "finishing"]).map_err(|error| error.0)?;
    Ok((phase, file.string("repository").map_err(|error| error.0)?))
}

/// One line of the triage index.
pub(crate) fn parse_triage_index_record(text: &str) -> Result<TriageIndexRecord, String> {
    let value = json_fields(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    let read = || -> Result<TriageIndexRecord, crate::error::ShapeError> {
        let labels = file.strings("labels")?;
        let recorded_at = file.string("recorded_at")?;
        let repository = file.string("repository")?;
        let run_id = file.string("run_id")?;
        let staged_diff = file.string("staged_diff")?;
        let staged_on = file.string("staged_on")?;
        let status = file.one_of("status", &["risky", "low-risk"])?;
        let ticket = file.string("ticket")?;
        file.literal("version", 1.0)?;
        Ok(TriageIndexRecord { labels, recorded_at, repository, run_id, staged_diff, staged_on, status, ticket })
    };
    read().map_err(|error| error.0)
}

/// One line of the ticket review index, read in the order the node runner's object literal reads its fields.
pub(crate) fn parse_ticket_review_record(text: &str) -> Result<TicketReviewRecord, String> {
    let value = json_fields(text)?;
    let file = Fields::of(&value, "").map_err(|error| error.0)?;
    let read = || -> Result<TicketReviewRecord, crate::error::ShapeError> {
        let base_commit = file.string("base_commit")?;
        // Records written before #259 carry no critic failures.
        let critic_failures = if file.has("critic_failures") {
            file.list("critic_failures", |failure| Ok(CriticFailureLine { critic_id: failure.string("critic_id")?, reason: failure.string("reason")? }))?
        } else {
            Vec::new()
        };
        let degraded = file.boolean("degraded")?;
        let head_commit = file.string("head_commit")?;
        let recorded_at = file.string("recorded_at")?;
        let repository = file.string("repository")?;
        let review = file.one_of("review", &["pass", "fail", "degraded"])?;
        let reviewed_staged_diff = file.nullable_string("reviewed_staged_diff")?;
        let run_id = file.string("run_id")?;
        let spec = file.string("spec")?;
        file.literal_str("stage", "ticket-review")?;
        let staged_diff = file.nullable_string("staged_diff")?;
        let ticket = file.string("ticket")?;
        let unfixed_critical = file.strings("unfixed_critical")?;
        file.literal("version", 1.0)?;
        Ok(TicketReviewRecord {
            base_commit,
            critic_failures,
            degraded,
            head_commit,
            recorded_at,
            repository,
            review,
            reviewed_staged_diff,
            run_id,
            spec,
            staged_diff,
            ticket,
            unfixed_critical,
        })
    };
    read().map_err(|error| error.0)
}

/// Which gate an invocation stands before.
fn gate_of(invocation: &GatedInvocation) -> &'static str {
    match invocation {
        GatedInvocation::Commit { .. } | GatedInvocation::Merge { .. } => "commit",
        GatedInvocation::Push { .. } | GatedInvocation::PrCreate { .. } => "pr",
    }
}

fn short(commit: &str) -> String {
    commit.chars().take(12).collect()
}

/// What a merge check found.
enum MergeVerdict {
    /// Every commit brought in is vouched for, and the merge adds nothing of its own.
    Vouched,
    /// Every commit brought in is vouched for; the merge's own changes beyond them are these.
    OwnChanges { diff: String, paths: Vec<String> },
    /// A commit the merge brings in is not vouched for, so the merge is judged whole.
    Unvouched { commit: String },
    /// The merge brings in more commits than the limit.
    TooMany { count: usize },
}

/// What a merge check reads besides git.
struct MergeEvidence {
    root: String,
    reviews: Vec<TicketReviewRecord>,
    triages: Vec<TriageIndexRecord>,
    rules: TriageRouting,
    packet: PacketLimits,
    limit: f64,
}

impl MergeEvidence {
    fn rules(&self) -> RiskRules<'_> {
        RiskRules { changed_lines_threshold: self.rules.changed_lines_threshold, labels: &self.rules.labels, risk_paths: &self.rules.risk_paths }
    }

    /// The labels of the last triage of a change staged on `commit`.
    fn labels_on(&self, commit: &str) -> Vec<String> {
        self.triages.iter().rev().find(|record| record.staged_on == commit).map(|record| record.labels.clone()).unwrap_or_default()
    }
}

/// The parts of a pushed or PR'd commit the PR gate checks: what is sent, how to retry, and the revision sent.
struct Sent {
    what: String,
    retry: &'static str,
    source: Option<String>,
}

struct Gate<'a> {
    runtime: &'a Runtime,
    routing: Option<Routing>,
}

impl Gate<'_> {
    fn routing(&mut self) -> Result<&Routing, Failure> {
        if self.routing.is_none() {
            self.routing = Some(load_routing(self.runtime)?);
        }
        Ok(self.routing.as_ref().expect("routing was loaded"))
    }

    fn decide(&mut self, raw: &Value) -> Decided {
        let runtime = self.runtime;
        let config = critique_config_path(&runtime.env)?;
        if runtime.files.exists(&config).unwrap_or(false) {
            let text = read_text(runtime, &config)?;
            if read_state(&config, config_disabled(&text))? {
                return Ok(allowed(None, "disabled"));
            }
        }
        let (command, cwd, gate, tool) = read_payload(raw).map_err(Failure::Error)?;
        if tool.as_deref().is_some_and(|tool| tool != "Bash") {
            return Ok(allowed(None, "not-gated"));
        }
        let home = runtime.env.get("HOME").map(String::as_str);
        let invocations: Vec<GatedInvocation> =
            gated_invocations(&command, &cwd, home).into_iter().filter(|invocation| gate.as_deref().is_none_or(|gate| gate_of(invocation) == gate)).collect();
        if invocations.is_empty() {
            return Ok(allowed(None, "not-gated"));
        }
        let results: Vec<Value> = invocations.iter().map(|invocation| self.check(invocation).unwrap_or_else(|error| failure(Some(gate_of(invocation)), error))).collect();
        let pick = results
            .iter()
            .find(|result| field(result, "status") == Some("blocked"))
            .or_else(|| results.iter().find(|result| field(result, "reason") == Some("failed-open")))
            .unwrap_or(&results[0]);
        Ok(pick.clone())
    }

    fn check(&mut self, invocation: &GatedInvocation) -> Decided {
        let gate = gate_of(invocation);
        let Some(cwd) = invocation.cwd() else {
            return Err(Failure::Error("the command line does not say which directory the command runs in".to_string()));
        };
        let repository = match resolve(self.runtime, cwd)? {
            Resolution::Refused(refusal) => {
                let nedap = refusal.as_object().and_then(|object| object.get("reason")).and_then(Value::as_str) == Some("nedap");
                return Ok(allowed(Some(gate), if nedap { "nedap" } else { "no-remote" }));
            }
            Resolution::Resolved(repository) => repository,
        };
        let allowlist = read_allowlist(self.runtime).map_err(|error| Failure::Unreadable(error.message))?;
        let Some(entry) = allowlist.get(&repository.key) else {
            return Ok(allowed(Some(gate), "not-opted-in"));
        };
        if !entry.code {
            return Ok(allowed(Some(gate), "code-not-allowed"));
        }
        if !self.active_flow(&repository)? {
            return Ok(allowed(Some(gate), "no-active-flow"));
        }
        match invocation {
            GatedInvocation::Commit { widened, .. } => self.commit_gate(&repository, *widened),
            GatedInvocation::Merge { revisions, fast_forward, head_moves, .. } => self.merge_gate(&repository, revisions, *fast_forward, *head_moves),
            GatedInvocation::Push { targets, head_moves, .. } => self.pr_gate(&repository, Some(targets), None, *head_moves),
            GatedInvocation::PrCreate { head, head_moves, .. } => self.pr_gate(&repository, None, head.clone(), *head_moves),
        }
    }

    /// Whether the repository has an active flow: a flow state of this repository whose phase is implementation or
    /// finishing. A file that does not read is skipped, and counts as unreadable state only when no readable file shows
    /// an active flow.
    fn active_flow(&self, repository: &Resolved) -> Result<bool, Failure> {
        let dir = join(&[&repository.common_dir, "critique", "flows"]);
        let mut unreadable: Option<Failure> = None;
        for name in self.runtime.dirs.list_files(&dir).into_iter().filter(|name| name.ends_with(".json")) {
            let path = join(&[&dir, &name]);
            match read_flow(self.runtime, &path) {
                Ok((phase, owner)) if owner == repository.key && (phase == "implementation" || phase == "finishing") => return Ok(true),
                Ok(_) => {}
                Err(error) => {
                    unreadable.get_or_insert(error);
                }
            }
        }
        match unreadable {
            Some(error) => Err(error),
            None => Ok(false),
        }
    }

    fn triage_rules(&mut self) -> Result<TriageRouting, Failure> {
        match &self.routing()?.triage {
            Some(triage) => Ok(triage.clone()),
            None => Err(Failure::Error("routing has no triage, so the commit gate has no risk rules".to_string())),
        }
    }

    fn commit_gate(&mut self, repository: &Resolved, widened: Widened) -> Decided {
        let triage = self.triage_rules()?;
        let rules = RiskRules { changed_lines_threshold: triage.changed_lines_threshold, labels: &triage.labels, risk_paths: &triage.risk_paths };
        let git = &self.runtime.git;
        let root = repository.root.as_str();
        let staged = read_staged(self.runtime, root)?;
        let index = widened == Widened::None;
        let diff = git.diff_against_head(root, index)?;
        let paths = git.paths_against_head(root, index)?;
        let untracked = if widened == Widened::Untracked { git.untracked_paths(root)? } else { Vec::new() };
        let critique_dir = join(&[&repository.common_dir, "critique"]);
        let triaged =
            self.lines(&triage_index_path(&critique_dir), parse_triage_index_record)?.into_iter().filter(|record| record.repository == repository.key && record.staged_on == staged.head_commit).last();
        let merge = if index { self.merge_in_progress(repository, &staged.head_commit)? } else { None };
        if matches!(merge, Some(MergeVerdict::Vouched)) {
            return Ok(allowed(Some("commit"), "merge-vouched"));
        }
        let own = match merge {
            Some(MergeVerdict::OwnChanges { diff, paths }) => Some((diff, paths)),
            _ => None,
        };
        let change = match &own {
            None => code_change(&diff, &[paths.clone(), untracked.clone()].concat()),
            Some((diff, paths)) => code_change(diff, paths),
        };
        let signals = risk_signals(&change, &triaged.map(|record| record.labels).unwrap_or_default(), &rules)?;
        if signals.is_empty() {
            return Ok(allowed(Some("commit"), "low-risk"));
        }
        let why = risk_reason(&signals);
        if !index && (!untracked.is_empty() || diff != git.diff_against_head(root, true)?) {
            return Ok(blocked(
                "commit",
                "unstaged",
                format!("Commit blocked: the change is risky ({why}) and this command commits changes that are not staged yet, so no /review-implementation run can vouch for them. Stage them in a command of their own, run /review-implementation, then commit."),
            ));
        }
        let review = self
            .lines(&ticket_review_index_path(&critique_dir), parse_ticket_review_record)?
            .into_iter()
            .filter(|record| record.repository == repository.key && record.staged_diff.as_deref() == Some(staged.sha256.as_str()))
            .last();
        let Some(review) = review else {
            let line = match own {
                None => format!("Commit blocked: the staged change is risky ({why}) and no passing /review-implementation run matches it. Run /review-implementation, then commit."),
                Some(_) => format!("Commit blocked: this merge's own changes beyond the reviewed commits it brings in are risky ({why}) and no passing /review-implementation run matches the staged merge. Run /review-implementation, then commit."),
            };
            return Ok(blocked("commit", "unreviewed", line));
        };
        if review.review == "fail" {
            return Ok(blocked("commit", "review-failed", review_failed_line(&review)));
        }
        // A record written before #259 says `pass` with `degraded: true` for the same run.
        if review.review == "degraded" || review.degraded {
            return Ok(blocked("commit", "review-degraded", review_degraded_line(&review)));
        }
        Ok(allowed(Some("commit"), "reviewed"))
    }

    /// A `git merge` or `git pull` about to run: each merge is let through when every commit it brings in is vouched
    /// for, and is otherwise judged on what it changes against HEAD.
    fn merge_gate(&mut self, repository: &Resolved, revisions: &[String], fast_forward: bool, head_moves: bool) -> Decided {
        if head_moves {
            return Ok(blocked(
                "commit",
                "head-unknown",
                "Merge blocked: this command moves HEAD before it merges, so the gate cannot tell what the merge brings in. Run the merge in a command of its own.".to_string(),
            ));
        }
        let root = repository.root.clone();
        let evidence = self.merge_evidence(repository)?;
        let git = &self.runtime.git;
        let head = git.resolve_commit(&root, "HEAD")?;
        let mut reasons: Vec<&'static str> = Vec::new();
        for revision in revisions {
            let theirs = git.resolve_commit(&root, revision)?;
            let base = git.merge_base(&root, &head, &theirs)?;
            if base == theirs || (base == head && fast_forward) {
                continue;
            }
            // Why the merge's commits do not vouch for it, as the end of its block line.
            let why = match self.judge(&evidence, &head, &theirs, None)? {
                MergeVerdict::Vouched | MergeVerdict::OwnChanges { .. } => {
                    reasons.push("merge-vouched");
                    continue;
                }
                MergeVerdict::TooMany { count } => format!("it brings in {count} commits, more than the {} the gate checks one by one", json::number_text(evidence.limit)),
                MergeVerdict::Unvouched { commit } => format!("{}, a commit it brings in, has no passing /review-implementation run", short(&commit)),
            };
            let auto = git.merge_tree(&root, &head, &theirs)?;
            if auto.conflicts {
                reasons.push("merge-stops");
                continue;
            }
            let diff = git.diff_commits(&root, &head, &auto.tree)?;
            let paths = git.changed_paths(&root, &head, &auto.tree)?;
            let signals = risk_signals(&code_change(&diff, &paths), &evidence.labels_on(&head), &evidence.rules())?;
            if signals.is_empty() {
                reasons.push("low-risk");
                continue;
            }
            return Ok(blocked(
                "commit",
                "unreviewed",
                format!("Merge blocked: merging {revision} is risky ({}) and {why}. Stage the merge with git merge --no-commit {revision}, run /review-implementation, then commit.", risk_reason(&signals)),
            ));
        }
        let reason = reasons.iter().find(|reason| **reason == "low-risk").or(reasons.first()).copied().unwrap_or("not-gated");
        Ok(allowed(Some("commit"), reason))
    }

    /// The check of the merge a `git commit` concludes while `MERGE_HEAD` is set; none when no merge is in progress.
    fn merge_in_progress(&mut self, repository: &Resolved, head: &str) -> Result<Option<MergeVerdict>, Failure> {
        let Ok(theirs) = self.runtime.git.resolve_commit(&repository.root, "MERGE_HEAD") else {
            return Ok(None);
        };
        let evidence = self.merge_evidence(repository)?;
        let result = self.runtime.git.write_index_tree(&repository.root)?;
        self.judge(&evidence, head, &theirs, Some(&result)).map(Some)
    }

    /// What a merge check reads: this repository's ticket reviews and triages, and the rules that judge them.
    fn merge_evidence(&mut self, repository: &Resolved) -> Result<MergeEvidence, Failure> {
        let critique_dir = join(&[&repository.common_dir, "critique"]);
        let routing = self.routing()?;
        let packet = routing.packet.clone();
        let limit = routing.gate.as_ref().map_or(DEFAULT_MERGE_COMMITS, |gate| gate.merge_commits);
        let reviews = self.lines(&ticket_review_index_path(&critique_dir), parse_ticket_review_record)?.into_iter().filter(|record| record.repository == repository.key).collect();
        let rules = self.triage_rules()?;
        let triages = self.lines(&triage_index_path(&critique_dir), parse_triage_index_record)?.into_iter().filter(|record| record.repository == repository.key).collect();
        Ok(MergeEvidence { limit, packet, reviews, root: repository.root.clone(), rules, triages })
    }

    /// Decides what a merge of `theirs` into `ours` adds that no review has vouched for. `result` is the tree the merge
    /// commit records; `None` for the clean auto-merge, which adds nothing of its own.
    fn judge(&self, evidence: &MergeEvidence, ours: &str, theirs: &str, result: Option<&str>) -> Result<MergeVerdict, Failure> {
        let links = self.runtime.git.commits_between(&evidence.root, ours, theirs)?;
        if links.len() as f64 > evidence.limit {
            return Ok(MergeVerdict::TooMany { count: links.len() });
        }
        let trees: HashMap<&str, &str> = links.iter().map(|link| (link.commit.as_str(), link.tree.as_str())).collect();
        for link in &links {
            if !self.vouched(evidence, link, &trees)? {
                return Ok(MergeVerdict::Unvouched { commit: link.commit.clone() });
            }
        }
        let own = match result {
            None => None,
            Some(result) => self.own_changes(evidence, ours, theirs, result)?,
        };
        Ok(match own {
            None => MergeVerdict::Vouched,
            Some((diff, paths)) => MergeVerdict::OwnChanges { diff, paths },
        })
    }

    /// Whether a commit a merge brings in is vouched for: by its latest ticket review, or with none, by being a commit
    /// the commit gate lets through alone, or a merge that adds nothing of its own.
    fn vouched(&self, evidence: &MergeEvidence, link: &CommitLink, trees: &HashMap<&str, &str>) -> Result<bool, Failure> {
        let git = &self.runtime.git;
        let staged = match link.parents.first() {
            None => None,
            Some(first) => {
                let tree = match trees.get(first.as_str()) {
                    Some(tree) => (*tree).to_string(),
                    None => git.tree_of(&evidence.root, first)?,
                };
                Some(staged_diff_hash(&tree, &git.tree_entries(&evidence.root, &link.commit)?))
            }
        };
        let review = evidence.reviews.iter().rev().find(|record| record.head_commit == link.commit || (staged.is_some() && record.staged_diff == staged));
        if let Some(review) = review {
            return Ok(review.review == "pass" && !review.degraded);
        }
        match link.parents.as_slice() {
            [] => Ok(false),
            [first, second] => Ok(self.own_changes(evidence, first, second, &link.commit)?.is_none()),
            [_, _, ..] => Ok(false),
            [first] => {
                let diff = git.diff_commits(&evidence.root, first, &link.commit)?;
                let paths = git.changed_paths(&evidence.root, first, &link.commit)?;
                Ok(risk_signals(&code_change(&diff, &paths), &evidence.labels_on(first), &evidence.rules())?.is_empty())
            }
        }
    }

    /// The merge's own changes beyond the clean auto-merge, pins and generated files apart; `None` when there are none.
    fn own_changes(&self, evidence: &MergeEvidence, ours: &str, theirs: &str, result: &str) -> Result<Option<(String, Vec<String>)>, Failure> {
        let git = &self.runtime.git;
        let auto = git.merge_tree(&evidence.root, ours, theirs)?.tree;
        let mut own = Vec::new();
        for path in git.changed_paths(&evidence.root, &auto, result)? {
            if !self.rebuilt(evidence, &auto, result, &path)? {
                own.push(path);
            }
        }
        if own.is_empty() {
            return Ok(None);
        }
        Ok(Some((diff_of_paths(&git.diff_commits(&evidence.root, &auto, result)?, &own), own)))
    }

    /// Whether a path the merge changed beyond the auto-merge is a generated file, or a manifest that differs only in its pins.
    fn rebuilt(&self, evidence: &MergeEvidence, auto: &str, result: &str, path: &str) -> Result<bool, Failure> {
        if exclusion_of(path, None, &evidence.packet)?.is_some_and(|exclusion| exclusion.kind == "generated") {
            return Ok(true);
        }
        let before = self.runtime.git.file_at(&evidence.root, auto, path).ok();
        let after = self.runtime.git.file_at(&evidence.root, result, path).ok();
        if is_generated(after.as_deref().or(before.as_deref()).unwrap_or_default()) {
            return Ok(true);
        }
        Ok(match (before, after) {
            (Some(before), Some(after)) => is_manifest(path) && differs_only_in_pins(&before, &after),
            _ => false,
        })
    }

    fn pr_gate(&mut self, repository: &Resolved, targets: Option<&PushTargets>, head: Option<String>, head_moves: bool) -> Decided {
        let root = repository.root.clone();
        let mut sent: Vec<Sent> = Vec::new();
        match targets {
            None => sent.push(Sent { retry: "open the PR", source: head, what: "PR".to_string() }),
            Some(targets) => {
                let patterns = self.routing()?.gate.as_ref().map(|gate| gate.feature_branches.clone()).unwrap_or_default();
                let all = matches!(targets, PushTargets::All);
                let list = match targets {
                    PushTargets::All => vec![PushTarget { branch: None, source: None }],
                    PushTargets::Some(list) => list.clone(),
                };
                for target in list {
                    let branch = if all {
                        None
                    } else {
                        match target.branch {
                            Some(branch) => Some(branch),
                            None => Some(self.runtime.git.current_branch(&root)?).filter(|branch| !branch.is_empty()),
                        }
                    };
                    if let Some(branch) = &branch {
                        if any_match(branch, &patterns)? {
                            continue;
                        }
                    }
                    sent.push(Sent { retry: "push", source: target.source, what: format!("Push to {}", branch.as_deref().unwrap_or("every branch")) });
                }
            }
        }
        if sent.is_empty() {
            return Ok(allowed(Some("pr"), "feature-branch"));
        }
        if head_moves {
            let verb = if targets.is_none() { "opens the PR" } else { "pushes" };
            return Ok(blocked(
                "pr",
                "head-unknown",
                format!("{} blocked: this command moves HEAD before it {verb}, so no /review-pr gate can hold for what it sends. Run that part in a command of its own after /review-pr.", sent[0].what),
            ));
        }
        let critique_dir = join(&[&repository.common_dir, "critique"]);
        for target in &sent {
            let head = self.runtime.git.resolve_commit(&root, target.source.as_deref().unwrap_or("HEAD"))?;
            let record = self.lines(&gate_index_path(&critique_dir), parse_gate_record)?.into_iter().filter(|line| line.repository == repository.key && line.head_commit == head).last();
            let Some(record) = record else {
                return Ok(blocked(
                    "pr",
                    "no-feature-review",
                    format!("{} blocked: no /review-pr run has recorded a gate for {}. Run /review-pr, then {}.", target.what, short(&head), target.retry),
                ));
            };
            if record.gate == "fail" {
                return Ok(blocked("pr", "gate-fail", gate_fail_line(target, &record, &head)));
            }
        }
        Ok(allowed(Some("pr"), "gate-pass"))
    }

    /// Every line of a JSON-lines index, oldest first; none when the file is absent. Any line that does not parse is
    /// unreadable state.
    fn lines<T>(&self, path: &str, parse: fn(&str) -> Result<T, String>) -> Result<Vec<T>, Failure> {
        if !self.runtime.files.exists(path).unwrap_or(false) {
            return Ok(Vec::new());
        }
        let text = read_text(self.runtime, path)?;
        let lines = text.split('\n').filter(|line| !model_gateway::text::js_trim(line).is_empty());
        lines.enumerate().map(|(n, line)| read_state(&format!("{path} line {}", n + 1), parse(line))).collect()
    }
}

/// Whether a branch matches any of the feature-branch globs.
fn any_match(branch: &str, patterns: &[String]) -> Result<bool, Failure> {
    for pattern in patterns {
        if matches_glob(branch, pattern)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The hook payload: `cwd`, `tool_input.command`, the tool's name, and optionally which gate to run.
fn read_payload(raw: &Value) -> Result<(String, String, Option<String>, Option<String>), String> {
    let read = || -> Result<_, crate::error::ShapeError> {
        let fields = Fields::of(raw, "")?;
        let command = fields.object("tool_input")?.optional_text("command")?.unwrap_or_default();
        let cwd = fields.string("cwd")?;
        let gate = if fields.has("gate") { Some(fields.one_of("gate", &["commit", "pr"])?) } else { None };
        let tool = fields.optional_string("tool_name")?;
        Ok((command, cwd, gate, tool))
    };
    read().map_err(|error| error.0)
}

fn review_failed_line(review: &TicketReviewRecord) -> String {
    let findings = review.unfixed_critical.join(", ");
    let them = if review.unfixed_critical.len() == 1 { "it" } else { "them" };
    format!("Commit blocked: /review-implementation run {} failed for this staged change, with critical {findings} accepted and not fixed. Fix and stage {them}, then run /review-implementation again.", review.run_id)
}

fn review_degraded_line(review: &TicketReviewRecord) -> String {
    let why = review.critic_failures.iter().map(|failure| format!("{}: {}", failure.critic_id, failure.reason)).collect::<Vec<_>>().join(", ");
    let why = if why.is_empty() { String::new() } else { format!(" ({why})") };
    format!(
        "Commit blocked: /review-implementation run {} is degraded: no critic reported on this staged change{why}. Run /review-implementation again; the commit waits for a run in which a critic reports.",
        review.run_id
    )
}

fn gate_fail_line(target: &Sent, record: &GateRecord, head: &str) -> String {
    format!("{} blocked: /review-pr says {} for {}. Fix and close the critical follow-ups, run /review-pr on the new head, then {}.", target.what, record.line, short(head), target.retry)
}

/// Appends one line to the gate log; a log that cannot be written is left alone, so logging never blocks.
fn log(raw: &Value, runtime: &Runtime, result: &Value) {
    let value = |key: &str| result.as_object().and_then(|object| object.get(key)).cloned().unwrap_or(Value::Null);
    let cwd = raw.as_object().and_then(|object| object.get("cwd")).filter(|cwd| cwd.as_str().is_some()).cloned().unwrap_or(Value::Null);
    let line = json::object(vec![("cwd", cwd), ("detail", value("detail")), ("failure", value("failure")), ("gate", value("gate")), ("recorded_at", Value::Str(to_iso(runtime.clock.now_ms())))]);
    if let Ok(path) = gate_log_path(&runtime.env) {
        let _ = runtime.files.mkdir_all(dirname(&path)).and_then(|()| runtime.files.append(&path, &format!("{}\n", json::compact(&line))));
    }
}
