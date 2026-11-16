//! The live LiteLLM model catalogue (#568), the port of
//! `domain/litellm-catalogue.ts` and `application/litellm-catalogue.ts`: what
//! the litellm repository's sync found when it probed every model of every
//! provider route. Setup fetches the published `model-catalog.json` into the
//! cache (scripts/litellm_catalogue.py); the gateway only reads that copy.
//!
//! A client that names a `litellm_route` has its listed models judged by that
//! route's probes: only a usable one is offered, a blocked one is skipped and
//! refused with the provider's own reason, and a free one is marked. A model
//! the catalogue does not know is offered as before.

use crate::config::{GatewayConfig, Tier};
use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::locations;
use crate::models::{
    cost_class_for, family_of, model_part, open_code_provider, ListedModel, ModelListing, SkippedModel, FAMILY_UNKNOWN,
};
use crate::ports::Runtime;
use crate::timestamps;
use std::collections::BTreeMap;

/// What one probe found for one model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueProbe {
    pub status: String,
    pub reason: String,
    pub free: bool,
    /// What the model does (`chat`, `embedding`, ...); `None` when the catalogue does not say.
    pub modality: Option<String>,
    /// The vendor-qualified id the model goes by everywhere (`reflection/beam-1.0-501b-a23b`), when the catalogue names it.
    pub canonical: Option<String>,
    pub context_length: Option<i64>,
}

/// One deployment behind a canonical group: the provider route and the id it serves the group's model under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueDeployment {
    pub route: String,
    pub upstream: String,
    /// Deployments of the lowest order are tried first.
    pub order: i64,
}

/// One canonical model name the proxy serves, and what serves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueGroup {
    /// A public group is a free tier: who may send the prompt, never the price.
    pub public: bool,
    pub modality: Option<String>,
    pub deployments: Vec<CatalogueDeployment>,
}

/// Each provider route's models, by id without the route prefix, in the order the catalogue lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiteLlmCatalogue {
    pub updated_at: String,
    pub routes: BTreeMap<String, Vec<(String, CatalogueProbe)>>,
    /// The canonical groups the proxy serves, by name; empty when the catalogue lists none.
    pub groups: BTreeMap<String, CatalogueGroup>,
}

/// A catalogue older than this means the sync, or setup's fetch, has stopped: two days, against an hourly sync.
pub const CATALOGUE_STALE_AFTER_MS: i64 = 48 * 60 * 60 * 1000;

/// The catalogue a parsed `model-catalog.json` holds, or why it holds none.
pub fn parse_catalogue(raw: &Value) -> Result<LiteLlmCatalogue, String> {
    let Some(object) = raw.as_object() else {
        return Err("not a JSON object".to_string());
    };
    let Some(Value::Str(updated_at)) = object.get("updated_at") else {
        return Err("no updated_at".to_string());
    };
    let Some(providers) = object.get("providers").and_then(Value::as_object) else {
        return Err("no providers object".to_string());
    };
    let mut routes = BTreeMap::new();
    for (route, provider) in providers.entries() {
        let Some(Value::Arr(items)) = provider.as_object().and_then(|provider| provider.get("models")) else {
            return Err(format!("provider {route} has no models list"));
        };
        let mut models: Vec<(String, CatalogueProbe)> = Vec::new();
        for item in items {
            let Some((id, probe)) = probe_of(route, item) else {
                continue;
            };
            match models.iter_mut().find(|(known, _)| *known == id) {
                Some(slot) => slot.1 = probe,
                None => models.push((id, probe)),
            }
        }
        routes.insert(route.to_string(), models);
    }
    Ok(LiteLlmCatalogue { updated_at: updated_at.clone(), routes, groups: parse_groups(object.get("groups")) })
}

