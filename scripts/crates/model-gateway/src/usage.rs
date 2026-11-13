//! What every client can do now and what it has spent.
//!
//! Availability is found passively. The gateway asks PATH, the client's own
//! status command where it has one, and the ledger's recent outcomes; it never
//! sends a prompt to find out. A reading it cannot get is `unknown`, never a
//! zero that would read as an exhausted plan.

use crate::caps::{self, run_records};
use crate::config::{ClientConfig, GatewayConfig, PlanLimit, PlanWindow, Settings};
use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ledger::{LedgerEntry, LedgerRecord, ReservationRecord, RunTokens};
use crate::models::SkippedModel;
use crate::timestamps;
use crate::workflows;

/// How many days of usage a report covers when the caller names none.
pub const DEFAULT_USAGE_DAYS: f64 = 7.0;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// What a `usage` call asks for: how many days, and optionally one workflow.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageInput {
    pub days: f64,
    pub workflow: Option<String>,
}

/// Reads a usage call's input, or says what is wrong with it.
pub fn read_usage_input(raw: &Value) -> Result<UsageInput, GatewayError> {
    let Some(object) = raw.as_object() else {
        return Err(GatewayError::invalid_input("the input must be a JSON object"));
    };
    let days = match object.get("days") {
        None => DEFAULT_USAGE_DAYS,
        Some(Value::Num(days)) => *days,
        Some(_) => return Err(GatewayError::invalid_input("days must be a whole number of days above 0")),
    };
    if !days.is_finite() || days.fract() != 0.0 || days <= 0.0 {
        return Err(GatewayError::invalid_input("days must be a whole number of days above 0"));
    }
    match object.get("workflow") {
        None => Ok(UsageInput { days, workflow: None }),
        Some(value) => Ok(UsageInput {
            days,
            workflow: Some(workflows::require_name(Some(value), "workflow must be a non-empty string")?),
        }),
    }
}

/// Whether a client's own status command says it is signed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignedIn {
    Yes,
    No,
    Unknown,
}

impl SignedIn {
    pub fn as_str(self) -> &'static str {
        match self {
            SignedIn::Yes => "yes",
            SignedIn::No => "no",
            SignedIn::Unknown => "unknown",
        }
    }
}

/// What a client's own status command said about it, and which command said it.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientStatusReading {
    /// The command, as the owner would type it.
    pub command: String,
    pub signed_in: SignedIn,
    /// One line saying what the reading rests on.
    pub detail: String,
}

/// What the gateway found out about one client before it read the ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientReading {
    pub client: String,
    pub installed: bool,
    /// What the client's own status command said; `None` when it has none.
    pub status: Option<ClientStatusReading>,
}

/// A client model whose own answer said it cannot run, and why: the gateway
/// remembers it until the reset the answer named, or, when it named none, until
/// a run of the client model succeeded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StandingRefusal {
    /// When the answer was given.
    pub at: String,
    pub client: String,
    pub model: String,
    /// One line naming what the answer said, and when it said it ends.
    pub reason: String,
}

impl StandingRefusal {
    /// The refusal as `models` skips a model: the client, the model and why.
    pub fn to_skipped_model(&self) -> SkippedModel {
        SkippedModel { client: self.client.clone(), model: self.model.clone(), reason: self.reason.clone() }
    }
}

/// Whether a client can run now, and what that rests on.
#[derive(Clone, Debug, PartialEq)]
pub struct Availability {
    /// `unable_to_start` is a plan window at 100%; `unavailable` is anything else that stops it.
    pub status: &'static str,
    pub installed: bool,
    pub signed_in: SignedIn,
    /// One line saying what the reading rests on.
    pub reason: String,
}

/// How a plan window stands: below the warning ratio, at it, or spent out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlanState {
    Ok,
    Warning,
    Exhausted,
}

impl PlanState {
    pub fn as_str(self) -> &'static str {
        match self {
            PlanState::Ok => "ok",
            PlanState::Warning => "warning",
            PlanState::Exhausted => "exhausted",
        }
    }
}

