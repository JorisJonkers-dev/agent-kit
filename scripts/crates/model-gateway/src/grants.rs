//! Grants: the owner's permission for a workflow to spend a metered-employer
//! client's money.
//!
//! A grant is written by the agent after it asked the owner, and it records the
//! question and the answer so a grant written without asking stands out. It
//! ends when its workflow ends, after 24 hours, or when its cap is spent,
//! whichever comes first; the gateway never renews one and never writes one
//! itself. Before a metered-employer run starts, its estimate is reserved
//! against the grant's remaining cap, and the run's actual spend replaces the
//! reservation when it ends.

use crate::caps;
use crate::config::CostClass;
use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ledger::LedgerEntry;
use crate::timestamps::{self, WORKFLOW_TTL_MS};
use crate::workflows::{self, WorkflowRecord};

/// The version the grants file is written as.
pub const GRANTS_VERSION: f64 = 1.0;

/// How long a grant lives when its workflow outlives it.
pub const GRANT_TTL_MS: i64 = WORKFLOW_TTL_MS;

/// One grant, as the machine's grants file holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    pub id: String,
    pub workflow: String,
    /// The metered-employer client the owner allowed, `codex` today.
    pub client: String,
    pub cap_usd: f64,
    /// The Codex node starts a Gold Band run may take under the grant; absent for a grant written for dollars.
    pub max_starts: Option<u64>,
    /// The question the owner was asked.
    pub question: String,
    /// What the owner answered.
    pub answer: String,
    pub written_at: String,
    /// When it lapses: 24 hours after it was written, or when its workflow's
    /// own 24 hours run out first.
    pub expires_at: String,
    /// When someone ended it, as an ISO timestamp; absent while it stands.
    pub ended_at: Option<String>,
}

/// Why a grant no longer covers a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GrantLapse {
    WorkflowEnded,
    Expired,
    CapSpent,
}

impl GrantLapse {
    pub fn as_str(self) -> &'static str {
        match self {
            GrantLapse::WorkflowEnded => "workflow-ended",
            GrantLapse::Expired => "expired",
            GrantLapse::CapSpent => "cap-spent",
        }
    }
}

/// One grant with what it holds for the runs that ask: its cap, what is spent
/// and what is reserved.
#[derive(Clone, Debug, PartialEq)]
pub struct GrantRoom {
    pub grant: Grant,
    pub spent_usd: f64,
    pub reserved_usd: f64,
    pub remaining_usd: f64,
}

/// One grant as `grant list` reports it: what it allows, and what it has left.
#[derive(Clone, Debug, PartialEq)]
pub struct GrantView {
    pub grant: Grant,
    pub spent_usd: f64,
    pub reserved_usd: f64,
    pub remaining_usd: f64,
}

/// A grant that no longer covers a run, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct LapsedGrant {
    pub view: GrantView,
    pub reason: GrantLapse,
}

/// The grants a file holds. A file that is not JSON, of another version, or
/// whose entries are not grants is one error naming what is wrong, rather than
/// a surprise when a run asks whether a grant covers it.
pub fn parse_grants_file(text: &str) -> Result<Vec<Grant>, String> {
    let raw = json::parse(text).map_err(|message| format!("not JSON: {message}"))?;
    let Some(object) = raw.as_object() else {
        return Err("the grants file must be a JSON object".to_string());
    };
    if object.get("version") != Some(&Value::Num(GRANTS_VERSION)) {
        return Err(format!("version must be {}", json::number_text(GRANTS_VERSION)));
    }
    let Some(Value::Arr(items)) = object.get("grants") else {
        return Err("grants must be a list".to_string());
    };
    items.iter().enumerate().map(|(index, value)| parse_grant(value, index)).collect()
}