/// The canonical groups of a catalogue; a group or deployment that is not well formed is passed over.
fn parse_groups(raw: Option<&Value>) -> BTreeMap<String, CatalogueGroup> {
    let mut groups = BTreeMap::new();
    let Some(Value::Obj(raw)) = raw else {
        return groups;
    };
    for (name, group) in raw.entries() {
        let Some(group) = group.as_object() else {
            continue;
        };
        let Some(Value::Arr(items)) = group.get("deployments") else {
            continue;
        };
        let deployments = items
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|deployment| {
                let route = deployment.get("route").and_then(Value::as_str)?;
                let upstream = deployment.get("upstream_id").and_then(Value::as_str)?;
                let order = match deployment.get("order") {
                    Some(Value::Num(order)) => *order as i64,
                    _ => 0,
                };
                Some(CatalogueDeployment { route: route.to_string(), upstream: upstream.to_string(), order })
            })
            .collect();
        let public = matches!(group.get("public"), Some(Value::Bool(true)));
        let modality = group.get("modality").and_then(Value::as_str).map(str::to_string);
        groups.insert(name.to_string(), CatalogueGroup { public, modality, deployments });
    }
    groups
}

/// One listed model as its id without the route prefix and its probe; `None` for an entry with no string id.
fn probe_of(route: &str, item: &Value) -> Option<(String, CatalogueProbe)> {
    let model = item.as_object()?;
    let Some(Value::Str(id)) = model.get("id") else {
        return None;
    };
    let meta = model.get("metadata").and_then(Value::as_object);
    let text = |key: &str| meta.and_then(|meta| meta.get(key)).and_then(Value::as_str).map(str::to_string);
    let id = id.strip_prefix(&format!("{route}/")).unwrap_or(id).to_string();
    let free = matches!(meta.and_then(|meta| meta.get("free")), Some(Value::Bool(true)));
    let context_length = match meta.and_then(|meta| meta.get("context_length")) {
        Some(Value::Num(length)) => Some(*length as i64),
        _ => None,
    };
    Some((
        id,
        CatalogueProbe {
            status: text("status").unwrap_or_else(|| "unknown".to_string()),
            reason: text("reason").unwrap_or_default(),
            free,
            modality: text("modality"),
            canonical: text("canonical_id"),
            context_length,
        },
    ))
}

/// A warning when the catalogue is older than two days, or names no instant; `None` when it is fresh.
pub fn staleness(catalogue: &LiteLlmCatalogue, path: &str, now_ms: i64) -> Option<String> {
    let Some(at) = timestamps::parse(&catalogue.updated_at) else {
        return Some(format!("the LiteLLM catalogue at {path} names no updated_at instant, so its age is unknown"));
    };
    if now_ms - at <= CATALOGUE_STALE_AFTER_MS {
        return None;
    }
    let hours = (now_ms - at) / 3_600_000;
    Some(format!("the LiteLLM catalogue at {path} is stale: updated {}, {hours} hours ago; setup refreshes it", catalogue.updated_at))
}

/// The probe that judges a client model on its route: a usable probe under any name wins, else the first found.
pub fn probe_for<'a>(catalogue: &'a LiteLlmCatalogue, route: &str, names: &[String]) -> Option<&'a CatalogueProbe> {
    let models = catalogue.routes.get(route)?;
    let prefix = format!("{route}/");
    let probes: Vec<&CatalogueProbe> = names
        .iter()
        .filter_map(|name| match name.strip_prefix(&prefix) {
            Some(bare) => Some(bare),
            None if name.contains('/') => None,
            None => Some(name.as_str()),
        })
        .filter_map(|bare| models.iter().find(|(id, _)| id == bare).map(|(_, probe)| probe))
        .collect();
    probes.iter().find(|probe| probe.status == "usable").or_else(|| probes.first()).copied()
}

/// Why a model the catalogue does not call usable is not offered.
pub fn not_usable_reason(route: &str, probe: &CatalogueProbe) -> String {
    if probe.status == "blocked" {
        return format!("blocked by the provider on LiteLLM route {route}: {}", probe.reason);
    }
    let reason = if probe.reason.is_empty() { String::new() } else { format!(": {}", probe.reason) };
    format!("the LiteLLM catalogue marks it {} on route {route}{reason}", probe.status)
}

