//! Delegation profiles: one named profile owns the models a delegation may use
//! and the tool scope it runs with. A request may name a profile or narrow it,
//! never widen it. The repository data policy always wins: it is intersected
//! with the profile, and a profile left with no allowed model is an error,
//! never a silent fallback. The profiles come from the shipped defaults, which
//! `scripts/render_registry.py` renders from `registry/delegation-profiles.yaml`.

use crate::config::{CostClass, DelegationProfile, GatewayConfig, Tier, ToolList, ToolScope, TOOL_KINDS};
use crate::error::GatewayError;
use crate::json::{object, Object, Value};
use crate::policy::PolicyDecision;
use std::collections::BTreeSet;

/// What a request narrows a profile to; every field may only take something away.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Narrowing {
    pub models: Option<Vec<String>>,
    pub mcp: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub plugins: Option<Vec<String>>,
    pub max_spend_usd: Option<f64>,
    /// The family of the writer whose work a critic reads; it is left out of the allowed models.
    pub writer_family: Option<String>,
}

impl Narrowing {
    fn tools(&self, kind: &str) -> Option<&Vec<String>> {
        match kind {
            "mcp" => self.mcp.as_ref(),
            "plugins" => self.plugins.as_ref(),
            _ => self.skills.as_ref(),
        }
    }

    /// The narrowing as the run's `input.json` holds it.
    pub fn to_json(&self) -> Value {
        let list = |items: &Vec<String>| Value::Arr(items.iter().map(|item| Value::str(item)).collect());
        let mut members: Vec<(&str, Value)> = Vec::new();
        if let Some(models) = &self.models {
            members.push(("models", list(models)));
        }
        for kind in ["mcp", "skills", "plugins"] {
            if let Some(items) = self.tools(kind) {
                members.push((kind, list(items)));
            }
        }
        if let Some(cap) = self.max_spend_usd {
            members.push(("max_spend_usd", Value::Num(cap)));
        }
        if let Some(family) = &self.writer_family {
            members.push(("writer_family", Value::str(family)));
        }
        object(members)
    }
}

const NARROWING_FIELDS: [&str; 6] = ["models", "mcp", "skills", "plugins", "max_spend_usd", "writer_family"];

/// Reads a request's narrowing, or says what is wrong with it.
pub fn read_narrowing(raw: Option<&Value>) -> Result<Option<Narrowing>, GatewayError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(object) = raw.as_object() else {
        return Err(GatewayError::invalid_input("narrow must be an object"));
    };
    if let Some((key, _)) = object.entries().into_iter().find(|(key, _)| !NARROWING_FIELDS.contains(key)) {
        return Err(GatewayError::invalid_input(format!("narrow.{key} is not a field; choose from {}", NARROWING_FIELDS.join(", "))));
    }
    let models = narrowing_list(object, "models")?;
    let mcp = narrowing_list(object, "mcp")?;
    let skills = narrowing_list(object, "skills")?;
    let plugins = narrowing_list(object, "plugins")?;
    let max_spend_usd = match object.get("max_spend_usd") {
        None => None,
        Some(Value::Num(cap)) if cap.is_finite() && *cap > 0.0 => Some(*cap),
        Some(_) => return Err(GatewayError::invalid_input("narrow.max_spend_usd must be a number above 0")),
    };
    let writer_family = match object.get("writer_family") {
        None => None,
        Some(Value::Str(family)) if !family.is_empty() => Some(family.clone()),
        Some(_) => return Err(GatewayError::invalid_input("narrow.writer_family must be a non-empty string")),
    };
    Ok(Some(Narrowing { models, mcp, skills, plugins, max_spend_usd, writer_family }))
}

fn narrowing_list(object: &Object, name: &str) -> Result<Option<Vec<String>>, GatewayError> {
    let Some(value) = object.get(name) else {
        return Ok(None);
    };
    let items: Option<Vec<String>> = match value {
        Value::Arr(items) => items.iter().map(|item| item.as_str().filter(|text| !text.is_empty()).map(str::to_string)).collect(),
        _ => None,
    };
    match items {
        Some(items) if name != "models" || !items.is_empty() => Ok(Some(items)),
        _ => Err(GatewayError::invalid_input(format!(
            "narrow.{name} must be a {}list of non-empty strings",
            if name == "models" { "non-empty " } else { "" }
        ))),
    }
}

/// One model a profile leaves a request, with what the delegate picks by.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileModel {
    pub client: String,
    pub model: String,
    pub family: String,
    pub tier: Option<Tier>,
    pub cost_class: CostClass,
}

impl ProfileModel {
    fn key(&self) -> String {
        format!("{}:{}", self.client, self.model)
    }