fn parse_grant(raw: &Value, index: usize) -> Result<Grant, String> {
    let where_ = format!("grants[{index}]");
    let Some(object) = raw.as_object() else {
        return Err(format!("{where_} must be an object"));
    };
    let cap_usd = match object.get("cap_usd") {
        Some(Value::Num(cap)) if cap.is_finite() && *cap > 0.0 => *cap,
        _ => return Err(format!("{where_}.cap_usd must be a number above 0")),
    };
    let max_starts = match object.get("max_starts") {
        None => None,
        Some(value) => Some(start_count(value).ok_or_else(|| format!("{where_}.max_starts must be a whole number above 0"))?),
    };
    let grant = Grant {
        answer: require_string(object.get("answer"), &format!("{where_}.answer"))?,
        cap_usd,
        max_starts,
        client: require_string(object.get("client"), &format!("{where_}.client"))?,
        expires_at: require_timestamp(object.get("expires_at"), &format!("{where_}.expires_at"))?,
        id: require_string(object.get("id"), &format!("{where_}.id"))?,
        question: require_string(object.get("question"), &format!("{where_}.question"))?,
        workflow: require_string(object.get("workflow"), &format!("{where_}.workflow"))?,
        written_at: require_timestamp(object.get("written_at"), &format!("{where_}.written_at"))?,
        ended_at: None,
    };
    let Some(ended) = object.get("ended_at") else {
        return Ok(grant);
    };
    let ended_at = require_timestamp(Some(ended), &format!("{where_}.ended_at"))?;
    Ok(Grant { ended_at: Some(ended_at), ..grant })
}

/// A count of node starts: a whole number above 0.
pub fn start_count(value: &Value) -> Option<u64> {
    match value {
        Value::Num(n) if n.is_finite() && n.fract() == 0.0 && *n >= 1.0 && *n <= 9_007_199_254_740_991.0 => Some(*n as u64),
        _ => None,
    }
}

fn require_string(value: Option<&Value>, where_: &str) -> Result<String, String> {
    match value {
        Some(Value::Str(text)) if !text.is_empty() => Ok(text.clone()),
        _ => Err(format!("{where_} must be a non-empty string")),
    }
}

/// A full ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SS(.sss)Z`) that names a
/// real moment. Anything `Date.parse` would read in the machine's local time,
/// or guess at, is refused: a hand-edited file must not move a grant's lapse.
fn require_timestamp(value: Option<&Value>, where_: &str) -> Result<String, String> {
    let fail = || format!("{where_} must be an ISO-8601 UTC timestamp like 2026-11-04T10:52:15Z");
    let Some(Value::Str(text)) = value else {
        return Err(fail());
    };
    let Some(parts) = utc_timestamp(text) else {
        return Err(fail());
    };
    let (year, month, day, hour, minute, second) = parts;
    // `Date.UTC` reads a year below 100 as 1900 + it, so the round trip below
    // refuses such a year, as the node runner does.
    let shifted = if year < 100 { year + 1900 } else { year };
    let stamp = format!("{shifted:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z");
    let Some(moment) = timestamps::parse(&stamp) else {
        return Err(fail());
    };
    if timestamps::to_iso(moment) != format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.000Z") {
        return Err(fail());
    }
    Ok(text.clone())
}

/// `^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?Z$`, as the
/// six numbers it names.
fn utc_timestamp(text: &str) -> Option<(i64, i64, i64, i64, i64, i64)> {
    let bytes = text.as_bytes();
    let digits = |from: usize, width: usize| -> Option<i64> {
        let slice = bytes.get(from..from + width)?;
        if !slice.iter().all(u8::is_ascii_digit) {
            return None;
        }
        std::str::from_utf8(slice).ok()?.parse().ok()
    };
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    for (at, expected) in separators {
        if bytes.get(at) != Some(&expected) {
            return None;
        }
    }
    let year = digits(0, 4)?;
    let month = digits(5, 2)?;
    let day = digits(8, 2)?;
    let hour = digits(11, 2)?;
    let minute = digits(14, 2)?;
    let second = digits(17, 2)?;
    let mut at = 19;
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if !(1..=3).contains(&(at - start)) {
            return None;
        }
    }
    if bytes.get(at) != Some(&b'Z') || at + 1 != bytes.len() {
        return None;
    }
    Some((year, month, day, hour, minute, second))
}

/// A grant's id: the UTC moment it was written, and a short suffix that keeps
/// two grants of one second apart.
pub fn grant_id(now_ms: i64, suffix: &str) -> String {
    format!("grant-{}-{suffix}", compact_timestamp(now_ms))
}

/// `toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '')`: the moment
/// without its separators or its milliseconds.
pub fn compact_timestamp(ms: i64) -> String {
    let mut out = String::new();
    let mut fraction = 0;
    for c in timestamps::to_iso(ms).chars() {
        if c == '-' || c == ':' {
            continue;
        }
        if c == '.' {
            fraction = 3;
            continue;
        }
        if fraction > 0 {
            fraction -= 1;
            continue;
        }
        out.push(c);
    }
    out
}