/// Every name a client model goes by: its id, and the id and names of the defaults entry it belongs to.
pub fn names_of(config: &GatewayConfig, client: &str, model: &str) -> Vec<String> {
    let mut names = vec![model.to_string()];
    if let Some(entry) = config.models.iter().find(|candidate| candidate.client == client && candidate.all_names().contains(&model)) {
        for name in entry.all_names() {
            if !names.iter().any(|known| known == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// The probe of one deployment: the model its route lists under the id the group is served by.
fn deployment_probe<'a>(catalogue: &'a LiteLlmCatalogue, deployment: &CatalogueDeployment) -> Option<&'a CatalogueProbe> {
    let bare = deployment.upstream.strip_prefix(&format!("{}/", deployment.route)).unwrap_or(&deployment.upstream);
    probe_for(catalogue, &deployment.route, &[format!("{}/{bare}", deployment.route)])
}

/// What the catalogue says of a canonical group: its route and probe are those of a usable deployment, else of the
/// first the catalogue knows, and it is free only when every deployment, fallbacks included, is a model its provider
/// marks free: a public group is a free tier that still bills once its quota is gone. `None` when the catalogue
/// probed none of its deployments.
fn group_probe(catalogue: &LiteLlmCatalogue, group: &CatalogueGroup) -> Option<(String, CatalogueProbe)> {
    let known: Vec<(&CatalogueDeployment, &CatalogueProbe)> =
        group.deployments.iter().filter_map(|deployment| deployment_probe(catalogue, deployment).map(|probe| (deployment, probe))).collect();
    let (deployment, probe) = known.iter().find(|(_, probe)| probe.status == "usable").or_else(|| known.first())?;
    let free = group.deployments.iter().all(|deployment| deployment_probe(catalogue, deployment).is_some_and(|probe| probe.free));
    let modality = group.modality.clone().or_else(|| probe.modality.clone());
    Some((deployment.route.clone(), CatalogueProbe { free, modality, ..(*probe).clone() }))
}

/// Where the catalogue places a client model: the route and probe that judge it. A Pi client's model is a canonical
/// group, judged by its deployments; another client's is judged on its own `litellm_route`. `None` when the client
/// has no route to judge by or the catalogue does not know the model.
fn judged(config: &GatewayConfig, catalogue: &LiteLlmCatalogue, client: &str, model: &str) -> Option<(String, CatalogueProbe)> {
    if crate::run::is_pi_client(client) {
        return group_probe(catalogue, catalogue.groups.get(model)?);
    }
    let route = config.clients.get(client)?.litellm_route.clone()?;
    let probe = probe_for(catalogue, &route, &names_of(config, client, model))?.clone();
    Some((route, probe))
}

/// Why the catalogue refuses a client model, or `None` when it does not.
fn refusal(config: &GatewayConfig, catalogue: &LiteLlmCatalogue, client: &str, model: &str) -> Option<String> {
    judged(config, catalogue, client, model).filter(|(_, probe)| probe.status != "usable").map(|(route, probe)| not_usable_reason(&route, &probe))
}

/// Where the catalogue places a model that `defaults.json` has no entry for and whose id names no known family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub family: String,
    pub tier: Tier,
    pub notes: String,
}

/// The family a vendor's catalogue slug stands for, where the gateway's own family differs from the slug.
const VENDOR_FAMILIES: [(&str, &str); 8] = [
    ("moonshotai", "moonshot"),
    ("z-ai", "zhipu"),
    ("zhipuai", "zhipu"),
    ("mistralai", "mistral"),
    ("deepseek-ai", "deepseek"),
    ("x-ai", "xai"),
    ("qwen", "alibaba"),
    ("meta-llama", "meta"),
];

/// The words of a model id that mark it small, and the ones that mark it large.
const TRIVIAL_WORDS: [&str; 6] = ["lite", "mini", "nano", "tiny", "micro", "small"];
const HARD_WORDS: [&str; 6] = ["pro", "large", "ultra", "super", "max", "plus"];

/// The family of a model the catalogue knows by `canonical` (`vendor/name`) or, failing that, by its own id: a
/// family the gateway already names by the id's prefix, else the vendor's slug, else the id's first word.
fn family_from(canonical: Option<&str>, model: &str) -> String {
    let id = canonical.unwrap_or(model);
    if let Some(family) = family_of(model_part(id)).or_else(|| family_of(model_part(model))) {
        return family.to_string();
    }
    if let Some((vendor, _)) = canonical.and_then(|canonical| canonical.split_once('/')) {
        let vendor = vendor.to_ascii_lowercase();
        return VENDOR_FAMILIES.iter().find(|(slug, _)| *slug == vendor).map_or(vendor.clone(), |(_, family)| family.to_string());
    }
    let word: String = model_part(model).to_ascii_lowercase().chars().take_while(char::is_ascii_alphabetic).collect();
    if word.is_empty() { "unknown".to_string() } else { word }
}

/// The tier a model's name suggests, never above `hard`: the catalogue carries no capability, so a model with no
/// `defaults.json` entry is placed on its name alone, `routine` unless the name says small or large.
fn tier_from(model: &str) -> Tier {
    let lower = model_part(model).to_ascii_lowercase();
    let words: Vec<&str> = lower.split(|char: char| !char.is_ascii_alphanumeric()).collect();
    if words.iter().any(|word| TRIVIAL_WORDS.contains(word)) {
        Tier::Trivial
    } else if words.iter().any(|word| HARD_WORDS.contains(word)) {
        Tier::Hard
    } else {
        Tier::Routine
    }
}

/// How the catalogue places a client model no `defaults.json` entry covers: a chat model it knows takes its family
/// from its canonical id and its tier from its name. `None` when the catalogue does not know the model, and the
/// catalogue's own reason when it lists the model as something other than chat.
pub fn place_from_catalogue(config: &GatewayConfig, catalogue: &LiteLlmCatalogue, client: &str, model: &str) -> Option<Result<Placement, String>> {
    let (_, probe) = judged(config, catalogue, client, model)?;
    if let Some(modality) = probe.modality.as_deref().filter(|modality| *modality != "chat") {
        return Some(Err(format!("the LiteLLM catalogue lists it as {modality}, not a chat model")));
    }
    let canonical = probe.canonical.as_deref();
    let context = probe.context_length.map_or(String::new(), |length| format!(", {length} tokens of context"));
    Some(Ok(Placement {
        family: family_from(canonical, model),
        tier: tier_from(canonical.unwrap_or(model)),
        notes: format!("Placed from the LiteLLM catalogue ({}{context}), not from defaults.json: its tier is a guess from its name.", canonical.unwrap_or(model)),
    }))
}

/// A skipped model the catalogue places, as the listing offers it; `None` when the client may not reach its family.
fn placed_model(config: &GatewayConfig, skipped: &SkippedModel, placement: Placement, free: bool) -> Option<ListedModel> {
    let client = config.clients.get(&skipped.client)?;
    if client.families.as_ref().is_some_and(|families| !families.contains(&placement.family)) {
        return None;
    }
    let provider = if skipped.client == "opencode" { open_code_provider(&skipped.model) } else { None };
    Some(ListedModel {
        client: skipped.client.clone(),
        model: skipped.model.clone(),
        family: placement.family,
        cost_class: cost_class_for(client, provider.as_deref()),
        tier: Some(placement.tier),
        notes: placement.notes,
        source: client.model_source,
        free,
    })
}

/// The listing with the catalogue applied; without a catalogue it is unchanged. A model the gateway skipped for want
/// of a family and a tier is placed from the catalogue when the catalogue knows it and calls it usable chat.
pub fn judge_listing(config: &GatewayConfig, listing: ModelListing, catalogue: Option<&LiteLlmCatalogue>) -> ModelListing {
    let Some(catalogue) = catalogue else {
        return listing;
    };
    // A model skipped for another reason that the catalogue also refuses is skipped for the provider's reason.
    let mut skipped: Vec<SkippedModel> = Vec::new();
    let mut models = listing.models;
    for entry in listing.skipped {
        if let Some(reason) = refusal(config, catalogue, &entry.client, &entry.model) {
            skipped.push(SkippedModel { reason, ..entry });
            continue;
        }
        if entry.reason != FAMILY_UNKNOWN {
            skipped.push(entry);
            continue;
        }
        match place_from_catalogue(config, catalogue, &entry.client, &entry.model) {
            Some(Ok(placement)) => models.extend(placed_model(config, &entry, placement, false)),
            Some(Err(reason)) => skipped.push(SkippedModel { reason, ..entry }),
            None => skipped.push(entry),
        }
    }
    models.sort_by(|left, right| left.client.cmp(&right.client).then_with(|| left.model.cmp(&right.model)));
    let mut judged_models: Vec<ListedModel> = Vec::new();
    for model in models {
        if let Some(reason) = refusal(config, catalogue, &model.client, &model.model) {
            skipped.push(SkippedModel { client: model.client, model: model.model, reason });
            continue;
        }
        let free = judged(config, catalogue, &model.client, &model.model).is_some_and(|(_, probe)| probe.free);
        judged_models.push(ListedModel { free, ..model });
    }
    skipped.sort_by(|left, right| left.client.cmp(&right.client).then_with(|| left.model.cmp(&right.model)));
    ModelListing { models: judged_models, missing: listing.missing, skipped }
}

/// The catalogue this machine holds, if it holds a readable one, and what is wrong with it, if anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogueReading {
    pub catalogue: Option<LiteLlmCatalogue>,
    pub warning: Option<String>,
}

