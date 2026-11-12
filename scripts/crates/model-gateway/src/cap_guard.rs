//! The caps a metered run is held to: what it is estimated to cost, the
//! reservation it holds before it starts, and the settlement that replaces it
//! with what the run really cost.
//!
//! A metered-personal run is held to the per-workflow and per-day caps; a
//! metered-employer run to the grant that covers its workflow. A reservation is
//! decided and written in one locked critical section that also settles the
//! reservations whose runs died, so two gateway processes cannot together
//! overshoot a cap.

use crate::caps::{self, Estimate, ReservationArgs};
use crate::config::{ClientConfig, CostClass, GatewayConfig, ModelEntry};
use crate::error::GatewayError;
use crate::grants::{self, Grant};
use crate::ledger::{LedgerEntry, ReservationRecord, SettledRecord};
use crate::ledger_file::{finished_costs, LedgerAppend, LedgerBatch, LedgerChange, LedgerStore};
use crate::models::cost_class_for;
use crate::ports::Runtime;
use crate::run::{open_code_provider, RunInput};
use crate::state_file::Workflows;
use crate::workflows::workflow_ends_at;
use std::collections::BTreeMap;

/// A run under a grant needs at least this long before the grant or its workflow lapses.
pub const MIN_GRANT_TIME_LEFT_S: i64 = 60;

/// What a metered run holds while it runs: the estimate it reserved, and the ledger line that holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reservation {
    pub estimate: Estimate,
    pub record: ReservationRecord,
    /// The time limit the run runs under: its own, clamped to what is left of its grant and workflow.
    pub time_limit_s: f64,
    /// For a run under a grant, the moment (epoch milliseconds) its grant or workflow lapses, whichever comes
    /// first; the run's check is held to it too.
    pub ends_at_ms: Option<i64>,
}

/// What a metered run's estimate is held against: the metered-personal caps, or the grant that covers a
/// metered-employer run. A subsidised run is held to neither, and its plan limit watches it instead.
#[derive(Clone, Debug, PartialEq)]
pub enum CapSource {
    MeteredPersonal,
    Grant(Grant),
}

/// One delegation of a batch plan, ready to have its estimate reserved up front.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedReservation {
    /// The delegation's id in the plan, which the result is keyed by.
    pub id: String,
    pub cap: CapSource,
    pub cost_class: CostClass,
    pub estimate: Estimate,
    pub input: RunInput,
    pub run_dir: String,
    pub run_id: String,
    /// How long the reservation holds if its run dies, before the grace period: the plan's longest possible duration.
    pub window_s: f64,
}

/// A delegation whose estimate does not fit, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRefusal {
    pub id: String,
    pub message: String,
}

/// What reserving a plan came to: the reservation each delegation holds, or the ones that did not fit (and then nothing is held).
#[derive(Clone, Debug, PartialEq)]
pub struct PlanReservation {
    pub held: BTreeMap<String, Reservation>,
    pub refused: Vec<PlanRefusal>,
}

/// What `reserve` is asked to hold.
pub struct ReserveArgs<'a> {
    pub cap: &'a CapSource,
    pub config: &'a GatewayConfig,
    pub cost_class: CostClass,
    pub estimate: Option<&'a Estimate>,
    pub input: &'a RunInput,
    pub now_ms: i64,
    pub run_dir: &'a str,
    pub run_id: &'a str,
}

pub struct CapGuard<'a> {
    pub runtime: &'a Runtime,
}

/// The ledger's entries followed by settlements the same change decides.
fn with_settled(entries: &[LedgerEntry], settled: &[SettledRecord]) -> Vec<LedgerEntry> {
    entries.iter().cloned().chain(settled.iter().cloned().map(LedgerEntry::Settled)).collect()
}