/// What `new_grant` is given: the owner's answer, and the workflow it was given for.
pub struct NewGrant<'a> {
    pub id: &'a str,
    pub workflow: &'a str,
    pub client: &'a str,
    pub cap_usd: f64,
    pub max_starts: Option<u64>,
    pub question: &'a str,
    pub answer: &'a str,
    pub written_at: i64,
    pub workflow_record: Option<&'a WorkflowRecord>,
}

/// A grant as the gateway writes it: the owner's answer, and the moment it
/// lapses. It lapses 24 hours after it was written, or when the workflow's own
/// 24 hours run out first, so a grant never outlives the work it was given for.
pub fn new_grant(args: &NewGrant) -> Result<Grant, GatewayError> {
    Ok(Grant {
        answer: args.answer.to_string(),
        cap_usd: args.cap_usd,
        max_starts: args.max_starts,
        client: args.client.to_string(),
        expires_at: expiry_of(args.written_at, args.workflow_record)?,
        id: args.id.to_string(),
        question: args.question.to_string(),
        workflow: args.workflow.to_string(),
        written_at: timestamps::to_iso(args.written_at),
        ended_at: None,
    })
}

/// When a grant written at `written_at` lapses: its own 24 hours, or its
/// workflow's, whichever ends first. A workflow that already ended, or whose 24
/// hours already ran out, would give a grant that is expired on arrival, so it
/// is refused instead.
fn expiry_of(written_at: i64, record: Option<&WorkflowRecord>) -> Result<String, GatewayError> {
    if let Some(record) = record {
        if workflows::ended(record, written_at) {
            let ended_at = record.ended_at.clone().unwrap_or_else(|| {
                timestamps::to_iso(timestamps::parse(&record.started_at).unwrap_or_default() + WORKFLOW_TTL_MS)
            });
            return Err(GatewayError::new(
                "workflow-ended",
                format!(
                    "workflow {} ended at {ended_at}: a grant for it would lapse on arrival, so start a new workflow",
                    record.name
                ),
            ));
        }
    }
    let own = written_at + GRANT_TTL_MS;
    let workflow = record.map_or(own, |record| timestamps::parse(&record.started_at).unwrap_or_default() + WORKFLOW_TTL_MS);
    Ok(timestamps::to_iso(own.min(workflow)))
}

/// What a grant holds right now: its cap, what its workflow has spent on
/// metered-employer runs since the grant was written, and what live
/// reservations of that workflow hold.
pub fn grant_room(entries: &[LedgerEntry], grant: &Grant) -> GrantRoom {
    let since = timestamps::parse(&grant.written_at);
    let spent: Vec<f64> = caps::metered_charges(entries, CostClass::MeteredEmployer)
        .into_iter()
        .filter(|charge| {
            charge.client == grant.client
                && charge.workflow == grant.workflow
                && timestamps::parse(&charge.at).is_some_and(|at| since.is_some_and(|since| at >= since))
        })
        .map(|charge| charge.usd)
        .collect();
    let reserved: Vec<f64> = caps::live_reservations(entries)
        .into_iter()
        .filter(|reservation| {
            reservation.cost_class == CostClass::MeteredEmployer
                && reservation.client == grant.client
                && reservation.workflow == grant.workflow
                && timestamps::parse(&reservation.reserved_at).is_some_and(|at| since.is_some_and(|since| at >= since))
        })
        .map(|reservation| reservation.estimate_usd)
        .collect();
    let spent_usd = caps::round(spent.iter().sum(), 6);
    let reserved_usd = caps::round(reserved.iter().sum(), 6);
    GrantRoom {
        grant: grant.clone(),
        spent_usd,
        reserved_usd,
        remaining_usd: caps::round(grant.cap_usd - spent_usd - reserved_usd, 6),
    }
}

/// Why a grant no longer covers a run, or `None` while it does: its workflow
/// ended, its 24 hours ran out, or its cap is spent, in that order.
pub fn grant_lapse(grant: &Grant, room: &GrantRoom, record: Option<&WorkflowRecord>, now_ms: i64) -> Option<GrantLapse> {
    if grant.ended_at.is_some() || record.is_some_and(|record| workflows::ended(record, now_ms)) {
        return Some(GrantLapse::WorkflowEnded);
    }
    if timestamps::parse(&grant.expires_at).is_some_and(|expires| now_ms >= expires) {
        return Some(GrantLapse::Expired);
    }
    if room.spent_usd >= grant.cap_usd {
        return Some(GrantLapse::CapSpent);
    }
    None
}

