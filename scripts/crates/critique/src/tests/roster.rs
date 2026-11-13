//! roster's own pieces: a manifest's critics, the fallbacks a retry may use, and the critics that were dropped.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::pricing::TokenPrices;
use crate::roster::{read_dropped_critic, read_roster_critic, DroppedCritic, RosterCritic, RosterFallback, DROP_REASONS};
use crate::routing::{AgyCriticClient, CriticClient, GatewayModel, VibeCriticClient};
use model_gateway::json::parse;

fn read<T>(text: &str, read: for<'a, 'b> fn(&'a Fields<'b>) -> Result<T, ShapeError>) -> T {
    let value = parse(text).expect("json");
    read(&Fields::of(&value, "").expect("object")).expect("read")
}

fn shape<T>(text: &str, read: for<'a, 'b> fn(&'a Fields<'b>) -> Result<T, ShapeError>) -> String {
    let value = parse(text).expect("json");
    match Fields::of(&value, "").and_then(|fields| read(&fields)) {
        Ok(_) => "read".to_string(),
        Err(error) => error.0,
    }
}

#[test]
fn a_critic_is_its_slot_its_client_and_its_fallbacks() {
    let text = r#"{"client":"vibe-main","role":"critic","lenses":["a","b"],"max_price_usd":2.5,"time_limit_s":600,"critic_id":"c1-vibe-main","client_config":{"kind":"vibe","binary":"vibe","agent":"critic-m","model":"m","family":"mistral","prices":{"input":1,"cached_input":0.1,"output":2},"gateway":{"client":"vibe","model":"alias"}},"fallbacks":[{"client":"agy-1","client_config":{"kind":"agy","binary":"agy","model":"gemini-3","family":"google"}}]}"#;
    assert_eq!(
        read(text, read_roster_critic),
        RosterCritic {
            client: "vibe-main".to_string(),
            role: "critic".to_string(),
            lenses: vec!["a".to_string(), "b".to_string()],
            max_price_usd: 2.5,
            time_limit_s: 600.0,
            critic_id: "c1-vibe-main".to_string(),
            client_config: CriticClient::Vibe(VibeCriticClient {
                binary: "vibe".to_string(),
                agent: "critic-m".to_string(),
                model: "m".to_string(),
                family: "mistral".to_string(),
                prices: Some(TokenPrices { input: 1.0, cached_input: 0.1, output: 2.0 }),
                gateway: Some(GatewayModel { client: "vibe".to_string(), model: "alias".to_string() }),
            }),
            fallbacks: vec![RosterFallback {
                client: "agy-1".to_string(),
                client_config: CriticClient::Agy(AgyCriticClient {
                    binary: "agy".to_string(),
                    model: "gemini-3".to_string(),
                    family: "google".to_string(),
                    gateway: None,
                }),
            }],
        }
    );
}

#[test]
fn a_critic_without_fallbacks_has_none() {
    let text = r#"{"client":"vibe-main","role":"critic","lenses":[],"max_price_usd":1,"time_limit_s":60,"critic_id":"c1","client_config":{"kind":"codex","binary":"codex","model":"m","family":"openai"},"fallbacks":[]}"#;
    let critic = read(text, read_roster_critic);
    assert!(critic.fallbacks.is_empty());
    assert_eq!(critic.client_config.kind(), "codex");
}

