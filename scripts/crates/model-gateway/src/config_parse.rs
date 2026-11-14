//! Reads and merges the gateway's configuration from JSON.
//!
//! agent-kit ships a defaults file with all of it; this machine's override file
//! carries the same top-level keys, every one optional, and is merged over the
//! defaults field by field. Both are validated here, so a broken file is one
//! error that names what is wrong rather than a surprise later.

use crate::config::{
    ClientConfig, CostClass, GatewayConfig, MeteredPersonalCaps, ModelEntry, ModelPrices, ModelSource, PlanLimit, PlanWindow, Settings, Tier, CONFIG_VERSION,
};
use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ports::Runtime;
use std::collections::BTreeMap;

/// The fields a client has; a machine file that names another is refused, not quietly ignored.
const CLIENT_FIELDS: [&str; 8] =
    ["binary", "cost_class", "model_source", "families", "provider_cost_classes", "providers", "plan_limits", "litellm_route"];

/// The top-level keys a machine file may name; one it does not is refused, not quietly dropped.
const OVERRIDE_KEYS: [&str; 5] = ["version", "clients", "disabled", "models", "settings"];

/// Reads parsed JSON as the shipped defaults, or says what is wrong with it.
pub fn parse_defaults(raw: &Value) -> Result<GatewayConfig, GatewayError> {
    let Some(object) = raw.as_object() else {
        return Err(invalid("the defaults must be a JSON object"));
    };
    if object.get("version") != Some(&Value::Num(CONFIG_VERSION)) {
        return Err(invalid(format!("version must be {}", json::number_text(CONFIG_VERSION))));
    }
    let Some(clients_raw) = object.get("clients").and_then(Value::as_object) else {
        return Err(invalid("clients must be an object keyed by client id"));
    };
    let mut clients = BTreeMap::new();
    for (id, value) in clients_raw.entries() {
        let Some(client) = value.as_object() else {
            return Err(invalid(format!("clients.{id} must be an object")));
        };
        clients.insert(id.to_string(), parse_client(id, client, None)?);
    }
    let Some(Value::Arr(models_raw)) = object.get("models") else {
        return Err(invalid("models must be a list"));
    };
    let mut models = Vec::new();
    for (index, value) in models_raw.iter().enumerate() {
        models.push(parse_model_entry(value, index, None)?);
    }
    let Some(settings_raw) = object.get("settings").and_then(Value::as_object) else {
        return Err(invalid("settings must be an object"));
    };
    let disabled = parse_disabled(object.get("disabled"))?;
    let settings = parse_settings(settings_raw)?;
    Ok(GatewayConfig { version: CONFIG_VERSION, clients, models, settings, disabled })
}

/// This machine's override merged over the defaults: a client field replaces
/// that field, a model entry matching `client` and `model` (or one of its
/// `names`) replaces its tier, notes or family, a new model entry adds a
/// declared model, a setting replaces that setting (deeply for the caps), and a
/// `disabled` list replaces the list.
pub fn apply_override(config: &GatewayConfig, raw: &Value) -> Result<GatewayConfig, GatewayError> {
    let Some(object) = raw.as_object() else {
        return Err(invalid("the override must be a JSON object"));
    };
    refuse_unknown_fields(object, "the override", &OVERRIDE_KEYS, "a key the override has")?;
    if let Some(version) = object.get("version") {
        if *version != Value::Num(CONFIG_VERSION) {
            return Err(invalid(format!("version must be {}", json::number_text(CONFIG_VERSION))));
        }
    }
    let clients = merge_clients(&config.clients, object.get("clients"))?;
    let disabled = match object.get("disabled") {
        None => config.disabled.clone(),
        Some(value) => parse_disabled(Some(value))?,
    };
    let models = merge_models(&config.models, object.get("models"))?;
    let settings = merge_settings(&config.settings, object.get("settings"))?;
    Ok(GatewayConfig { version: CONFIG_VERSION, clients, models, settings, disabled })
}