/// The grant that covers a workflow now: the live one the owner wrote last for
/// it (the latest `written_at`, the later in the file when two share one), or
/// `None` when every grant of it lapsed. A workflow this machine never started
/// has no record, and its grant stands until its own 24 hours run out.
pub fn live_grant(
    grants: &[Grant],
    workflow: &str,
    client: &str,
    entries: &[LedgerEntry],
    record: Option<&WorkflowRecord>,
    now_ms: i64,
) -> Option<Grant> {
    let mut newest: Option<&Grant> = None;
    for grant in grants {
        if grant.workflow != workflow || grant.client != client {
            continue;
        }
        if grant_lapse(grant, &grant_room(entries, grant), record, now_ms).is_some() {
            continue;
        }
        let take = match newest {
            None => true,
            Some(newest) => match (timestamps::parse(&grant.written_at), timestamps::parse(&newest.written_at)) {
                (Some(at), Some(held)) => at >= held,
                _ => false,
            },
        };
        if take {
            newest = Some(grant);
        }
    }
    newest.cloned()
}

/// Every grant, split into the ones that still cover a run and the ones that
/// lapsed with why.
pub fn grant_views(
    grants: &[Grant],
    entries: &[LedgerEntry],
    records: &[WorkflowRecord],
    now_ms: i64,
) -> (Vec<GrantView>, Vec<LapsedGrant>) {
    let mut live: Vec<GrantView> = Vec::new();
    let mut lapsed: Vec<LapsedGrant> = Vec::new();
    for grant in grants {
        let room = grant_room(entries, grant);
        let view = GrantView {
            grant: grant.clone(),
            spent_usd: room.spent_usd,
            reserved_usd: room.reserved_usd,
            remaining_usd: room.remaining_usd,
        };
        let record = records.iter().find(|record| record.name == grant.workflow);
        match grant_lapse(grant, &room, record, now_ms) {
            None => live.push(view),
            Some(reason) => lapsed.push(LapsedGrant { view, reason }),
        }
    }
    (live, lapsed)
}

/// The grant as the grants file writes it: the keys `JSON.stringify` gives the
/// node runner's literal, and `ended_at` last when it has one.
pub fn grant_json(grant: &Grant) -> Value {
    Value::Obj(grant_members(grant))
}

/// The grant as `grant list` reports it: what it allows, then what it has left.
pub fn grant_view_json(view: &GrantView) -> Value {
    let mut object = grant_members(&view.grant);
    object.set("remaining_usd", Value::Num(view.remaining_usd));
    object.set("reserved_usd", Value::Num(view.reserved_usd));
    object.set("spent_usd", Value::Num(view.spent_usd));
    Value::Obj(object)
}

/// A lapsed grant as `grant list` reports it: the view, then why it lapsed.
pub fn lapsed_grant_json(lapsed: &LapsedGrant) -> Value {
    let mut object = grant_members(&lapsed.view.grant);
    object.set("remaining_usd", Value::Num(lapsed.view.remaining_usd));
    object.set("reserved_usd", Value::Num(lapsed.view.reserved_usd));
    object.set("spent_usd", Value::Num(lapsed.view.spent_usd));
    object.set("reason", Value::str(lapsed.reason.as_str()));
    Value::Obj(object)
}

/// The grants file as the adapter writes it: `{grants, version}`, pretty-printed
/// with a newline at the end.
pub fn grant_file_text(grants: &[Grant]) -> String {
    let mut object = Object::new();
    object.set("grants", Value::Arr(grants.iter().map(grant_json).collect()));
    object.set("version", Value::Num(GRANTS_VERSION));
    format!("{}\n", json::pretty(&Value::Obj(object)))
}

pub fn grant_members(grant: &Grant) -> Object {
    let mut object = Object::new();
    object.set("answer", Value::str(&grant.answer));
    object.set("cap_usd", Value::Num(grant.cap_usd));
    object.set("client", Value::str(&grant.client));
    object.set("expires_at", Value::str(&grant.expires_at));
    object.set("id", Value::str(&grant.id));
    if let Some(max_starts) = grant.max_starts {
        object.set("max_starts", Value::Num(max_starts as f64));
    }
    object.set("question", Value::str(&grant.question));
    object.set("workflow", Value::str(&grant.workflow));
    object.set("written_at", Value::str(&grant.written_at));
    if let Some(ended_at) = &grant.ended_at {
        object.set("ended_at", Value::str(ended_at));
    }
    object
}
