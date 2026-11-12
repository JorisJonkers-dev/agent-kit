//! The `batch` command: a whole plan of delegations handed over in one call.
//!
//! The plan is checked as a whole before anything runs: its dependencies, then every delegation's client model,
//! data policy and grant, and last the sum of the metered estimates, which are reserved together in one locked
//! ledger update. A plan with any problem is refused with all of them, launches nothing, creates no worktree and
//! starts no workflow. Each delegation then goes through `RunCommand`, so retry, climb, fall-through, policy,
//! caps and grants all apply; independent ones run in parallel, at most `max_parallel_per_client` at once per
//! client, and one that fails for good skips only the delegations that depend on it.

use crate::batch::{
    batch_status, dependency_problems, estimated_prompt, plan_refused, prompt_with_outputs, read_batch_input, BatchDelegation, BatchInput,
    DelegationStatus, DependencyOutput, PlanProblem,
};
use crate::cap_guard::{CapGuard, PlannedReservation, Reservation};
use crate::caps::round;
use crate::config::GatewayConfig;
use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::escalation::{AttemptSummary, DEFAULT_MAX_ATTEMPTS};
use crate::escalation_run::Delegation;
use crate::json::{self, Object, Value};
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord};
use crate::ledger_file::LedgerStore;
use crate::ports::Runtime;
use crate::run_command::{BatchTag, Checked, DelegationOptions, RunCommand, RunResult};
use crate::slots::ClientSlots;
use std::collections::BTreeMap;
use std::sync::mpsc;

/// One delegation of a batch, as the result lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct DelegationReport {
    pub id: String,
    pub status: DelegationStatus,
    pub client: String,
    pub model: String,
    /// What every attempt of the delegation cost; 0 for one that never ran, and `None` when the ledger that holds
    /// it could not be read.
    pub cost_usd: Option<f64>,
    /// The file the delegation's full output is in; `None` for one that never produced any.
    pub output_path: Option<String>,
    /// The delegation's own summary, cut at `summary_max_chars`; for a failure that produced none, what went wrong.
    pub summary: String,
    /// Why a failed delegation failed: its last run's reason, `unavailable`, or the code of the refusal that stopped it.
    pub reason: Option<String>,
    /// The attempts that led to the result, when there was more than one.
    pub attempts: Option<Vec<AttemptSummary>>,
    /// The delegation that failed for good, which made the gateway skip this one.
    pub skipped_because: Option<String>,
    /// The worktree and branch of a writing delegation, where its staged change is kept until `worktrees clean`.
    pub worktree: Option<String>,
    pub branch: Option<String>,
}

impl DelegationReport {
    /// The report as the node runner prints it: its members in the order it spreads them, which is the same order
    /// for a run, an unavailable delegation, a failure and a skip.
    pub fn to_json(&self) -> Value {
        let mut members: Vec<(&str, Value)> = Vec::new();
        if let Some(attempts) = &self.attempts {
            members.push(("attempts", Value::Arr(attempts.iter().map(AttemptSummary::to_json).collect())));
        }
        members.push(("client", Value::str(&self.client)));
        members.push(("cost_usd", self.cost_usd.map_or(Value::Null, Value::Num)));
        members.push(("id", Value::str(&self.id)));
        members.push(("model", Value::str(&self.model)));
        members.push(("output_path", self.output_path.as_deref().map_or(Value::Null, Value::str)));
        if let Some(reason) = &self.reason {
            members.push(("reason", Value::str(reason)));
        }
        if let Some(branch) = &self.branch {
            members.push(("branch", Value::str(branch)));
        }
        if let Some(worktree) = &self.worktree {
            members.push(("worktree", Value::str(worktree)));
        }
        if let Some(skipped_because) = &self.skipped_because {
            members.push(("skipped_because", Value::str(skipped_because)));
        }
        members.push(("status", Value::str(self.status.as_str())));
        members.push(("summary", Value::str(&self.summary)));
        json::object(members)
    }
}

/// One delegation after the plan was checked: what it is, and what it waits for.
struct Planned {
    delegation: BatchDelegation,
    checked: Checked,
}

/// Hands a whole plan of delegations to the gateway and reports each one.
pub struct BatchCommand<'a> {
    pub runtime: &'a Runtime,
    pub runs: &'a RunCommand<'a>,
}