impl CapGuard<'_> {
    /// What a metered run is estimated to cost, or a `no-estimate` refusal for a client model the catalogue
    /// prices nowhere or gives no tier: a run the gateway cannot estimate is never run uncapped. A subsidised
    /// run is held to nothing, so it has no estimate.
    pub fn estimate(&self, input: &RunInput, entry: Option<&ModelEntry>, config: &GatewayConfig) -> Result<Option<Estimate>, GatewayError> {
        if held_cost_class(input, config)?.is_none() {
            return Ok(None);
        }
        let estimate = caps::estimate_run(&input.prompt, entry.map(|entry| entry.tier), entry.and_then(|entry| entry.prices.as_ref()), &config.settings);
        match estimate {
            Some(estimate) => Ok(Some(estimate)),
            None => Err(GatewayError::new(
                "no-estimate",
                format!(
                    "{}:{} cannot be estimated: the catalogue gives it no price or no tier, so a metered run on it cannot be held to the caps",
                    input.client, input.model
                ),
            )),
        }
    }

    /// Reserves the run's estimate against the caps or the grant that holds it, or refuses it with the cap it
    /// does not fit. The decision and the reservation are one locked critical section over the ledger, which
    /// also settles every reservation whose run left no settlement and whose expiry has passed.
    pub fn reserve(&self, args: &ReserveArgs) -> Result<Option<Reservation>, GatewayError> {
        let Some(estimate) = args.estimate else {
            return Ok(None);
        };
        let input = args.input;
        let held = match args.cap {
            CapSource::Grant(grant) => Some(self.grant_time_limit(grant, input, args.now_ms)?),
            CapSource::MeteredPersonal => None,
        };
        let time_limit_s = held.map_or(input.time_limit_s, |(limit, _)| limit);
        let store = LedgerStore { runtime: self.runtime };
        let costs = finished_costs(self.runtime, &store.read()?);
        let record = caps::reservation_record(&ReservationArgs {
            run_id: args.run_id,
            workflow: &input.workflow,
            client: &input.client,
            cost_class: args.cost_class,
            model: &input.model,
            estimate_usd: estimate.usd,
            reserved_at_ms: args.now_ms,
            time_limit_s,
            run_dir: args.run_dir,
        });
        let decided = store.update_with(|entries| {
            let settled = caps::settle_expired(entries, args.now_ms, &costs);
            let all = with_settled(entries, &settled);
            let message = match args.cap {
                CapSource::Grant(grant) => grants::grant_refusal(&all, grant, estimate.usd, &input.client, &input.model),
                CapSource::MeteredPersonal => cap_refusal(&all, input, estimate.usd, args.config, args.now_ms),
            };
            let reserved = if message.is_none() { Some(record.clone()) } else { None };
            Ok(LedgerAppend { settled, reserved, result: message })
        });
        if let Some(message) = decided? {
            return Err(GatewayError::new("cap-exceeded", message));
        }
        Ok(Some(Reservation { estimate: estimate.clone(), record, time_limit_s, ends_at_ms: held.map(|(_, ends_at)| ends_at) }))
    }

    /// The time limit a run under a grant may have: its own, clamped to the time left until the grant lapses
    /// and until its workflow's 24 hours (or its end) run out, so a run launched just before either keeps no
    /// spend running past it. With under a minute left the run is refused (`grant-expiring`). Returns the limit
    /// and the moment it ends at.
    fn grant_time_limit(&self, grant: &Grant, input: &RunInput, now_ms: i64) -> Result<(f64, i64), GatewayError> {
        let records = (Workflows { runtime: self.runtime }).read()?;
        let expires = crate::timestamps::parse(&grant.expires_at).unwrap_or_default();
        let ends_at = records.iter().find(|candidate| candidate.name == input.workflow).map_or(expires, |record| expires.min(workflow_ends_at(record)));
        let left_s = (ends_at - now_ms).div_euclid(1000);
        if left_s < MIN_GRANT_TIME_LEFT_S {
            return Err(GatewayError::new(
                "grant-expiring",
                format!(
                    "the grant covering workflow {} has {} seconds left: wait for it to lapse, then ask the owner for a new grant, or start a new workflow",
                    input.workflow,
                    left_s.max(0)
                ),
            ));
        }
        Ok((input.time_limit_s.min(left_s as f64), ends_at))
    }

    /// Releases a reservation with what the run really cost, so the caps count spend rather than the estimate.
    pub fn settle(&self, reservation: Option<&Reservation>, cost_usd: f64, now_ms: i64) -> Result<(), GatewayError> {
        let Some(reservation) = reservation else {
            return Ok(());
        };
        self.append(caps::settled_record(&reservation.record, cost_usd, now_ms))
    }

    /// Releases a reservation the run never used, at no cost: the workflow could not be started after the
    /// estimate was held, so the caps must not keep holding it.
    pub fn release(&self, reservation: Option<&Reservation>, now_ms: i64) -> Result<(), GatewayError> {
        let Some(reservation) = reservation else {
            return Ok(());
        };
        self.append(SettledRecord { reason: Some("not-started".to_string()), ..caps::settled_record(&reservation.record, 0.0, now_ms) })
    }

    /// One settlement, appended under the ledger's lock.
    fn append(&self, record: SettledRecord) -> Result<(), GatewayError> {
        (LedgerStore { runtime: self.runtime }).update(|_| LedgerChange { append: vec![record], result: () })
    }

    /// Reserves a whole batch plan's estimates at once: each is checked, in plan order, against the caps or its
    /// grant with the ones before it already held, and every record is written in the same locked ledger update
    /// that decided them, so no other process can take room in between. A plan any of whose estimates does not
    /// fit holds nothing: the refusals come back, and no record is written.
    pub fn reserve_plan(&self, plan: &[PlannedReservation], config: &GatewayConfig, now_ms: i64) -> Result<PlanReservation, GatewayError> {
        let store = LedgerStore { runtime: self.runtime };
        let costs = finished_costs(self.runtime, &store.read()?);
        store.update_many(|entries| {
            let settled = caps::settle_expired(entries, now_ms, &costs);
            let mut all = with_settled(entries, &settled);
            let mut held = BTreeMap::new();
            let mut records: Vec<ReservationRecord> = Vec::new();
            let mut refused = Vec::new();
            for item in plan {
                let message = match &item.cap {
                    CapSource::Grant(grant) => grants::grant_refusal(&all, grant, item.estimate.usd, &item.input.client, &item.input.model),
                    CapSource::MeteredPersonal => cap_refusal(&all, &item.input, item.estimate.usd, config, now_ms),
                };
                if let Some(message) = message {
                    refused.push(PlanRefusal { id: item.id.clone(), message });
                    continue;
                }
                let record = caps::reservation_record(&ReservationArgs {
                    run_id: &item.run_id,
                    workflow: &item.input.workflow,
                    client: &item.input.client,
                    cost_class: item.cost_class,
                    model: &item.input.model,
                    estimate_usd: item.estimate.usd,
                    reserved_at_ms: now_ms,
                    time_limit_s: item.window_s,
                    run_dir: &item.run_dir,
                });
                held.insert(
                    item.id.clone(),
                    Reservation { estimate: item.estimate.clone(), record: record.clone(), time_limit_s: item.input.time_limit_s, ends_at_ms: None },
                );
                all.push(LedgerEntry::Reservation(record.clone()));
                records.push(record);
            }
            // A plan with any refusal holds nothing, so only the settlements are written.
            let reserved = if refused.is_empty() { records } else { Vec::new() };
            Ok(LedgerBatch { settled, reserved, result: PlanReservation { held, refused } })
        })
    }

    /// Gives back the reservations of a batch's delegations, whatever of them is still held. A run that left a
    /// result before it was lost counts what that result says it cost (settled as a lapsed reservation is,
    /// which is what makes the caps and grants count it); only one that left none never ran, and is settled at
    /// no cost. One a run already settled, or one that has its own line, is left alone, so it can be called
    /// again for the same plan.
    pub fn release_unrun(&self, records: &[ReservationRecord], now_ms: i64) -> Result<(), GatewayError> {
        let store = LedgerStore { runtime: self.runtime };
        let costs = finished_costs(self.runtime, &store.read()?);
        store.update(|entries| {
            let live: Vec<&str> = caps::live_reservations(entries).into_iter().map(|reservation| reservation.run_id.as_str()).collect();
            let append = records
                .iter()
                .filter(|record| live.contains(&record.run_id.as_str()))
                .map(|record| match costs.get(&record.run_id) {
                    None => SettledRecord { reason: Some("not-run".to_string()), ..caps::settled_record(record, 0.0, now_ms) },
                    Some(cost) => SettledRecord { reason: Some("expired".to_string()), ..caps::settled_record(record, *cost, now_ms) },
                })
                .collect();
            LedgerChange { append, result: () }
        })
    }
}

