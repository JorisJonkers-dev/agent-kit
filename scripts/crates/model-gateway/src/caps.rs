//! The caps: live reservations, settlements, charges and the estimate.
//!
//! A run's estimate is reserved against the caps before it starts, and the
//! run's actual spend replaces it when it ends. A reservation whose run left a
//! result behind is settled from that result at once; one whose run left
//! nothing lapses at the run's time limit plus a grace period, and counts its
//! full estimate as spent. A run that wrote its own ledger line is counted from
//! that line, so its reservation holds nothing more.

use crate::config::CostClass;
use crate::json::{self, Object, Value};
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, SettledRecord};
use crate::timestamps;
use std::collections::BTreeMap;

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

/// The settlement as the ledger writes it: one JSON object, its keys in the
/// order the node runner's literal writes them.
pub fn settled_record_json(record: &SettledRecord) -> Value {
    let mut object = Object::new();
    object.set("client", Value::str(&record.client));
    object.set("cost_class", Value::str(record.cost_class.as_str()));
    object.set("cost_usd", Value::Num(record.cost_usd));
    object.set("kind", Value::str("settled"));
    if let Some(reason) = &record.reason {
        object.set("reason", Value::str(reason));
    }
    object.set("reserved_at", Value::str(&record.reserved_at));
    object.set("run_id", Value::str(&record.run_id));
    object.set("settled_at", Value::str(&record.settled_at));
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