    fn to_json(&self) -> Value {
        object(vec![
            ("client", Value::str(&self.client)),
            ("cost_class", Value::str(self.cost_class.as_str())),
            ("family", Value::str(&self.family)),
            ("model", Value::str(&self.model)),
            ("tier", self.tier.map_or(Value::Null, |tier| Value::str(tier.as_str()))),
        ])
    }
}

/// The allowed models, cheapest first, or why there are none.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    pub models: Vec<ProfileModel>,
    /// Set exactly when `models` is empty.
    pub error: Option<String>,
}

fn refused(message: impl Into<String>) -> GatewayError {
    GatewayError::new("profile-refused", message)
}

/// The named profile, or the refusal that says the defaults state no such profile.
pub fn require_profile<'a>(config: &'a GatewayConfig, name: &str) -> Result<&'a DelegationProfile, GatewayError> {
    config.profiles.get(name).ok_or_else(|| {
        let known: Vec<&str> = config.profiles.keys().map(String::as_str).collect();
        let known = if known.is_empty() { "none".to_string() } else { known.join(", ") };
        GatewayError::new("unknown-profile", format!("no delegation profile {name}: the defaults state {known}"))
    })
}

/// Refuses a narrowing that widens the profile: a model, a tool or a spend the profile does not give.
pub fn require_narrowing(profile: &DelegationProfile, narrow: Option<&Narrowing>) -> Result<(), GatewayError> {
    let Some(narrow) = narrow else {
        return Ok(());
    };
    let name = &profile.name;
    for key in narrow.models.iter().flatten() {
        if !profile.models.contains(key) {
            return Err(refused(format!(
                "narrow.models names {key}, which profile {name} does not allow: a request may narrow a profile, never widen it"
            )));
        }
    }
    for kind in TOOL_KINDS {
        let (Some(asked), ToolList::Only(given)) = (narrow.tools(kind), profile.tools.of(kind)) else {
            continue;
        };
        if let Some(extra) = asked.iter().find(|item| !given.contains(item)) {
            return Err(refused(format!(
                "narrow.{kind} names {extra}, which profile {name} does not grant (its {kind} scope is {}): a request may narrow a profile, never widen it",
                if given.is_empty() { "deny-all".to_string() } else { given.join(", ") }
            )));
        }
    }
    if let (Some(cap), Some(asked)) = (profile.max_spend_usd, narrow.max_spend_usd) {
        if asked > cap {
            return Err(refused(format!(
                "narrow.max_spend_usd {} is above profile {name}'s cap of {}: a request may narrow a profile, never widen it",
                crate::json::number_text(asked),
                crate::json::number_text(cap)
            )));
        }
    }
    Ok(())
}

/// The tool scope a run holds: the profile's, narrowed.
pub fn effective_tools(profile: &DelegationProfile, narrow: Option<&Narrowing>) -> ToolScope {
    let pick = |kind: &str| match narrow.and_then(|narrow| narrow.tools(kind)) {
        Some(items) => ToolList::Only(items.clone()),
        None => profile.tools.of(kind).clone(),
    };
    ToolScope { mcp: pick("mcp"), plugins: pick("plugins"), skills: pick("skills") }
}

/// Whether a skill or plugin entry is one plain name, which is the folder or file it names in Pi's home and
/// never a path that reaches out of it.
pub fn plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphanumeric()) && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The limit the chosen client cannot enforce, as a reason, or `None` when it can hold the scope. Only Pi
/// enforces: its per-run home is built from the scope, so it holds a list of skills and plugins (extensions) by
/// name, and holds mcp at none. It cannot hold `any` (its home is built only from named items) or an MCP server
/// (the confined launch reaches only the route's proxy port and starts no other program, so a server could never
/// connect). Every other client is unable to hold a restricted scope.
pub fn unenforceable(profile: &str, client: &str, is_pi: bool, tools: &ToolScope) -> Option<String> {
    if !is_pi {
        let restricted: Vec<&str> = TOOL_KINDS.into_iter().filter(|kind| *tools.of(kind) != ToolList::Any).collect();
        if restricted.is_empty() {
            return None;
        }
        return Some(format!(
            "profile {profile} restricts {} (a scope is deny-all unless the profile lists it), and {client} cannot enforce a tool scope: only Pi does",
            restricted.join(", ")
        ));
    }
    let open: Vec<&str> = TOOL_KINDS.into_iter().filter(|kind| *tools.of(kind) == ToolList::Any).collect();
    if !open.is_empty() {
        return Some(format!(
            "profile {profile} leaves {} open with any, and {client} builds its per-run home only from named items: list the {} it needs, or leave them out for deny-all",
            open.join(", "),
            open.join(", ")
        ));
    }
    let named = |list: &ToolList| -> Vec<String> { if let ToolList::Only(items) = list { items.clone() } else { Vec::new() } };
    let servers = named(&tools.mcp);
    if !servers.is_empty() {
        return Some(format!(
            "profile {profile} grants mcp {}, and {client} cannot enforce it: Pi has a built-in MCP client, but a confined launch reaches only the route's proxy port and starts no other program, so a listed server could never connect; mcp stays deny-all on Pi",
            servers.join(", ")
        ));
    }
    for kind in ["skills", "plugins"] {
        if let Some(bad) = named(tools.of(kind)).iter().find(|item| !plain_name(item)) {
            return Some(format!(
                "profile {profile} lists {kind} entry {bad}, which is not a plain name: a {} is named by the folder or file it is in Pi's home, never a path",
                if kind == "skills" { "skill" } else { "plugin" }
            ));
        }
    }
    None
}