/// The shipped defaults with this machine's override applied over them. A
/// missing override is no override; an unreadable or invalid one is an error
/// that names the file.
pub fn load_config(runtime: &Runtime) -> Result<GatewayConfig, GatewayError> {
    let defaults = read_defaults(runtime)?;
    let path = crate::locations::override_path(&runtime.env)?;
    match runtime.files.exists(&path) {
        Ok(false) => return Ok(defaults),
        Ok(true) => {}
        Err(error) => return Err(invalid(format!("{path}: cannot read the override: {}", error.message))),
    }
    let text = match runtime.files.read_text(&path) {
        Ok(text) => text,
        Err(error) => return Err(invalid(format!("{path}: cannot read the override: {}", error.message))),
    };
    let raw = json::parse(&text).map_err(|message| invalid(format!("{path}: not JSON: {message}")))?;
    apply_override(&defaults, &raw).map_err(|error| invalid(format!("{path}: {}", error.message)))
}

/// The shipped defaults, which the runtime carries as text rather than as a file.
fn read_defaults(runtime: &Runtime) -> Result<GatewayConfig, GatewayError> {
    let path = &runtime.defaults_path;
    let raw = json::parse(&runtime.defaults).map_err(|message| invalid(format!("{path}: not JSON: {message}")))?;
    parse_defaults(&raw).map_err(|error| invalid(format!("{path}: {}", error.message)))
}

fn merge_clients(base: &BTreeMap<String, ClientConfig>, raw: Option<&Value>) -> Result<BTreeMap<String, ClientConfig>, GatewayError> {
    let Some(raw) = raw else {
        return Ok(base.clone());
    };
    let Some(object) = raw.as_object() else {
        return Err(invalid("clients must be an object keyed by client id"));
    };
    let mut merged = base.clone();
    for (id, value) in object.entries() {
        let Some(client) = value.as_object() else {
            return Err(invalid(format!("clients.{id} must be an object")));
        };
        let parsed = parse_client(id, client, merged.get(id))?;
        merged.insert(id.to_string(), parsed);
    }
    Ok(merged)
}

/// One client, every field it does not name taken from `base`; a new client
/// must name all three. `provider_cost_classes` merges key by key, so an
/// override that names one provider keeps the cost class of the others;
/// `plan_limits` replaces the list, so an override that names one window
/// replaces the client's whole plan.
fn parse_client(id: &str, raw: &Object, base: Option<&ClientConfig>) -> Result<ClientConfig, GatewayError> {
    let where_ = format!("clients.{id}");
    refuse_unknown_fields(raw, &where_, &CLIENT_FIELDS, "a field a client has")?;
    let binary = match raw.get("binary") {
        None => base.map(|client| client.binary.clone()),
        Some(value) => Some(require_string(Some(value), &format!("{where_}.binary"))?),
    };
    let cost_class = match raw.get("cost_class") {
        None => base.map(|client| client.cost_class),
        Some(value) => Some(cost_class(value, &format!("{where_}.cost_class"))?),
    };
    let model_source = match raw.get("model_source") {
        None => base.map(|client| client.model_source),
        Some(value) => Some(model_source(value, &format!("{where_}.model_source"))?),
    };
    let families = match raw.get("families") {
        None => base.and_then(|client| client.families.clone()),
        Some(value) => Some(require_strings(Some(value), &format!("{where_}.families"))?),
    };
    let provider_cost_classes = match raw.get("provider_cost_classes") {
        None => base.and_then(|client| client.provider_cost_classes.clone()),
        Some(value) => {
            let mut merged = base.and_then(|client| client.provider_cost_classes.clone()).unwrap_or_default();
            for (provider, class) in parse_provider_cost_classes(value, &format!("{where_}.provider_cost_classes"))? {
                merged.insert(provider, class);
            }
            Some(merged)
        }
    };
    let providers = match raw.get("providers") {
        None => base.and_then(|client| client.providers.clone()),
        Some(value) => Some(require_strings(Some(value), &format!("{where_}.providers"))?),
    };
    let plan_limits = match raw.get("plan_limits") {
        None => base.and_then(|client| client.plan_limits.clone()),
        Some(value) => Some(parse_plan_limits(value, &format!("{where_}.plan_limits"))?),
    };
    let litellm_route = match raw.get("litellm_route") {
        None => base.and_then(|client| client.litellm_route.clone()),
        Some(value) => Some(require_string(Some(value), &format!("{where_}.litellm_route"))?),
    };
    let (Some(binary), Some(cost_class), Some(model_source)) = (binary, cost_class, model_source) else {
        return Err(invalid(format!("{where_} is new, so it needs binary, cost_class and model_source")));
    };
    Ok(ClientConfig { binary, cost_class, model_source, families, provider_cost_classes, providers, plan_limits, litellm_route })
}