/// Reads the live LiteLLM catalogue setup fetched; nothing about it ever fails a command.
pub fn read_catalogue(runtime: &Runtime) -> CatalogueReading {
    let Ok(path) = locations::litellm_catalogue_path(&runtime.env) else {
        return CatalogueReading::default();
    };
    let unchecked = "so the models are listed unchecked";
    let warn = |warning: String| CatalogueReading { catalogue: None, warning: Some(warning) };
    match runtime.files.exists(&path) {
        Ok(true) => {}
        Ok(false) => return warn(format!("no LiteLLM catalogue at {path}: setup fetches it, {unchecked}")),
        Err(_) => return warn(format!("the LiteLLM catalogue at {path} cannot be read, {unchecked}")),
    }
    let Ok(text) = runtime.files.read_text(&path) else {
        return warn(format!("the LiteLLM catalogue at {path} cannot be read, {unchecked}"));
    };
    let Ok(raw) = json::parse(&text) else {
        return warn(format!("the LiteLLM catalogue at {path} is not JSON, {unchecked}"));
    };
    match parse_catalogue(&raw) {
        Err(problem) => warn(format!("the LiteLLM catalogue at {path} is not a catalogue ({problem}), {unchecked}")),
        Ok(catalogue) => {
            let warning = staleness(&catalogue, &path, runtime.clock.now_ms());
            CatalogueReading { catalogue: Some(catalogue), warning }
        }
    }
}

