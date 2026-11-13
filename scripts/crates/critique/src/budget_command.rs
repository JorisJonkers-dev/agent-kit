//! `budget`: what the next costly stage of one feature would draw on: the Vibe spend of its flow and Vibe's plan limit
//! as the model gateway reads it, and today's Antigravity launches against the daily cap and the reserve. It reads the
//! ledgers and routing through the code the stages enforce the limits with, and launches and writes nothing. The node
//! runner's `application/budget-view.ts` and `domain/budget-view.ts`.

use crate::budget::{agy_invocations_on, spend_by_flow, utc_day, FlowSpend};
use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::limits::AntigravityQuota;
use crate::plan::plan_limit;
use crate::repository::{refusal, resolve, Resolution};
use crate::routing::{RosterSlot, Routing};
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::stage::CRITIQUE_STAGES;
use crate::stage_table::{flow_key, FlowKey};
use crate::state_command::{flow_keys, locate, read_flow_name, Located};
use model_gateway::json::{self, Value};
use model_gateway::usage::plan_window_json;

/// Why the stage's routed Antigravity critics would not launch now, as prepare words it; `None` when none is held.
pub fn quota_reached(count: usize, quota: &AntigravityQuota, stage: &str) -> Option<String> {
    let count_f = count as f64;
    if count_f >= quota.daily_cap {
        return Some(format!("antigravity: {count} invocations today have reached the daily cap of {}", json::number_text(quota.daily_cap)));
    }
    if count_f >= quota.reserve_threshold && !quota.reserved_stages.iter().any(|reserved| reserved == stage) {
        return Some(format!(
            "antigravity: {count} invocations today have reached the reserve threshold of {}; the rest is kept for {}",
            json::number_text(quota.reserve_threshold),
            quota.reserved_stages.join(" and ")
        ));
    }
    None
}

fn strings_json(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn spend_json(spend: &FlowSpend) -> Value {
    json::object(vec![("flow", Value::str(&spend.flow)), ("spent_usd", Value::Num(spend.spent_usd))])
}

/// What one feature has drawn on.
struct FeatureDraw<'a> {
    spec: Option<&'a FlowSpend>,
    draft: Option<&'a FlowSpend>,
    agy_today: usize,
    /// The gateway's reason Vibe cannot start, at 100% of a plan window; `None` while it can.
    plan: Option<String>,
}

fn client_kind<'a>(routing: &'a Routing, name: &str) -> Option<&'a str> {
    routing.clients.iter().find(|(client, _)| client == name).map(|(_, client)| client.kind())
}

/// Each stage routing defines, in flow order, with what Vibe's plan limit and today's Antigravity count would do to it.
fn stage_outlooks(routing: &Routing, draw: &FeatureDraw<'_>) -> Vec<Value> {
    CRITIQUE_STAGES
        .iter()
        .filter_map(|stage| {
            let charged = match flow_key(stage) {
                FlowKey::Draft => draw.draft,
                FlowKey::Spec => draw.spec,
                FlowKey::Run => None,
            };
            let routes = routing.stages.iter().find(|(name, _)| name == stage).map(|(_, routes)| routes)?;
            let held: Vec<(&RosterSlot, bool)> =
                routes.base.iter().map(|slot| (slot, false)).chain(routes.deep.iter().map(|slot| (slot, true))).filter(|(slot, _)| client_kind(routing, &slot.client) == Some("agy")).collect();
            let quota = if held.is_empty() { None } else { quota_reached(draw.agy_today, &routing.limits.antigravity_quota, stage) };
            let falls_back = if quota.is_none() { Vec::new() } else { held.iter().map(|(slot, deep)| slot_fallback(routing, slot, *deep)).collect() };
            Some(json::object(vec![
                ("antigravity", quota.map_or(Value::Null, Value::Str)),
                ("falls_back", Value::Arr(falls_back)),
                ("flow", charged.map_or(Value::Null, |spend| Value::str(&spend.flow))),
                ("stage", Value::str(stage)),
                ("vibe", draw.plan.as_deref().map_or(Value::Null, Value::str)),
            ]))
        })
        .collect()
}

/// The first of a held slot's fallbacks that is not Antigravity, in routing order; null leaves the slot empty.
fn slot_fallback(routing: &Routing, slot: &RosterSlot, deep: bool) -> Value {
    let fallbacks = routing.fallbacks.iter().find(|(client, _)| *client == slot.client).map(|(_, list)| list.as_slice()).unwrap_or_default();
    let to = fallbacks.iter().find(|name| client_kind(routing, name) != Some("agy"));
    let mut pairs = vec![("client", Value::str(&slot.client)), ("to", to.map_or(Value::Null, |name| Value::str(name)))];
    if deep {
        pairs.push(("deep", Value::Bool(true)));
    }
    json::object(pairs)
}

/// Runs `budget`.
pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let fields = Fields::of(raw, "").map_err(|error| error.into_error("invalid-input", "budget input"))?;
    let name = read_flow_name(&fields, "budget")?;
    let repository = match resolve(runtime, &name.repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository,
    };
    let located = match locate(runtime, &repository, &name)? {
        Located::Refused(refusal) => return Ok(refusal),
        Located::Located(located) => located,
    };
    let read = crate::ledger_source::LedgerSource::new(runtime).read()?;
    let (_, keys) = flow_keys(&read.ledger, &located, name.draft.as_deref());
    let routing = load_routing(runtime)?;
    let spends = spend_by_flow(&read.invocations, &repository.key, &keys);
    let now = runtime.clock.now_ms();
    let invocations = agy_invocations_on(&read.invocations, now);
    // A gateway configuration this machine cannot read is a refusal: the budget is a report, and the owner is told
    // which file to fix.
    let reading = match plan_limit(runtime, "vibe") {
        Ok(reading) => reading,
        Err(error) => return Ok(refusal("invalid-gateway-config", &format!("the model gateway's configuration cannot be read: {}", error.message))),
    };
    // Before /to-spec the flow is the draft's own; after it the spec's comes first, then its draft's.
    let (spec, draft) = if located.spec.is_none() { (None, spends.first()) } else { (spends.first(), spends.get(1)) };
    let draw = FeatureDraw { agy_today: invocations, draft, plan: reading.unable_to_start.clone(), spec };
    let quota = &routing.limits.antigravity_quota;
    let antigravity = json::object(vec![
        ("daily_cap", Value::Num(quota.daily_cap)),
        ("reserve_threshold", Value::Num(quota.reserve_threshold)),
        ("reserved_stages", strings_json(&quota.reserved_stages)),
        ("day", Value::Str(utc_day(now))),
        ("invocations", Value::count(invocations)),
    ]);
    let vibe = json::object(vec![
        ("flows", Value::Arr(spends.iter().map(spend_json).collect())),
        ("plan_limits", Value::Arr(reading.windows.iter().map(plan_window_json).collect())),
        ("spent_usd", Value::Num(spends.iter().fold(0.0, |total, spend| total + spend.spent_usd))),
        ("warnings", strings_json(&reading.warnings)),
    ]);
    Ok(json::object(vec![
        ("antigravity", antigravity),
        ("flow", Value::str(&located.flow)),
        ("stages", Value::Arr(stage_outlooks(&routing, &draw))),
        ("status", Value::str("read")),
        ("vibe", vibe),
    ]))
}