/// The windows of a client's plan, each naming its span and either its dollars or its runs.
fn parse_plan_limits(raw: &Value, where_: &str) -> Result<Vec<PlanLimit>, GatewayError> {
    let Value::Arr(items) = raw else {
        return Err(invalid(format!("{where_} must be a list of windows")));
    };
    let mut limits = Vec::new();
    for (index, value) in items.iter().enumerate() {
        let at = format!("{where_}[{index}]");
        let Some(object) = value.as_object() else {
            return Err(invalid(format!("{at} must be an object")));
        };
        let window = plan_window(object.get("window"), &format!("{at}.window"))?;
        let dollars = object.get("limit_usd");
        let runs = object.get("limit_runs");
        if dollars.is_none() == runs.is_none() {
            return Err(invalid(format!("{at} must name exactly one of limit_usd and limit_runs")));
        }
        if let Some(dollars) = dollars {
            let limit = require_number(Some(dollars), &format!("{at}.limit_usd"))?;
            if limit <= 0.0 {
                return Err(invalid(format!("{at}.limit_usd must be above 0")));
            }
            limits.push(PlanLimit::Usd { window, limit_usd: limit });
            continue;
        }
        let limit = require_number(runs, &format!("{at}.limit_runs"))?;
        if !is_integer(limit) || limit <= 0.0 {
            return Err(invalid(format!("{at}.limit_runs must be a whole number above 0")));
        }
        limits.push(PlanLimit::Runs { window, limit_runs: limit });
    }
    Ok(limits)
}

/// Refuses a key a file may not name, so a machine file that misspells one learns it, rather than losing it.
fn refuse_unknown_fields(raw: &Object, where_: &str, allowed: &[&str], what: &str) -> Result<(), GatewayError> {
    match raw.entries().into_iter().find(|(key, _)| !allowed.contains(key)) {
        None => Ok(()),
        Some((key, _)) => Err(invalid(format!("{where_}.{key} is not {what}; choose from {}", allowed.join(", ")))),
    }
}

/// The cost class of each provider a client's models run on, by provider name.
fn parse_provider_cost_classes(raw: &Value, where_: &str) -> Result<Vec<(String, CostClass)>, GatewayError> {
    let Some(object) = raw.as_object() else {
        return Err(invalid(format!("{where_} must be an object keyed by provider name")));
    };
    let mut classes = Vec::new();
    for (provider, value) in object.entries() {
        classes.push((provider.to_string(), cost_class(value, &format!("{where_}.{provider}"))?));
    }
    Ok(classes)
}

fn merge_models(base: &[ModelEntry], raw: Option<&Value>) -> Result<Vec<ModelEntry>, GatewayError> {
    let Some(raw) = raw else {
        return Ok(base.to_vec());
    };
    let Value::Arr(items) = raw else {
        return Err(invalid("models must be a list"));
    };
    let mut merged = base.to_vec();
    for (index, value) in items.iter().enumerate() {
        let Some(object) = value.as_object() else {
            return Err(invalid(format!("models[{index}] must be an object")));
        };
        let client = require_string(object.get("client"), &format!("models[{index}].client"))?;
        let model = require_string(object.get("model"), &format!("models[{index}].model"))?;
        let at = merged
            .iter()
            .position(|entry| entry.client == client && (entry.model == model || entry.names.as_ref().is_some_and(|names| names.contains(&model))));
        let entry = match at {
            None => parse_model_entry(value, index, None)?,
            Some(at) => {
                let target = &merged[at];
                let mut replaced = object.clone();
                replaced.set("model", Value::str(&target.model));
                parse_model_entry(&Value::Obj(replaced), index, Some(target))?
            }
        };
        match at {
            None => merged.push(entry),
            Some(at) => merged[at] = entry,
        }
    }
    Ok(merged)
}

