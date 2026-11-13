//! Resolving a stage's roster for one run: which critic runs through which client, which are dropped and why. The node
//! runner's `domain/routing.ts`, the half of it that fills a roster.

use crate::error::CritiqueError;
use crate::roster::{DroppedCritic, RosterCritic, RosterFallback};
use crate::routing::{stage_routing, CriticClient, RosterSlot, Routing, StageRouting};
use std::collections::HashMap;

/// What a roster is resolved for.
pub struct RosterRequest<'a> {
    pub stage: &'a str,
    pub depth: &'a str,
    pub allowed_families: &'a [String],
    pub orchestrator_family: &'a str,
    /// Whether the artifact holds a decision record, which adds the stage's ADR critics.
    pub adr: bool,
}

/// Whether a client may run in this run: ready, and within its quota. A client past the Antigravity quota is set up
/// but may not be launched today.
#[derive(Clone, Debug, PartialEq)]
pub struct Availability {
    pub ready: bool,
    pub detail: String,
    /// `quota` for a client past its quota.
    pub reason: Option<&'static str>,
}

/// A run's roster.
#[derive(Clone, Debug, PartialEq)]
pub struct Roster {
    pub depth: String,
    pub distinct_families: bool,
    pub critics: Vec<RosterCritic>,
    pub dropped: Vec<DroppedCritic>,
}

/// Whether Codex may run for a `nedap` repository now: a live model-gateway grant covers the run's workflow, or
/// `detail` says what is missing.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkGrant {
    Live { workflow: String },
    Missing { detail: String },
}

pub struct WorkRosterRequest<'a> {
    pub stage: &'a str,
    pub orchestrator_family: &'a str,
    pub grant: &'a WorkGrant,
}

pub fn client_family(client: &CriticClient) -> &str {
    match client {
        CriticClient::Vibe(client) => &client.family,
        CriticClient::Agy(client) => &client.family,
        CriticClient::Codex(client) => &client.family,
        CriticClient::Claude(client) => &client.family,
    }
}

pub fn client_model(client: &CriticClient) -> &str {
    match client {
        CriticClient::Vibe(client) => &client.model,
        CriticClient::Agy(client) => &client.model,
        CriticClient::Codex(client) => &client.model,
        CriticClient::Claude(client) => &client.model,
    }
}

fn client_named<'a>(routing: &'a Routing, stage: &str, name: &str) -> Result<&'a CriticClient, CritiqueError> {
    match routing.clients.iter().find(|(key, _)| key == name) {
        Some((_, client)) => Ok(client),
        None => Err(CritiqueError::new("invalid-routing", format!("routing stage {stage} names unknown client {name}"))),
    }
}

/// The slot's routed client and the clients tried after it, in routing order.
fn slot_candidates(routing: &Routing, client: &str) -> Vec<String> {
    let fallbacks = routing.fallbacks.iter().find(|(key, _)| key == client).map(|(_, names)| names.clone()).unwrap_or_default();
    std::iter::once(client.to_string()).chain(fallbacks).collect()
}

/// A stage's slots in the order they fill: base, then the ADR critics when the artifact holds a record, then deep at
/// deep depth.
fn slots_at<'a>(stage: &'a StageRouting, request: &RosterRequest) -> Vec<(&'a RosterSlot, bool)> {
    let mut slots: Vec<(&RosterSlot, bool)> = stage.base.iter().map(|slot| (slot, false)).collect();
    if request.adr {
        slots.extend(stage.adr.iter().map(|slot| (slot, false)));
    }
    if request.depth == "deep" {
        slots.extend(stage.deep.iter().map(|slot| (slot, true)));
    }
    slots
}

fn family_reason(family: &str, request: &RosterRequest) -> Option<&'static str> {
    if !request.allowed_families.iter().any(|allowed| allowed == family) {
        return Some("family-not-allowed");
    }
    (family == request.orchestrator_family).then_some("orchestrator-family")
}

