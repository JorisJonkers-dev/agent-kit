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

use crate::config::GatewayConfig;
use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::locations;
use crate::models::{ListedModel, ModelListing, SkippedModel};
use crate::ports::Runtime;
use crate::timestamps;
use std::collections::BTreeMap;

/// What one probe found for one model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueProbe {
    pub status: String,
    pub reason: String,
    pub free: bool,
}

/// Each provider route's models, by id without the route prefix, in the order the catalogue lists them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiteLlmCatalogue {
    pub updated_at: String,
    pub routes: BTreeMap<String, Vec<(String, CatalogueProbe)>>,
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
    Ok(LiteLlmCatalogue { updated_at: updated_at.clone(), routes })
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
    Some((id, CatalogueProbe { status: text("status").unwrap_or_else(|| "unknown".to_string()), reason: text("reason").unwrap_or_default(), free }))
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

/// The probe of a client model when the client has a route and the catalogue knows the model, with its route.
fn judged<'a>(config: &GatewayConfig, catalogue: &'a LiteLlmCatalogue, client: &str, model: &str) -> Option<(String, &'a CatalogueProbe)> {
    let route = config.clients.get(client)?.litellm_route.clone()?;
    let probe = probe_for(catalogue, &route, &names_of(config, client, model))?;
    Some((route, probe))
}

/// Why the catalogue refuses a client model, or `None` when it does not.
fn refusal(config: &GatewayConfig, catalogue: &LiteLlmCatalogue, client: &str, model: &str) -> Option<String> {
    judged(config, catalogue, client, model).filter(|(_, probe)| probe.status != "usable").map(|(route, probe)| not_usable_reason(&route, probe))
}

/// The listing with the catalogue applied; without a catalogue it is unchanged.
pub fn judge_listing(config: &GatewayConfig, listing: ModelListing, catalogue: Option<&LiteLlmCatalogue>) -> ModelListing {
    let Some(catalogue) = catalogue else {
        return listing;
    };
    // A model skipped for another reason that the catalogue also refuses is skipped for the provider's reason.
    let mut skipped: Vec<SkippedModel> = listing
        .skipped
        .into_iter()
        .map(|entry| match refusal(config, catalogue, &entry.client, &entry.model) {
            Some(reason) => SkippedModel { reason, ..entry },
            None => entry,
        })
        .collect();
    let mut models: Vec<ListedModel> = Vec::new();
    for model in listing.models {
        if let Some(reason) = refusal(config, catalogue, &model.client, &model.model) {
            skipped.push(SkippedModel { client: model.client, model: model.model, reason });
            continue;
        }
        let free = judged(config, catalogue, &model.client, &model.model).is_some_and(|(_, probe)| probe.free);
        models.push(ListedModel { free, ..model });
    }
    skipped.sort_by(|left, right| left.client.cmp(&right.client).then_with(|| left.model.cmp(&right.model)));
    ModelListing { models, missing: listing.missing, skipped }
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
    if config.clients.get(client).and_then(|client| client.litellm_route.as_ref()).is_none() {
        return Ok(());
    }
    let reading = read_catalogue(runtime);
    match reading.catalogue.as_ref().and_then(|catalogue| refusal(config, catalogue, client, model)) {
        Some(reason) => Err(GatewayError::new("model-not-usable", format!("{client}:{model} cannot run: {reason}"))),
        None => Ok(()),
    }
}