/// One model entry, every field it does not name taken from `base`; a new entry must name all five.
fn parse_model_entry(raw: &Value, index: usize, base: Option<&ModelEntry>) -> Result<ModelEntry, GatewayError> {
    let where_ = format!("models[{index}]");
    let Some(object) = raw.as_object() else {
        return Err(invalid(format!("{where_} must be an object")));
    };
    let client = require_string(object.get("client"), &format!("{where_}.client"))?;
    let model = require_string(object.get("model"), &format!("{where_}.model"))?;
    let family = match object.get("family") {
        None => base.map(|entry| entry.family.clone()),
        Some(value) => Some(require_string(Some(value), &format!("{where_}.family"))?),
    };
    let tier = match object.get("tier") {
        None => base.map(|entry| entry.tier),
        Some(value) => Some(tier(value, &format!("{where_}.tier"))?),
    };
    let notes = match object.get("notes") {
        None => base.map(|entry| entry.notes.clone()),
        Some(value) => Some(require_string(Some(value), &format!("{where_}.notes"))?),
    };
    let prices = match object.get("prices") {
        None => base.and_then(|entry| entry.prices.clone()),
        Some(value) => Some(parse_model_prices(value, &format!("{where_}.prices"))?),
    };
    let (Some(family), Some(tier), Some(notes)) = (family, tier, notes) else {
        return Err(invalid(format!("{where_} ({client}:{model}) is new, so it needs family, tier and notes")));
    };
    let names = match object.get("names") {
        None => base.and_then(|entry| entry.names.clone()),
        Some(value) => Some(require_strings(Some(value), &format!("{where_}.names"))?),
    };
    Ok(ModelEntry { client, model, names, family, tier, notes, prices })
}

/// The catalogue's copy of a model's prices: input and output above 0, cached input at least 0.
fn parse_model_prices(raw: &Value, where_: &str) -> Result<ModelPrices, GatewayError> {
    let Some(object) = raw.as_object() else {
        return Err(invalid(format!("{where_} must be an object")));
    };
    let input = require_number(object.get("input"), &format!("{where_}.input"))?;
    let output = require_number(object.get("output"), &format!("{where_}.output"))?;
    let cached_input = require_number(object.get("cached_input"), &format!("{where_}.cached_input"))?;
    if input <= 0.0 || output <= 0.0 {
        return Err(invalid(format!("{where_}.input and {where_}.output must be above 0")));
    }
    if cached_input < 0.0 {
        return Err(invalid(format!("{where_}.cached_input must be at least 0")));
    }
    Ok(ModelPrices { input, cached_input, output })
}

fn merge_settings(base: &Settings, raw: Option<&Value>) -> Result<Settings, GatewayError> {
    let Some(raw) = raw else {
        return Ok(base.clone());
    };
    let Some(object) = raw.as_object() else {
        return Err(invalid("settings must be an object"));
    };
    let availability_window_minutes = number_field(object, "availability_window_minutes", base.availability_window_minutes, "settings.")?;
    let max_batch_delegations = count_field(object, "max_batch_delegations", base.max_batch_delegations)?;
    let max_parallel_per_client = count_field(object, "max_parallel_per_client", base.max_parallel_per_client)?;
    let metered_personal_caps = merge_caps(&base.metered_personal_caps, object.get("metered_personal_caps"))?;
    let output_allowance_tokens = merge_allowances(&base.output_allowance_tokens, object.get("output_allowance_tokens"))?;
    let plan_warning_ratio = ratio_field(object, "plan_warning_ratio", base.plan_warning_ratio)?;
    let summary_max_chars = number_field(object, "summary_max_chars", base.summary_max_chars, "settings.")?;
    Ok(Settings {
        plan_warning_ratio,
        summary_max_chars,
        max_parallel_per_client,
        max_batch_delegations,
        availability_window_minutes,
        metered_personal_caps,
        output_allowance_tokens,
    })
}