/// The clients a stage could run, routed or fallback, whose family the allowlist permits and the orchestrator does not
/// share: the ones whose readiness is worth checking.
pub fn candidate_clients<'a>(routing: &'a Routing, request: &RosterRequest) -> Result<Vec<(String, &'a CriticClient)>, CritiqueError> {
    let stage = stage_routing(routing, request.stage)?;
    let mut names: Vec<String> = Vec::new();
    for (slot, _) in slots_at(stage, request) {
        for name in slot_candidates(routing, &slot.client) {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    let mut candidates = Vec::new();
    for name in names {
        let client = client_named(routing, request.stage, &name)?;
        if family_reason(client_family(client), request).is_none() {
            candidates.push((name, client));
        }
    }
    Ok(candidates)
}

/// Why a client may not run, or `None`: the family rules, then the readiness it was asked for.
fn refusal(stage: &StageRouting, critics: &[RosterCritic], request: &RosterRequest, readiness: &HashMap<String, Availability>, name: &str, client: &CriticClient) -> Option<(Option<String>, String)> {
    let family = client_family(client);
    let reason = family_reason(family, request)
        .or_else(|| (stage.distinct_families && critics.iter().any(|critic| client_family(&critic.client_config) == family)).then_some("same-family"));
    if let Some(reason) = reason {
        return Some((None, reason.to_string()));
    }
    let ready = readiness.get(name).filter(|ready| !ready.ready)?;
    Some((Some(ready.detail.clone()), ready.reason.unwrap_or("unavailable").to_string()))
}

/// Resolves a stage's roster at the request's depth: the base slots, then at deep depth the deep slots. Each slot
/// runs its routed client, or else the first of that client's fallbacks that may run, with the slot's role and lenses.
/// A slot no client can fill stays empty, so the stage runs with fewer critics rather than two of one family.
pub fn resolve_roster(routing: &Routing, request: &RosterRequest, readiness: &HashMap<String, Availability>) -> Result<Roster, CritiqueError> {
    let stage = stage_routing(routing, request.stage)?;
    let mut critics: Vec<RosterCritic> = Vec::new();
    let mut dropped: Vec<DroppedCritic> = Vec::new();
    for (slot, deep) in slots_at(stage, request) {
        let primary = client_named(routing, request.stage, &slot.client)?;
        let primary_refusal = refusal(stage, &critics, request, readiness, &slot.client, primary);
        let candidates = slot_candidates(routing, &slot.client);
        let mut name: Option<&String> = None;
        for candidate in &candidates {
            if refusal(stage, &critics, request, readiness, candidate, client_named(routing, request.stage, candidate)?).is_none() {
                name = Some(candidate);
                break;
            }
        }
        let mut fallbacks = Vec::new();
        for candidate in candidates.iter().filter(|candidate| Some(*candidate) != name) {
            let client = client_named(routing, request.stage, candidate)?;
            let ready = readiness.get(candidate).is_none_or(|ready| ready.ready);
            if family_reason(client_family(client), request).is_none() && ready {
                fallbacks.push(RosterFallback { client: candidate.clone(), client_config: client.clone() });
            }
        }
        let chosen = match name {
            None => None,
            Some(name) => Some(RosterCritic {
                client: name.clone(),
                role: slot.role.clone(),
                lenses: slot.lenses.clone(),
                max_price_usd: slot.max_price_usd,
                time_limit_s: slot.time_limit_s,
                critic_id: format!("c{}-{name}", critics.len() + 1),
                client_config: client_named(routing, request.stage, name)?.clone(),
                fallbacks,
            }),
        };
        if let Some((detail, reason)) = primary_refusal {
            dropped.push(DroppedCritic {
                client: slot.client.clone(),
                family: client_family(primary).to_string(),
                reason,
                detail,
                replaced_by: chosen.as_ref().map(|critic| critic.critic_id.clone()),
                deep: deep.then_some(true),
            });
        }
        critics.extend(chosen);
    }
    Ok(Roster { critics, depth: request.depth.to_string(), distinct_families: stage.distinct_families, dropped })
}

/// The gateway client a stage's Codex work slot is granted under: the one its routing names, else its kind.
pub fn work_grant_client(routing: &Routing, stage: &str) -> Result<String, CritiqueError> {
    for slot in &stage_routing(routing, stage)?.work {
        if let Some((_, CriticClient::Codex(codex))) = routing.clients.iter().find(|(name, _)| *name == slot.client) {
            return Ok(codex.gateway.as_ref().map_or("codex".to_string(), |gateway| gateway.client.clone()));
        }
    }
    Ok("codex".to_string())
}

/// Codex needs the grant; the work Claude agent runs on the work account and needs none.
fn work_reason(client: &CriticClient, request: &WorkRosterRequest) -> Option<(Option<String>, &'static str)> {
    if client_family(client) == request.orchestrator_family {
        return Some((None, "orchestrator-family"));
    }
    match (client, request.grant) {
        (CriticClient::Codex(_), WorkGrant::Missing { detail }) => Some((Some(detail.clone()), "no-grant")),
        _ => None,
    }
}

/// The work clients of a stage whose family is not the orchestrator's and that the grant lets run: the ones whose
/// readiness is worth checking.
pub fn work_candidate_clients<'a>(routing: &'a Routing, request: &WorkRosterRequest) -> Result<Vec<(String, &'a CriticClient)>, CritiqueError> {
    let mut candidates = Vec::new();
    for slot in &stage_routing(routing, request.stage)?.work {
        let client = client_named(routing, request.stage, &slot.client)?;
        if work_reason(client, request).is_none() {
            candidates.push((slot.client.clone(), client));
        }
    }
    Ok(candidates)
}

/// Resolves a `nedap` repository's roster from the stage's work slots: Codex on the nedap account under a live grant,
/// and the work Claude agent, each never in the orchestrator's own family. A slot that may not run is dropped with its
/// reason and has no fallback; the roster holds no family a work slot does not name, and no deep critics.
pub fn resolve_work_roster(routing: &Routing, request: &WorkRosterRequest, readiness: &HashMap<String, Availability>) -> Result<Roster, CritiqueError> {
    let mut critics: Vec<RosterCritic> = Vec::new();
    let mut dropped: Vec<DroppedCritic> = Vec::new();
    for slot in &stage_routing(routing, request.stage)?.work {
        let client = client_named(routing, request.stage, &slot.client)?;
        let refused = work_reason(client, request).map(|(detail, reason)| (detail, reason.to_string())).or_else(|| {
            let ready = readiness.get(&slot.client).filter(|ready| !ready.ready)?;
            Some((Some(ready.detail.clone()), ready.reason.unwrap_or("unavailable").to_string()))
        });
        if let Some((detail, reason)) = refused {
            dropped.push(DroppedCritic { client: slot.client.clone(), family: client_family(client).to_string(), reason, detail, replaced_by: None, deep: None });
            continue;
        }
        critics.push(RosterCritic {
            client: slot.client.clone(),
            role: slot.role.clone(),
            lenses: slot.lenses.clone(),
            max_price_usd: slot.max_price_usd,
            time_limit_s: slot.time_limit_s,
            critic_id: format!("c{}-{}", critics.len() + 1, slot.client),
            client_config: client.clone(),
            fallbacks: Vec::new(),
        });
    }
    Ok(Roster { critics, depth: "base".to_string(), distinct_families: true, dropped })
}
