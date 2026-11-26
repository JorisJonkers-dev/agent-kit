//! Every review and critic route defaults to a Flash-class, small or free model.
//!
//! Owner rule (2026-11-26): routing goes with Flash generally, and reviews use Flash or
//! free models when possible. A full-size cheap model (GLM-5.3, Mistral Large, Kimi) is never
//! picked for a review automatically; it stays reachable only by naming it in a request. The two
//! paid frontier clients are the orchestrator and deep tier, not a cheap review slot, and are the
//! only listed exceptions.

use model_gateway::json::{parse, Value};
use std::collections::BTreeSet;

const ROUTING: &str = include_str!("../../../../../skills/workstation/critique/routing.json");
const DEFAULTS: &str = include_str!("../../../../../skills/workstation/model-gateway/defaults.json");
const TIERS: &str = include_str!("../../../../../registry/litellm-tiers.json");
const REGISTRY: &str = include_str!("../../../../../registry/estate-tooling.yaml");

/// A model id that is Flash-class, small or free.
const CHEAP: [&str; 6] = ["flash", "free", "-fast", "small", "mini", "lite"];

/// Routing clients that may name a model outside the cheap class: the paid frontier orchestrator
/// and deep tier (the `work` pool), never a cheap review slot.
const ROUTING_EXCEPTIONS: [(&str, &str); 2] = [("codex-work", "paid frontier orchestrator tier"), ("claude-work", "paid frontier orchestrator tier")];

/// Gateway `<client>:<model>` keys a review profile may list for the same reason.
const PROFILE_EXCEPTIONS: [(&str, &str); 3] = [
    ("claude:claude-sonnet-5-5", "paid frontier orchestrator or deep tier"),
    ("claude:claude-haiku-5-5", "Haiku is Anthropic's small tier"),
    ("codex:gpt-5.5", "paid frontier orchestrator or deep tier"),
];

const REVIEW_PROFILES: [&str; 4] = ["cross-critic", "critique", "red-team", "refute"];

fn cheap(model: &str) -> bool {
    CHEAP.iter().any(|word| model.to_lowercase().contains(word))
}

fn member<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("no {key}"))
}

fn text(value: &Value) -> &str {
    value.as_str().expect("a string")
}

/// Every client a stage slot, the triage and cold-read models and the fallbacks name.
fn collect(value: &Value, found: &mut BTreeSet<String>) {
    match value {
        Value::Obj(object) => {
            for (key, inner) in object.entries() {
                if key == "client" {
                    if let Some(name) = inner.as_str() {
                        found.insert(name.to_string());
                    }
                }
                collect(inner, found);
            }
        }
        Value::Arr(items) => items.iter().for_each(|inner| collect(inner, found)),
        _ => {}
    }
}

fn reviewer_clients(routing: &Value) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for part in ["stages", "triage", "cold_read"] {
        collect(member(routing, part), &mut found);
    }
    for (source, targets) in member(routing, "fallbacks").as_object().expect("fallbacks").entries() {
        found.insert(source.to_string());
        collect_names(targets, &mut found);
    }
    found
}

fn collect_names(value: &Value, found: &mut BTreeSet<String>) {
    if let Value::Arr(items) = value {
        items.iter().for_each(|item| {
            found.insert(text(item).to_string());
        });
    }
}

#[test]
fn every_reviewer_client_in_routing_is_flash_small_or_free() {
    let routing = parse(ROUTING).expect("routing");
    let clients = member(&routing, "clients");
    let mut offenders = Vec::new();
    for name in reviewer_clients(&routing) {
        let entry = member(clients, &name);
        let free = matches!(entry.as_object().and_then(|object| object.get("free")), Some(Value::Bool(true)));
        let model = text(member(entry, "model"));
        if !free && !cheap(model) && !ROUTING_EXCEPTIONS.iter().any(|(client, _)| *client == name) {
            offenders.push(format!("{name} -> {model}"));
        }
    }
    assert!(offenders.is_empty(), "review routes must name a Flash-class, small or free model: {offenders:?}");
}

