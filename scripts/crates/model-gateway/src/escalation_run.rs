//! A delegation that does not come back clean. A failed result is retried once
//! on the same client model with the failure attached; a second failure climbs
//! to the next stronger permitted model that is no costlier; a client that
//! cannot start is swapped for another client model of the same cost class that
//! covers the tier. When nothing is left the last result goes back with
//! `escalation: "exhausted"`, or the delegation is `unavailable` when no client
//! model of its cost class can start. Each attempt is a run of its own.
//!
//! Batch delegations call `run` once each: it takes the single-run path as a
//! function, so it knows nothing of how a run is launched.

use crate::availability::client_availability;
use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::escalation::{
    attempt_input, bind_family, candidate_key, candidate_pool, climb_order, escalation_notice, fall_through_order, is_cannot_start, is_refusal,
    requested_candidate, retry_prompt, AttemptLink, AttemptSummary, Candidate, EscalationEnd, EscalationReport, UnavailableResult, Unstartable,
    DEFAULT_MAX_ATTEMPTS,
};
use crate::grant_guard::GrantGuard;
use crate::models::family_of_model_id;
use crate::policy::{decide_policy, resolve_scope, PolicySubject};
use crate::ports::Runtime;
use crate::run::RunInput;
use crate::text::{dirname, join, js_slice, js_trim};
use std::collections::BTreeSet;

/// What the escalation needs to know of one finished run, whatever else its result carries.
pub trait AttemptedRun: Clone {
    /// Whether the run's status is `ok`.
    fn is_ok(&self) -> bool;
    fn run_id(&self) -> &str;
    fn client(&self) -> &str;
    fn model(&self) -> &str;
    fn family(&self) -> Option<&str>;
    fn summary(&self) -> Option<&str>;
    /// A run answered by another family opens its summary with the escalation; the run keeps the key's place.
    fn set_summary(&mut self, summary: String);
    fn reason(&self) -> Option<&str>;
    fn output_path(&self) -> &str;
    /// The output of the check a writing run ran, when it ran one.
    fn check_output_path(&self) -> Option<&str>;
}

/// The check that a client can start: the reason it cannot, or `None` when it can.
pub type StartCheck<'a> = &'a dyn Fn(&RunInput) -> Result<Option<String>, GatewayError>;

/// What one attempt is launched with: how it links to the first, and the check that its client can start.
#[derive(Default)]
pub struct AttemptOptions<'a> {
    /// Absent on the first attempt, which nothing links to.
    pub link: Option<AttemptLink>,
    /// Called once everything that can refuse the run has passed and before anything is launched; a reason says
    /// the client cannot start, and the attempt then ends with `AttemptError::CannotStart` instead of a run.
    pub start_check: Option<StartCheck<'a>>,
}

/// Why an attempt ended without a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptError {
    /// The attempt's client cannot start, so the delegation swaps the client rather than failing.
    CannotStart(String),
    /// The gateway refused the attempt.
    Refused(GatewayError),
    /// Anything else that went wrong, by its message.
    Other(String),
}

impl AttemptError {
    /// The error a delegation that cannot go on without its first run ends with.
    fn into_gateway(self) -> GatewayError {
        match self {
            AttemptError::Refused(error) => error,
            AttemptError::CannotStart(message) | AttemptError::Other(message) => GatewayError::new("unexpected", message),
        }
    }
}

impl From<GatewayError> for AttemptError {
    /// A refusal of the gateway; a platform failure (code `unexpected`) is whatever else went wrong, by its message.
    fn from(error: GatewayError) -> Self {
        if error.code == "unexpected" {
            AttemptError::Other(error.message)
        } else {
            AttemptError::Refused(error)
        }
    }
}

/// A delegation's last run, with what the escalation did to get there.
#[derive(Clone, Debug, PartialEq)]
pub struct Escalated<R> {
    pub run: R,
    pub report: EscalationReport,
}

/// How a delegation ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Delegation<R> {
    Run(Escalated<R>),
    Unavailable(UnavailableResult),
}

/// Where the delegation's catalogue stands: the model asked for and every model it may move to.
struct Plan {
    summary_max: f64,
    requested: Candidate,
    pool: Vec<Candidate>,
}