/// The tool scope a run holds, or `None` when it names no profile (or one the defaults do not state).
pub fn run_scope(config: &GatewayConfig, profile: Option<&str>, narrow: Option<&Narrowing>) -> Option<ToolScope> {
    config.profiles.get(profile?).map(|profile| effective_tools(profile, narrow))
}

/// A tool scope as the ledger and `models` write it: each kind a list of names, or `any`.
pub fn tools_json(scope: &ToolScope) -> Value {
    let list = |list: &ToolList| match list {
        ToolList::Any => Value::str("any"),
        ToolList::Only(items) => Value::Arr(items.iter().map(|item| Value::str(item)).collect()),
    };
    object(vec![("mcp", list(&scope.mcp)), ("plugins", list(&scope.plugins)), ("skills", list(&scope.skills))])
}

/// The models of the profile a request may use. `available` is what the data policy permits (and, for a
/// listing, what is usable now), so the policy is the intersection; a profile with nothing left is an error
/// with its reason. With `for_run`, a profile that leaves out the writer's family needs the writer's family named.
pub fn resolve_profile(profile: &DelegationProfile, available: &[ProfileModel], policy_reason: &str, narrow: Option<&Narrowing>, for_run: bool) -> Resolution {
    let name = &profile.name;
    let asked = narrow.and_then(|narrow| narrow.models.as_ref()).unwrap_or(&profile.models);
    let pinned = profile.pin.as_ref().map_or(String::new(), |pin| format!(" (pinned to {pin})"));
    let wanted: Vec<&String> = profile.models.iter().filter(|key| asked.contains(key) && profile.pin.as_ref().is_none_or(|pin| pin == *key)).collect();
    let permitted: Vec<ProfileModel> =
        wanted.iter().filter_map(|key| available.iter().find(|model| model.key() == **key)).cloned().collect();
    let failed = |error: String| Resolution { models: Vec::new(), error: Some(error) };
    if permitted.is_empty() {
        return failed(format!(
            "profile {name} has no allowed model{pinned}: none of its {} models is both usable now and permitted by the data policy ({policy_reason})",
            wanted.len()
        ));
    }
    let mut left = permitted;
    if profile.exclude_writer_family == Some(true) {
        match narrow.and_then(|narrow| narrow.writer_family.as_ref()) {
            None if for_run => {
                return failed(format!("profile {name} leaves out the writer's family, so the request must name it as narrow.writer_family"));
            }
            None => {}
            Some(writer) => {
                left.retain(|model| model.family != *writer);
                if left.is_empty() {
                    return failed(format!("profile {name} has no allowed model once the writer's family {writer} is left out"));
                }
            }
        }
    }
    let families: BTreeSet<&str> = left.iter().map(|model| model.family.as_str()).collect();
    if let Some(distinct) = profile.distinct_families {
        if (families.len() as f64) < distinct {
            let seen: Vec<&str> = families.into_iter().collect();
            return failed(format!(
                "profile {name} needs {} distinct families but only {} remain ({})",
                crate::json::number_text(distinct),
                seen.len(),
                seen.join(", ")
            ));
        }
    }
    left.sort_by_key(|model| {
        let tier = model.tier.map_or(Tier::ALL.len(), |tier| Tier::ALL.iter().position(|candidate| *candidate == tier).unwrap_or(0));
        let cost = CostClass::ALL.iter().position(|candidate| *candidate == model.cost_class).unwrap_or(0);
        (cost, tier, profile.models.iter().position(|key| *key == model.key()))
    });
    Resolution { models: left, error: None }
}