fn parse_settings(raw: &Object) -> Result<Settings, GatewayError> {
    let Some(caps) = raw.get("metered_personal_caps").and_then(Value::as_object) else {
        return Err(invalid("settings.metered_personal_caps must be an object"));
    };
    let Some(allowances) = raw.get("output_allowance_tokens").and_then(Value::as_object) else {
        return Err(invalid("settings.output_allowance_tokens must be an object keyed by tier"));
    };
    let availability_window_minutes = require_number(raw.get("availability_window_minutes"), "settings.availability_window_minutes")?;
    let max_batch_delegations = require_count(raw.get("max_batch_delegations"), "settings.max_batch_delegations")?;
    let max_parallel_per_client = require_count(raw.get("max_parallel_per_client"), "settings.max_parallel_per_client")?;
    let per_day_usd = require_number(caps.get("per_day_usd"), "settings.metered_personal_caps.per_day_usd")?;
    let per_workflow_usd = require_number(caps.get("per_workflow_usd"), "settings.metered_personal_caps.per_workflow_usd")?;
    let mut output_allowance_tokens = [0.0; 4];
    for (index, tier) in Tier::ALL.into_iter().enumerate().rev() {
        output_allowance_tokens[index] = require_allowance(allowances, tier)?;
    }
    let plan_warning_ratio = require_ratio(raw.get("plan_warning_ratio"), "settings.plan_warning_ratio")?;
    let summary_max_chars = require_number(raw.get("summary_max_chars"), "settings.summary_max_chars")?;
    Ok(Settings {
        plan_warning_ratio,
        summary_max_chars,
        max_parallel_per_client,
        max_batch_delegations,
        availability_window_minutes,
        metered_personal_caps: MeteredPersonalCaps { per_workflow_usd, per_day_usd },
        output_allowance_tokens,
    })
}

/// The output tokens one tier's run is allowed, which a machine file may override tier by tier.
fn require_allowance(raw: &Object, tier: Tier) -> Result<f64, GatewayError> {
    let where_ = format!("settings.output_allowance_tokens.{}", tier.as_str());
    let value = require_number(raw.get(tier.as_str()), &where_)?;
    if !is_integer(value) || value <= 0.0 {
        return Err(invalid(format!("{where_} must be a whole number of tokens above 0")));
    }
    Ok(value)
}

fn merge_allowances(base: &[f64; 4], raw: Option<&Value>) -> Result<[f64; 4], GatewayError> {
    let Some(raw) = raw else {
        return Ok(*base);
    };
    let Some(object) = raw.as_object() else {
        return Err(invalid("settings.output_allowance_tokens must be an object keyed by tier"));
    };
    let mut allowances = *base;
    for (index, tier) in Tier::ALL.into_iter().enumerate().rev() {
        if object.get(tier.as_str()).is_some() {
            allowances[index] = require_allowance(object, tier)?;
        }
    }
    Ok(allowances)
}

fn merge_caps(base: &MeteredPersonalCaps, raw: Option<&Value>) -> Result<MeteredPersonalCaps, GatewayError> {
    let Some(raw) = raw else {
        return Ok(base.clone());
    };
    let Some(object) = raw.as_object() else {
        return Err(invalid("settings.metered_personal_caps must be an object"));
    };
    let per_day_usd = number_field(object, "per_day_usd", base.per_day_usd, "settings.metered_personal_caps.")?;
    let per_workflow_usd = number_field(object, "per_workflow_usd", base.per_workflow_usd, "settings.metered_personal_caps.")?;
    Ok(MeteredPersonalCaps { per_workflow_usd, per_day_usd })
}

fn number_field(raw: &Object, name: &str, base: f64, prefix: &str) -> Result<f64, GatewayError> {
    match raw.get(name) {
        None => Ok(base),
        Some(value) => require_number(Some(value), &format!("{prefix}{name}")),
    }
}

/// A setting that counts things: a whole number of at least 1.
fn count_field(raw: &Object, name: &str, base: f64) -> Result<f64, GatewayError> {
    match raw.get(name) {
        None => Ok(base),
        Some(value) => require_count(Some(value), &format!("settings.{name}")),
    }
}