/// What a client that cannot start leads to: another client model to try, or the end of the delegation.
enum Step<R> {
    Swap(Candidate),
    End(Delegation<R>),
}

fn summary_of(run: &impl AttemptedRun) -> AttemptSummary {
    AttemptSummary {
        run_id: run.run_id().to_string(),
        client: run.client().to_string(),
        model: run.model().to_string(),
        ok: run.is_ok(),
        reason: run.reason().map(str::to_string),
    }
}

pub struct EscalatedRun<'a> {
    pub runtime: &'a Runtime,
}

/// The state of one delegation as it walks through its attempts.
struct Walk<'a> {
    escalated: &'a EscalatedRun<'a>,
    input: &'a RunInput,
    plan: Option<Plan>,
    attempts: Vec<AttemptSummary>,
    skipped: Vec<Unstartable>,
    dead: BTreeSet<String>,
    /// The model the next swap is measured from: the one the slot wanted, or the one that failed twice when climbing.
    from: Option<Candidate>,
    climbing: bool,
}

impl<'a> EscalatedRun<'a> {
    /// Runs the delegation: `attempt` is the single-run path, called once per attempt. It is a trait object so
    /// the loop is compiled once per kind of run, whatever closure launches it.
    pub fn run<R: AttemptedRun>(
        &self,
        input: &RunInput,
        attempt: &mut dyn FnMut(&RunInput, &AttemptOptions) -> Result<R, AttemptError>,
    ) -> Result<Delegation<R>, GatewayError> {
        if input.escalate == Some(false) {
            let only = attempt(input, &AttemptOptions::default()).map_err(AttemptError::into_gateway)?;
            let attempts = vec![summary_of(&only)];
            return Ok(Delegation::Run(Escalated { run: only, report: EscalationReport { attempts, ..EscalationReport::default() } }));
        }
        let max_attempts = input.max_attempts.unwrap_or(DEFAULT_MAX_ATTEMPTS);
        let start_check = |checked: &RunInput| self.start_check(checked);
        let mut walk = Walk {
            escalated: self,
            input,
            plan: None,
            attempts: Vec::new(),
            skipped: Vec::new(),
            dead: BTreeSet::new(),
            from: None,
            climbing: false,
        };
        // The model to launch next; `None` is the model the caller asked for, launched as the caller sent it.
        let mut current: Option<Candidate> = None;
        let mut failures = 0;
        // The attempts that count toward `max_attempts`: a run whose client could not start does not.
        let mut counted = 0.0;
        let mut prompt = input.prompt.clone();
        let mut last: Option<R> = None;
        // The last attempt that really ran: a run with an output path and a cost, which a later attempt that cannot start never replaces.
        let mut real: Option<R> = None;
        loop {
            if let Some(run) = &last {
                if counted >= max_attempts {
                    return walk.done(run.clone(), Some(EscalationEnd::Exhausted), None);
                }
            }
            let link = walk.attempts.first().map(|first| AttemptLink { attempt: walk.attempts.len() + 1, attempt_of: Some(first.run_id.clone()) });
            let next = match &current {
                None => input.clone(),
                Some(candidate) => attempt_input(input, candidate, &prompt),
            };
            let run = match attempt(&next, &AttemptOptions { link, start_check: Some(&start_check) }) {
                Ok(run) => run,
                Err(AttemptError::CannotStart(message)) => {
                    match walk.cannot_start(&current, &real, &message)? {
                        Step::Swap(swapped) => current = Some(swapped),
                        Step::End(ended) => return Ok(ended),
                    }
                    continue;
                }
                Err(error) => {
                    let Some(run) = &last else {
                        return Err(error.into_gateway());
                    };
                    let stopped = match error {
                        AttemptError::Refused(refused) => format!("{}: {}", refused.code, refused.message),
                        AttemptError::Other(message) | AttemptError::CannotStart(message) => message,
                    };
                    return walk.done(run.clone(), Some(EscalationEnd::Stopped), Some(stopped));
                }
            };
            walk.attempts.push(summary_of(&run));
            last = Some(run.clone());
            if run.is_ok() {
                return walk.done(run, None, None);
            }
            if is_cannot_start(run.reason()) {
                let reason = run.reason().unwrap_or("launch-failed").to_string();
                match walk.cannot_start(&current, &real, &reason)? {
                    Step::Swap(swapped) => current = Some(swapped),
                    Step::End(ended) => return Ok(ended),
                }
                continue;
            }
            real = Some(run.clone());
            counted += 1.0;
            if is_refusal(run.reason()) {
                return walk.done(run, None, None);
            }
            failures += 1;
            let again = match &current {
                Some(candidate) => candidate.clone(),
                None => walk.planned()?.requested.clone(),
            };
            walk.from = Some(again.clone());
            current = Some(again);
            walk.climbing = false;
            prompt = retry_prompt(&input.prompt, run.reason().unwrap_or("failed"), &self.failure_detail(&run));
            if failures == 1 {
                continue;
            }
            walk.climbing = true;
            let Some(stronger) = walk.replacement()? else {
                return walk.done(run, Some(EscalationEnd::Exhausted), None);
            };
            current = Some(stronger);
            failures = 0;
        }
    }

