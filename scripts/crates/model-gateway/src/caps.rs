//! The caps: live reservations, settlements, charges and the estimate.
//!
//! A run's estimate is reserved against the caps before it starts, and the
//! run's actual spend replaces it when it ends. A reservation whose run left a
//! result behind is settled from that result at once; one whose run left
//! nothing lapses at the run's time limit plus a grace period, and counts its
//! full estimate as spent. A run that wrote its own ledger line is counted from
//! that line, so its reservation holds nothing more.

use crate::config::{CostClass, MeteredPersonalCaps, ModelPrices, Settings, Tier};
use crate::json::{self, Object, Value};
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, SettledRecord};
use crate::timestamps;
use std::collections::BTreeMap;

/// How many characters of a prompt the gateway counts as one token.
pub const CHARS_PER_TOKEN: f64 = 3.5;

/// How long a reservation outlives its run's time limit, so a settle that races the run still finds it.
pub const RESERVATION_GRACE_S: f64 = 60.0;

/// What a metered run is estimated to cost before it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    /// The prompt's tokens: its characters over 3.5, rounded up.
    pub prompt_tokens: f64,
    /// The run's difficulty tier's output allowance.
    pub output_tokens: f64,
    pub usd: f64,
}

/// The prompt's tokens: its ASCII characters over 3.5, plus one token for each
/// character that is not ASCII (CJK text costs about a token a character), rounded up.
pub fn prompt_tokens(prompt: &str) -> f64 {
    let ascii = prompt.chars().filter(char::is_ascii).count();
    let other = prompt.chars().count() - ascii;
    (ascii as f64 / CHARS_PER_TOKEN + other as f64).ceil()
}

/// What a run is estimated to cost: its prompt's tokens at the model's input
/// price, plus its tier's output allowance at the model's output price. A model
/// the catalogue prices nowhere, or gives no tier, cannot be estimated, and a
/// metered run on it is refused rather than run uncapped.
pub fn estimate_run(prompt: &str, tier: Option<Tier>, prices: Option<&ModelPrices>, settings: &Settings) -> Option<Estimate> {
    let (tier, prices) = (tier?, prices?);
    let prompt_tokens = prompt_tokens(prompt);
    let output_tokens = settings.allowance(tier);
    Some(Estimate {
        prompt_tokens,
        output_tokens,
        usd: round_up((prompt_tokens * prices.input + output_tokens * prices.output) / 1_000_000.0),
    })
}

/// An estimate, rounded up so a hard spend limit is never below what the run may
/// cost. The epsilon keeps float noise (1.4 * 1000 landing a hair above 1400)
/// from rounding a whole micro-dollar up.
fn round_up(value: f64) -> f64 {
    (value * 1_000_000.0 - 1e-6).ceil() / 1_000_000.0
}

/// The caps a metered-personal run is held to, as the machine names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapName {
    PerWorkflowUsd,
    PerDayUsd,
}

/// One cap, and what it holds for the run that asks: its limit, what is spent and what is reserved.
#[derive(Clone, Debug, PartialEq)]
pub struct CapRoom {
    pub cap: CapName,
    pub limit_usd: f64,
    pub spent_usd: f64,
    pub reserved_usd: f64,
}

/// The UTC day a moment falls on, as `YYYY-MM-DD`.
fn utc_day(at: &str) -> String {
    at.chars().take(10).collect()
}

fn sum(values: impl Iterator<Item = f64>) -> f64 {
    values.fold(0.0, |total, value| total + value)
}

fn room(cap: CapName, limit_usd: f64, spent_usd: f64, reserved_usd: f64) -> CapRoom {
    CapRoom { cap, limit_usd, reserved_usd: round(reserved_usd, 6), spent_usd: round(spent_usd, 6) }
}