/// The models a run's profile leaves it for the repository the decision was made about: the catalogue's models
/// the data policy permits, cut to the profile's (narrowed) models.
pub fn profile_models(config: &GatewayConfig, decision: &PolicyDecision, name: &str, narrow: Option<&Narrowing>) -> Result<Resolution, GatewayError> {
    let profile = require_profile(config, name)?;
    require_narrowing(profile, narrow)?;
    let permitted: BTreeSet<(&str, &str)> =
        decision.permissions.iter().filter(|permission| permission.permitted).map(|permission| (permission.client.as_str(), permission.model.as_str())).collect();
    let available: Vec<ProfileModel> = config
        .models
        .iter()
        .filter(|entry| permitted.contains(&(entry.client.as_str(), entry.model.as_str())))
        .filter_map(|entry| {
            let client = config.clients.get(&entry.client)?;
            Some(ProfileModel { client: entry.client.clone(), model: entry.model.clone(), family: entry.family.clone(), tier: Some(entry.tier), cost_class: client.cost_class })
        })
        .collect();
    Ok(resolve_profile(profile, &available, &decision.reason, narrow, true))
}

/// Holds a run to its profile, or refuses it with the reason: the profile leaves no allowed model for the
/// repository, the client model is not one of them, the request widens the profile, or the profile's tool scope
/// names a limit the client cannot enforce.
pub fn hold_to_profile(
    config: &GatewayConfig,
    decision: &PolicyDecision,
    name: &str,
    narrow: Option<&Narrowing>,
    client: &str,
    model: &str,
    provider: Option<&str>,
    is_pi: bool,
) -> Result<(), GatewayError> {
    let resolution = profile_models(config, decision, name, narrow)?;
    if let Some(error) = resolution.error {
        return Err(refused(error));
    }
    // A Hermes run on a route's provider runs the model the profile allows on that route's Pi client, and so does
    // an OpenCode run on its `litellm-<route>/<model>` provider.
    let (allowed_as, allowed_model) = match (client, crate::run::opencode_pi_model(config, client, model)) {
        (_, Some((pi_client, pi_model))) => (pi_client, pi_model),
        (crate::run::HERMES, None) => (provider.and_then(|provider| crate::run::hermes_route(config, provider)).unwrap_or_else(|| client.to_string()), model),
        _ => (client.to_string(), model),
    };
    if !resolution.models.iter().any(|allowed| allowed.client == allowed_as && allowed.model == allowed_model) {
        let allowed: Vec<String> = resolution.models.iter().map(ProfileModel::key).collect();
        return Err(refused(format!(
            "{client}:{model} is not allowed by profile {name} here: the profile and the data policy leave {}",
            allowed.join(", ")
        )));
    }
    let profile = require_profile(config, name)?;
    if !is_pi && profile.unenforced_clients.as_ref().is_some_and(|clients| clients.iter().any(|named| named == client)) {
        return Ok(());
    }
    match unenforceable(name, client, is_pi, &effective_tools(profile, narrow)) {
        Some(reason) => Err(refused(reason)),
        None => Ok(()),
    }
}

/// Every profile, by name, as `models` lists it: what it allows among the models `models` lists as permitted
/// (the repository's data policy and what is usable now), or why it allows nothing.
pub fn list_profiles(config: &GatewayConfig, permitted: &[ProfileModel], policy_reason: &str) -> Value {
    let strings = |items: &[String]| Value::Arr(items.iter().map(|item| Value::str(item)).collect());
    Value::Arr(
        config
            .profiles
            .values()
            .map(|profile| {
                let resolution = resolve_profile(profile, permitted, policy_reason, None, false);
                let mut members = vec![
                    ("description", Value::str(&profile.description)),
                    ("models", Value::Arr(resolution.models.iter().map(ProfileModel::to_json).collect())),
                    ("name", Value::str(&profile.name)),
                    ("tools", tools_json(&profile.tools)),
                ];
                members.extend(profile.pin.as_deref().map(|pin| ("pin", Value::str(pin))));
                members.extend(profile.ladder.as_deref().map(|ladder| ("ladder", strings(ladder))));
                members.extend(profile.distinct_families.map(|count| ("distinct_families", Value::Num(count))));
                members.extend(profile.exclude_writer_family.map(|flag| ("exclude_writer_family", Value::Bool(flag))));
                members.extend(profile.max_spend_usd.map(|cap| ("max_spend_usd", Value::Num(cap))));
                members.extend(profile.unenforced_clients.as_deref().map(|clients| ("unenforced_clients", strings(clients))));
                members.extend(resolution.error.as_deref().map(|error| ("error", Value::str(error))));
                object(members)
            })
            .collect(),
    )
}