/// One window of a client's plan, with what the ledger says is spent in it.
#[derive(Clone, Debug, PartialEq)]
pub enum PlanWindowUsage {
    Usd { window: PlanWindow, limit_usd: f64, spent_usd: f64, ratio: f64, state: PlanState },
    Runs { window: PlanWindow, limit_runs: f64, runs: f64, ratio: f64, state: PlanState },
}

impl PlanWindowUsage {
    pub fn window(&self) -> PlanWindow {
        match self {
            PlanWindowUsage::Usd { window, .. } | PlanWindowUsage::Runs { window, .. } => *window,
        }
    }

    pub fn state(&self) -> PlanState {
        match self {
            PlanWindowUsage::Usd { state, .. } | PlanWindowUsage::Runs { state, .. } => *state,
        }
    }

    pub fn ratio(&self) -> f64 {
        match self {
            PlanWindowUsage::Usd { ratio, .. } | PlanWindowUsage::Runs { ratio, .. } => *ratio,
        }
    }
}

/// What a set of runs came to.
#[derive(Clone, Debug, PartialEq)]
pub struct UsageTotals {
    pub runs: f64,
    /// The tokens the runs reported; `None` when one of them reported none,
    /// since the gateway cannot know the total.
    pub tokens: Option<RunTokens>,
    pub cost_usd: f64,
}

/// One UTC day's usage.
#[derive(Clone, Debug, PartialEq)]
pub struct DayUsage {
    /// The UTC day, as `YYYY-MM-DD`.
    pub day: String,
    pub totals: UsageTotals,
}

/// One workflow's usage.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowUsage {
    pub workflow: String,
    pub totals: UsageTotals,
}

/// What every client on this machine can do now and what it has spent: the
/// usage of the last `days` days, by day and by workflow, and each client's
/// plan windows against the ledger.
pub fn usage_report(
    input: &UsageInput,
    config: &GatewayConfig,
    readings: &[ClientReading],
    entries: &[LedgerEntry],
    now_ms: i64,
) -> Value {
    let records = run_records(entries);
    let held = caps::live_reservations(entries);
    let since = start_of_day(now_ms) - ((input.days - 1.0) * DAY_MS as f64) as i64;
    let counted: Vec<&LedgerRecord> = records
        .iter()
        .copied()
        .filter(|record| {
            timestamps::parse(&record.at).is_some_and(|at| at >= since)
                && input.workflow.as_ref().is_none_or(|workflow| &record.workflow == workflow)
        })
        .collect();
    let mut clients: Vec<Value> = Vec::new();
    for (id, client) in &config.clients {
        let reading = readings.iter().find(|reading| &reading.client == id);
        let mine: Vec<&LedgerRecord> = counted.iter().copied().filter(|record| &record.client == id).collect();
        let reservations: Vec<&ReservationRecord> = held.iter().copied().filter(|reservation| &reservation.client == id).collect();
        let (windows, availability) = client_standing(id, client, reading, &records, now_ms, &config.settings);
        let reserved_usd = caps::round(reservations.iter().map(|reservation| reservation.estimate_usd).sum(), 6);
        let mut entry = Object::new();
        entry.set("availability", availability_json(&availability));
        entry.set("by_day", Value::Arr(by_day(&mine).iter().map(day_usage_json).collect()));
        entry.set("by_workflow", Value::Arr(by_workflow(&mine).iter().map(workflow_usage_json).collect()));
        entry.set("client", Value::str(id));
        entry.set("cost_class", Value::str(client.cost_class.as_str()));
        entry.set("plan_limits", Value::Arr(windows.iter().map(plan_window_json).collect()));
        entry.set("quota", quota_of(id, reading.and_then(|reading| reading.status.as_ref())));
        entry.set("reserved_runs", Value::count(reservations.len()));
        entry.set("reserved_usd", Value::Num(reserved_usd));
        entry.set("usage", totals_json(&totals_of(&mine)));
        entry.set("warnings", Value::Arr(warnings_of(id, &windows).iter().map(|warning| Value::str(warning)).collect()));
        clients.push(Value::Obj(entry));
    }
    let mut result = Object::new();
    result.set("clients", Value::Arr(clients));
    result.set("days", Value::Num(input.days));
    result.set("status", Value::str("ok"));
    if let Some(workflow) = &input.workflow {
        result.set("workflow", Value::str(workflow));
    }
    Value::Obj(result)
}