/// Refuses a run on a client model the catalogue says cannot be used on its route, with the provider's reason.
pub fn require_usable(runtime: &Runtime, config: &GatewayConfig, client: &str, model: &str) -> Result<(), GatewayError> {
    if config.clients.get(client).and_then(|client| client.litellm_route.as_ref()).is_none() && !crate::run::is_pi_client(client) {
        return Ok(());
    }
    let reading = read_catalogue(runtime);
    match reading.catalogue.as_ref().and_then(|catalogue| refusal(config, catalogue, client, model)) {
        Some(reason) => Err(GatewayError::new("model-not-usable", format!("{client}:{model} cannot run: {reason}"))),
        None => Ok(()),
    }
}

/// The family of a client model no `defaults.json` entry covers and whose id names none, from the catalogue; the
/// refusal says why when the catalogue cannot place it. A Pi client names a model by its canonical group, so an
/// upstream id such as `beam-free` is answered with the group that serves it.
pub fn place_unlisted(runtime: &Runtime, config: &GatewayConfig, client: &str, model: &str) -> Result<String, GatewayError> {
    let reading = read_catalogue(runtime);
    let placed = reading.catalogue.as_ref().and_then(|catalogue| place_from_catalogue(config, catalogue, client, model));
    match placed {
        Some(Ok(placement)) => Ok(placement.family),
        Some(Err(reason)) => Err(GatewayError::new("unknown-model", format!("{client}:{model} is not a model the gateway can place: {reason}"))),
        None => {
            let group = reading.catalogue.as_ref().filter(|_| crate::run::is_pi_client(client)).and_then(|catalogue| group_serving(catalogue, model));
            let hint = group.map_or(String::new(), |group| format!("; Pi names it {group}, the group that serves it"));
            Err(GatewayError::new(
                "unknown-model",
                format!("{client}:{model} is not a model the gateway can place: add it to the catalogue in defaults.json or the machine override{hint}"),
            ))
        }
    }
}

/// The canonical group one of whose deployments serves the upstream id.
fn group_serving<'a>(catalogue: &'a LiteLlmCatalogue, upstream: &str) -> Option<&'a str> {
    catalogue.groups.iter().find(|(_, group)| group.deployments.iter().any(|deployment| deployment.upstream == upstream)).map(|(name, _)| name.as_str())
}