/// The cap the run's estimate does not fit, with what it holds; `None` when it fits both: what the ledger says
/// is spent and what live reservations hold, in the run's workflow and on the reservation's UTC day, plus the
/// estimate itself. The workflow cap is checked first, so a run that fits neither is refused for its workflow.
///
/// Every line counts by the cost class it was charged under, never by its client's own class: a subsidised
/// client's metered provider is held too.
pub fn check_caps(entries: &[LedgerEntry], workflow: &str, estimate_usd: f64, caps: &MeteredPersonalCaps, now_ms: i64) -> Option<CapRoom> {
    let day = utc_day(&timestamps::to_iso(now_ms));
    let charges = metered_charges(entries, CostClass::MeteredPersonal);
    let held: Vec<&ReservationRecord> =
        live_reservations(entries).into_iter().filter(|reservation| reservation.cost_class == CostClass::MeteredPersonal).collect();
    let workflow_room = room(
        CapName::PerWorkflowUsd,
        caps.per_workflow_usd,
        sum(charges.iter().filter(|charge| charge.workflow == workflow).map(|charge| charge.usd)),
        sum(held.iter().filter(|reservation| reservation.workflow == workflow).map(|reservation| reservation.estimate_usd)),
    );
    if !fits(workflow_room.limit_usd, workflow_room.spent_usd, workflow_room.reserved_usd, estimate_usd) {
        return Some(workflow_room);
    }
    let day_room = room(
        CapName::PerDayUsd,
        caps.per_day_usd,
        sum(charges.iter().filter(|charge| utc_day(&charge.at) == day).map(|charge| charge.usd)),
        sum(held.iter().filter(|reservation| utc_day(&reservation.reserved_at) == day).map(|reservation| reservation.estimate_usd)),
    );
    if fits(day_room.limit_usd, day_room.spent_usd, day_room.reserved_usd, estimate_usd) {
        None
    } else {
        Some(day_room)
    }
}

/// Why a run does not fit a cap, naming the cap, its limit, what is spent and reserved, and the estimate.
pub fn cap_refusal_message(room: &CapRoom, estimate_usd: f64, workflow: &str, now_ms: i64) -> String {
    let (name, place) = match room.cap {
        CapName::PerWorkflowUsd => ("per-workflow", format!("in workflow {workflow}")),
        CapName::PerDayUsd => ("per-day", format!("on {}", utc_day(&timestamps::to_iso(now_ms)))),
    };
    format!(
        "does not fit the {name} cap of {}: {} spent and {} reserved {place}, and its estimate is {}",
        usd(room.limit_usd),
        usd(room.spent_usd),
        usd(room.reserved_usd),
        usd(estimate_usd)
    )
}

/// What a reservation is made of: the run, its estimate, and how long it holds.
pub struct ReservationArgs<'a> {
    pub run_id: &'a str,
    pub workflow: &'a str,
    pub client: &'a str,
    pub cost_class: CostClass,
    pub model: &'a str,
    pub estimate_usd: f64,
    pub reserved_at_ms: i64,
    pub time_limit_s: f64,
    pub run_dir: &'a str,
}

/// The reservation a metered run holds while it runs: its estimate, and when it lapses.
pub fn reservation_record(args: &ReservationArgs) -> ReservationRecord {
    // `new Date(x)` keeps whole milliseconds, truncating towards zero, as `as i64` does.
    let expires = args.reserved_at_ms as f64 + (args.time_limit_s + RESERVATION_GRACE_S) * 1000.0;
    ReservationRecord {
        run_id: args.run_id.to_string(),
        workflow: args.workflow.to_string(),
        client: args.client.to_string(),
        cost_class: args.cost_class,
        model: args.model.to_string(),
        estimate_usd: args.estimate_usd,
        reserved_at: timestamps::to_iso(args.reserved_at_ms),
        expires_at: timestamps::to_iso(expires as i64),
        run_dir: args.run_dir.to_string(),
    }
}

/// The settlement that releases a reservation: what the run really cost.
pub fn settled_record(reservation: &ReservationRecord, cost_usd: f64, at_ms: i64) -> SettledRecord {
    SettledRecord {
        run_id: reservation.run_id.clone(),
        workflow: reservation.workflow.clone(),
        client: reservation.client.clone(),
        cost_class: reservation.cost_class,
        cost_usd,
        reserved_at: reservation.reserved_at.clone(),
        settled_at: timestamps::to_iso(at_ms),
        reason: None,
    }
}

/// One settled charge against a cap: what a run cost, for which client, under
/// which workflow, at which moment.
#[derive(Clone, Debug, PartialEq)]
pub struct Charge {
    /// The moment the spend counts at: the run's own line, or the reservation it settled.
    pub at: String,
    /// The client the run went to, which a grant is given for.
    pub client: String,
    pub usd: f64,
    pub workflow: String,
}

