//! The audit of the shipped names (#943): a delegation profile, a tier and a critique route name models through the
//! proxy's addresses (`free/<model>`, `paid/<model>`, `auto/<model>`, or a tier the proxy serves), never through a
//! provider's own model id on a provider's route. Nothing is listed as debt any more: a new provider-specific entry
//! fails the audit. The one exception is the Vibe CLI clients of the triage question and the cold read.

use crate::json::{parse, Value};
use std::collections::BTreeSet;

/// The clients that name their own model, which the proxy does not serve: not provider-specific routes.
const NATIVE_CLIENTS: [&str; 3] = ["claude", "codex", "agy"];

/// The critique clients that still name a provider's own model id: the Vibe CLI harness that runs the triage
/// question and the cold read (`triage.model.client` and `cold_read.client` must be a Vibe client), whose plan
/// models have no proxy address. No critic of a stage's roster is a Vibe client.
const VIBE_HARNESS_CLIENTS: [&str; 3] = ["vibe-deepseek-flash", "vibe-glm-flash", "vibe-mistral-small"];

fn addressed(model: &str) -> bool {
    ["free/", "paid/", "auto/", "tier-"].iter().any(|prefix| model.starts_with(prefix))
}

fn shown(found: BTreeSet<String>) -> Vec<String> {
    found.into_iter().collect()
}

#[test]
fn no_profile_or_tier_names_a_provider_specific_model() {
    let defaults = parse(crate::SHIPPED_DEFAULTS).expect("the shipped defaults");
    let profiles = defaults.as_object().and_then(|object| object.get("profiles")).and_then(Value::as_object).expect("the profiles");
    let mut found = BTreeSet::new();
    for (_, profile) in profiles.entries() {
        let Some(Value::Arr(models)) = profile.as_object().and_then(|object| object.get("models")) else { panic!("a profile lists its models") };
        for key in models.iter().filter_map(Value::as_str) {
            let (client, model) = key.split_once(':').expect("a client:model key");
            if !NATIVE_CLIENTS.contains(&client) && (!addressed(model) || ["pi-vibe", "pi-vibe-study", "pi-opencode-go"].contains(&client)) {
                found.insert(key.to_string());
            }
        }
    }
    assert_eq!(shown(found), Vec::<String>::new(), "a profile names a provider-specific model: name it through free/, paid/ or auto/");
}

#[test]
fn no_critique_route_names_a_provider_specific_model() {
    let routing = parse(crate::SHIPPED_ROUTING).expect("the shipped routing");
    let clients = routing.as_object().and_then(|object| object.get("clients")).and_then(Value::as_object).expect("the clients");
    let mut found = BTreeSet::new();
    for (name, client) in clients.entries() {
        let object = client.as_object().expect("a client");
        let kind = object.get("kind").and_then(Value::as_str).unwrap_or_default();
        let provider = object.get("provider").and_then(Value::as_str);
        let model = object.get("model").and_then(Value::as_str).unwrap_or_default();
        let provider_route = match kind {
            "pi" => !addressed(model),
            "vibe" => true,
            _ => false,
        };
        if provider_route && !VIBE_HARNESS_CLIENTS.contains(&name) {
            found.insert(name.to_string());
        }
        // A Pi route on the free chain is reached through the free provider only.
        for pricing in ["free", "paid", "auto"] {
            if kind == "pi" && model.starts_with(&format!("{pricing}/")) {
                assert_eq!(provider, Some(format!("litellm-{pricing}").as_str()), "{name}");
            }
        }
    }
    assert_eq!(shown(found), Vec::<String>::new(), "a critique route names a provider-specific model: name it through free/, paid/ or auto/");
}
