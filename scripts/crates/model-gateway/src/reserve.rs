//! `grant reserve`: holds one Codex node start against a workflow's grant, before the node starts.
//!
//! The decision and the reservation line are one locked ledger update, so two
//! parallel nodes cannot both pass on the grant's last start. A start is a
//! reservation of no dollars under the model `node-start`; it holds for about a
//! minute and then settles, and a settled start still counts, so a start is
//! never given back.

use crate::caps;
use crate::config::CostClass;
use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::grants::{self, Grant};
use crate::json::{self, Object, Value};
use crate::ledger::{LedgerEntry, ReservationRecord};
use crate::ledger_file::{finished_costs, LedgerAppend, LedgerStore};
use crate::ports::Runtime;
use crate::sha256::sha256_hex;
use crate::state_file::{state_dir, Workflows};
use crate::text::join;
use crate::timestamps;
use crate::workflows::{self, WorkflowRecord};

/// A node start costs no dollars.
const NODE_START_ESTIMATE_USD: f64 = 0.0;
/// A node start holds for one second and the grace period after it.
const NODE_START_TIME_LIMIT_S: i64 = 1;
/// The grace period after a reservation's time limit, in seconds.
const RESERVATION_GRACE_S: i64 = 60;
/// The model a node start is reserved under.
pub const NODE_START_MODEL: &str = "node-start";

/// What a node start asks for.
struct StartInput {
    attempt: f64,
    client: String,
    node: String,
    workflow: String,
}

fn read_input(input: &Object) -> Result<StartInput, GatewayError> {
    let node = match input.get("node") {
        Some(Value::Str(node)) if !node.trim().is_empty() => node.clone(),
        _ => return Err(GatewayError::invalid_input("a node start must name its node")),
    };
    let attempt = match input.get("attempt") {
        None | Some(Value::Null) => 1.0,
        Some(Value::Num(attempt)) if attempt.is_finite() && attempt.fract() == 0.0 && *attempt >= 0.0 => *attempt,
        Some(_) => return Err(GatewayError::invalid_input("attempt must be a whole number, 0 or more")),
    };
    let client = match input.get("client") {
        Some(Value::Str(client)) if !client.trim().is_empty() => client.clone(),
        _ => return Err(GatewayError::invalid_input("a grant must name its client")),
    };
    let workflow = workflows::require_name(input.get("workflow"), "a node start must name its workflow")?;
    Ok(StartInput { attempt, client, node, workflow })
}

/// `start-` and the first 16 hex digits of the SHA-256 of `["<workflow>","<client>","<node>",<attempt>]`.
fn start_id(input: &StartInput) -> String {
    let key = Value::Arr(vec![
        Value::str(&input.workflow),
        Value::str(&input.client),
        Value::str(&input.node),
        Value::Num(input.attempt),
    ]);
    format!("start-{}", &sha256_hex(json::compact(&key).as_bytes())[..16])
}

/// The newest grant of the workflow and client, refused when there is none or it lapsed.
fn start_grant(grants: &[Grant], input: &StartInput, record: Option<&WorkflowRecord>, now_ms: i64) -> Result<Grant, GatewayError> {
    let Some(newest) = grants::newest_grant(grants, &input.workflow, &input.client) else {
        return Err(GatewayError::new("no-grant", grants::no_grant_message(&input.workflow, &input.client)));
    };
    let nothing = grants::GrantRoom { grant: newest.clone(), spent_usd: 0.0, reserved_usd: 0.0, remaining_usd: 0.0 };
    match grants::grant_lapse(&newest, &nothing, record, now_ms) {
        Some(lapse) => Err(GatewayError::new(
            "no-grant",
            format!("grant {} for workflow {} has lapsed ({}): ask the owner for a new grant", newest.id, input.workflow, lapse.as_str()),
        )),
        None => Ok(newest),
    }
}

/// The node starts the grant has held: every reservation of its workflow and client since it was written.
fn starts_taken(entries: &[LedgerEntry], grant: &Grant) -> u64 {
    let since = timestamps::parse(&grant.written_at);
    entries
        .iter()
        .filter(|entry| match entry {
            LedgerEntry::Reservation(record) => {
                record.model == NODE_START_MODEL
                    && record.client == grant.client
                    && record.workflow == grant.workflow
                    && timestamps::parse(&record.reserved_at).is_some_and(|at| since.is_some_and(|since| at >= since))
            }
            _ => false,
        })
        .count() as u64
}