impl BatchCommand<'_> {
    /// Reads the plan, checks it as a whole, reserves its estimates together and runs it.
    pub fn run(&self, raw: &Value) -> Result<Value, GatewayError> {
        self.runs.refuse_nested()?;
        let config = load_config(self.runtime)?;
        let input = read_batch_input(raw, config.settings.max_batch_delegations)?;
        let planned = self.check(&input, &config)?;
        let held = self.reserve(&input, &planned, &config)?;
        let batch_id = self.batch_id();
        let unrun: Vec<ReservationRecord> = held.values().map(|reservation| reservation.record.clone()).collect();
        let reports = self.execute(&input, &config, &held, &batch_id);
        self.release_quietly(&unrun);
        Ok(self.result(&batch_id, &input, &reports, &unrun))
    }

    /// Every delegation's problems before anything is launched: the plan's dependencies, and for each delegation
    /// what a run of it would be refused for. A plan with any is refused with all of them.
    fn check(&self, input: &BatchInput, config: &GatewayConfig) -> Result<Vec<Planned>, GatewayError> {
        let mut problems: Vec<PlanProblem> = dependency_problems(&input.delegations);
        let mut planned: Vec<Planned> = Vec::new();
        for delegation in &input.delegations {
            let mut checked_input = delegation.input.clone();
            checked_input.prompt = estimated_prompt(&delegation.input.prompt, &delegation.depends_on, config.settings.summary_max_chars);
            match self.runs.check(&checked_input) {
                Ok(checked) => planned.push(Planned { delegation: delegation.clone(), checked }),
                Err(error) => problems.push(PlanProblem {
                    code: error.code.clone(),
                    id: delegation.id.clone(),
                    message: error.message.clone(),
                }),
            }
        }
        if !problems.is_empty() {
            return Err(plan_refused(problems));
        }
        Ok(planned)
    }

    /// Reserves the estimates of every metered delegation in one locked ledger update. A plan whose sum does not
    /// fit is refused, and holds nothing.
    fn reserve(
        &self,
        input: &BatchInput,
        planned: &[Planned],
        config: &GatewayConfig,
    ) -> Result<BTreeMap<String, Reservation>, GatewayError> {
        let now = self.runtime.clock.now_ms();
        let window = plan_window(input);
        let mut plan: Vec<PlannedReservation> = Vec::new();
        for item in planned {
            let Some(estimate) = item.checked.estimate.clone() else {
                continue;
            };
            // `allot` only names the run's id and directory; nothing is created until the run writes into it, so a
            // refused plan leaves none behind.
            let (run_dir, run_id) = self.runs.allot(item.checked.root.as_deref(), now)?;
            let mut delegation_input = item.delegation.input.clone();
            delegation_input.prompt =
                estimated_prompt(&item.delegation.input.prompt, &item.delegation.depends_on, config.settings.summary_max_chars);
            plan.push(PlannedReservation {
                id: item.delegation.id.clone(),
                cap: item.checked.cap.clone(),
                cost_class: item.checked.cost_class,
                estimate,
                input: delegation_input,
                run_dir,
                run_id,
                window_s: window,
            });
        }
        let reserved = (CapGuard { runtime: self.runtime }).reserve_plan(&plan, config, now)?;
        if !reserved.refused.is_empty() {
            let problems = reserved
                .refused
                .iter()
                .map(|refusal| PlanProblem { code: "cap-exceeded".to_string(), id: refusal.id.clone(), message: refusal.message.clone() })
                .collect();
            return Err(plan_refused(problems));
        }
        Ok(reserved.held)
    }

    /// Runs the plan: a delegation starts when everything it waits for is done and its client has a free slot, and
    /// one whose dependency failed for good (or was itself skipped) is skipped, its reservation given back at once.
    /// The slots a delegation is scheduled by are those of the client it was planned for; each attempt then takes a
    /// slot of the client it really runs on. A delegation that can never start is reported skipped, never dropped.
    fn execute(
        &self,
        input: &BatchInput,
        config: &GatewayConfig,
        held: &BTreeMap<String, Reservation>,
        batch_id: &str,
    ) -> BTreeMap<String, DelegationReport> {
        let limit = config.settings.max_parallel_per_client as usize;
        let mut reports: BTreeMap<String, DelegationReport> = BTreeMap::new();
        let mut outputs: BTreeMap<String, DependencyOutput> = BTreeMap::new();
        let mut pending: Vec<BatchDelegation> = input.delegations.clone();
        let mut planned: BTreeMap<String, usize> = BTreeMap::new();
        let slots = ClientSlots::new(limit);
        let (sender, receiver) = mpsc::channel::<(String, String, DelegationReport)>();
        let mut running = 0usize;
        std::thread::scope(|scope| {
            loop {
                self.skip_blocked(&mut pending, &mut reports, held);
                let mut index = 0;
                while index < pending.len() {
                    let delegation = pending[index].clone();
                    let client = delegation.input.client.clone();
                    let waits: Vec<Option<DependencyOutput>> =
                        delegation.depends_on.iter().map(|id| outputs.get(id).cloned()).collect();
                    if planned.get(&client).copied().unwrap_or(0) >= limit || waits.iter().any(Option::is_none) {
                        index += 1;
                        continue;
                    }
                    pending.remove(index);
                    *planned.entry(client.clone()).or_insert(0) += 1;
                    let given: Vec<DependencyOutput> = waits.into_iter().flatten().collect();
                    let tag = BatchTag { batch_id: batch_id.to_string(), delegation: delegation.id.clone() };
                    let reservation = held.get(&delegation.id).cloned();
                    let sender = sender.clone();
                    let slots = &slots;
                    let id = delegation.id.clone();
                    scope.spawn(move || {
                        // The last resort that keeps one crash from losing the batch: a delegation that panicked is a failed one.
                        let crashed = std::panic::AssertUnwindSafe(|| self.delegate(&delegation, &given, reservation, tag, slots));
                        let report = std::panic::catch_unwind(crashed).unwrap_or_else(|panic| {
                            let message = if let Some(text) = panic.downcast_ref::<&str>() {
                                (*text).to_string()
                            } else if let Some(text) = panic.downcast_ref::<String>() {
                                text.clone()
                            } else {
                                "a delegation panicked".to_string()
                            };
                            let error = GatewayError::new("unexpected", message);
                            failed_report(&delegation, &error, None, Some("unexpected".to_string()), None)
                        });
                        let _ = sender.send((id, client, report));
                    });
                    running += 1;
                }
                if running == 0 {
                    break;
                }
                // The scheduler holds a sender of its own, so the channel never closes while a delegation runs.
                let (id, client, report) = receiver.recv().expect("a running delegation reports");
                running -= 1;
                if report.status == DelegationStatus::Done {
                    outputs.insert(
                        id.clone(),
                        DependencyOutput { id: id.clone(), output_path: report.output_path.clone(), summary: report.summary.clone() },
                    );
                }
                let count = planned.entry(client).or_insert(1);
                *count = count.saturating_sub(1);
                reports.insert(id, report);
            }
        });
        // A checked plan leaves nothing pending here: a dependency either finishes, fails (which skips its dependents) or
        // was refused as a cycle, and `max_parallel_per_client` is at least 1. Loud rather than a delegation dropped from the result.
        assert!(pending.is_empty(), "a delegation was left that could never start");
        reports
    }

    /// Skips every pending delegation that waits for one that failed or was skipped, to the end of the chain.
    fn skip_blocked(
        &self,
        pending: &mut Vec<BatchDelegation>,
        reports: &mut BTreeMap<String, DelegationReport>,
        held: &BTreeMap<String, Reservation>,
    ) {
        loop {
            let blocked = {
                let seen: &BTreeMap<String, DelegationReport> = reports;
                pending.iter().find(|delegation| delegation.depends_on.iter().any(|id| stopped(seen, id).is_some())).cloned()
            };
            let Some(blocked) = blocked else {
                return;
            };
            let cause = {
                let seen: &BTreeMap<String, DelegationReport> = reports;
                blocked.depends_on.iter().find_map(|id| stopped(seen, id)).cloned()
            };
            let because = cause
                .as_ref()
                .and_then(|report| report.skipped_because.clone())
                .or_else(|| cause.as_ref().map(|report| report.id.clone()))
                .unwrap_or_default();
            self.skip(&blocked, &because, pending, reports, held);
        }
    }

    /// Takes a delegation off the pending list as skipped, because of the delegation named, and gives its
    /// reservation back.
    fn skip(
        &self,
        delegation: &BatchDelegation,
        because: &str,
        pending: &mut Vec<BatchDelegation>,
        reports: &mut BTreeMap<String, DelegationReport>,
        held: &BTreeMap<String, Reservation>,
    ) {
        pending.retain(|entry| entry.id != delegation.id);
        reports.insert(
            delegation.id.clone(),
            DelegationReport {
                id: delegation.id.clone(),
                status: DelegationStatus::Skipped,
                client: delegation.input.client.clone(),
                model: delegation.input.model.clone(),
                cost_usd: Some(0.0),
                output_path: None,
                summary: String::new(),
                reason: None,
                attempts: None,
                skipped_because: Some(because.to_string()),
                worktree: None,
                branch: None,
            },
        );
        if let Some(reservation) = held.get(&delegation.id) {
            self.release_quietly(std::slice::from_ref(&reservation.record));
        }
    }

    /// One delegation through the single-delegation path, whatever it comes to: a refusal or a crash is a failed
    /// delegation, never a lost batch. One that threw reports what its attempts cost, from the ledger lines they
    /// left, and gives its reservation back (or settles it at what its run cost).
    fn delegate(
        &self,
        delegation: &BatchDelegation,
        outputs: &[DependencyOutput],
        reservation: Option<Reservation>,
        tag: BatchTag,
        slots: &ClientSlots,
    ) -> DelegationReport {
        let mut input = delegation.input.clone();
        input.prompt = prompt_with_outputs(&delegation.input.prompt, outputs);
        let slot = |client: &str| slots.acquire(client);
        let given = DelegationOptions { reservation: reservation.clone(), tag: Some(tag.clone()), slot: Some(&slot) };
        match self.runs.delegation(&input, &given) {
            Ok(outcome) => report_of(delegation, outcome),
            Err(error) => {
                // Whatever the ledger does from here, the delegation is reported: it may have spent money and
                // staged a change.
                if let Some(reservation) = &reservation {
                    self.release_quietly(std::slice::from_ref(&reservation.record));
                }
                let Some(ledger) = self.read_quietly() else {
                    return failed_report(delegation, &error, None, Some("ledger-unreadable".to_string()), None);
                };
                let lines = tagged_lines(&ledger, &tag);
                let attempts = if lines.is_empty() { None } else { Some(lines.iter().map(attempt_of).collect()) };
                failed_report(delegation, &error, Some(cost_of(&lines)), None, attempts)
            }
        }
    }

    /// The ledger's lines, or `None` when it cannot be read: a lost ledger must not lose the batch's result.
    fn read_quietly(&self) -> Option<Vec<LedgerEntry>> {
        (LedgerStore { runtime: self.runtime }).read().ok()
    }

    /// Gives reservations back, best effort: one the ledger cannot take back lapses at its expiry.
    fn release_quietly(&self, records: &[ReservationRecord]) {
        let _ = (CapGuard { runtime: self.runtime }).release_unrun(records, self.runtime.clock.now_ms());
    }

    /// The batch's one result: each delegation in plan order with what its attempts cost, and what the plan gave back.
    fn result(
        &self,
        batch_id: &str,
        input: &BatchInput,
        reports: &BTreeMap<String, DelegationReport>,
        reservations: &[ReservationRecord],
    ) -> Value {
        let ledger = self.read_quietly();
        let mut delegations: Vec<DelegationReport> = Vec::new();
        for (delegation, report) in input.delegations.iter().filter_map(|delegation| reports.get(&delegation.id).map(|report| (delegation, report))) {
            let mut report = report.clone();
            if let Some(ledger) = &ledger {
                let tag = BatchTag { batch_id: batch_id.to_string(), delegation: delegation.id.clone() };
                report.cost_usd = Some(cost_of(&tagged_lines(ledger, &tag)));
            }
            delegations.push(report);
        }
        let costs: Vec<Option<f64>> = delegations.iter().map(|report| report.cost_usd).collect();
        let cost_usd = if costs.iter().any(Option::is_none) {
            Value::Null
        } else {
            Value::Num(round(costs.iter().map(|cost| cost.unwrap_or(0.0)).sum(), 6))
        };
        let released = ledger.as_ref().map(|ledger| {
            let given_back: Vec<&ReservationRecord> = reservations
                .iter()
                .filter(|record| {
                    ledger.iter().any(|entry| match entry {
                        LedgerEntry::Settled(settled) => settled.run_id == record.run_id && settled.reason.as_deref() == Some("not-run"),
                        _ => false,
                    })
                })
                .collect();
            round(given_back.iter().map(|record| record.estimate_usd).sum(), 6)
        });
        let statuses: Vec<DelegationStatus> = delegations.iter().map(|report| report.status).collect();
        let mut object = Object::new();
        object.set("batch_id", Value::str(batch_id));
        object.set("cost_usd", cost_usd);
        object.set("delegations", Value::Arr(delegations.iter().map(DelegationReport::to_json).collect()));
        object.set("reservations_released_usd", released.map_or(Value::Null, Value::Num));
        object.set("status", Value::str(batch_status(&statuses).as_str()));
        object.set("workflow", Value::str(&input.workflow));
        Value::Obj(object)
    }

    /// The batch's own id, which every ledger line of its attempts carries.
    fn batch_id(&self) -> String {
        format!("batch-{}", crate::run::run_id(self.runtime.clock.now_ms(), &crate::commands::random_suffix()))
    }
}