/// One client's plan windows and whether it can run now, read from the ledger's runs and what the gateway
/// found out about the client before it read the ledger.
pub fn client_standing(
    id: &str,
    client: &ClientConfig,
    reading: Option<&ClientReading>,
    records: &[&LedgerRecord],
    now_ms: i64,
    settings: &Settings,
) -> (Vec<PlanWindowUsage>, Availability) {
    let windows = plan_windows(id, client.plan_limits.as_deref().unwrap_or(&[]), records, now_ms, settings.plan_warning_ratio);
    let availability =
        availability_of(id, &client.binary, reading, records, now_ms, settings.availability_window_minutes, &windows);
    (windows, availability)
}

/// Each window of a client's plan, with the spend the ledger holds in it. A
/// window reaches back from now, so a plan that allows $12 per 5 hours is
/// watched over the last five hours whatever the calendar says.
pub fn plan_windows(
    client: &str,
    limits: &[PlanLimit],
    records: &[&LedgerRecord],
    now_ms: i64,
    warning_ratio: f64,
) -> Vec<PlanWindowUsage> {
    limits
        .iter()
        .map(|limit| {
            let since = now_ms - limit.window().span_ms();
            let mine: Vec<&LedgerRecord> = records
                .iter()
                .copied()
                .filter(|record| record.client == client && timestamps::parse(&record.at).is_some_and(|at| at >= since))
                .collect();
            match limit {
                PlanLimit::Usd { window, limit_usd } => {
                    let spent_usd = caps::round(mine.iter().map(|record| record.cost_usd).sum(), 6);
                    let ratio = caps::round(spent_usd / limit_usd, 4);
                    PlanWindowUsage::Usd {
                        window: *window,
                        limit_usd: *limit_usd,
                        spent_usd,
                        ratio,
                        state: state_of(ratio, warning_ratio),
                    }
                }
                PlanLimit::Runs { window, limit_runs } => {
                    let runs = mine.len() as f64;
                    let ratio = caps::round(runs / limit_runs, 4);
                    PlanWindowUsage::Runs {
                        window: *window,
                        limit_runs: *limit_runs,
                        runs,
                        ratio,
                        state: state_of(ratio, warning_ratio),
                    }
                }
            }
        })
        .collect()
}

/// One line per plan window at or above the warning ratio, for `usage` and for
/// every run's result.
pub fn warnings_of(client: &str, windows: &[PlanWindowUsage]) -> Vec<String> {
    windows
        .iter()
        .filter(|window| window.state() != PlanState::Ok)
        .map(|window| {
            format!(
                "{client} has used {}% of {}",
                json::number_text(caps::round(window.ratio() * 100.0, 0)),
                describe_window(window)
            )
        })
        .collect()
}

/// A plan window as a sentence names it: its span, and what is spent of what it
/// allows.
pub fn describe_window(window: &PlanWindowUsage) -> String {
    match window {
        PlanWindowUsage::Usd { window, limit_usd, spent_usd, .. } => format!(
            "its {} plan limit (${} of ${})",
            window.as_str(),
            caps::to_fixed(*spent_usd, 2),
            caps::to_fixed(*limit_usd, 2)
        ),
        PlanWindowUsage::Runs { window, limit_runs, runs, .. } => format!(
            "its {} plan limit ({} of {} runs)",
            window.as_str(),
            json::number_text(*runs),
            json::number_text(*limit_runs)
        ),
    }
}