/// The runs the ledger holds, oldest first: every line that is not a
/// reservation or a settlement.
pub fn run_records(entries: &[LedgerEntry]) -> Vec<&LedgerRecord> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            LedgerEntry::Run(record) => Some(record),
            _ => None,
        })
        .collect()
}

/// The reservations no settlement has released yet, and whose run left no line
/// of its own: what the caps hold right now. A run that wrote its own line has
/// been counted from that line, so its reservation holds nothing more, whether
/// or not the settlement that would release it ever came.
pub fn live_reservations(entries: &[LedgerEntry]) -> Vec<&ReservationRecord> {
    let settled: Vec<&str> = entries
        .iter()
        .filter_map(|entry| match entry {
            LedgerEntry::Settled(record) => Some(record.run_id.as_str()),
            _ => None,
        })
        .collect();
    let ran: Vec<&str> = run_records(entries).into_iter().map(|record| record.run_id.as_str()).collect();
    entries
        .iter()
        .filter_map(|entry| match entry {
            LedgerEntry::Reservation(record) if !settled.contains(&record.run_id.as_str()) && !ran.contains(&record.run_id.as_str()) => {
                Some(record)
            }
            _ => None,
        })
        .collect()
}

/// The settlements a ledger needs: every live reservation whose run left a
/// result behind, settled at once at the cost that result reports, and every
/// one whose expiry has passed, settled at its full estimate. A run that
/// finished and wrote its result is over, whatever its expiry says; the expiry
/// is the fallback for a run that left neither a line nor a result.
pub fn settle_expired(entries: &[LedgerEntry], now_ms: i64, costs: &BTreeMap<String, f64>) -> Vec<SettledRecord> {
    live_reservations(entries)
        .into_iter()
        .filter(|reservation| {
            costs.contains_key(&reservation.run_id)
                || timestamps::parse(&reservation.expires_at).is_some_and(|expires| expires <= now_ms)
        })
        .map(|reservation| SettledRecord {
            run_id: reservation.run_id.clone(),
            workflow: reservation.workflow.clone(),
            client: reservation.client.clone(),
            cost_class: reservation.cost_class,
            cost_usd: costs.get(&reservation.run_id).copied().unwrap_or(reservation.estimate_usd),
            reserved_at: reservation.reserved_at.clone(),
            settled_at: timestamps::to_iso(now_ms),
            reason: Some("expired".to_string()),
        })
        .collect()
}

/// What the ledger says runs of one cost class have cost, by workflow and by
/// moment. A run's own line carries its spend; a settlement counts only for a
/// run that has no line, which is a reservation whose run left nothing but its
/// result. Both count by the cost class they were charged under, so a metered
/// provider of a subsidised client is held too.
pub fn metered_charges(entries: &[LedgerEntry], cost_class: CostClass) -> Vec<Charge> {
    let runs: Vec<&LedgerRecord> = run_records(entries)
        .into_iter()
        .filter(|run| run.cost_class == Some(cost_class))
        .collect();
    // A JavaScript `Map` keeps the first insertion's place when a key is set again.
    let mut settled: Vec<(&str, &SettledRecord)> = Vec::new();
    for entry in entries {
        if let LedgerEntry::Settled(record) = entry {
            match settled.iter_mut().find(|(run_id, _)| *run_id == record.run_id) {
                Some(slot) => slot.1 = record,
                None => settled.push((record.run_id.as_str(), record)),
            }
        }
    }
    let mut charges: Vec<Charge> = runs
        .iter()
        .map(|run| Charge {
            at: settled
                .iter()
                .find(|(run_id, _)| *run_id == run.run_id)
                .map_or_else(|| run.at.clone(), |(_, record)| record.reserved_at.clone()),
            client: run.client.clone(),
            usd: run.cost_usd,
            workflow: run.workflow.clone(),
        })
        .collect();
    for (_, settle) in &settled {
        if settle.reason.as_deref() != Some("expired") || settle.cost_class != cost_class {
            continue;
        }
        if runs.iter().any(|run| run.run_id == settle.run_id) {
            continue;
        }
        charges.push(Charge {
            at: settle.reserved_at.clone(),
            client: settle.client.clone(),
            usd: settle.cost_usd,
            workflow: settle.workflow.clone(),
        });
    }
    charges
}