/// The report of a delegation that stopped, when the report is not one of a run: `None` when it is done.
fn stopped<'a>(reports: &'a BTreeMap<String, DelegationReport>, id: &str) -> Option<&'a DelegationReport> {
    reports.get(id).filter(|report| report.status != DelegationStatus::Done)
}

/// What a delegation's result says: its last run's, and the attempts that led to it.
fn report_of(delegation: &BatchDelegation, outcome: Delegation<RunResult>) -> DelegationReport {
    match outcome {
        Delegation::Unavailable(unavailable) => DelegationReport {
            id: delegation.id.clone(),
            status: DelegationStatus::Failed,
            client: unavailable.client,
            model: unavailable.model,
            cost_usd: Some(0.0),
            output_path: None,
            summary: unavailable.message,
            reason: Some("unavailable".to_string()),
            attempts: Some(unavailable.attempts),
            skipped_because: None,
            worktree: None,
            branch: None,
        },
        Delegation::Run(escalated) => {
            let run = escalated.run;
            let attempts = escalated.report.attempts;
            let write = run.write.as_ref().map(|write| (write.branch.clone(), write.worktree.clone()));
            DelegationReport {
                id: delegation.id.clone(),
                status: if run.ok { DelegationStatus::Done } else { DelegationStatus::Failed },
                client: run.client,
                model: run.model,
                cost_usd: Some(run.cost_usd),
                output_path: Some(run.output_path),
                summary: run.summary,
                reason: run.reason,
                attempts: if attempts.len() > 1 { Some(attempts) } else { None },
                skipped_because: None,
                worktree: write.as_ref().map(|(_, worktree)| worktree.clone()),
                branch: write.map(|(branch, _)| branch),
            }
        }
    }
}