    /// The catalogue the delegation may move through: the model asked for, and every permitted model it may be
    /// moved to. A metered-employer model is in that pool only while a live grant covers the workflow, so a client
    /// that cannot start is swapped for one of them only when the owner allowed it.
    fn plan(&self, input: &RunInput) -> Result<Plan, GatewayError> {
        let config = load_config(self.runtime)?;
        let entry = config.models.iter().find(|candidate| candidate.client == input.client && candidate.model == input.model);
        let family = entry
            .map(|entry| entry.family.clone())
            .or_else(|| family_of_model_id(&input.model).map(str::to_string))
            .unwrap_or_default();
        let requested = requested_candidate(&config, input, entry, &family);
        let scope = resolve_scope(self.runtime, input.repo.as_deref())?;
        let granted = (GrantGuard { runtime: self.runtime }).live_clients(&input.workflow, self.runtime.clock.now_ms())?;
        let subjects: Vec<PolicySubject> = config
            .models
            .iter()
            .map(|model| PolicySubject { client: model.client.clone(), model: model.model.clone(), family: model.family.clone() })
            .collect();
        let permissions = decide_policy(&config, &scope, &subjects).permissions;
        let pool = bind_family(input, &requested, candidate_pool(&config, input, &permissions, &granted));
        Ok(Plan { pool, requested, summary_max: config.settings.summary_max_chars })
    }

    /// Why a client cannot start, or `None` when it can: what `usage` reports as its availability.
    fn start_check(&self, input: &RunInput) -> Result<Option<String>, GatewayError> {
        let config = load_config(self.runtime)?;
        Ok(match client_availability(self.runtime, &config, &input.client)? {
            Some(availability) if availability.status != "available" => Some(format!("{}: {}", availability.status, availability.reason)),
            _ => None,
        })
    }

    /// What a failed attempt tells the next one: the check's output for a writing run whose check failed, else
    /// what the client printed on stderr, else its output. Whatever cannot be read says nothing.
    fn failure_detail(&self, run: &impl AttemptedRun) -> String {
        let check = if run.reason().is_some_and(|reason| reason.starts_with("check-")) { run.check_output_path() } else { None };
        let stderr = join(&[dirname(run.output_path()), "stderr.txt"]);
        for path in [check, Some(stderr.as_str()), Some(run.output_path())].into_iter().flatten() {
            // A missing stderr file is a client that printed none.
            let text = self.runtime.files.read_text(path).unwrap_or_default();
            if !js_trim(&text).is_empty() {
                return text;
            }
        }
        String::new()
    }
}