fn state_of(ratio: f64, warning_ratio: f64) -> PlanState {
    if ratio >= 1.0 {
        return PlanState::Exhausted;
    }
    if ratio >= warning_ratio {
        PlanState::Warning
    } else {
        PlanState::Ok
    }
}

/// One line naming what a run's own answer said: the refusal it reported, and
/// the reset it named when it named one.
pub fn refusal_reason(record: &LedgerRecord) -> String {
    let said = format!("{} at {}", record.reason.as_deref().unwrap_or("failed"), record.at);
    match &record.unavailable_until {
        None => said,
        Some(until) => format!("{said}: unavailable until {until}"),
    }
}

/// The client models whose own answers said they cannot run: a quota answer
/// with the reset it named on the run's line, and the answer for a model the
/// account cannot use, which names no moment. An answer stands until the
/// moment it named, and one that named none stands until a later run of the
/// client model succeeded; one whose moment has passed stands no more.
pub fn standing_refusals(records: &[&LedgerRecord], now_ms: i64) -> Vec<StandingRefusal> {
    let mut sorted: Vec<&LedgerRecord> = records.to_vec();
    sort_by_at(&mut sorted);
    let mut standing: Vec<(String, &LedgerRecord)> = Vec::new();
    for record in sorted {
        let key = format!("{}:{}", record.client, record.model);
        if record.ok {
            standing.retain(|(held, _)| *held != key);
        } else if record.unavailable_until.is_some()
            || record.reason.as_deref() == Some("model-unavailable")
            || record.reason.as_deref() == Some("rate-limited")
        {
            match standing.iter_mut().find(|(held, _)| *held == key) {
                Some(slot) if ends_at(record) >= ends_at(slot.1) => slot.1 = record,
                Some(_) => {}
                None => standing.push((key, record)),
            }
        }
    }
    let mut refusals: Vec<StandingRefusal> = standing
        .into_iter()
        .filter(|(_, record)| ends_at(record) > now_ms as f64)
        .map(|(_, record)| StandingRefusal {
            at: record.at.clone(),
            client: record.client.clone(),
            model: record.model.clone(),
            reason: refusal_reason(record),
        })
        .collect();
    refusals.sort_by_key(|refusal| timestamps::parse(&refusal.at).unwrap_or(i64::MIN));
    refusals
}

/// When an answer stops standing: the reset it named, or never, until a run of
/// the client model succeeds.
fn ends_at(record: &LedgerRecord) -> f64 {
    match &record.unavailable_until {
        None => f64::INFINITY,
        Some(until) => timestamps::parse(until).map_or(f64::NAN, |ms| ms as f64),
    }
}

/// Whether a client can run now. A client that is not installed, one whose own
/// status command says it is signed out, and one whose last run in the
/// availability window was rate-limited or refused for its credentials are all
/// unavailable; a client model whose own answer said it cannot run makes the
/// client unavailable until that answer no longer stands; and a plan window at
/// 100% makes it unable to start. A status command the gateway cannot run
/// leaves the reading `unknown`, which does not stop a run: silence is not a
/// refusal.
pub fn availability_of(
    client: &str,
    binary: &str,
    reading: Option<&ClientReading>,
    records: &[&LedgerRecord],
    now_ms: i64,
    window_minutes: f64,
    windows: &[PlanWindowUsage],
) -> Availability {
    if !reading.is_some_and(|reading| reading.installed) {
        return Availability {
            status: "unavailable",
            installed: false,
            signed_in: SignedIn::Unknown,
            reason: format!("not installed: {binary} is not on PATH"),
        };
    }
    let status = reading.and_then(|reading| reading.status.as_ref());
    let signed_in = status.map_or(SignedIn::Unknown, |status| status.signed_in);
    let reason = match status {
        None => format!("{client} has no status command, so the gateway cannot tell whether it is signed in"),
        Some(status) => format!("{}: {}", status.command, status.detail),
    };
    if signed_in == SignedIn::No {
        return Availability { status: "unavailable", installed: true, signed_in, reason };
    }
    if let Some(exhausted) = windows.iter().find(|window| window.state() == PlanState::Exhausted) {
        return Availability {
            status: "unable_to_start",
            installed: true,
            signed_in,
            reason: format!("at 100% of {}", describe_window(exhausted)),
        };
    }
    let mine: Vec<&LedgerRecord> = records.iter().copied().filter(|record| record.client == client).collect();
    // A model the account cannot use stops that model (`models` skips it), not the client's other models.
    let refused: Vec<&LedgerRecord> = mine
        .iter()
        .copied()
        .filter(|record| record.reason.as_deref() != Some("model-unavailable") && record.unavailable_until.is_some())
        .collect();
    if let Some(latest) = standing_refusals(&refused, now_ms).last() {
        return Availability { status: "unavailable", installed: true, signed_in, reason: latest.reason.clone() };
    }
    if let Some(error) = recent_error(&mine, now_ms, window_minutes) {
        return Availability {
            status: "unavailable",
            installed: true,
            signed_in,
            reason: format!(
                "{} at {}, with no later run of it that succeeded",
                error.reason.as_deref().unwrap_or("failed"),
                error.at
            ),
        };
    }
    Availability { status: "available", installed: true, signed_in, reason }
}

