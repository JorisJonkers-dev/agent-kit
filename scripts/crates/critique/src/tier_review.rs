//! The tier-review roster: reviewers resolved from the chain of the `tier-review` LiteLLM alias, never from the alias
//! itself.
//!
//! One call to `tier-review` lets LiteLLM pick the first step that is available, so three calls may all land on the
//! same family. This module reads the chain (the snapshot of litellm's `tier_groups`, compiled in so it cannot drift
//! from `registry/litellm-tiers.json`) and picks one step per family in ladder order: free first, then Vibe Flash, then
//! Go Flash. Each reviewer then calls its step model directly, with the tier's own key, which holds the steps' access
//! groups. A step is skipped when its data scope bars it, its family is not allowed, is the writer's, or is already
//! taken, or when the gateway's ledger says its last answer was a refusal that still stands. What the reviewers do after
//! that (a retry through another family, the check of what served each one) is fan-out's and the Pi client's.

use crate::error::CritiqueError;
use crate::roster::{DroppedCritic, RosterCritic, RosterFallback};
use crate::roster_resolve::Roster;
use crate::routing::{CriticClient, PiCriticClient, RosterSlot, DATA_SCOPE_REASON};
use crate::runtime::Runtime;
use model_gateway::json::{self, Value};
use model_gateway::ledger::LedgerEntry;
use model_gateway::ledger_file::LedgerStore;
use model_gateway::usage::standing_refusals;

/// The snapshot of litellm's tier chains.
const SNAPSHOT: &str = include_str!("../../../../registry/litellm-tiers.json");

/// The tier that serves review.
pub const TIER: &str = "tier-review";

/// The Pi provider of the tier's key, written by setup.
pub const PROVIDER: &str = "litellm-tier-review";

/// The gateway client the tier's reviewers are recorded under.
pub const GATEWAY_CLIENT: &str = "pi-tier-review";

/// How many families a review spans at least.
pub const MIN_FAMILIES: usize = 3;

/// How deep a chain may nest tiers before it is read as a loop.
const MAX_NESTING: usize = 4;

/// One step of a chain: a model of one family, at one cost.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub model: String,
    pub cost: String,
}

/// The model families by the name a step model starts with, once the LiteLLM group prefix is cut off.
const FAMILIES: [(&str, &str); 12] = [
    ("nemotron", "nvidia"),
    ("gemma", "google"),
    ("gemini", "google"),
    ("glm", "zhipu"),
    ("zai-glm", "zhipu"),
    ("gpt-oss", "openai"),
    ("deepseek", "deepseek"),
    ("qwen", "alibaba"),
    ("minimax", "minimax"),
    ("mimo", "xiaomi"),
    ("mistral", "mistral"),
    ("beam", "reflection"),
];

/// The family of a step model, or none for a model no family is known for.
pub fn family_of(model: &str) -> Option<&'static str> {
    let bare = model.rsplit('/').next().unwrap_or(model);
    FAMILIES.iter().find(|(prefix, _)| bare.starts_with(prefix)).map(|(_, family)| *family)
}

/// The chain of a tier in ladder order, the tiers it falls through to expanded in place.
pub fn ladder(snapshot: &str, tier: &str) -> Result<Vec<Step>, CritiqueError> {
    let invalid = |message: String| CritiqueError::new("invalid-tiers", message);
    let parsed = json::parse(snapshot).map_err(|error| invalid(format!("the tier snapshot is not JSON: {error}")))?;
    let groups = parsed.as_object().and_then(|root| root.get("tier_groups")).and_then(Value::as_object).ok_or_else(|| invalid("the tier snapshot has no tier_groups".to_string()))?;
    let mut steps = Vec::new();
    expand(groups, tier, 0, &mut steps)?;
    Ok(steps)
}

fn expand(groups: &json::Object, tier: &str, depth: usize, steps: &mut Vec<Step>) -> Result<(), CritiqueError> {
    let invalid = |message: String| CritiqueError::new("invalid-tiers", message);
    if depth > MAX_NESTING {
        return Err(invalid(format!("tier {tier} nests tiers more than {MAX_NESTING} deep")));
    }
    let Some(Value::Arr(chain)) = groups.get(tier).and_then(Value::as_object).and_then(|group| group.get("steps")) else {
        return Err(invalid(format!("the tier snapshot has no tier {tier} with steps")));
    };
    for step in chain.iter().filter_map(Value::as_object) {
        match (step.get("model").and_then(Value::as_str), step.get("tier").and_then(Value::as_str)) {
            (Some(model), _) => steps.push(Step { model: model.to_string(), cost: step.get("cost").and_then(Value::as_str).unwrap_or("").to_string() }),
            (None, Some(inner)) => expand(groups, inner, depth + 1, steps)?,
            (None, None) => return Err(invalid(format!("a step of tier {tier} names neither a model nor a tier"))),
        }
    }
    Ok(())
}

