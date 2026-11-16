//! `check-routing`: fails when a delegation profile or a critique roster names a model the LiteLLM catalogue snapshot
//! calls blocked or failed. The snapshot (`registry/litellm-catalogue.json`) is what the last probe found, so a model
//! it marks `blocked`, `error` or `not-chat` is one a delegation or a critic would be refused or fail on, and the
//! routing or profile that names it has to change before the next run finds out. A model the snapshot does not know
//! passes: the catalogue has no word on it.
//!
//! What is read: the shipped `defaults.json` profiles (`<client>:<model>` keys of the `pi-<route>` clients) and the
//! shipped `routing.json` Pi clients that a stage slot or a fallback names. Pi names a model by its canonical group,
//! which the catalogue judges by its deployments (a group with one usable deployment is usable).

use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::routing::{parse_routing, CriticClient, Routing};
use model_gateway::json::{self, Object, Value};
use model_gateway::litellm_catalogue::{group_probe, not_usable_reason, parse_catalogue, LiteLlmCatalogue};
use crate::runtime::Runtime;

/// Why the catalogue refuses a canonical group on a Pi route, or `None` when the group is usable or unknown.
fn group_problem(catalogue: &LiteLlmCatalogue, model: &str) -> Option<String> {
    let (route, probe) = group_probe(catalogue, catalogue.groups.get(model)?)?;
    (probe.status != "usable" && probe.status != "unknown").then(|| not_usable_reason(&route, &probe))
}

/// Every `client:model` a profile of `defaults.json` allows that the catalogue refuses, as `profile NAME: ...`.
pub fn profile_problems(catalogue: &LiteLlmCatalogue, defaults: &Value) -> Vec<String> {
    let profiles = defaults.as_object().and_then(|object| object.get("profiles")).and_then(Value::as_object);
    let mut problems = Vec::new();
    for (name, profile) in profiles.map(Object::entries).unwrap_or_default() {
        let models = match profile.as_object().and_then(|object| object.get("models")) {
            Some(Value::Arr(models)) => models.as_slice(),
            _ => &[],
        };
        for key in models.iter().filter_map(Value::as_str) {
            let (client, model) = key.split_once(':').unwrap_or((key, key));
            let refusal = group_problem(catalogue, model).filter(|_| model_gateway::run::is_pi_client(client));
            problems.extend(refusal.map(|reason| format!("profile {name}: {key} cannot run: {reason}")));
        }
    }
    problems
}

/// Every Pi client a stage slot or a fallback of the routing names whose model the catalogue refuses, as
/// `routing client NAME: ...`. A client no roster names is not checked: it is never launched.
pub fn routing_problems(catalogue: &LiteLlmCatalogue, routing: &Routing) -> Vec<String> {
    let mut named: Vec<&str> = Vec::new();
    for (_, stage) in &routing.stages {
        for slot in stage.base.iter().chain(&stage.deep).chain(&stage.adr).chain(&stage.work) {
            named.push(&slot.client);
        }
    }
    for (client, fallbacks) in &routing.fallbacks {
        named.push(client);
        named.extend(fallbacks.iter().map(String::as_str));
    }
    let mut problems = Vec::new();
    for (name, client) in &routing.clients {
        let CriticClient::Pi(pi) = client else { continue };
        if !named.contains(&name.as_str()) {
            continue;
        }
        if let Some(reason) = group_problem(catalogue, &pi.model) {
            problems.push(format!("routing client {name}: {} on {} cannot run: {reason}", pi.model, pi.provider));
        }
    }
    problems
}

/// The check over the shipped profiles and routing, with the catalogue the input names.
pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let path = Fields::of(raw, "").and_then(|fields| fields.string("catalogue")).map_err(|error| error.into_error("invalid-input", "check-routing input"))?;
    let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
    let invalid = |message: String| CritiqueError::new("invalid-input", format!("check-routing input: {path}: {message}"));
    let catalogue = parse_catalogue(&json::parse(&text).map_err(&invalid)?).map_err(&invalid)?;
    check(&catalogue, model_gateway::SHIPPED_DEFAULTS, model_gateway::SHIPPED_ROUTING)
}

/// One shipped file read as JSON; text that is not JSON names the file.
fn shipped(name: &str, text: &str) -> Result<Value, CritiqueError> {
    match json::parse(text) {
        Ok(value) => Ok(value),
        Err(message) => Err(CritiqueError::unexpected(format!("{name}: {message}"))),
    }
}

/// The check of `defaults` and `routing` (the text of the two shipped files) against a catalogue.
pub fn check(catalogue: &LiteLlmCatalogue, defaults: &str, routing: &str) -> Result<Value, CritiqueError> {
    let defaults = shipped("defaults.json", defaults)?;
    let routing = parse_routing(&shipped("routing.json", routing)?).map_err(|error| error.into_error("invalid-routing", "routing.json"))?;
    let mut problems = profile_problems(catalogue, &defaults);
    problems.extend(routing_problems(catalogue, &routing));
    let mut result = Object::new();
    result.set("problems", Value::Arr(problems.iter().map(|problem| Value::str(problem)).collect()));
    result.set("status", Value::str(if problems.is_empty() { "passed" } else { "failed" }));
    Ok(Value::Obj(result))
}
