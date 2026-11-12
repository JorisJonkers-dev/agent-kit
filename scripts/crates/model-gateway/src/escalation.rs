//! What happens to a delegation that does not come back clean: it is retried
//! once on the same client model with the failure attached, then climbs to the
//! next stronger permitted subsidised model, and a client that cannot start is
//! swapped for another of the same cost class. The gateway never moves into a
//! costlier cost class on its own.

use crate::config::{CostClass, GatewayConfig, ModelEntry, Tier};
use crate::json::{self, Value};
use crate::models::{cost_class_for, is_provider_allowed, model_key, open_code_provider};
use crate::policy::ModelPermission;
use crate::run::{RunInput, PROVIDER_CLIENTS, RUN_CLIENTS};
use crate::text::{js_slice, js_trim};
use std::collections::BTreeSet;

/// A delegation gets this many attempts in all: the first, the retry, and one climb.
pub const DEFAULT_MAX_ATTEMPTS: f64 = 3.0;

/// How many characters of a failed attempt's output the next attempt is shown.
pub const FAILURE_TAIL_CHARS: usize = 1000;

/// The reasons that are refusals, never failures of a run: a delegation the gateway refuses is refused again on
/// the same terms, so none is retried. Refusals are errors that stop the call before anything is launched; the
/// codes are listed here as well so one that surfaces as a result's reason (the caps and the grants) is still
/// never retried.
const REFUSAL_REASONS: [&str; 17] = [
    "cap-exceeded",
    "grant-expiring",
    "no-grant",
    "invalid-config",
    "invalid-input",
    "nested-delegation",
    "no-estimate",
    "not-supported-yet",
    "policy-refused",
    "provider-not-allowed",
    "repository-unreadable",
    "unknown-client",
    "unknown-model",
    "unknown-workflow",
    "workflow-ended",
    "write-needs-repository",
    "worktree-failed",
];

/// What a client's own error channel says when it will not start: it is signed out or out of allowance, the
/// model it named is not one the account may use, or its launch failed.
const CANNOT_START_REASONS: [&str; 4] = ["auth", "launch-failed", "model-unavailable", "rate-limited"];

/// Whether a failed result's reason is a refusal, which never retries.
pub fn is_refusal(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| REFUSAL_REASONS.contains(&reason))
}

/// Whether a failed result's reason says the client could not start, which swaps the client rather than retrying it.
pub fn is_cannot_start(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| CANNOT_START_REASONS.contains(&reason))
}

/// The link a retried or climbed attempt carries to the first attempt of its delegation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptLink {
    /// This attempt's place in the delegation, 1 for the first.
    pub attempt: usize,
    /// The run id of the delegation's first attempt; absent on the first.
    pub attempt_of: Option<String>,
}

/// One attempt, as the result lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptSummary {
    pub run_id: String,
    pub client: String,
    pub model: String,
    pub ok: bool,
    pub reason: Option<String>,
}

impl AttemptSummary {
    pub fn to_json(&self) -> Value {
        let mut members = vec![
            ("client", Value::str(&self.client)),
            ("model", Value::str(&self.model)),
            ("run_id", Value::str(&self.run_id)),
            ("status", Value::str(if self.ok { "ok" } else { "failed" })),
        ];
        if let Some(reason) = &self.reason {
            members.push(("reason", Value::str(reason)));
        }
        json::object(members)
    }
}

/// A client model the delegation could not start, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unstartable {
    pub client: String,
    pub model: String,
    pub reason: String,
}

impl Unstartable {
    pub fn to_json(&self) -> Value {
        json::object(vec![
            ("client", Value::str(&self.client)),
            ("model", Value::str(&self.model)),
            ("reason", Value::str(&self.reason)),
        ])
    }
}

/// How a delegation ended when it ended on a failed result with nothing left to try, or could not go on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EscalationEnd {
    /// The delegation ends on a failed result with nothing left to try.
    Exhausted,
    /// A later attempt could not start and no client model could stand in for it.
    Stopped,
}

impl EscalationEnd {
    pub fn as_str(self) -> &'static str {
        match self {
            EscalationEnd::Exhausted => "exhausted",
            EscalationEnd::Stopped => "stopped",
        }
    }
}

/// What a delegation adds to its last run's result once it is escalated.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct EscalationReport {
    pub attempts: Vec<AttemptSummary>,
    pub escalation: Option<EscalationEnd>,
    /// The client models passed over because they could not start.
    pub skipped: Vec<Unstartable>,
    /// Why the escalation stopped early: what a later attempt could not start with, or the refusal it ended on.
    pub stopped_reason: Option<String>,
}