/// The last rate-limit or auth error a client's runs left in the availability
/// window, unless a later run of it succeeded: one success clears the errors
/// before it, and an error after it stands.
fn recent_error<'a>(records: &[&'a LedgerRecord], now_ms: i64, window_minutes: f64) -> Option<&'a LedgerRecord> {
    let since = now_ms as f64 - window_minutes * 60_000.0;
    let mut sorted: Vec<&LedgerRecord> = records.to_vec();
    sort_by_at(&mut sorted);
    let mut error: Option<&LedgerRecord> = None;
    for record in sorted {
        if timestamps::parse(&record.at).is_some_and(|at| (at as f64) < since) {
            continue;
        }
        if record.ok {
            error = None;
        } else if matches!(record.reason.as_deref(), Some("rate-limited" | "auth")) {
            error = Some(record);
        }
    }
    error
}

/// What a client itself says is left of its plan. No client's status command
/// reports one, so it is `unknown`.
fn quota_of(client: &str, status: Option<&ClientStatusReading>) -> Value {
    let reason = match status {
        None => format!("{client} has no status command, so the gateway cannot read its quota"),
        Some(status) => format!("{} reports no quota", status.command),
    };
    json::object(vec![("reason", Value::str(&reason)), ("status", Value::str("unknown"))])
}

/// What a set of runs came to. A run that reported no token count leaves the
/// total unknown rather than short: the gateway never reports a reading it
/// could not get as zero, so the tokens are left out and the run still counts.
pub fn totals_of(records: &[&LedgerRecord]) -> UsageTotals {
    let mut tokens = RunTokens { input: 0.0, output: 0.0, cached: 0.0 };
    let mut cost = 0.0;
    let mut known = true;
    for record in records {
        cost += record.cost_usd;
        let Some(reported) = &record.tokens else {
            known = false;
            continue;
        };
        tokens.cached += reported.cached;
        tokens.input += reported.input;
        tokens.output += reported.output;
    }
    UsageTotals { runs: records.len() as f64, tokens: known.then_some(tokens), cost_usd: caps::round(cost, 6) }
}

/// The runs grouped by the UTC day they ended on, oldest day first.
pub fn by_day(records: &[&LedgerRecord]) -> Vec<DayUsage> {
    let mut days: Vec<(String, Vec<&LedgerRecord>)> = Vec::new();
    for record in records {
        let day: String = record.at.chars().take(10).collect();
        match days.iter_mut().find(|(held, _)| *held == day) {
            Some((_, mine)) => mine.push(record),
            None => days.push((day, vec![record])),
        }
    }
    let mut usage: Vec<DayUsage> = days.into_iter().map(|(day, mine)| DayUsage { day, totals: totals_of(&mine) }).collect();
    usage.sort_by(|left, right| left.day.cmp(&right.day));
    usage
}

