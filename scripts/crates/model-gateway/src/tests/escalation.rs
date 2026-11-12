//! Tests for the escalation's domain: which reasons retry, the order a climb and a
//! swap take, the pool a delegation may move through and the prompt a retry gets.
//! Ported from the node runner's tests of the same module.

use super::machine::DEFAULTS;
use crate::config::{CostClass, GatewayConfig, ModelEntry, Tier};
use crate::config_parse::parse_defaults;
use crate::escalation::*;
use crate::json::{compact, parse};
use crate::policy::{decide_policy, DataScope, ModelPermission, PolicySubject, SendPermission};
use crate::run::{read_run_input, Carries, RunInput};
use crate::allowlist::AllowlistEntry;
use std::collections::BTreeSet;

fn candidate(client: &str, model: &str, tier: Option<Tier>, cost_class: CostClass) -> Candidate {
    Candidate { client: client.to_string(), cost_class, family: "any".to_string(), model: model.to_string(), tier, provider: None }
}

fn subsidised(client: &str, model: &str, tier: Option<Tier>) -> Candidate {
    candidate(client, model, tier, CostClass::Subsidised)
}

fn pool() -> Vec<Candidate> {
    vec![
        subsidised("claude", "opus", Some(Tier::Frontier)),
        subsidised("claude", "sonnet", Some(Tier::Hard)),
        subsidised("claude", "haiku", Some(Tier::Routine)),
        subsidised("vibe", "glm", Some(Tier::Hard)),
        candidate("hermes", "personal-hard", Some(Tier::Hard), CostClass::MeteredPersonal),
        candidate("hermes", "personal-frontier", Some(Tier::Frontier), CostClass::MeteredPersonal),
        candidate("codex", "employer-frontier", Some(Tier::Frontier), CostClass::MeteredEmployer),
    ]
}

fn keys(candidates: &[Candidate]) -> Vec<&str> {
    candidates.iter().map(|entry| entry.model.as_str()).collect()
}

fn only_subsidised(pool: Vec<Candidate>) -> Vec<Candidate> {
    pool.into_iter().filter(|entry| entry.cost_class == CostClass::Subsidised).collect()
}

fn run_input(extra: &str) -> RunInput {
    let mut text = r#"{"client":"claude","mode":"read-only","model":"claude-sonnet-5-5","prompt":"p","time_limit_s":60,"workflow":"w""#.to_string();
    text.push_str(extra);
    text.push('}');
    read_run_input(&parse(&text).expect("JSON")).expect("a run input")
}

#[test]
fn a_climb_lists_the_stronger_subsidised_models_by_tier_and_then_by_catalogue_order() {
    let from = subsidised("vibe", "small", Some(Tier::Trivial));
    assert_eq!(keys(&climb_order(&from, &only_subsidised(pool()))), ["haiku", "sonnet", "glm", "opus"]);
}

#[test]
fn a_climb_lists_only_subsidised_models_whatever_the_cost_class_it_starts_from() {
    let personal = candidate("hermes", "personal-routine", Some(Tier::Routine), CostClass::MeteredPersonal);
    assert_eq!(keys(&climb_order(&personal, &pool())), ["sonnet", "glm", "opus"]);
    assert_eq!(keys(&climb_order(&subsidised("claude", "haiku", Some(Tier::Routine)), &pool())), ["sonnet", "glm", "opus"]);
}

#[test]
fn a_climb_never_lists_a_model_of_no_stronger_tier_itself_or_one_with_no_tier() {
    let untiered = subsidised("vibe", "unknown", None);
    assert!(climb_order(&subsidised("claude", "opus", Some(Tier::Frontier)), &pool()).is_empty());
    let mut with_untiered = pool();
    with_untiered.push(untiered.clone());
    assert_eq!(keys(&climb_order(&subsidised("claude", "sonnet", Some(Tier::Hard)), &with_untiered)), ["opus"]);
    assert!(climb_order(&untiered, &pool()).is_empty());
}