/// The review tier's chain.
pub fn review_ladder() -> Result<Vec<Step>, CritiqueError> {
    ladder(SNAPSHOT, TIER)
}

/// What the roster is resolved for.
pub struct Request<'a> {
    pub allowed_families: &'a [String],
    pub writer_family: &'a str,
    pub public_repository: bool,
    /// The step models whose last answer was a refusal that still stands.
    pub cooled: &'a [String],
    pub want: usize,
}

/// A step's id as a client and directory name: no slash.
fn client_name(model: &str) -> String {
    model.replace('/', "-")
}

fn client_of(step: &Step, family: &str) -> CriticClient {
    let free = step.cost == "free";
    CriticClient::Pi(PiCriticClient {
        binary: "pi".to_string(),
        provider: PROVIDER.to_string(),
        model: step.model.clone(),
        family: family.to_string(),
        data_scope: if free { "public" } else { "any" }.to_string(),
        free,
        tier: Some(TIER.to_string()),
    })
}

/// Why a step may not review here, or none: its data scope, then its family.
fn bar(step: &Step, family: &str, request: &Request<'_>) -> Option<(&'static str, Option<String>)> {
    if step.cost == "free" && !request.public_repository {
        return Some(("data-scope", Some(DATA_SCOPE_REASON.to_string())));
    }
    if !request.allowed_families.iter().any(|allowed| allowed == family) {
        return Some(("family-not-allowed", None));
    }
    if family == request.writer_family {
        return Some(("orchestrator-family", None));
    }
    request.cooled.contains(&step.model).then(|| ("unavailable", Some("its last answer through the tier's key was a refusal that still stands".to_string())))
}

/// The reviewers of one run: the first step of each of `request.want` families in ladder order, each with the slot (role,
/// lenses, caps) of its place, cycling through `slots`, and the steps of other families as its fallbacks. A step with no
/// known family is returned in the second part, since nothing says whether it may review.
pub fn resolve_roster(steps: &[Step], slots: &[RosterSlot], request: &Request<'_>) -> (Roster, Vec<String>) {
    let mut unplaced: Vec<String> = Vec::new();
    let mut dropped: Vec<DroppedCritic> = Vec::new();
    let mut eligible: Vec<(&Step, &'static str)> = Vec::new();
    for step in steps {
        let Some(family) = family_of(&step.model) else {
            unplaced.push(step.model.clone());
            continue;
        };
        match bar(step, family, request) {
            Some((reason, detail)) => dropped.push(DroppedCritic { client: client_name(&step.model), family: family.to_string(), reason: reason.to_string(), detail, replaced_by: None, deep: None }),
            None => eligible.push((step, family)),
        }
    }
    let mut picks: Vec<(&Step, &'static str)> = Vec::new();
    for (step, family) in &eligible {
        if picks.len() < request.want && !picks.iter().any(|(_, taken)| taken == family) {
            picks.push((step, family));
        }
    }
    // A step of a family a pick already has is no use as a fallback, and is reported as the same family.
    let other_family = |family: &str| !picks.iter().any(|(_, taken)| *taken == family);
    for (step, family) in eligible.iter().filter(|(step, family)| !other_family(family) && !picks.iter().any(|(pick, _)| pick.model == step.model)) {
        dropped.push(DroppedCritic { client: client_name(&step.model), family: family.to_string(), reason: "same-family".to_string(), detail: None, replaced_by: None, deep: None });
    }
    let fallbacks: Vec<RosterFallback> = eligible.iter().filter(|(_, family)| other_family(family)).map(|(step, family)| RosterFallback { client: client_name(&step.model), client_config: client_of(step, family) }).collect();
    let critics = picks
        .iter()
        .enumerate()
        .map(|(at, (step, family))| {
            let slot = &slots[at % slots.len()];
            RosterCritic {
                client: client_name(&step.model),
                role: slot.role.clone(),
                lenses: slot.lenses.clone(),
                max_price_usd: slot.max_price_usd,
                time_limit_s: slot.time_limit_s,
                critic_id: format!("c{}-{}", at + 1, client_name(&step.model)),
                client_config: client_of(step, family),
                fallbacks: fallbacks.clone(),
            }
        })
        .collect();
    (Roster { depth: "base".to_string(), distinct_families: true, critics, dropped }, unplaced)
}

/// The step models of the tier's reviewers whose last answer was a refusal that still stands: a quota or rate answer the
/// gateway's ledger holds for the tier's client, until the reset it named or a later run that succeeded.
pub fn cooled_models(runtime: &Runtime) -> Result<Vec<String>, CritiqueError> {
    let entries = LedgerStore { runtime: &runtime.gateway }.read().map_err(|error| CritiqueError::unexpected(error.message))?;
    let records: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry {
            LedgerEntry::Run(record) if record.client == GATEWAY_CLIENT => Some(record),
            _ => None,
        })
        .collect();
    Ok(standing_refusals(&records, runtime.clock.now_ms()).into_iter().map(|refusal| refusal.model).collect())
}
