//! A run's roster as its manifest freezes it: the critics that run, the client each runs through, the fallbacks a
//! retry may use, and the critics that were dropped with their reason. The node runner's `domain/routing.ts`, the
//! half of it a manifest holds.

use crate::error::ShapeError;
use crate::family::CRITIC_FAMILIES;
use crate::fields::Fields;
use crate::routing::{read_critic_client, read_roster_slot, CriticClient};

/// The reasons a routed critic does not run.
pub const DROP_REASONS: [&str; 6] = ["family-not-allowed", "orchestrator-family", "same-family", "unavailable", "quota", "no-grant"];

/// A client a critic may be retried through.
#[derive(Clone, Debug, PartialEq)]
pub struct RosterFallback {
    pub client: String,
    pub client_config: CriticClient,
}

/// A critic resolved for one run: the slot plus the client it runs through.
#[derive(Clone, Debug, PartialEq)]
pub struct RosterCritic {
    pub client: String,
    pub role: String,
    pub lenses: Vec<String>,
    /// Vibe's price cap. Antigravity is quota, not money, so agy ignores it.
    pub max_price_usd: f64,
    pub time_limit_s: f64,
    pub critic_id: String,
    pub client_config: CriticClient,
    /// The slot's other clients, in routing order, that the allowlist permits, the orchestrator does not share and
    /// that are ready here: what fan-out may retry a failed critic through. A retry has none.
    pub fallbacks: Vec<RosterFallback>,
}

/// A routed critic that does not run, and the fallback critic that runs in its slot, if one could.
#[derive(Clone, Debug, PartialEq)]
pub struct DroppedCritic {
    pub client: String,
    pub family: String,
    pub reason: String,
    /// Why an unavailable client cannot run on this machine.
    pub detail: Option<String>,
    /// The critic id of the fallback that took the slot.
    pub replaced_by: Option<String>,
    /// Set for a critic of the deep roster, which is left out rather than leaving the base short.
    pub deep: Option<bool>,
}

/// One critic of a run's roster: its slot, the client it runs through and its fallbacks.
pub fn read_roster_critic(fields: &Fields<'_>) -> Result<RosterCritic, ShapeError> {
    let slot = read_roster_slot(fields)?;
    Ok(RosterCritic {
        client: slot.client,
        role: slot.role,
        lenses: slot.lenses,
        max_price_usd: slot.max_price_usd,
        time_limit_s: slot.time_limit_s,
        client_config: read_critic_client(&fields.object("client_config")?)?,
        critic_id: fields.string("critic_id")?,
        fallbacks: fields.list("fallbacks", |fallback| read_roster_fallback(fallback))?,
    })
}

/// One client a critic may be retried through.
fn read_roster_fallback(fields: &Fields<'_>) -> Result<RosterFallback, ShapeError> {
    Ok(RosterFallback { client: fields.string("client")?, client_config: read_critic_client(&fields.object("client_config")?)? })
}

/// A routed critic that does not run, with why and what took its slot.
pub fn read_dropped_critic(fields: &Fields<'_>) -> Result<DroppedCritic, ShapeError> {
    let detail = fields.optional_string("detail")?;
    let replaced_by = fields.optional_string("replaced_by")?;
    Ok(DroppedCritic {
        client: fields.string("client")?,
        family: fields.one_of("family", CRITIC_FAMILIES)?,
        reason: fields.one_of("reason", &DROP_REASONS)?,
        detail,
        replaced_by,
        deep: if fields.has("deep") && fields.boolean("deep")? { Some(true) } else { None },
    })
}