/// The ledger lines the attempts of one batch delegation left: each is a run of its own, tagged with the batch and
/// the delegation.
fn tagged_lines(ledger: &[LedgerEntry], tag: &BatchTag) -> Vec<LedgerRecord> {
    crate::caps::run_records(ledger)
        .into_iter()
        .filter(|line| line.batch_id.as_deref() == Some(tag.batch_id.as_str()) && line.delegation.as_deref() == Some(tag.delegation.as_str()))
        .cloned()
        .collect()
}

/// What a delegation's attempts cost: their ledger lines added up.
fn cost_of(lines: &[LedgerRecord]) -> f64 {
    round(lines.iter().map(|line| line.cost_usd).sum(), 6)
}

/// A delegation that failed, with what is known of its cost: `None` when the ledger that holds it could not be read.
fn failed_report(
    delegation: &BatchDelegation,
    error: &GatewayError,
    cost_usd: Option<f64>,
    reason: Option<String>,
    attempts: Option<Vec<AttemptSummary>>,
) -> DelegationReport {
    DelegationReport {
        id: delegation.id.clone(),
        status: DelegationStatus::Failed,
        client: delegation.input.client.clone(),
        model: delegation.input.model.clone(),
        cost_usd,
        output_path: None,
        summary: error.message.clone(),
        reason: Some(reason.unwrap_or_else(|| error.code.clone())),
        attempts,
        skipped_because: None,
        worktree: None,
        branch: None,
    }
}

/// One ledger line, as an attempt the result lists.
fn attempt_of(line: &LedgerRecord) -> AttemptSummary {
    AttemptSummary {
        run_id: line.run_id.clone(),
        client: line.client.clone(),
        model: line.model.clone(),
        ok: line.ok,
        reason: line.reason.clone(),
    }
}

/// How long the plan's reservations hold if their runs die: as long as every delegation could take in turn, with
/// all its attempts. The window only matters to a run that died; a living one settles its own.
fn plan_window(input: &BatchInput) -> f64 {
    input
        .delegations
        .iter()
        .map(|delegation| {
            let attempts = if delegation.input.escalate == Some(false) {
                1.0
            } else {
                delegation.input.max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS)
            };
            delegation.input.time_limit_s * attempts
        })
        .sum()
}
