//! `run --tier`: a tier names the delegation profile rendered for it, and anything else is an error, never a default.

use super::machine::Machine;
use crate::config_parse::parse_defaults;
use crate::json::{self, parse, Object, Value};
use crate::profiles::list_profiles;
use crate::tier::choose;

fn defaults(profiles: &str) -> String {
    format!(
        r#"{{
  "version": 1,
  "clients": {{"pi-free": {{"binary": "pi", "cost_class": "subsidised", "model_source": "live"}}}},
  "models": [{{"client": "pi-free", "model": "tier-free", "family": "litellm-tier", "tier": "routine", "notes": "n"}}],
  "profiles": {profiles},
  "settings": {{"availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3,
    "metered_personal_caps": {{"per_day_usd": 5, "per_workflow_usd": 2}},
    "output_allowance_tokens": {{"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500}},
    "plan_warning_ratio": 0.8, "summary_max_chars": 1500}}
}}"#
    )
}

const TIERED: &str = r#"{"tier-free": {"description": "d", "models": ["pi-free:tier-free"], "tier": "tier-free", "key_models": ["free", "tier-free"]}, "other": {"description": "d", "models": ["pi-free:tier-free"]}}"#;

fn input(text: &str) -> Object {
    match parse(text).expect("JSON") {
        Value::Obj(object) => object,
        other => panic!("not an object: {other:?}"),
    }
}

fn chosen(machine: &Machine, text: &str) -> Result<String, String> {
    choose(&input(text), &machine.runtime).map(|object| json::compact(&Value::Obj(object))).map_err(|error| format!("{}: {}", error.code, error.message))
}

#[test]
fn a_tier_names_its_profile_on_pi_and_an_input_without_one_is_left_alone() {
    let machine = Machine::new(&defaults(TIERED));
    assert_eq!(chosen(&machine, r#"{"tier":"tier-free","prompt":"p"}"#).as_deref(), Ok(r#"{"prompt":"p","profile":"tier-free","harness":"pi"}"#));
    assert_eq!(chosen(&machine, r#"{"prompt":"p"}"#).as_deref(), Ok(r#"{"prompt":"p"}"#));
}

#[test]
fn an_unknown_tier_is_an_error_that_names_the_tiers_there_are_and_a_profile_that_is_no_tier_does_not_count() {
    let machine = Machine::new(&defaults(TIERED));
    assert_eq!(chosen(&machine, r#"{"tier":"nope"}"#), Err("unknown-tier: no tier nope: the defaults state tier-free".to_string()));
    assert_eq!(chosen(&machine, r#"{"tier":"other"}"#), Err("unknown-tier: no tier other: the defaults state tier-free".to_string()));
    let bare = Machine::new(&defaults("{}"));
    assert_eq!(chosen(&bare, r#"{"tier":"tier-free"}"#), Err("unknown-tier: no tier tier-free: the defaults state none".to_string()));
}

#[test]
fn a_tier_is_never_mixed_with_a_choice_of_its_own_or_left_empty() {
    let machine = Machine::new(&defaults(TIERED));
    for extra in ["profile", "harness", "client", "model"] {
        let refused = chosen(&machine, &format!(r#"{{"tier":"tier-free","{extra}":"x"}}"#)).expect_err("refused");
        assert!(refused.starts_with("invalid-input: tier picks the profile, the client and the model"), "{extra}: {refused}");
    }
    for bad in [r#""""#, "3", "null"] {
        assert_eq!(chosen(&machine, &format!(r#"{{"tier":{bad}}}"#)), Err("invalid-input: tier must be a non-empty string".to_string()), "{bad}");
    }
}

#[test]
fn a_tier_profile_is_listed_with_its_tier_and_must_be_named_for_it() {
    let config = parse_defaults(&parse(&defaults(TIERED)).expect("JSON")).expect("config");
    assert_eq!(config.profiles["tier-free"].tier.as_deref(), Some("tier-free"));
    assert_eq!(config.profiles["other"].tier, None);
    let listed = json::pretty(&list_profiles(&config, &[], "the policy"));
    assert_eq!(listed.matches("\"tier\": \"tier-free\"").count(), 1, "{listed}");
    let wrong = r#"{"tier-free": {"description": "d", "models": ["pi-free:tier-free"], "tier": "tier-vibe"}}"#;
    let refused = parse_defaults(&parse(&defaults(wrong)).expect("JSON")).expect_err("refused");
    assert_eq!(refused.message, "profiles.tier-free.tier must be the profile's own name: a tier profile is named for the tier");
}

#[test]
fn a_tier_whose_provider_this_machines_pi_lacks_is_refused_naming_the_key_it_needs() {
    let machine = Machine::new(&defaults(TIERED));
    let models = format!("{}/home/.pi/agent/models.json", machine.sandbox.text());
    // No readable models.json says nothing: the launch reports its own refusal.
    assert!(chosen(&machine, r#"{"tier":"tier-free"}"#).is_ok());
    machine.write_at(&models, r#"{"providers":{"litellm-vibe":{}}}"#);
    let refused = chosen(&machine, r#"{"tier":"tier-free"}"#).expect_err("no key");
    assert!(refused.starts_with("tier-key-missing: no LiteLLM key for tier-free on this machine"), "{refused}");
    assert!(refused.contains("no litellm-free provider") && refused.contains("holding free, tier-free"), "{refused}");
    machine.write_at(&models, r#"{"providers":{"litellm-free":{}}}"#);
    assert!(chosen(&machine, r#"{"tier":"tier-free"}"#).is_ok());
    // A profile that states no key_models is refused without them.
    let bare = Machine::new(&defaults(&TIERED.replace(r#", "key_models": ["free", "tier-free"]"#, "")));
    bare.write_at(&format!("{}/home/.pi/agent/models.json", bare.sandbox.text()), r#"{"providers":{}}"#);
    assert!(chosen(&bare, r#"{"tier":"tier-free"}"#).expect_err("no key").contains("and the tier needs a key;"));
    assert_eq!(parse_defaults(&parse(&defaults(TIERED)).expect("JSON")).expect("config").profiles["tier-free"].key_models, Some(vec!["free".to_string(), "tier-free".to_string()]));
}