/// The settlement as the ledger writes it: one JSON object. A settlement that lapsed (`expired`) comes out of
/// `settle_expired`, whose literal writes its keys in order, `reason` between `kind` and `reserved_at`. A
/// released one (`not-started`, `not-run`) is the plain settlement with `reason` spread on, so the node runner
/// writes `reason` last.
pub fn settled_record_json(record: &SettledRecord) -> Value {
    let reason_last = record.reason.as_deref().is_some_and(|reason| reason != "expired");
    let mut object = Object::new();
    object.set("client", Value::str(&record.client));
    object.set("cost_class", Value::str(record.cost_class.as_str()));
    object.set("cost_usd", Value::Num(record.cost_usd));
    object.set("kind", Value::str("settled"));
    if let (Some(reason), false) = (&record.reason, reason_last) {
        object.set("reason", Value::str(reason));
    }
    object.set("reserved_at", Value::str(&record.reserved_at));
    object.set("run_id", Value::str(&record.run_id));
    object.set("settled_at", Value::str(&record.settled_at));
    object.set("workflow", Value::str(&record.workflow));
    if let (Some(reason), true) = (&record.reason, reason_last) {
        object.set("reason", Value::str(reason));
    }
    Value::Obj(object)
}

/// The reservation as the ledger writes it: one JSON object, its keys in the
/// order the node runner's literal writes them.
pub fn reservation_record_json(record: &ReservationRecord) -> Value {
    let mut object = Object::new();
    object.set("client", Value::str(&record.client));
    object.set("cost_class", Value::str(record.cost_class.as_str()));
    object.set("estimate_usd", Value::Num(record.estimate_usd));
    object.set("expires_at", Value::str(&record.expires_at));
    object.set("kind", Value::str("reservation"));
    object.set("model", Value::str(&record.model));
    object.set("reserved_at", Value::str(&record.reserved_at));
    object.set("run_dir", Value::str(&record.run_dir));
    object.set("run_id", Value::str(&record.run_id));
    object.set("workflow", Value::str(&record.workflow));
    Value::Obj(object)
}

/// Whether the estimate still fits what a cap has left, to the cent the ledger
/// counts in.
pub fn fits(limit_usd: f64, spent_usd: f64, reserved_usd: f64, estimate_usd: f64) -> bool {
    round(spent_usd + reserved_usd + estimate_usd, 6) <= limit_usd
}

/// Dollars as a message names them.
pub fn usd(value: f64) -> String {
    format!("${}", to_fixed(value, 2))
}

/// A sum of dollars or a ratio, rounded so the JSON does not carry float noise.
/// `Math.round` is `floor(x + 0.5)`, which rounds a half towards positive
/// infinity, unlike Rust's `f64::round`.
pub fn round(value: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    let scaled = value * factor;
    let floor = scaled.floor();
    (if scaled - floor >= 0.5 { floor + 1.0 } else { floor }) / factor
}

/// `Number.prototype.toFixed`: the exact binary value rounded to `places`
/// decimals, a half away from zero, and always with that many decimals. Rust's
/// `{:.2}` rounds a half to even, so `0.125` would print as `0.12` where
/// JavaScript prints `0.13`.
pub fn to_fixed(value: f64, places: usize) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string();
    }
    // Above 10^21 `toFixed` gives the same text as `String(value)`.
    if value.abs() >= 1e21 {
        return json::number_text(value);
    }
    let negative = value < 0.0;
    // Enough decimals to see the exact binary value, so a half is a half.
    let text = format!("{:.*}", places + 25, value.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((text.as_str(), ""));
    let (kept, rest) = fraction.split_at(places);
    let mut digits: Vec<u8> = whole.bytes().chain(kept.bytes()).collect();
    let up = rest.as_bytes().first().is_some_and(|digit| *digit >= b'5');
    if up {
        let mut at = digits.len();
        loop {
            if at == 0 {
                digits.insert(0, b'1');
                break;
            }
            at -= 1;
            if digits[at] == b'9' {
                digits[at] = b'0';
            } else {
                digits[at] += 1;
                break;
            }
        }
    }
    let split = digits.len() - places;
    let whole: String = digits[..split].iter().map(|digit| *digit as char).collect();
    let fraction: String = digits[split..].iter().map(|digit| *digit as char).collect();
    let sign = if negative { "-" } else { "" };
    if places == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{fraction}")
    }
}
