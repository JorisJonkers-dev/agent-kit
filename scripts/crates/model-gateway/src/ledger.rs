//! One line of the gateway's ledger: a run, a reservation or a settlement. The
//! ledger is machine-wide JSON lines; this is the data model, and `ledger_file`
//! reads and appends it.

use crate::config::CostClass;

/// What the client reported a run used.
#[derive(Clone, Debug, PartialEq)]
pub struct RunTokens {
    pub input: f64,
    pub output: f64,
    pub cached: f64,
}

/// One run's own line: what it was, what it cost and how it ended. Every field the node runner reads but does not require is optional here, as there.
#[derive(Clone, Debug, PartialEq)]
pub struct LedgerRecord {
    pub at: String,
    pub client: String,
    pub cost_class: Option<CostClass>,
    pub cost_usd: f64,
    pub model: String,
    /// `true` for outcome `ok`.
    pub ok: bool,
    pub reason: Option<String>,
    pub unavailable_until: Option<String>,
    pub run_id: String,
    pub tokens: Option<RunTokens>,
    pub workflow: String,
}

/// A metered run's estimate, held against the caps while it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct ReservationRecord {
    pub run_id: String,
    pub workflow: String,
    pub client: String,
    pub cost_class: CostClass,
    pub model: String,
    pub estimate_usd: f64,
    pub reserved_at: String,
    pub expires_at: String,
    pub run_dir: String,
}

/// A reservation released.
#[derive(Clone, Debug, PartialEq)]
pub struct SettledRecord {
    pub run_id: String,
    pub workflow: String,
    pub client: String,
    pub cost_class: CostClass,
    pub cost_usd: f64,
    pub reserved_at: String,
    pub settled_at: String,
    /// `expired`, `not-started` or `not-run`.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LedgerEntry {
    Run(LedgerRecord),
    Reservation(ReservationRecord),
    Settled(SettledRecord),
}