impl EscalationReport {
    /// The members a result gains, in the order the node runner spreads them after the run's own: `attempts`,
    /// `escalation`, `skipped`, `stopped_reason`; the last three only when there is something to say.
    pub fn members(&self) -> Vec<(&'static str, Value)> {
        let mut members = vec![("attempts", Value::Arr(self.attempts.iter().map(AttemptSummary::to_json).collect()))];
        if let Some(end) = self.escalation {
            members.push(("escalation", Value::str(end.as_str())));
        }
        if !self.skipped.is_empty() {
            members.push(("skipped", Value::Arr(self.skipped.iter().map(Unstartable::to_json).collect())));
        }
        if let Some(reason) = &self.stopped_reason {
            members.push(("stopped_reason", Value::str(reason)));
        }
        members
    }
}

/// A delegation no permitted client model of its cost class can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnavailableResult {
    pub workflow: String,
    pub client: String,
    pub model: String,
    /// One line saying why nothing was started.
    pub message: String,
    /// The reason each candidate could not start.
    pub reasons: Vec<Unstartable>,
    pub attempts: Vec<AttemptSummary>,
}

impl UnavailableResult {
    pub fn to_json(&self) -> Value {
        json::object(vec![
            ("attempts", Value::Arr(self.attempts.iter().map(AttemptSummary::to_json).collect())),
            ("client", Value::str(&self.client)),
            ("message", Value::str(&self.message)),
            ("model", Value::str(&self.model)),
            ("reasons", Value::Arr(self.reasons.iter().map(Unstartable::to_json).collect())),
            ("status", Value::str("unavailable")),
            ("workflow", Value::str(&self.workflow)),
        ])
    }
}

/// The prompt a retried or climbed attempt gets: the caller's, and what the last attempt came to.
pub fn retry_prompt(original: &str, reason: &str, detail: &str) -> String {
    let tail = js_slice(js_trim(detail), -(FAILURE_TAIL_CHARS as f64), None);
    if tail.is_empty() {
        format!("{original}\n\nA previous attempt failed: {reason}")
    } else {
        format!("{original}\n\nA previous attempt failed: {reason}; {tail}")
    }
}

/// One client model a delegation may run on, with what places it among the others.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub client: String,
    pub model: String,
    pub family: String,
    /// The highest tier the catalogue says it handles; `None` when it names none, which keeps it out of every climb.
    pub tier: Option<Tier>,
    pub cost_class: CostClass,
    /// The provider a client that runs on one is named with.
    pub provider: Option<String>,
}

/// The key two candidates are the same model by.
pub fn candidate_key(client: &str, model: &str) -> String {
    model_key(client, model)
}

fn tier_rank(tier: Tier) -> usize {
    Tier::ALL.iter().position(|candidate| *candidate == tier).unwrap_or_default()
}

/// The model the caller asked for, as a candidate: its tier comes from the catalogue entry, and is `None` for a
/// model the catalogue does not know.
pub fn requested_candidate(config: &GatewayConfig, input: &RunInput, entry: Option<&ModelEntry>, family: &str) -> Candidate {
    let provider = provider_of(input);
    Candidate {
        client: input.client.clone(),
        cost_class: config.clients.get(&input.client).map_or(CostClass::Subsidised, |client| cost_class_for(client, provider.as_deref())),
        family: family.to_string(),
        model: input.model.clone(),
        tier: entry.map(|entry| entry.tier),
        provider: input.provider.clone(),
    }
}

/// The provider a run reaches its client model on: the one it names, else the one an OpenCode model names.
fn provider_of(input: &RunInput) -> Option<String> {
    match &input.provider {
        Some(provider) => Some(provider.clone()),
        None if input.client == "opencode" => open_code_provider(&input.model),
        None => None,
    }
}

/// Every catalogue model the delegation may be moved to: one a client that can run a delegation offers, that
/// the data policy permits for what the run carries, that this machine has not disabled, and whose provider the
/// machine allows. A metered-employer model joins the pool only while a live grant covers the workflow, so a
/// client that cannot start is swapped for one of them only when the owner allowed that workflow to spend on it.
/// A client that runs on a provider the caller names is open only to the provider the caller named.
pub fn candidate_pool(config: &GatewayConfig, input: &RunInput, permissions: &[ModelPermission], granted: &BTreeSet<String>) -> Vec<Candidate> {
    let permitted: BTreeSet<String> =
        permissions.iter().filter(|permission| carries(permission, input)).map(|permission| model_key(&permission.client, &permission.model)).collect();
    let mut pool = Vec::new();
    for entry in &config.models {
        let Some(client) = config.clients.get(&entry.client) else {
            continue;
        };
        if !RUN_CLIENTS.contains(&entry.client.as_str()) || !permitted.contains(&model_key(&entry.client, &entry.model)) {
            continue;
        }
        if entry.all_names().iter().any(|name| config.disabled.contains(&model_key(&entry.client, name))) {
            continue;
        }
        let named = PROVIDER_CLIENTS.contains(&entry.client.as_str());
        if named && (entry.client != input.client || input.provider.is_none()) {
            continue;
        }
        if entry.client == "opencode" && !is_provider_allowed(open_code_provider(&entry.model).as_deref(), client.providers.as_deref()) {
            continue;
        }
        let provider = if named {
            input.provider.clone()
        } else if entry.client == "opencode" {
            open_code_provider(&entry.model)
        } else {
            None
        };
        let cost_class = cost_class_for(client, provider.as_deref());
        if cost_class == CostClass::MeteredEmployer && !granted.contains(&entry.client) {
            continue;
        }
        pool.push(Candidate {
            client: entry.client.clone(),
            cost_class,
            family: entry.family.clone(),
            model: entry.model.clone(),
            tier: Some(entry.tier),
            provider: if named { provider } else { None },
        });
    }
    pool
}