#[test]
fn a_swap_lists_the_models_of_the_same_cost_class_at_the_wanted_tier_or_above_the_nearest_first() {
    assert_eq!(keys(&fall_through_order(&subsidised("vibe", "glm", Some(Tier::Hard)), &pool())), ["sonnet", "opus"]);
    let personal = candidate("hermes", "personal-hard", Some(Tier::Hard), CostClass::MeteredPersonal);
    assert_eq!(keys(&fall_through_order(&personal, &pool())), ["personal-frontier"]);
}

#[test]
fn a_swap_lists_nothing_for_a_model_that_has_no_tier() {
    assert!(fall_through_order(&subsidised("vibe", "unknown", None), &pool()).is_empty());
}

#[test]
fn refusals_and_launches_that_never_started_are_read_from_a_results_reason() {
    for reason in ["policy-refused", "cap-exceeded", "no-estimate", "workflow-ended", "provider-not-allowed", "unknown-client", "grant-expiring"] {
        assert!(is_refusal(Some(reason)), "{reason}");
    }
    for reason in [None, Some("timeout"), Some("exit-1"), Some("check-failed"), Some("parse-error")] {
        assert!(!is_refusal(reason), "{reason:?}");
    }
    for reason in ["auth", "rate-limited", "launch-failed", "model-unavailable"] {
        assert!(is_cannot_start(Some(reason)), "{reason}");
    }
    for reason in [None, Some("timeout"), Some("exit-1"), Some("check-failed")] {
        assert!(!is_cannot_start(reason), "{reason:?}");
    }
}

#[test]
fn a_retry_attaches_the_failure_to_the_original_prompt_cut_to_its_last_characters() {
    assert_eq!(retry_prompt("do it", "exit-1", "  boom\n"), "do it\n\nA previous attempt failed: exit-1; boom");
    assert_eq!(retry_prompt("do it", "timeout", "  \n"), "do it\n\nA previous attempt failed: timeout");
    let long = format!("{}{}", "a".repeat(FAILURE_TAIL_CHARS), "b".repeat(FAILURE_TAIL_CHARS));
    assert_eq!(
        retry_prompt("do it", "exit-1", &long),
        format!("do it\n\nA previous attempt failed: exit-1; {}", "b".repeat(FAILURE_TAIL_CHARS))
    );
}