#[test]
fn a_critic_names_the_first_wrong_field() {
    let slot = r#""client":"c","role":"r","lenses":[],"max_price_usd":1,"time_limit_s":1"#;
    let client = r#""client_config":{"kind":"claude","binary":"claude","model":"m","family":"anthropic"}"#;
    assert_eq!(shape(r#"{"role":"r"}"#, read_roster_critic), "client must be a non-empty string");
    assert_eq!(shape(r#"{"client":"c"}"#, read_roster_critic), "lenses must be a list");
    assert_eq!(shape(r#"{"client":"c","lenses":[]}"#, read_roster_critic), "max_price_usd must be a number");
    assert_eq!(shape(r#"{"client":"c","lenses":[],"max_price_usd":1}"#, read_roster_critic), "role must be a non-empty string");
    assert_eq!(shape(r#"{"client":"c","lenses":[],"max_price_usd":1,"role":"r"}"#, read_roster_critic), "time_limit_s must be a number");
    let no_client = format!(r#"{{{slot},"critic_id":"c1","fallbacks":[]}}"#);
    assert_eq!(shape(&no_client, read_roster_critic), "client_config must be a JSON object");
    let no_id = format!(r#"{{{slot},{client},"fallbacks":[]}}"#);
    assert_eq!(shape(&no_id, read_roster_critic), "critic_id must be a non-empty string");
    let no_fallbacks = format!(r#"{{{slot},{client},"critic_id":"c1"}}"#);
    assert_eq!(shape(&no_fallbacks, read_roster_critic), "fallbacks must be a list");
    let bad_item = format!(r#"{{{slot},{client},"critic_id":"c1","fallbacks":[5]}}"#);
    assert_eq!(shape(&bad_item, read_roster_critic), "fallbacks[0] must be a JSON object");
    let no_fallback_client = format!(r#"{{{slot},{client},"critic_id":"c1","fallbacks":[{{}}]}}"#);
    assert_eq!(shape(&no_fallback_client, read_roster_critic), "fallbacks[0].client must be a non-empty string");
    let no_fallback_config = format!(r#"{{{slot},{client},"critic_id":"c1","fallbacks":[{{"client":"a"}}]}}"#);
    assert_eq!(shape(&no_fallback_config, read_roster_critic), "fallbacks[0].client_config must be a JSON object");
}

#[test]
fn a_dropped_critic_carries_why_and_what_took_its_slot() {
    let text = r#"{"client":"agy-1","family":"google","reason":"quota","detail":"out of quota","replaced_by":"c2-vibe","deep":true}"#;
    assert_eq!(
        read(text, read_dropped_critic),
        DroppedCritic {
            client: "agy-1".to_string(),
            family: "google".to_string(),
            reason: "quota".to_string(),
            detail: Some("out of quota".to_string()),
            replaced_by: Some("c2-vibe".to_string()),
            deep: Some(true),
        }
    );
}

#[test]
fn a_dropped_critic_without_detail_or_a_replacement_has_neither() {
    let text = r#"{"client":"a","family":"mistral","reason":"same-family"}"#;
    assert_eq!(
        read(text, read_dropped_critic),
        DroppedCritic {
            client: "a".to_string(),
            family: "mistral".to_string(),
            reason: "same-family".to_string(),
            detail: None,
            replaced_by: None,
            deep: None,
        }
    );
    let base = r#"{"client":"a","family":"mistral","reason":"same-family","deep":false}"#;
    assert_eq!(read(base, read_dropped_critic).deep, None);
}

#[test]
fn a_dropped_critic_names_the_first_wrong_field() {
    assert_eq!(shape(r#"{"detail":""}"#, read_dropped_critic), "detail must be a non-empty string");
    assert_eq!(shape(r#"{"replaced_by":""}"#, read_dropped_critic), "replaced_by must be a non-empty string");
    assert_eq!(shape(r#"{"family":"mistral","reason":"quota"}"#, read_dropped_critic), "client must be a non-empty string");
    assert_eq!(shape(r#"{"client":"a","reason":"quota"}"#, read_dropped_critic), "family must be one of mistral, zhipu, deepseek, google, anthropic, openai");
    assert_eq!(
        shape(r#"{"client":"a","family":"mistral"}"#, read_dropped_critic),
        "reason must be one of family-not-allowed, orchestrator-family, same-family, unavailable, quota, no-grant"
    );
    assert_eq!(shape(r#"{"client":"a","family":"mistral","reason":"quota","deep":"yes"}"#, read_dropped_critic), "deep must be true or false");
}

#[test]
fn the_drop_reasons_are_the_ones_a_manifest_may_name() {
    assert_eq!(DROP_REASONS, ["family-not-allowed", "orchestrator-family", "same-family", "unavailable", "quota", "no-grant"]);
}
