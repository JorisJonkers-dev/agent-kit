//! The machine-wide ledger file, read and appended under a lock.
//!
//! The ledger is one JSON line per run, reservation and settlement, appended
//! under an exclusive lock file beside it. Two gateway processes appending at
//! once take turns, so a line is never interleaved with another; a lock left
//! behind by a process that died is broken once it is old enough. A change
//! reads the ledger, decides and appends under that same lock, so two gateway
//! processes never both fit a reservation the other would have refused.

use crate::caps;
use crate::config::CostClass;
use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, RunTokens, SettledRecord};
use crate::locations;
use crate::lock::FileLock;
use crate::ports::Runtime;
use crate::text::{dirname, join};
use std::collections::BTreeMap;

/// The ledger file, read and appended through the runtime's files and clock.
pub struct LedgerStore<'a> {
    pub runtime: &'a Runtime,
}

/// What a change to the ledger asks for: the settlements to append, and what it decided.
pub struct LedgerChange<T> {
    pub append: Vec<SettledRecord>,
    pub result: T,
}

impl LedgerStore<'_> {
    /// Every record the ledger holds, oldest first.
    pub fn read(&self) -> Result<Vec<LedgerEntry>, GatewayError> {
        self.load()
    }

    /// Applies `change` to the ledger under the lock and appends what it settles.
    pub fn update<T>(&self, change: impl FnOnce(&[LedgerEntry]) -> LedgerChange<T>) -> Result<T, GatewayError> {
        let path = locations::ledger_path(&self.runtime.env)?;
        self.runtime.files.mkdir_all(dirname(&path)).map_err(GatewayError::from_fs)?;
        let lock = FileLock::new("ledger", self.runtime.lock);
        let files = self.runtime.files.as_ref();
        let clock = self.runtime.clock.as_ref();
        lock.run(&format!("{path}.lock"), files, clock, || {
            let changed = change(&self.load()?);
            if !changed.append.is_empty() {
                let text: String = changed
                    .append
                    .iter()
                    .map(|record| format!("{}\n", json::compact(&caps::settled_record_json(record))))
                    .collect();
                self.runtime.files.append(&path, &text).map_err(GatewayError::from_fs)?;
            }
            Ok(changed.result)
        })
    }

    /// Every record the ledger holds, oldest first. A ledger that is not there
    /// yet holds none; a line a writer left half-written is skipped, so one
    /// crashed run never hides the rest of the history.
    fn load(&self) -> Result<Vec<LedgerEntry>, GatewayError> {
        let path = locations::ledger_path(&self.runtime.env)?;
        let text = match self.runtime.files.read_text(&path) {
            Ok(text) => text,
            Err(error) if error.code == "ENOENT" => return Ok(Vec::new()),
            Err(error) => return Err(GatewayError::from_fs(error)),
        };
        Ok(text.lines().filter(|line| !line.trim().is_empty()).filter_map(parse_ledger_line).collect())
    }
}

/// One line of the ledger as the record it holds, or `None` when the line is
/// not one the gateway can read: a torn line, a value that is not an object, or
/// a reservation or settlement missing a field it must carry.
///
/// A line that is JSON but not an object is skipped here, where the node runner
/// would crash on it: one such line must not hide the rest of the history.
pub fn parse_ledger_line(line: &str) -> Option<LedgerEntry> {
    let value = json::parse(line).ok()?;
    let object = value.as_object()?;
    match object.get("kind").and_then(Value::as_str) {
        Some("reservation") => reservation_record(object).map(LedgerEntry::Reservation),
        Some("settled") => settled_record(object).map(LedgerEntry::Settled),
        _ => Some(LedgerEntry::Run(run_record(object))),
    }
}

/// A reservation line: every field it must carry, or `None` when one is missing or wrong.
fn reservation_record(object: &Object) -> Option<ReservationRecord> {
    Some(ReservationRecord {
        run_id: string_member(object, "run_id")?,
        workflow: string_member(object, "workflow")?,
        client: string_member(object, "client")?,
        cost_class: cost_class_member(object)?,
        model: string_member(object, "model")?,
        estimate_usd: number_member(object, "estimate_usd").filter(|estimate| estimate.is_finite())?,
        reserved_at: string_member(object, "reserved_at")?,
        expires_at: string_member(object, "expires_at")?,
        run_dir: string_member(object, "run_dir")?,
    })
}