impl Walk<'_> {
    /// The catalogue, read once the delegation needs it.
    fn planned(&mut self) -> Result<&Plan, GatewayError> {
        if self.plan.is_none() {
            self.plan = Some(self.escalated.plan(self.input)?);
        }
        Ok(self.plan.as_ref().expect("the plan was just read"))
    }

    /// A delegation's last run, with what the escalation did to get there. A run answered by another family
    /// opens its summary with the escalation: both families and the cause.
    fn done<R: AttemptedRun>(&mut self, mut run: R, escalation: Option<EscalationEnd>, stopped_reason: Option<String>) -> Result<Delegation<R>, GatewayError> {
        let stated = match (run.is_ok(), run.family(), run.summary()) {
            (true, Some(family), Some(summary)) => Some((family.to_string(), summary.to_string())),
            _ => None,
        };
        if let Some((family, summary)) = stated {
            let (requested, summary_max) = {
                let plan = self.planned()?;
                (plan.requested.clone(), plan.summary_max)
            };
            if family != requested.family {
                let cause = self
                    .skipped
                    .iter()
                    .find(|who| who.client == requested.client && who.model == requested.model)
                    .map(|who| who.reason.clone())
                    .or_else(|| self.attempts.first().and_then(|first| first.reason.clone()))
                    .unwrap_or_else(|| "failed".to_string());
                let opened = format!("{}\n{summary}", escalation_notice(&requested.client, &requested.family, &cause, &family));
                run.set_summary(js_slice(&opened, 0.0, Some(summary_max)));
            }
        }
        let report = EscalationReport { attempts: self.attempts.clone(), escalation, skipped: self.skipped.clone(), stopped_reason };
        Ok(Delegation::Run(Escalated { run, report }))
    }

    /// The delegation that no permitted client model of its cost class can start.
    fn unavailable<R>(&mut self) -> Result<Delegation<R>, GatewayError> {
        let requested = self.planned()?.requested.clone();
        let from = self.from.clone().unwrap_or(requested);
        let binding = if self.input.family_binding == Some(false) {
            String::new()
        } else {
            format!("; the family {} is binding, so no other family answers", from.family)
        };
        Ok(Delegation::Unavailable(UnavailableResult {
            attempts: self.attempts.clone(),
            client: self.input.client.clone(),
            message: format!(
                "{}:{} cannot start, and no other permitted {} client model covers its tier; the gateway never moves into a costlier cost class by itself{binding}",
                from.client,
                from.model,
                from.cost_class.as_str()
            ),
            model: self.input.model.clone(),
            reasons: self.skipped.clone(),
            workflow: self.input.workflow.clone(),
        }))
    }

    fn pass_over(&mut self, client: &str, model: &str, reason: &str) {
        self.dead.insert(candidate_key(client, model));
        self.skipped.push(Unstartable { client: client.to_string(), model: model.to_string(), reason: reason.to_string() });
    }

    /// The next client model the delegation may move to: a climb moves to a stronger one, any other swap to one
    /// that covers the tier, and neither to one that already could not start.
    fn replacement(&mut self) -> Result<Option<Candidate>, GatewayError> {
        let climbing = self.climbing;
        let from = self.from.clone();
        let plan = self.planned()?;
        let from = from.as_ref().unwrap_or(&plan.requested);
        let order = if climbing { climb_order(from, &plan.pool) } else { fall_through_order(from, &plan.pool) };
        Ok(order.into_iter().find(|candidate| !self.dead.contains(&candidate_key(&candidate.client, &candidate.model))))
    }

    /// The client of the attempt that just ran cannot start: pass it over, and swap it for another client model,
    /// or end the delegation when none can stand in for it.
    fn cannot_start<R: AttemptedRun>(&mut self, current: &Option<Candidate>, real: &Option<R>, reason: &str) -> Result<Step<R>, GatewayError> {
        match current {
            Some(candidate) => self.pass_over(&candidate.client, &candidate.model, reason),
            None => self.pass_over(&self.input.client, &self.input.model, reason),
        }
        if let Some(swapped) = self.replacement()? {
            return Ok(Step::Swap(swapped));
        }
        let Some(real) = real else {
            return Ok(Step::End(self.unavailable()?));
        };
        let ended = if self.climbing {
            self.done(real.clone(), Some(EscalationEnd::Exhausted), None)?
        } else {
            self.done(real.clone(), Some(EscalationEnd::Stopped), Some(reason.to_string()))?
        };
        Ok(Step::End(ended))
    }
}