/// The runs grouped by workflow, in workflow name order.
pub fn by_workflow(records: &[&LedgerRecord]) -> Vec<WorkflowUsage> {
    let mut workflows: Vec<(String, Vec<&LedgerRecord>)> = Vec::new();
    for record in records {
        match workflows.iter_mut().find(|(held, _)| *held == record.workflow) {
            Some((_, mine)) => mine.push(record),
            None => workflows.push((record.workflow.clone(), vec![record])),
        }
    }
    let mut usage: Vec<WorkflowUsage> = workflows
        .into_iter()
        .map(|(workflow, mine)| WorkflowUsage { workflow, totals: totals_of(&mine) })
        .collect();
    usage.sort_by(|left, right| left.workflow.cmp(&right.workflow));
    usage
}

/// The UTC midnight that starts the day `now` falls in.
fn start_of_day(now_ms: i64) -> i64 {
    now_ms.div_euclid(DAY_MS) * DAY_MS
}

/// Oldest first, as `Date.parse(left.at) - Date.parse(right.at)` orders them: a
/// moment that does not parse compares as equal, which keeps the file's order.
fn sort_by_at(records: &mut [&LedgerRecord]) {
    // An unreadable moment sorts first, so the order stays total (a comparator that gives up may panic the sort).
    records.sort_by_key(|record| timestamps::parse(&record.at).unwrap_or(i64::MIN));
}

fn availability_json(availability: &Availability) -> Value {
    json::object(vec![
        ("installed", Value::Bool(availability.installed)),
        ("reason", Value::str(&availability.reason)),
        ("signed_in", Value::str(availability.signed_in.as_str())),
        ("status", Value::str(availability.status)),
    ])
}

pub fn plan_window_json(window: &PlanWindowUsage) -> Value {
    match window {
        PlanWindowUsage::Usd { window, limit_usd, spent_usd, ratio, state } => json::object(vec![
            ("limit_usd", Value::Num(*limit_usd)),
            ("ratio", Value::Num(*ratio)),
            ("spent_usd", Value::Num(*spent_usd)),
            ("state", Value::str(state.as_str())),
            ("window", Value::str(window.as_str())),
        ]),
        PlanWindowUsage::Runs { window, limit_runs, runs, ratio, state } => json::object(vec![
            ("limit_runs", Value::Num(*limit_runs)),
            ("ratio", Value::Num(*ratio)),
            ("runs", Value::Num(*runs)),
            ("state", Value::str(state.as_str())),
            ("window", Value::str(window.as_str())),
        ]),
    }
}

/// The totals as the report writes them: `cost_usd`, `runs`, and `tokens` only
/// when every run reported one.
fn totals_json(totals: &UsageTotals) -> Value {
    let mut object = Object::new();
    for (key, value) in totals_members(totals) {
        object.set(key, value);
    }
    Value::Obj(object)
}

fn day_usage_json(usage: &DayUsage) -> Value {
    let mut object = Object::new();
    object.set("day", Value::str(&usage.day));
    for (key, value) in totals_members(&usage.totals) {
        object.set(key, value);
    }
    Value::Obj(object)
}

fn workflow_usage_json(usage: &WorkflowUsage) -> Value {
    let mut object = Object::new();
    object.set("workflow", Value::str(&usage.workflow));
    for (key, value) in totals_members(&usage.totals) {
        object.set(key, value);
    }
    Value::Obj(object)
}

fn totals_members(totals: &UsageTotals) -> Vec<(&'static str, Value)> {
    let mut members = vec![("cost_usd", Value::Num(totals.cost_usd)), ("runs", Value::Num(totals.runs))];
    if let Some(tokens) = &totals.tokens {
        members.push((
            "tokens",
            json::object(vec![
                ("cached", Value::Num(tokens.cached)),
                ("input", Value::Num(tokens.input)),
                ("output", Value::Num(tokens.output)),
            ]),
        ));
    }
    members
}
