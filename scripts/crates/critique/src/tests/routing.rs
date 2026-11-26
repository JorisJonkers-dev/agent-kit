//! routing's own pieces: what the goldens reach only through a command's error text.

use crate::fields::Fields;
use crate::routing::{data_scope_bar, read_critic_client, CriticClient};
use model_gateway::json::parse;

#[test]
fn a_client_names_the_kind_it_was_read_as() {
    let clients = [
        (r#"{"kind":"vibe","binary":"v","agent":"a","model":"m","family":"mistral"}"#, "vibe"),
        (r#"{"kind":"agy","binary":"a","model":"gemini-1","family":"google"}"#, "agy"),
        (r#"{"kind":"codex","binary":"c","model":"m","family":"openai"}"#, "codex"),
        (r#"{"kind":"claude","binary":"c","model":"m","family":"anthropic"}"#, "claude"),
    ];
    for (text, kind) in clients {
        let value = parse(text).expect("json");
        let client = read_critic_client(&Fields::of(&value, "").expect("object")).expect("client");
        assert_eq!(client.kind(), kind);
    }
}

fn client(text: &str) -> Result<CriticClient, String> {
    let value = parse(text).expect("json");
    read_critic_client(&Fields::of(&value, "").expect("object")).map_err(|error| error.0)
}

const PI: &str = r#"{"kind":"pi","provider":"litellm-public","model":"public/gemini-3.8-flash","family":"google","data_scope":"public"}"#;

#[test]
fn a_pi_client_names_its_route_data_scope() {
    assert_eq!(client(PI).map(|client| client.kind()), Ok("pi"));
    assert!(client(&PI.replace("public\"}", "any\"}")).is_ok());
    assert_eq!(client(&PI.replace(",\"data_scope\":\"public\"", "")), Err("data_scope must be one of any, public".to_string()));
    assert_eq!(client(&PI.replace("\"data_scope\":\"public\"", "\"data_scope\":\"private\"")), Err("data_scope must be one of any, public".to_string()));
}

#[test]
fn only_a_public_scope_pi_client_is_barred_and_only_from_a_private_repository() {
    let public = client(PI).expect("client");
    let any = client(&PI.replace("public\"}", "any\"}")).expect("client");
    let vibe = client(r#"{"kind":"vibe","binary":"v","agent":"a","model":"m","family":"mistral"}"#).expect("client");
    assert_eq!([data_scope_bar(&public, false), data_scope_bar(&public, true)], [true, false]);
    assert_eq!([data_scope_bar(&any, false), data_scope_bar(&vibe, false)], [false, false]);
}

/// Routing whose spec stage's free roster is `free` and whose `pi-free` client and fallbacks are as given.
fn free_routing(free_client: &str, free_slots: &str, fallbacks: &str) -> Result<crate::routing::Routing, String> {
    let slot = |client: &str| format!(r#"{{"client":"{client}","role":"gap-critic","lenses":[],"max_price_usd":0.05,"time_limit_s":240}}"#);
    let text = format!(
        r#"{{"version":1,"clients":{{"pi-free":{free_client},"pi-also-free":{{"kind":"pi","provider":"litellm-free","model":"free/b","family":"cohere","data_scope":"public","free":true}},"pi-paid":{{"kind":"pi","provider":"litellm-public","model":"public/c","family":"google","data_scope":"public"}}}},"fallbacks":{{{fallbacks}}},"stages":{{"spec":{{"distinct_families":true,"base":[{}],"free":[{free_slots}]}}}}}}"#,
        slot("pi-paid")
    );
    crate::routing::parse_routing(&parse(&text).expect("json")).map_err(|error| error.0)
}

const FREE_CLIENT: &str = r#"{"kind":"pi","provider":"litellm-free","model":"free/a","family":"nvidia","data_scope":"public","free":true}"#;

#[test]
fn a_free_roster_holds_free_pi_clients_whose_fallbacks_are_free_too() {
    let slot = r#"{"client":"pi-free","role":"gap-critic","lenses":[],"max_price_usd":0.05,"time_limit_s":240}"#;
    let routing = free_routing(FREE_CLIENT, slot, r#""pi-free":["pi-also-free"],"pi-paid":["pi-free"]"#).expect("routing");
    let stage = &routing.stages[0].1;
    assert_eq!(stage.free.iter().map(|slot| slot.client.as_str()).collect::<Vec<_>>(), ["pi-free"]);
    assert!(matches!(&routing.clients[0].1, CriticClient::Pi(pi) if pi.free));
    // A paid client may fall back to a free one; it is the free roster's clients that may not fall back to a paid one.
    assert_eq!(free_routing(FREE_CLIENT, slot, r#""pi-free":["pi-paid"]"#).expect_err("paid fallback"), "fallbacks.pi-free names pi-paid, which is not a free pi client: the client is in a free-only roster");
    let paid_slot = slot.replace("pi-free", "pi-paid");
    assert_eq!(free_routing(FREE_CLIENT, &paid_slot, "").expect_err("paid slot"), "stages.spec.free names pi-paid, which is not a free pi client: a free-only roster holds nothing else");
    let private = FREE_CLIENT.replace("\"public\"", "\"any\"");
    assert_eq!(free_routing(&private, slot, "").expect_err("free on a private route"), "free must be false unless data_scope is public: a free-only roster is for public material alone");
}

#[test]
fn a_free_model_is_reached_only_as_free_model_on_the_free_route() {
    let refusal = "a free model is reached only as free/<model> (on litellm-free, or as a tier's step), marked free: true, and litellm-free serves nothing else";
    assert!(matches!(client(FREE_CLIENT), Ok(CriticClient::Pi(ref pi)) if pi.free));
    // Unmarked, on another route, or by another name: each is refused.
    assert_eq!(client(&FREE_CLIENT.replace(",\"free\":true", "")).map(|_| ()), Err(refusal.to_string()));
    assert_eq!(client(&FREE_CLIENT.replace("free/a", "openrouter/a")).map(|_| ()), Err(refusal.to_string()));
    assert_eq!(client(&FREE_CLIENT.replace("litellm-free", "litellm-openrouter").replace("free/a", "openrouter/a")).map(|_| ()), Err(refusal.to_string()));
    // A tier's public-only route reaches its free steps by their free/<model> name.
    let step = FREE_CLIENT.replace("litellm-free", "litellm-tier-review");
    assert!(matches!(client(&step), Ok(CriticClient::Pi(ref pi)) if pi.free));
    // A paid client on another route with any other name is not touched by the rule.
    assert!(matches!(client(PI), Ok(CriticClient::Pi(ref pi)) if !pi.free));
}

#[test]
fn a_free_runs_refuters_are_its_free_slots_and_their_fallbacks_only() {
    let slot = r#"{"client":"pi-free","role":"gap-critic","lenses":[],"max_price_usd":0.05,"time_limit_s":240}"#;
    let routing = free_routing(FREE_CLIENT, slot, r#""pi-free":["pi-also-free"]"#).expect("routing");
    let names = |free: bool| crate::refute_round::refuter_candidates(&routing, "spec", false, free).expect("candidates").into_iter().map(|candidate| candidate.client).collect::<Vec<_>>();
    assert_eq!(names(true), ["pi-free", "pi-also-free"]);
    assert_eq!(names(false), ["pi-paid"]);
}