/// The cost class a run is held to a cap by, or `None` for a subsidised run, which its plan limit watches
/// instead: metered-personal against the caps, and metered-employer against its grant. A client the
/// configuration does not declare is refused (`unknown-client`), never treated as subsidised.
pub fn held_cost_class(input: &RunInput, config: &GatewayConfig) -> Result<Option<CostClass>, GatewayError> {
    let Some(client) = config.clients.get(&input.client) else {
        return Err(GatewayError::new(
            "unknown-client",
            format!(
                "the gateway knows no client {}, so it may not run: a client the configuration does not declare is never held to nothing",
                input.client
            ),
        ));
    };
    Ok(Some(run_cost_class(client, input)).filter(|class| *class != CostClass::Subsidised))
}

/// Why a run does not fit the metered-personal caps, naming the cap, its limit, what is spent and reserved, and the estimate.
fn cap_refusal(entries: &[LedgerEntry], input: &RunInput, estimate_usd: f64, config: &GatewayConfig, now_ms: i64) -> Option<String> {
    let room = caps::check_caps(entries, &input.workflow, estimate_usd, &config.settings.metered_personal_caps, now_ms)?;
    Some(format!("{}:{} {}", input.client, input.model, caps::cap_refusal_message(&room, estimate_usd, &input.workflow, now_ms)))
}

/// The cost class of one run: the class of the provider it runs on, else its client's own.
pub fn run_cost_class(client: &ClientConfig, input: &RunInput) -> CostClass {
    let provider = if input.client == "opencode" { open_code_provider(&input.model) } else { input.provider.as_deref() };
    cost_class_for(client, provider)
}