/// Whether the policy lets a delegation with this run's contents reach the model: what the run reads and what the caller marks.
fn carries(permission: &ModelPermission, input: &RunInput) -> bool {
    if !permission.permitted {
        return false;
    }
    let (marks_code, marks_history) = input.carries.map_or((false, false), |marks| (marks.code, marks.history));
    let code = input.repo.is_some() || marks_code;
    !(code && !permission.may_send.code) && !(marks_history && !permission.may_send.history)
}

/// The candidates that pass `keep`, by tier and then by the catalogue's order.
fn ordered(pool: &[Candidate], keep: impl Fn(&Candidate, Tier) -> bool) -> Vec<Candidate> {
    let mut kept: Vec<(usize, &Candidate)> =
        pool.iter().filter_map(|candidate| candidate.tier.filter(|tier| keep(candidate, *tier)).map(|tier| (tier_rank(tier), candidate))).collect();
    // A stable sort keeps the catalogue's order within a tier.
    kept.sort_by_key(|(rank, _)| *rank);
    kept.into_iter().map(|(_, candidate)| candidate.clone()).collect()
}

/// The client models a climb may move to from the one that failed twice: the next stronger subsidised ones,
/// whatever cost class the failed model is in, so a climb never spends money; a tier strictly higher than the
/// failed model's, by tier and then by the catalogue's order. A model with no tier has nowhere to climb from.
pub fn climb_order(from: &Candidate, pool: &[Candidate]) -> Vec<Candidate> {
    let Some(tier) = from.tier else {
        return Vec::new();
    };
    let above = tier_rank(tier);
    ordered(pool, |candidate, candidate_tier| {
        candidate_key(&candidate.client, &candidate.model) != candidate_key(&from.client, &from.model) && candidate.cost_class == CostClass::Subsidised && tier_rank(candidate_tier) > above
    })
}

/// The client models a client that cannot start may be swapped for: the same cost class, and a tier at least the
/// wanted model's, the nearest tier first and then the catalogue's order. A model with no tier covers no tier.
pub fn fall_through_order(wanted: &Candidate, pool: &[Candidate]) -> Vec<Candidate> {
    let Some(tier) = wanted.tier else {
        return Vec::new();
    };
    let floor = tier_rank(tier);
    ordered(pool, |candidate, candidate_tier| {
        candidate_key(&candidate.client, &candidate.model) != candidate_key(&wanted.client, &wanted.model) && candidate.cost_class == wanted.cost_class && tier_rank(candidate_tier) >= floor
    })
}

/// The run input of an attempt on a candidate: the caller's, on that client model, with the prompt of this attempt.
pub fn attempt_input(input: &RunInput, candidate: &Candidate, prompt: &str) -> RunInput {
    RunInput {
        client: candidate.client.clone(),
        model: candidate.model.clone(),
        prompt: prompt.to_string(),
        provider: candidate.provider.clone(),
        // The node runner drops `provider` and puts the candidate's back last.
        tail: if candidate.provider.is_some() { vec!["provider"] } else { Vec::new() },
        ..input.clone()
    }
}

/// The catalogue a delegation may move through: all of it when the caller let the gateway change family, else
/// only the models of the family it asked for.
pub fn bind_family(input: &RunInput, requested: &Candidate, pool: Vec<Candidate>) -> Vec<Candidate> {
    if input.family_binding == Some(false) {
        return pool;
    }
    pool.into_iter().filter(|candidate| candidate.family == requested.family).collect()
}

/// The line a summary starts with when a delegation was answered by another family than the one it asked for:
/// both families and the cause.
pub fn escalation_notice(from_client: &str, from_family: &str, cause: &str, to_family: &str) -> String {
    format!("escalated from {from_family} ({from_client}: {cause}) to {to_family}")
}