#[test]
fn an_attempts_input_is_built_on_the_model_it_is_moved_to_keeping_the_provider_only_for_a_client_that_names_one() {
    let input = run_input(r#","client":"vibe","model":"glm","provider":"stale""#);
    let moved = attempt_input(&input, &subsidised("claude", "opus", Some(Tier::Frontier)), "again");
    assert_eq!((moved.client.as_str(), moved.model.as_str(), moved.prompt.as_str(), moved.provider.as_deref()), ("claude", "opus", "again", None));
    assert_eq!(moved.workflow, "w");
    let named = Candidate { provider: Some("anthropic".to_string()), ..subsidised("hermes", "m", Some(Tier::Hard)) };
    assert_eq!(attempt_input(&input, &named, "again").provider.as_deref(), Some("anthropic"));
}

#[test]
fn the_family_binds_the_pool_unless_the_caller_lets_the_gateway_change_family() {
    let anthropic = Candidate { family: "anthropic".to_string(), ..subsidised("claude", "a", Some(Tier::Hard)) };
    let zai = Candidate { family: "zai".to_string(), ..subsidised("vibe", "z", Some(Tier::Hard)) };
    let both = vec![anthropic.clone(), zai.clone()];
    assert_eq!(bind_family(&run_input(""), &anthropic, both.clone()), vec![anthropic.clone()]);
    assert_eq!(bind_family(&run_input(r#","family_binding":true"#), &anthropic, both.clone()), vec![anthropic.clone()]);
    assert_eq!(bind_family(&run_input(r#","family_binding":false"#), &anthropic, both.clone()), both);
}

#[test]
fn the_notice_names_both_families_and_the_cause() {
    assert_eq!(escalation_notice("claude", "anthropic", "rate-limited", "zai"), "escalated from anthropic (claude: rate-limited) to zai");
}

#[test]
fn the_report_lists_its_members_in_the_order_the_result_spreads_them() {
    let attempt = AttemptSummary { run_id: "r1".to_string(), client: "vibe".to_string(), model: "glm".to_string(), ok: false, reason: Some("timeout".to_string()) };
    let ok = AttemptSummary { reason: None, ok: true, run_id: "r2".to_string(), ..attempt.clone() };
    let skipped = Unstartable { client: "vibe".to_string(), model: "glm".to_string(), reason: "auth".to_string() };
    let full = EscalationReport {
        attempts: vec![attempt.clone(), ok.clone()],
        escalation: Some(EscalationEnd::Stopped),
        skipped: vec![skipped.clone()],
        stopped_reason: Some("why".to_string()),
    };
    let text: Vec<String> = full.members().iter().map(|(key, value)| format!("{key}={}", compact(value))).collect();
    assert_eq!(
        text,
        [
            r#"attempts=[{"client":"vibe","model":"glm","run_id":"r1","status":"failed","reason":"timeout"},{"client":"vibe","model":"glm","run_id":"r2","status":"ok"}]"#,
            r#"escalation="stopped""#,
            r#"skipped=[{"client":"vibe","model":"glm","reason":"auth"}]"#,
            r#"stopped_reason="why""#,
        ]
    );
    let bare = EscalationReport { attempts: vec![ok], ..EscalationReport::default() };
    assert_eq!(bare.members().len(), 1);
    assert_eq!(EscalationEnd::Exhausted.as_str(), "exhausted");
}

#[test]
fn the_unavailable_result_keeps_the_key_order_of_the_node_runners_literal() {
    let result = UnavailableResult {
        workflow: "w".to_string(),
        client: "vibe".to_string(),
        model: "glm".to_string(),
        message: "m".to_string(),
        reasons: vec![Unstartable { client: "vibe".to_string(), model: "glm".to_string(), reason: "auth".to_string() }],
        attempts: Vec::new(),
    };
    assert_eq!(
        compact(&result.to_json()),
        r#"{"attempts":[],"client":"vibe","message":"m","model":"glm","reasons":[{"client":"vibe","model":"glm","reason":"auth"}],"status":"unavailable","workflow":"w"}"#
    );
}

/// A catalogue with one subsidised and one metered-employer model, both permitted for the test repository.
fn config() -> GatewayConfig {
    let mut config = parse_defaults(&parse(DEFAULTS).expect("JSON")).expect("defaults");
    config.models.retain(|entry| entry.client == "claude" && entry.model == "claude-sonnet-5-5" || entry.client == "codex");
    config
}

fn repository_scope() -> DataScope {
    DataScope::Repository {
        key: "github.com/acme/widgets".to_string(),
        entry: Some(AllowlistEntry { code: true, families: vec!["anthropic".to_string(), "openai".to_string()], history: true }),
    }
}

fn permissions(config: &GatewayConfig, scope: &DataScope) -> Vec<ModelPermission> {
    let subjects: Vec<PolicySubject> = config
        .models
        .iter()
        .map(|model| PolicySubject { client: model.client.clone(), model: model.model.clone(), family: model.family.clone() })
        .collect();
    decide_policy(config, scope, &subjects).permissions
}

fn repo_run() -> RunInput {
    run_input(r#","repo":"/work""#)
}

fn names(candidates: &[Candidate]) -> Vec<String> {
    candidates.iter().map(|entry| format!("{}:{}", entry.client, entry.model)).collect()
}

fn grant_for(clients: &[&str]) -> BTreeSet<String> {
    clients.iter().map(|client| client.to_string()).collect()
}

#[test]
fn a_metered_employer_model_stays_out_of_the_pool_while_no_live_grant_covers_the_workflow() {
    let config = config();
    let pool = candidate_pool(&config, &repo_run(), &permissions(&config, &repository_scope()), &grant_for(&[]));
    assert_eq!(names(&pool), ["claude:claude-sonnet-5-5"]);
}

#[test]
fn a_metered_employer_model_joins_the_pool_while_a_live_grant_covers_the_workflow() {
    let config = config();
    let pool = candidate_pool(&config, &repo_run(), &permissions(&config, &repository_scope()), &grant_for(&["codex"]));
    assert_eq!(names(&pool), ["claude:claude-sonnet-5-5", "codex:gpt-5.5"]);
    let other = candidate_pool(&config, &repo_run(), &permissions(&config, &repository_scope()), &grant_for(&["opencode"]));
    assert_eq!(names(&other), ["claude:claude-sonnet-5-5"]);
}

#[test]
fn a_model_whose_client_the_configuration_does_not_declare_stays_out() {
    let mut config = config();
    config.models.push(ModelEntry {
        client: "nobody".to_string(),
        model: "ghost".to_string(),
        names: None,
        family: "openai".to_string(),
        tier: Tier::Frontier,
        notes: "n".to_string(),
        prices: None,
    });
    let pool = candidate_pool(&config, &repo_run(), &permissions(&config, &repository_scope()), &grant_for(&["codex"]));
    assert_eq!(names(&pool), ["claude:claude-sonnet-5-5", "codex:gpt-5.5"]);
}

#[test]
fn a_metered_employer_model_is_never_a_climb_target_whatever_the_grant() {
    let config = config();
    let granted = candidate_pool(&config, &repo_run(), &permissions(&config, &repository_scope()), &grant_for(&["codex"]));
    assert!(climb_order(&subsidised("claude", "claude-sonnet-5-5", Some(Tier::Hard)), &granted).is_empty());
}

#[test]
fn a_client_that_cannot_start_falls_through_to_a_metered_employer_model_only_under_a_grant() {
    let config = config();
    let employer = candidate("codex", "gpt-5.5", Some(Tier::Frontier), CostClass::MeteredEmployer);
    let granted = candidate_pool(&config, &repo_run(), &permissions(&config, &repository_scope()), &grant_for(&["codex"]));
    assert!(fall_through_order(&employer, &granted).is_empty());
    let spare = candidate("codex", "gpt-5.6", Some(Tier::Frontier), CostClass::MeteredEmployer);
    let mut wider = granted;
    wider.push(spare.clone());
    assert_eq!(fall_through_order(&employer, &wider), vec![spare]);
}

#[test]
fn a_model_the_owner_disabled_by_any_of_its_names_stays_out() {
    let mut config = config();
    config.models[0].names = Some(vec!["sonnet".to_string()]);
    config.disabled = vec!["claude:sonnet".to_string()];
    let pool = candidate_pool(&config, &repo_run(), &permissions(&config, &repository_scope()), &grant_for(&["codex"]));
    assert_eq!(names(&pool), ["codex:gpt-5.5"]);
}

fn permission(client: &str, model: &str, permitted: bool, code: bool, history: bool) -> ModelPermission {
    ModelPermission {
        client: client.to_string(),
        model: model.to_string(),
        permitted,
        inline_only: false,
        may_send: SendPermission { code, history },
    }
}

#[test]
fn the_policy_decides_what_a_run_may_carry_by_what_it_reads_and_what_the_caller_marks() {
    let config = config();
    let sonnet = "claude-sonnet-5-5";
    let pooled = |input: &RunInput, may: ModelPermission| names(&candidate_pool(&config, input, &[may], &grant_for(&[])));
    // A repository means code; a mark of code means code; a mark of history means history.
    assert_eq!(pooled(&repo_run(), permission("claude", sonnet, true, true, false)), ["claude:claude-sonnet-5-5"]);
    assert!(pooled(&repo_run(), permission("claude", sonnet, true, false, true)).is_empty());
    assert!(pooled(&run_input(r#","carries":{"code":true,"history":false}"#), permission("claude", sonnet, true, false, true)).is_empty());
    assert!(pooled(&run_input(r#","carries":{"code":false,"history":true}"#), permission("claude", sonnet, true, true, false)).is_empty());
    assert_eq!(
        pooled(&run_input(r#","carries":{"code":false,"history":true}"#), permission("claude", sonnet, true, false, true)),
        ["claude:claude-sonnet-5-5"]
    );
    assert_eq!(pooled(&run_input(""), permission("claude", sonnet, true, false, false)), ["claude:claude-sonnet-5-5"]);
    assert!(pooled(&run_input(""), permission("claude", sonnet, false, true, true)).is_empty());
    let marked = run_input(r#","carries":{"code":true,"history":true}"#);
    assert_eq!(marked.carries, Some(Carries { code: true, history: true }));
}

/// The pool of a catalogue with the clients that run on a provider.
fn providers_config() -> GatewayConfig {
    let mut config = parse_defaults(&parse(DEFAULTS).expect("JSON")).expect("defaults");
    config.models.retain(|entry| matches!(entry.client.as_str(), "hermes" | "opencode"));
    config
}

fn open_scope() -> DataScope {
    DataScope::Outside
}

#[test]
fn a_client_that_runs_on_a_provider_is_open_only_to_the_provider_the_caller_named() {
    let config = providers_config();
    let granted = grant_for(&[]);
    let allowed = permissions(&config, &open_scope());
    // Without a provider, hermes is out; opencode's provider comes from its model.
    let plain = candidate_pool(&config, &run_input(r#","client":"opencode","model":"opencode-go/kimi-k2""#), &allowed, &granted);
    assert_eq!(names(&plain), ["opencode:opencode-go/kimi-k2"]);
    assert_eq!((plain[0].cost_class, plain[0].provider.as_deref()), (CostClass::Subsidised, None));
    // A hermes run naming a provider opens hermes, with the provider on its candidate; another client's run does not.
    let named = candidate_pool(&config, &run_input(r#","client":"hermes","model":"hermes-hard","provider":"anthropic""#), &allowed, &granted);
    assert_eq!(names(&named), ["hermes:hermes-hard", "opencode:opencode-go/kimi-k2"]);
    assert_eq!((named[0].cost_class, named[0].provider.as_deref()), (CostClass::MeteredPersonal, Some("anthropic")));
    let elsewhere = candidate_pool(&config, &run_input(r#","provider":"anthropic""#), &allowed, &granted);
    assert_eq!(names(&elsewhere), ["opencode:opencode-go/kimi-k2"]);
}

#[test]
fn an_opencode_model_whose_provider_the_machine_does_not_allow_stays_out() {
    let mut config = providers_config();
    config.models.push(ModelEntry {
        client: "opencode".to_string(),
        model: "opencode/zen".to_string(),
        names: None,
        family: "moonshot".to_string(),
        tier: Tier::Hard,
        notes: "n".to_string(),
        prices: None,
    });
    let pool = candidate_pool(&config, &run_input(r#","client":"opencode","model":"opencode-go/kimi-k2""#), &permissions(&config, &open_scope()), &grant_for(&[]));
    assert_eq!(names(&pool), ["opencode:opencode-go/kimi-k2"]);
}

#[test]
fn the_requested_model_is_a_candidate_with_the_tier_the_catalogue_gives_it() {
    let config = parse_defaults(&parse(DEFAULTS).expect("JSON")).expect("defaults");
    let entry = config.models.iter().find(|entry| entry.model == "claude-sonnet-5-5");
    let known = requested_candidate(&config, &run_input(""), entry, "anthropic");
    assert_eq!(
        known,
        Candidate {
            client: "claude".to_string(),
            model: "claude-sonnet-5-5".to_string(),
            family: "anthropic".to_string(),
            tier: Some(Tier::Hard),
            cost_class: CostClass::Subsidised,
            provider: None,
        }
    );
    // A model the catalogue does not know has no tier, and one of a client it does not declare is subsidised.
    let unknown = requested_candidate(&config, &run_input(r#","client":"agy","model":"ghost""#), None, "");
    assert_eq!((unknown.tier, unknown.cost_class), (None, CostClass::Subsidised));
    // An OpenCode model names its provider, which sets its cost class; a provider the caller names stays on the candidate.
    let opencode = requested_candidate(&config, &run_input(r#","client":"opencode","model":"opencode-go/kimi-k2""#), None, "moonshot");
    assert_eq!((opencode.cost_class, opencode.provider), (CostClass::Subsidised, None));
    let paid = requested_candidate(&config, &run_input(r#","client":"opencode","model":"other/x""#), None, "");
    assert_eq!(paid.cost_class, CostClass::MeteredPersonal);
    let named = requested_candidate(&config, &run_input(r#","client":"hermes","model":"hermes-hard","provider":"anthropic""#), None, "");
    assert_eq!((named.cost_class, named.provider.as_deref()), (CostClass::MeteredPersonal, Some("anthropic")));
}
