//! A prepared run as JSON, with the keys in the order the node runner builds its objects in, since the manifest is
//! written as `JSON.stringify` prints them.

use crate::limits::CritiqueLimits;
use crate::manifest::{DepthChoice, DepthSignal};
use crate::roster::{DroppedCritic, RosterCritic};
use crate::roster_resolve::{client_family, client_model};
use crate::routing::{CriticClient, GatewayModel};
use model_gateway::json::{self, Object, Value};

fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn gateway_value(gateway: &GatewayModel) -> Value {
    json::object(vec![("client", Value::str(&gateway.client)), ("model", Value::str(&gateway.model))])
}

/// A critic client as routing's reader builds it.
pub fn client_value(client: &CriticClient) -> Value {
    let mut pairs: Vec<(&str, Value)> = Vec::new();
    let gateway = match client {
        CriticClient::Vibe(vibe) => {
            pairs.extend([("agent", Value::str(&vibe.agent)), ("binary", Value::str(&vibe.binary)), ("family", Value::str(&vibe.family)), ("kind", Value::str("vibe")), ("model", Value::str(&vibe.model))]);
            if let Some(prices) = &vibe.prices {
                pairs.push(("prices", json::object(vec![("cached_input", Value::Num(prices.cached_input)), ("input", Value::Num(prices.input)), ("output", Value::Num(prices.output))])));
            }
            vibe.gateway.as_ref()
        }
        CriticClient::Agy(agy) => {
            pairs.extend([("binary", Value::str(&agy.binary)), ("family", Value::str(&agy.family)), ("kind", Value::str("agy")), ("model", Value::str(&agy.model))]);
            agy.gateway.as_ref()
        }
        CriticClient::Codex(codex) => {
            pairs.extend([("binary", Value::str(&codex.binary)), ("family", Value::str(&codex.family)), ("kind", Value::str("codex")), ("model", Value::str(&codex.model))]);
            codex.gateway.as_ref()
        }
        CriticClient::Claude(claude) => {
            pairs.extend([("binary", Value::str(&claude.binary)), ("family", Value::str(&claude.family)), ("kind", Value::str("claude")), ("model", Value::str(&claude.model))]);
            claude.gateway.as_ref()
        }
        CriticClient::Pi(pi) => {
            pairs.extend([("binary", Value::str(&pi.binary)), ("family", Value::str(&pi.family)), ("kind", Value::str("pi")), ("model", Value::str(&pi.model)), ("provider", Value::str(&pi.provider))]);
            None
        }
    };
    if let Some(gateway) = gateway {
        pairs.push(("gateway", gateway_value(gateway)));
    }
    json::object(pairs)
}

/// One critic of a roster, as the manifest freezes it: the slot, then the client it runs through.
pub fn critic_value(critic: &RosterCritic) -> Value {
    let fallbacks = critic.fallbacks.iter().map(|fallback| json::object(vec![("client", Value::str(&fallback.client)), ("client_config", client_value(&fallback.client_config))]));
    json::object(vec![
        ("client", Value::str(&critic.client)),
        ("lenses", strings(&critic.lenses)),
        ("max_price_usd", Value::Num(critic.max_price_usd)),
        ("role", Value::str(&critic.role)),
        ("time_limit_s", Value::Num(critic.time_limit_s)),
        ("client_config", client_value(&critic.client_config)),
        ("critic_id", Value::str(&critic.critic_id)),
        ("fallbacks", Value::Arr(fallbacks.collect())),
    ])
}

/// The identity a roster critic carries into fan-out, record and the ledger.
pub fn identity_value(critic: &RosterCritic) -> Value {
    let (family, model) = (client_family(&critic.client_config), client_model(&critic.client_config));
    json::object(vec![
        ("client", Value::str(&critic.client)),
        ("critic_id", Value::str(&critic.critic_id)),
        ("family", Value::str(family)),
        ("lenses", strings(&critic.lenses)),
        ("model", Value::str(model)),
        ("role", Value::str(&critic.role)),
    ])
}

/// A routed critic that does not run.
pub fn dropped_value(dropped: &DroppedCritic) -> Value {
    let mut pairs = vec![("client", Value::str(&dropped.client)), ("family", Value::str(&dropped.family))];
    if let Some(detail) = &dropped.detail {
        pairs.push(("detail", Value::str(detail)));
    }
    pairs.push(("reason", Value::str(&dropped.reason)));
    if let Some(replaced_by) = &dropped.replaced_by {
        pairs.push(("replaced_by", Value::str(replaced_by)));
    }
    if dropped.deep == Some(true) {
        pairs.push(("deep", Value::Bool(true)));
    }
    json::object(pairs)
}

/// What routing caps beyond each critic's own price cap and time limit.
pub fn limits_value(limits: &CritiqueLimits) -> Value {
    let quota = &limits.antigravity_quota;
    json::object(vec![
        (
            "antigravity_quota",
            json::object(vec![("daily_cap", Value::Num(quota.daily_cap)), ("reserve_threshold", Value::Num(quota.reserve_threshold)), ("reserved_stages", strings(&quota.reserved_stages))]),
        ),
        ("search", json::object(vec![("max_bytes", Value::Num(limits.search.max_bytes)), ("max_matches", Value::Num(limits.search.max_matches))])),
    ])
}

/// How a run's depth was chosen.
pub fn depth_choice_value(choice: &DepthChoice) -> Value {
    let mut measures = Object::new();
    for (name, measured) in &choice.measures {
        measures.set(name, json::object(vec![("threshold", Value::Num(measured.threshold)), ("value", Value::Num(measured.value))]));
    }
    let signals = choice.signals.iter().map(|signal| match signal {
        DepthSignal::Threshold { measure, threshold, value } => {
            json::object(vec![("kind", Value::str("threshold")), ("measure", Value::str(measure)), ("threshold", Value::Num(*threshold)), ("value", Value::Num(*value))])
        }
        DepthSignal::RiskTerm { term } => json::object(vec![("kind", Value::str("risk-term")), ("term", Value::str(term))]),
        DepthSignal::Label { label } => json::object(vec![("kind", Value::str("label")), ("label", Value::str(label))]),
    });
    json::object(vec![("chosen_by", Value::str(&choice.chosen_by)), ("measures", Value::Obj(measures)), ("signals", Value::Arr(signals.collect()))])
}