#[test]
fn the_routing_exceptions_are_real_clients_that_are_not_cheap_with_a_reason() {
    let routing = parse(ROUTING).expect("routing");
    let clients = member(&routing, "clients");
    for (name, reason) in ROUTING_EXCEPTIONS {
        assert!(!reason.is_empty());
        assert!(!cheap(text(member(member(clients, name), "model"))), "{name} is cheap now: drop its exception");
    }
}

/// Clients that name a full-size model only so the Gold Band lint can read the family of a template
/// binding (`mistral-large-4-0`, `glm-5.3`) from this file; no slot, fallback, triage or cold-read
/// model may name them, so nothing picks them for a review.
const FAMILY_ANCHORS: [&str; 2] = ["pi-vibe-mistral-large", "pi-vibe-glm"];

#[test]
fn the_full_model_clients_are_family_anchors_that_no_review_route_names() {
    let routing = parse(ROUTING).expect("routing");
    let clients = member(&routing, "clients").as_object().expect("clients");
    let used = reviewer_clients(&routing);
    for name in ["vibe-glm", "vibe-mistral-large", "pi-opencode-go-glm", "pi-opencode-go-mistral-large"] {
        assert!(clients.get(name).is_none(), "{name} is the full model: use its flash or small client");
    }
    for name in FAMILY_ANCHORS {
        assert!(clients.get(name).is_some(), "{name} anchors a family for the Gold Band lint");
        assert!(!used.contains(name), "{name} is a full model and must stay out of every review route");
    }
}

/// `(name, model)` of each active `vibe_critic_agents` entry of the registry.
fn active_critic_agents() -> Vec<(String, String)> {
    let section = REGISTRY.split("\nvibe_critic_agents:\n").nth(1).expect("the critic agents");
    let section = section.split("\n# ----").next().expect("the section");
    let mut agents = Vec::new();
    for block in section.split("  - name: ").skip(1) {
        let name = block.lines().next().expect("a name").trim().to_string();
        let value = |key: &str| block.lines().find_map(|line| line.trim().strip_prefix(key)).map(|rest| rest.trim().to_string());
        if value("surfaces:").as_deref() != Some("[]") {
            agents.push((name, value("model:").expect("a model")));
        }
    }
    agents
}

#[test]
fn the_vibe_critic_agents_pin_flash_or_small_models_that_routing_names() {
    let agents = active_critic_agents();
    assert!(!agents.is_empty());
    for (name, model) in &agents {
        assert!(cheap(model), "{name} pins {model}");
    }
    let routing = parse(ROUTING).expect("routing");
    for (name, entry) in member(&routing, "clients").as_object().expect("clients").entries() {
        if text(member(entry, "kind")) == "vibe" {
            let agent = text(member(entry, "agent"));
            let pinned = agents.iter().find(|(known, _)| known == agent).unwrap_or_else(|| panic!("{name}: no active agent {agent}"));
            assert_eq!(pinned.1, text(member(entry, "model")), "{name}");
        }
    }
}

#[test]
fn the_review_profiles_of_the_gateway_pick_flash_small_or_free_models() {
    let defaults = parse(DEFAULTS).expect("defaults");
    let profiles = member(&defaults, "profiles");
    let mut listed = BTreeSet::new();
    for profile in REVIEW_PROFILES {
        let Value::Arr(models) = member(member(profiles, profile), "models") else { panic!("models") };
        for key in models.iter().map(text) {
            listed.insert(key.to_string());
            let exception = PROFILE_EXCEPTIONS.iter().any(|(known, _)| *known == key);
            let model = key.split_once(':').expect("client:model").1;
            assert!(exception || cheap(model), "{profile}: {key}");
        }
    }
    for (key, reason) in PROFILE_EXCEPTIONS {
        assert!(listed.contains(key) && !reason.is_empty(), "{key} is no longer listed: drop its exception");
    }
}

#[test]
fn the_review_tier_runs_only_flash_small_or_free_models() {
    let tiers = parse(TIERS).expect("tiers");
    let Value::Arr(steps) = member(member(member(&tiers, "tier_groups"), "tier-review"), "steps") else { panic!("steps") };
    for step in steps {
        let model = text(member(step, "model"));
        assert!(text(member(step, "cost")) == "free" || cheap(model), "{model}");
    }
}