/// A setting that is a share of a whole: a ratio from 0 to 1.
fn ratio_field(raw: &Object, name: &str, base: f64) -> Result<f64, GatewayError> {
    match raw.get(name) {
        None => Ok(base),
        Some(value) => require_ratio(Some(value), &format!("settings.{name}")),
    }
}

fn require_count(value: Option<&Value>, where_: &str) -> Result<f64, GatewayError> {
    let count = require_number(value, where_)?;
    if !is_integer(count) || count < 1.0 {
        return Err(invalid(format!("{where_} must be a whole number of at least 1")));
    }
    Ok(count)
}

fn require_ratio(value: Option<&Value>, where_: &str) -> Result<f64, GatewayError> {
    let ratio = require_number(value, where_)?;
    if ratio < 0.0 || ratio > 1.0 {
        return Err(invalid(format!("{where_} must be between 0 and 1")));
    }
    Ok(ratio)
}

fn parse_disabled(raw: Option<&Value>) -> Result<Vec<String>, GatewayError> {
    match raw {
        None => Ok(Vec::new()),
        Some(value) => require_strings(Some(value), "disabled"),
    }
}

fn require_strings(value: Option<&Value>, where_: &str) -> Result<Vec<String>, GatewayError> {
    let Some(Value::Arr(items)) = value else {
        return Err(invalid(format!("{where_} must be a list of non-empty strings")));
    };
    let mut strings = Vec::new();
    for item in items {
        match item {
            Value::Str(text) if !text.is_empty() => strings.push(text.clone()),
            _ => return Err(invalid(format!("{where_} must be a list of non-empty strings"))),
        }
    }
    Ok(strings)
}

fn require_string(value: Option<&Value>, where_: &str) -> Result<String, GatewayError> {
    match value {
        Some(Value::Str(text)) if !text.is_empty() => Ok(text.clone()),
        _ => Err(invalid(format!("{where_} must be a non-empty string"))),
    }
}

fn require_number(value: Option<&Value>, where_: &str) -> Result<f64, GatewayError> {
    match value {
        Some(Value::Num(number)) if number.is_finite() => Ok(*number),
        _ => Err(invalid(format!("{where_} must be a number"))),
    }
}

/// The index of `value` in `allowed`, refused with the list when it is not one of them.
fn require_one_of(value: Option<&Value>, allowed: &[&str], where_: &str) -> Result<usize, GatewayError> {
    match value {
        Some(Value::Str(text)) => match allowed.iter().position(|candidate| candidate == text) {
            Some(index) => Ok(index),
            None => Err(invalid(format!("{where_} must be one of {}", allowed.join(", ")))),
        },
        _ => Err(invalid(format!("{where_} must be one of {}", allowed.join(", ")))),
    }
}

fn cost_class(value: &Value, where_: &str) -> Result<CostClass, GatewayError> {
    let index = require_one_of(Some(value), &CostClass::ALL.map(CostClass::as_str), where_)?;
    Ok(CostClass::ALL[index])
}

fn model_source(value: &Value, where_: &str) -> Result<ModelSource, GatewayError> {
    let index = require_one_of(Some(value), &ModelSource::ALL.map(ModelSource::as_str), where_)?;
    Ok(ModelSource::ALL[index])
}

fn tier(value: &Value, where_: &str) -> Result<Tier, GatewayError> {
    let index = require_one_of(Some(value), &Tier::ALL.map(Tier::as_str), where_)?;
    Ok(Tier::ALL[index])
}

fn plan_window(value: Option<&Value>, where_: &str) -> Result<PlanWindow, GatewayError> {
    let index = require_one_of(value, &PlanWindow::ALL.map(PlanWindow::as_str), where_)?;
    Ok(PlanWindow::ALL[index])
}

/// `Number.isInteger`.
fn is_integer(number: f64) -> bool {
    number.fract() == 0.0 && number.is_finite()
}

fn invalid(message: impl Into<String>) -> GatewayError {
    GatewayError::new("invalid-config", message)
}