fn start_result(grant: &Grant, input: &StartInput, run_id: &str, reserved_at: &str, max_starts: u64, started: u64, repeated: bool) -> Value {
    let mut reservation = Object::new();
    reservation.set("attempt", Value::Num(input.attempt));
    reservation.set("client", Value::str(&input.client));
    reservation.set("grant", Value::str(&grant.id));
    reservation.set("max_starts", Value::Num(max_starts as f64));
    reservation.set("node", Value::str(&input.node));
    reservation.set("remaining", Value::Num(max_starts as f64 - started as f64));
    reservation.set("repeated", Value::Bool(repeated));
    reservation.set("reserved_at", Value::str(reserved_at));
    reservation.set("run_id", Value::str(run_id));
    reservation.set("started", Value::Num(started as f64));
    reservation.set("workflow", Value::str(&input.workflow));
    let mut object = Object::new();
    object.set("reservation", Value::Obj(reservation));
    object.set("status", Value::str("ok"));
    Value::Obj(object)
}

/// Reserves one node start, or says why it may not start.
pub fn reserve(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let input = read_input(input)?;
    let config = load_config(runtime)?;
    let Some(client) = config.clients.get(&input.client) else {
        return Err(GatewayError::new("unknown-model", format!("the gateway knows no client {}", input.client)));
    };
    if client.cost_class != CostClass::MeteredEmployer {
        return Err(GatewayError::invalid_input(format!(
            "{} is {}: a grant is for a metered-employer client, which is the one that spends someone else's money",
            input.client,
            client.cost_class.as_str()
        )));
    }
    let now = runtime.clock.now_ms();
    let grants = (crate::grants_file::GrantsStore { runtime }).read()?;
    let records = (Workflows { runtime }).read()?;
    let record = records.iter().find(|candidate| candidate.name == input.workflow);
    let store = LedgerStore { runtime };
    let costs = finished_costs(runtime, &store.read()?);
    let run_id = start_id(&input);
    let reserved_at = timestamps::to_iso(now);
    let reservation = ReservationRecord {
        run_id: run_id.clone(),
        workflow: input.workflow.clone(),
        client: input.client.clone(),
        cost_class: CostClass::MeteredEmployer,
        model: NODE_START_MODEL.to_string(),
        estimate_usd: NODE_START_ESTIMATE_USD,
        reserved_at: reserved_at.clone(),
        expires_at: timestamps::to_iso(now + (NODE_START_TIME_LIMIT_S + RESERVATION_GRACE_S) * 1000),
        // A start has no run directory of its own, and a path that holds no result is one no settle reads a cost from.
        run_dir: join(&[&state_dir(&runtime.env)?, "node-starts", &run_id]),
    };
    store.update_with(|entries| {
        let settled = caps::settle_expired(entries, now, &costs);
        let repeated = entries.iter().any(|entry| match entry {
            LedgerEntry::Reservation(held) => held.run_id == run_id,
            LedgerEntry::Settled(held) => held.run_id == run_id,
            LedgerEntry::Run(_) => false,
        });
        let grant = start_grant(&grants, &input, record, now)?;
        let Some(max_starts) = grant.max_starts else {
            return Err(GatewayError::new(
                "no-max-starts",
                format!(
                    "grant {} for workflow {} has no max_starts, so it allows no Codex node starts: ask the owner for a grant written with max_starts",
                    grant.id, input.workflow
                ),
            ));
        };
        let started = starts_taken(entries, &grant);
        if repeated {
            return Ok(LedgerAppend {
                settled,
                reserved: None,
                result: start_result(&grant, &input, &run_id, &reserved_at, max_starts, started, true),
            });
        }
        if started >= max_starts {
            return Err(GatewayError::new(
                "cap-exceeded",
                format!(
                    "grant {} allows {max_starts} Codex node starts for workflow {}: {started} taken, no start left for node {}",
                    grant.id, input.workflow, input.node
                ),
            ));
        }
        let result = start_result(&grant, &input, &run_id, &reserved_at, max_starts, started + 1, false);
        Ok(LedgerAppend { settled, reserved: Some(reservation), result })
    })
}
