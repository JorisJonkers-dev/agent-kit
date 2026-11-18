use serde_json::json;

use super::*;

#[test]
fn the_family_comes_from_the_routing_file() {
    let routing = Routing::load(None);
    let family = |agent, model| routing.family(agent, model);
    assert_eq!(
        family("vibe", "mistral-large-4-0").as_deref(),
        Some("mistral")
    );
    assert_eq!(
        family("pi", "litellm-vibe/glm-5.3").as_deref(),
        Some("zhipu")
    );
    assert_eq!(family("vibe", "glm-5-3").as_deref(), Some("zhipu"));
    assert_eq!(
        family("pi", "litellm-vibe-study/deepseek-v4.1-flash").as_deref(),
        Some("deepseek")
    );
    assert_eq!(
        family("vibe-study", "deepseek-v4-1-flash-1017").as_deref(),
        Some("deepseek")
    );
    assert_eq!(
        family("agy", "gemini-3.8-flash-high").as_deref(),
        Some("google")
    );
    // No client names the model: the agent id names a client kind of one family.
    assert_eq!(
        family("claude-work", "sonnet").as_deref(),
        Some("anthropic")
    );
    assert_eq!(
        family("claude+memory-write", "opus").as_deref(),
        Some("anthropic")
    );
    assert_eq!(family("codex", "gpt-5.5").as_deref(), Some("openai"));
    // A kind of several families and an unknown kind have none.
    assert_eq!(family("pi", "mistral-medium-3.5"), None);
    assert_eq!(family("vibe", "stub-model"), None);
    assert_eq!(family("gate-x", "pinned-command"), None);
    assert_eq!(family("", ""), None);
    // A model name matches on a whole segment only.
    assert_eq!(family("pi", "mistral-largely"), None);
}

#[test]
fn a_merge_patch_changes_what_it_names_and_removes_what_is_null() {
    let mut target = json!({"a":{"b":1,"c":2},"d":[1,2],"e":1});
    merge_patch(&mut target, &json!({"a":{"b":null,"x":{"y":1}},"d":[3],"e":"s"}));
    assert_eq!(target, json!({"a":{"c":2,"x":{"y":1}},"d":[3],"e":"s"}));
    let mut scalar = json!(1);
    merge_patch(&mut scalar, &json!({"k":1}));
    assert_eq!(scalar, json!({"k":1}));
    merge_patch(&mut scalar, &json!(7));
    assert_eq!(scalar, json!(7));
}

#[test]
fn a_roster_lists_the_clients_of_a_stage_slots_and_skips_unknown_ones() {
    let routing = Routing::load(None);
    let base = routing.roster("spec", ["base", "deep"]);
    assert!(!base.is_empty());
    assert!(base.iter().all(|(client, family)| !client.is_empty() && !family.is_empty()));
    assert!(routing.roster("no-such-stage", ["base", "deep"]).is_empty());
    let patch = json!({"stages":{"x":{"base":[{"client":"nobody"}]}}});
    assert!(Routing::load(Some(&patch)).roster("x", ["base", "deep"]).is_empty());
}

#[test]
fn a_tier_binding_resolves_through_the_tier_profiles() {
    let tiers = tier::Tiers::load();
    assert_eq!(
        tiers.names(),
        ["tier-free", "tier-go-cheap", "tier-review", "tier-strong-open", "tier-vibe"]
    );
    let ok = tiers.named("pi", "litellm-free/tier-free").unwrap();
    assert_eq!(ok, tier::Named { tier: "tier-free".into(), problem: None });
    assert_eq!(tiers.named("pi", "litellm-tier-review/tier-review").unwrap().problem, None);
    let wrong = tiers.named("pi", "litellm-vibe/tier-free").unwrap().problem.unwrap();
    assert!(wrong.contains("through the provider litellm-free, not litellm-vibe"), "{wrong}");
    let bare = tiers.named("pi", "tier-free").unwrap().problem.unwrap();
    assert!(bare.ends_with("not none"), "{bare}");
    let unknown = tiers.named("pi", "litellm-free/tier-cheapest").unwrap().problem.unwrap();
    assert!(unknown.starts_with("tier tier-cheapest is not a LiteLLM tier (known: tier-free"), "{unknown}");
    // Not a tier binding: another agent, or a model that is no tier's name.
    assert_eq!(tiers.named("vibe", "tier-free"), None);
    assert_eq!(tiers.named("pi", "litellm-vibe/glm-5.3"), None);
}

#[test]
fn a_tier_without_a_profile_is_reported() {
    let mut tiers = tier::Tiers::load();
    tiers.providers.remove("tier-free");
    let problem = tiers.named("pi", "litellm-free/tier-free").unwrap().problem.unwrap();
    assert!(problem.contains("no profile in the model gateway's defaults"), "{problem}");
}

#[test]
fn a_chain_follows_the_tiers_it_falls_to() {
    let tiers = tier::Tiers::load();
    let vibe = tiers.chain("tier-vibe");
    assert_eq!(vibe.first().map(String::as_str), Some("vibe-tier/deepseek-v4-1-flash-1017"));
    assert!(vibe.iter().any(|m| m == "opencode-go-tier/mimo-v2.6-flash"), "{vibe:?}");
    assert!(tiers.chain("tier-nope").is_empty());
}

#[test]
fn a_step_is_placed_by_the_vendor_word_in_its_id() {
    assert_eq!(tier::Tiers::step_family("vibe-tier/zai-glm-5-3-flash"), Some("zhipu"));
    assert_eq!(tier::Tiers::step_family("free/gemma-4-31b"), Some("google"));
    assert_eq!(tier::Tiers::step_family("free/beam-free"), None);
    // Every step of every shipped tier but the free tier's `beam-free` has a family.
    let tiers = tier::Tiers::load();
    for name in tiers.names() {
        for step in tiers.chain(&name) {
            assert!(tier::Tiers::step_family(&step).is_some() || step == "free/beam-free" || step == "free/hy4-flash-free", "{step}");
        }
    }
}
