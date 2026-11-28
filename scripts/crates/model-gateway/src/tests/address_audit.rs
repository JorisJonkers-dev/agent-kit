//! The audit of the shipped names (#943): a delegation profile, a tier and a critique route name models through the
//! proxy's addresses (`free/<model>`, `paid/<model>`, `auto/<model>`, or a tier the proxy serves), never through a
//! provider's own model id on a provider's route. The entries that still do are listed here and the lists may only
//! shrink: a new provider-specific entry fails the audit, and so does a listed one that has been moved to an address
//! (delete it from the list then).

use crate::json::{parse, Value};
use std::collections::BTreeSet;

/// The clients that name their own model, which the proxy does not serve: not provider-specific routes.
const NATIVE_CLIENTS: [&str; 3] = ["claude", "codex", "agy"];

/// Profile models that still name a provider's model on a provider's Pi route (`pi-vibe`, `pi-opencode-go`).
const PROFILE_DEBT: [&str; 13] = [
    "pi-opencode-go:deepseek-v4.1-flash",
    "pi-opencode-go:glm-5.3-flash",
    "pi-opencode-go:kimi-k2.7-code",
    "pi-opencode-go:mimo-v2.6-flash",
    "pi-opencode-go:minimax-m3.1-flash",
    "pi-opencode-go:qwen3.8-flash",
    "pi-vibe:deepseek-v4.1-flash",
    "pi-vibe:glm-5.3",
    "pi-vibe:glm-5.3-flash",
    "pi-vibe:ministral-3b",
    "pi-vibe:ministral-8b",
    "pi-vibe:mistral-large-4",
    "pi-vibe:mistral-small",
];

/// Critique roles' clients that still name a provider's route or a provider's own model id.
const ROUTING_DEBT: [&str; 11] = [
    "pi-opencode-go-deepseek-flash",
    "pi-opencode-go-glm-flash",
    "pi-opencode-go-mistral-small",
    "pi-vibe-deepseek-flash",
    "pi-vibe-glm",
    "pi-vibe-glm-flash",
    "pi-vibe-mistral-large",
    "pi-vibe-mistral-small",
    "vibe-deepseek-flash",
    "vibe-glm-flash",
    "vibe-mistral-small",
];

fn addressed(model: &str) -> bool {
    ["free/", "paid/", "auto/", "tier-"].iter().any(|prefix| model.starts_with(prefix))
}

fn shown(found: BTreeSet<String>) -> Vec<String> {
    found.into_iter().collect()
}

#[test]
fn no_profile_or_tier_names_a_provider_specific_model_beyond_the_listed_debt() {
    let defaults = parse(crate::SHIPPED_DEFAULTS).expect("the shipped defaults");
    let profiles = defaults.as_object().and_then(|object| object.get("profiles")).and_then(Value::as_object).expect("the profiles");
    let mut found = BTreeSet::new();
    for (_, profile) in profiles.entries() {
        let Some(Value::Arr(models)) = profile.as_object().and_then(|object| object.get("models")) else { panic!("a profile lists its models") };
        for key in models.iter().filter_map(Value::as_str) {
            let (client, model) = key.split_once(':').expect("a client:model key");
            if !NATIVE_CLIENTS.contains(&client) && !addressed(model) {
                found.insert(key.to_string());
            }
        }
    }
    assert_eq!(shown(found), PROFILE_DEBT, "a profile names a provider-specific model: name it through free/, paid/ or auto/, or take it off the debt list");
}

#[test]
fn no_critique_route_names_a_provider_specific_model_beyond_the_listed_debt() {
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
        if provider_route {
            found.insert(name.to_string());
        }
        // A Pi route on the free chain is reached through the free provider only.
        if kind == "pi" && model.starts_with("free/") {
            assert_eq!(provider, Some("litellm-free"), "{name}");
        }
    }
    assert_eq!(shown(found), ROUTING_DEBT, "a critique route names a provider-specific model: name it through free/, paid/ or auto/, or take it off the debt list");
}