/// A settlement line: every field it must carry, or `None` when one is missing or wrong.
fn settled_record(object: &Object) -> Option<SettledRecord> {
    Some(SettledRecord {
        run_id: string_member(object, "run_id")?,
        workflow: string_member(object, "workflow")?,
        client: string_member(object, "client")?,
        cost_class: cost_class_member(object)?,
        cost_usd: number_member(object, "cost_usd")?,
        reserved_at: string_member(object, "reserved_at")?,
        settled_at: string_member(object, "settled_at")?,
        reason: string_member(object, "reason"),
    })
}

/// A run's own line, read leniently: a field the node runner reads but does not
/// require is its empty value here, so an older line still counts.
fn run_record(object: &Object) -> LedgerRecord {
    LedgerRecord {
        at: string_member(object, "at").unwrap_or_default(),
        client: string_member(object, "client").unwrap_or_default(),
        cost_class: object.get("cost_class").and_then(Value::as_str).and_then(CostClass::parse),
        cost_usd: number_member(object, "cost_usd").unwrap_or(0.0),
        model: string_member(object, "model").unwrap_or_default(),
        ok: object.get("outcome").and_then(Value::as_str) == Some("ok"),
        reason: string_member(object, "reason"),
        unavailable_until: string_member(object, "unavailable_until"),
        run_id: string_member(object, "run_id").unwrap_or_default(),
        tokens: tokens_member(object),
        workflow: string_member(object, "workflow").unwrap_or_default(),
    }
}

/// What the client reported a run used, when the line carries all three readings.
fn tokens_member(object: &Object) -> Option<RunTokens> {
    let tokens = object.get("tokens")?.as_object()?;
    Some(RunTokens {
        input: number_member(tokens, "input")?,
        output: number_member(tokens, "output")?,
        cached: number_member(tokens, "cached")?,
    })
}

fn string_member(object: &Object, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

fn number_member(object: &Object, key: &str) -> Option<f64> {
    match object.get(key) {
        Some(Value::Num(number)) => Some(*number),
        _ => None,
    }
}

fn cost_class_member(object: &Object) -> Option<CostClass> {
    object.get("cost_class").and_then(Value::as_str).and_then(CostClass::parse)
}

/// Settles every reservation whose run left a result behind, at once and
/// whatever its expiry, and every one whose expiry has passed, from the run
/// directory's result where it left one and at its full estimate otherwise.
/// Every gateway call that reads the ledger for the caps runs this first, so a
/// run whose process died still counts, and one that finished counts what it
/// really cost rather than its estimate.
pub fn settle_stale(runtime: &Runtime, now_ms: i64) -> Result<(), GatewayError> {
    let store = LedgerStore { runtime };
    let entries = store.read()?;
    let mut costs: BTreeMap<String, f64> = BTreeMap::new();
    for reservation in caps::live_reservations(&entries) {
        if let Some(cost) = result_cost(runtime, &reservation.run_dir) {
            costs.insert(reservation.run_id.clone(), cost);
        }
    }
    store.update(|entries| LedgerChange { append: caps::settle_expired(entries, now_ms, &costs), result: () })
}

/// What a run directory's result says the run cost, or `None` when it left none
/// the gateway can read: a directory it cannot look at, a file it cannot read or
/// parse, or a result without a cost all leave the reservation to its expiry,
/// which counts its full estimate.
fn result_cost(runtime: &Runtime, run_dir: &str) -> Option<f64> {
    let path = join(&[run_dir, "result.json"]);
    if !matches!(runtime.files.exists(&path), Ok(true)) {
        return None;
    }
    let value = json::parse(&runtime.files.read_text(&path).ok()?).ok()?;
    match value.as_object()?.get("cost_usd") {
        Some(Value::Num(cost)) if cost.is_finite() && *cost >= 0.0 => Some(*cost),
        _ => None,
    }
}
