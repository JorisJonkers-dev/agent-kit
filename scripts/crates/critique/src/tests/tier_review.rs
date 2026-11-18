//! The tier-review roster, the chain it is read from and the routing record of a reviewer.

use crate::fields::Fields;
use crate::routing::RosterSlot;
use crate::tier_review::{family_of, ladder, resolve_roster, review_ladder, Request, Step};
use crate::tier_routing::{TierRouting, COLLAPSED, UNSEEN};
use model_gateway::json::{self, parse, Object};
use model_gateway::routing::{Capture, Routing};

fn step(model: &str, cost: &str) -> Step {
    Step { model: model.to_string(), cost: cost.to_string() }
}

fn slots() -> Vec<RosterSlot> {
    ["a", "b"].iter().map(|role| RosterSlot { client: "c".to_string(), role: role.to_string(), lenses: vec![format!("lens-{role}")], max_price_usd: 1.0, time_limit_s: 60.0 }).collect()
}

fn strings(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn request<'a>(allowed: &'a [String], cooled: &'a [String], public: bool) -> Request<'a> {
    Request { allowed_families: allowed, writer_family: "anthropic", public_repository: public, cooled, want: 3 }
}

#[test]
fn the_review_chain_is_the_snapshot_in_ladder_order_from_free_to_vibe_to_go() {
    let steps = review_ladder().expect("the chain");
    let costs: Vec<&str> = steps.iter().map(|step| step.cost.as_str()).collect();
    assert_eq!(steps[0].model, "free/nemotron-3-ultra-550b-a55b");
    let rank = |cost: &str| ["free", "vibe", "go"].iter().position(|known| *known == cost).expect("a known cost");
    assert!(costs.windows(2).all(|pair| rank(pair[0]) <= rank(pair[1])), "{costs:?}");
    // The strong tier's models never come into review: they live only in tier-strong-open.
    assert!(steps.iter().all(|step| family_of(&step.model).is_some() && !step.model.contains("kimi") && !step.model.contains("luna")), "{steps:?}");
}

#[test]
fn a_tier_that_falls_through_to_another_is_read_with_that_tier_in_place() {
    let snapshot = r#"{"tier_groups":{"a":{"steps":[{"model":"x/glm-1","cost":"free"},{"tier":"b"}]},"b":{"steps":[{"model":"y/qwen-1"}]}}}"#;
    assert_eq!(ladder(snapshot, "a").expect("a chain"), [step("x/glm-1", "free"), step("y/qwen-1", "")]);
}

#[test]
fn a_snapshot_that_cannot_be_read_as_a_chain_is_an_error_naming_why() {
    let error = |snapshot: &str, tier: &str| ladder(snapshot, tier).expect_err("an error").message;
    assert!(error("{", "a").contains("not JSON"));
    assert!(error("{}", "a").contains("no tier_groups"));
    assert!(error(r#"{"tier_groups":{}}"#, "a").contains("no tier a with steps"));
    assert!(error(r#"{"tier_groups":{"a":{"steps":[{"cost":"free"}]}}}"#, "a").contains("names neither a model nor a tier"));
    assert!(error(r#"{"tier_groups":{"a":{"steps":[{"tier":"a"}]}}}"#, "a").contains("nests tiers more than 4 deep"));
}

#[test]
fn a_step_model_has_the_family_of_the_name_it_starts_with_once_its_group_is_cut_off() {
    for (model, family) in [("free/nemotron-3-ultra-550b-a55b", "nvidia"), ("vibe-tier/zai-glm-5-3-flash", "zhipu"), ("free/gpt-oss-120b", "openai"), ("opencode-go-tier/qwen3.8-flash", "alibaba")] {
        assert_eq!(family_of(model), Some(family), "{model}");
    }
    assert_eq!(family_of("deepseek-v4"), Some("deepseek"));
    assert_eq!(family_of("free/unknown-model"), None);
}

#[test]
fn one_step_of_each_of_three_families_is_picked_in_ladder_order_with_the_slots_in_turn() {
    let steps = [step("free/nemotron-x", "free"), step("free/glm-4", "free"), step("free/glm-5", "free"), step("free/gemma-x", "free"), step("free/gpt-oss-x", "free")];
    let allowed = strings(&["nvidia", "zhipu", "google", "openai"]);
    let (roster, unplaced) = resolve_roster(&steps, &slots(), &request(&allowed, &[], true));
    assert!(unplaced.is_empty() && roster.distinct_families && roster.depth == "base");
    let picked: Vec<(&str, &str, &str)> = roster.critics.iter().map(|critic| (critic.critic_id.as_str(), critic.client_config.family(), critic.role.as_str())).collect();
    assert_eq!(picked, [("c1-free-nemotron-x", "nvidia", "a"), ("c2-free-glm-4", "zhipu", "b"), ("c3-free-gemma-x", "google", "a")]);
    // The second GLM is the same family as a pick, and the OSS model is of a family not yet in the review: only it is a fallback.
    assert_eq!(roster.dropped.iter().map(|dropped| (dropped.client.as_str(), dropped.reason.as_str())).collect::<Vec<_>>(), [("free-glm-5", "same-family")]);
    assert_eq!(roster.critics[0].fallbacks.iter().map(|fallback| fallback.client.as_str()).collect::<Vec<_>>(), ["free-gpt-oss-x"]);
    assert_eq!(roster.critics[0].fallbacks, roster.critics[2].fallbacks);
}

#[test]
fn a_step_is_passed_over_for_its_scope_its_family_the_writer_or_a_standing_refusal() {
    let steps = [
        step("free/nemotron-x", "free"),
        step("vibe/gemma-x", "vibe"),
        step("vibe/deepseek-x", "vibe"),
        step("go/qwen-x", "go"),
        step("go/glm-x", "go"),
        step("go/mimo-x", "go"),
        step("go/mystery-x", "go"),
        step("go/gpt-oss-x", "go"),
    ];
    let allowed = strings(&["nvidia", "google", "deepseek", "alibaba", "zhipu", "xiaomi", "openai", "anthropic"]);
    let cooled = strings(&["go/glm-x"]);
    let (roster, unplaced) = resolve_roster(&steps, &slots(), &request(&allowed, &cooled, false));
    assert_eq!(unplaced, ["go/mystery-x"]);
    let reasons: Vec<(&str, &str)> = roster.dropped.iter().map(|dropped| (dropped.client.as_str(), dropped.reason.as_str())).collect();
    assert_eq!(reasons, [("free-nemotron-x", "data-scope"), ("go-glm-x", "unavailable")]);
    assert_eq!(roster.critics.iter().map(|critic| critic.client_config.family()).collect::<Vec<_>>(), ["google", "deepseek", "alibaba"]);
    // A family the allowlist does not name is passed over, and the writer's is never reviewed from.
    let (narrow, _) = resolve_roster(&steps, &slots(), &request(&allowed[..2], &[], true));
    let reasons: Vec<&str> = narrow.dropped.iter().map(|dropped| dropped.reason.as_str()).collect();
    assert_eq!(reasons.iter().filter(|reason| **reason == "family-not-allowed").count(), 5);
    let all = strings(&["nvidia", "google", "anthropic", "deepseek"]);
    let writer = [step("x/claude-x", "vibe")];
    assert!(resolve_roster(&writer, &slots(), &request(&all, &[], true)).0.critics.is_empty());
    let own = Request { writer_family: "google", ..request(&all, &[], true) };
    let (roster, _) = resolve_roster(&steps, &slots(), &own);
    assert!(roster.dropped.iter().any(|dropped| (dropped.client.as_str(), dropped.reason.as_str()) == ("vibe-gemma-x", "orchestrator-family")));
}

fn recorded(served: &str, model_id: Option<&str>, fallbacks: Option<u64>) -> Routing {
    Routing { capture: Capture::Recorded, model_id: model_id.map(str::to_string), served_model: Some(served.to_string()), attempted_fallbacks: fallbacks }
}

#[test]
fn a_reviewer_counts_only_when_the_proxy_saw_the_model_it_asked_for_answer() {
    let routed = |routing: &Routing| TierRouting::of("tier-review", "free/glm-4.7-flash", routing);
    assert_eq!(routed(&recorded("free/glm-4.7-flash", Some("d1"), Some(0))).collapse(), None);
    // LiteLLM may name the model without its group.
    assert_eq!(routed(&recorded("glm-4.7-flash", Some("d1"), Some(0))).collapse(), None);
    let fell = routed(&recorded("free/other", Some("d2"), Some(1))).collapse().expect("a collapse");
    assert_eq!(fell.0, COLLAPSED);
    assert!(fell.1.contains("answered with free/other after 1 attempted fallbacks"), "{}", fell.1);
    let other = routed(&recorded("free/other", Some("d2"), Some(0))).collapse().expect("a collapse");
    assert_eq!(other.0, COLLAPSED);
    let fallback_without_a_name = routed(&Routing { served_model: None, ..recorded("", None, Some(2)) }).collapse().expect("a collapse");
    assert!(fallback_without_a_name.1.contains("another deployment"), "{}", fallback_without_a_name.1);
    for unseen in [
        Routing { capture: Capture::Hidden, model_id: None, served_model: None, attempted_fallbacks: None },
        Routing { capture: Capture::NoRequest, model_id: None, served_model: None, attempted_fallbacks: None },
        recorded("free/glm-4.7-flash", Some("d1"), None),
    ] {
        let collapse = routed(&unseen).collapse().expect("unseen");
        assert_eq!(collapse.0, UNSEEN);
    }
    assert!(routed(&Routing { capture: Capture::Hidden, model_id: None, served_model: None, attempted_fallbacks: Some(0) }).collapse().expect("unseen").1.contains("fallback count 0"));
    assert!(routed(&Routing { capture: Capture::Hidden, model_id: None, served_model: None, attempted_fallbacks: None }).collapse().expect("unseen").1.contains("unreadable"));
}

#[test]
fn a_routing_is_kept_on_the_run_and_written_to_the_ledger_line_in_the_delegation_records_own_names() {
    let routing = TierRouting::of("tier-review", "free/glm-4.7-flash", &recorded("free/glm-4.7-flash", Some("d1"), Some(0)));
    let shown = json::compact(&routing.to_json());
    assert_eq!(
        shown,
        r#"{"attempted_fallbacks":0,"fallback_fired":false,"litellm_model_id":"d1","requested_model":"free/glm-4.7-flash","routing_capture":"recorded","served_model":"free/glm-4.7-flash","tier":"tier-review"}"#
    );
    let value = parse(&shown).expect("JSON");
    assert_eq!(TierRouting::read(&Fields::of(&value, "").expect("fields")).expect("read back"), routing);
    let hidden = TierRouting::of("tier-review", "m", &Routing::unseen("pi-tier-review"));
    let value = parse(&json::compact(&hidden.to_json())).expect("JSON");
    assert_eq!(TierRouting::read(&Fields::of(&value, "").expect("fields")).expect("read back"), hidden);
    assert_eq!(hidden.fallback_fired(), None);
    let mut line = Object::new();
    routing.ledger_members(&mut line);
    let keys: Vec<&str> = line.entries().into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["attempted_fallbacks", "fallback_fired", "litellm_model_id", "routing_capture", "served_model", "tier", "harness"]);
}

#[test]
fn a_run_with_a_routing_prints_it_and_reads_it_back() {
    use crate::critic::{read_critic_run, CriticRun};
    use crate::refute_round::run_json;
    let routing = TierRouting::of("tier-review", "free/glm-4.7-flash", &recorded("free/glm-4.7-flash", Some("d1"), Some(0)));
    let run = CriticRun { routing: Some(routing), ..super::refute_round::full_run() };
    let printed = run_json(&run);
    assert_eq!(read_critic_run(&Fields::of(&printed, "").expect("fields")).expect("a run"), run);
    // A run of any other client has none.
    let plain = run_json(&super::refute_round::full_run());
    assert!(!json::compact(&plain).contains("routing"));
}
