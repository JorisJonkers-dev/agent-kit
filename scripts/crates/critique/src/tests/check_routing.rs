//! `check-routing`: the shipped profiles and routing against the catalogue snapshot, and what it reports when a
//! roster names a model the catalogue refuses.

use super::cli::runtime;
use super::scenarios::Sandbox;
use crate::check_routing::{check, profile_problems, routing_problems, run};
use crate::error::CritiqueError;
use crate::routing::parse_routing;
use model_gateway::json::{parse, Value};
use model_gateway::litellm_catalogue::{parse_catalogue, LiteLlmCatalogue};

const SNAPSHOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../registry/litellm-catalogue.json");

fn json(text: &str) -> Value {
    parse(text).expect("json")
}

fn snapshot() -> LiteLlmCatalogue {
    parse_catalogue(&json(&std::fs::read_to_string(SNAPSHOT).expect("the repository's catalogue snapshot"))).expect("a catalogue")
}

/// A catalogue with one route, `google`, probing the given models, and one group per model served by it.
fn catalogue(models: &[(&str, &str)]) -> LiteLlmCatalogue {
    let listed: Vec<String> = models.iter().map(|(id, status)| format!("{{\"id\":\"google/{id}\",\"metadata\":{{\"status\":\"{status}\",\"free\":false}}}}")).collect();
    let groups: Vec<String> = models
        .iter()
        .map(|(id, _)| format!("\"public/{id}\":{{\"public\":true,\"deployments\":[{{\"route\":\"google\",\"upstream_id\":\"{id}\",\"order\":1}}]}}"))
        .collect();
    let text = format!("{{\"updated_at\":\"2026-11-15T21:17:08Z\",\"providers\":{{\"google\":{{\"models\":[{}]}}}},\"groups\":{{{}}}}}", listed.join(","), groups.join(","));
    parse_catalogue(&json(&text)).expect("a catalogue")
}

fn routing(model: &str, extra_stage: &str, fallbacks: &str) -> crate::routing::Routing {
    let slot = |client: &str| format!("{{\"client\":\"{client}\",\"role\":\"gap-critic\",\"lenses\":[],\"max_price_usd\":0.3,\"time_limit_s\":240}}");
    let text = format!(
        "{{\"version\":1,\"clients\":{{\
         \"pi-a\":{{\"kind\":\"pi\",\"provider\":\"litellm-public\",\"model\":\"{model}\",\"family\":\"google\",\"data_scope\":\"public\"}},\
         \"pi-b\":{{\"kind\":\"pi\",\"provider\":\"litellm-public\",\"model\":\"public/bad\",\"family\":\"mistral\",\"data_scope\":\"public\"}},\
         \"pi-unrouted\":{{\"kind\":\"pi\",\"provider\":\"litellm-public\",\"model\":\"public/bad\",\"family\":\"zhipu\",\"data_scope\":\"public\"}},\
         \"claude-w\":{{\"kind\":\"claude\",\"binary\":\"claude\",\"model\":\"m\",\"family\":\"anthropic\"}}}},\
         \"fallbacks\":{{{fallbacks}}},\
         \"stages\":{{\"spec\":{{\"distinct_families\":true,\"base\":[{}],\"work\":[{}]{extra_stage}}}}}}}",
        slot("pi-a"),
        slot("claude-w")
    );
    parse_routing(&json(&text)).expect("routing")
}

#[test]
fn the_shipped_profiles_and_routing_name_only_models_the_snapshot_can_run() {
    let defaults = json(model_gateway::SHIPPED_DEFAULTS);
    let routing = parse_routing(&json(model_gateway::SHIPPED_ROUTING)).expect("the shipped routing");
    assert_eq!(profile_problems(&snapshot(), &defaults), Vec::<String>::new());
    assert_eq!(routing_problems(&snapshot(), &routing), Vec::<String>::new());
}

#[test]
fn a_profile_that_names_a_blocked_or_failed_pi_model_is_reported_with_the_reason() {
    let found = catalogue(&[("good", "usable"), ("bad", "error"), ("worse", "blocked"), ("odd", "unknown")]);
    let defaults = json(
        "{\"profiles\":{\"mixed\":{\"models\":[\"pi-public:public/good\",\"pi-public:public/bad\",\"pi-public:public/worse\",\"pi-public:public/odd\",\"pi-public:public/missing\",\"claude:public/bad\",\"nocolon\"]},\"empty\":{},\"odd\":5}}",
    );
    assert_eq!(
        profile_problems(&found, &defaults),
        vec![
            "profile mixed: pi-public:public/bad cannot run: the LiteLLM catalogue marks it error on route google".to_string(),
            "profile mixed: pi-public:public/worse cannot run: blocked by the provider on LiteLLM route google: ".to_string(),
        ]
    );
    assert_eq!(profile_problems(&found, &json("{}")), Vec::<String>::new());
    assert_eq!(profile_problems(&found, &json("[]")), Vec::<String>::new());
}

#[test]
fn a_routing_client_that_a_roster_names_and_the_catalogue_refuses_is_reported() {
    let found = catalogue(&[("good", "usable"), ("bad", "error")]);
    // pi-a is in a base slot; pi-b only as a fallback; pi-unrouted nowhere; claude-w is not a Pi client.
    let named = routing("public/bad", "", "\"pi-a\":[\"pi-b\"]");
    assert_eq!(
        routing_problems(&found, &named),
        vec![
            "routing client pi-a: public/bad on litellm-public cannot run: the LiteLLM catalogue marks it error on route google".to_string(),
            "routing client pi-b: public/bad on litellm-public cannot run: the LiteLLM catalogue marks it error on route google".to_string(),
        ]
    );
    let usable = routing("public/good", "", "");
    assert_eq!(routing_problems(&found, &usable), Vec::<String>::new());
}

#[test]
fn deep_and_adr_slots_count_as_named() {
    let found = catalogue(&[("bad", "error")]);
    let slot = "{\"client\":\"pi-b\",\"role\":\"gap-critic\",\"lenses\":[],\"max_price_usd\":0.3,\"time_limit_s\":240}";
    let deep = routing("public/good", &format!(",\"deep\":[{slot}]"), "");
    assert_eq!(routing_problems(&found, &deep).len(), 1);
    let adr = routing("public/good", &format!(",\"adr\":[{slot}]"), "");
    assert_eq!(routing_problems(&found, &adr).len(), 1);
}

#[test]
fn check_routing_passes_on_the_snapshot_and_fails_on_a_catalogue_that_refuses_a_shipped_model() {
    let runtime = runtime("");
    let passed = run(&json(&format!("{{\"catalogue\":\"{SNAPSHOT}\"}}")), &runtime).expect("a result");
    assert_eq!(passed, json("{\"problems\":[],\"status\":\"passed\"}"));

    let sandbox = Sandbox::new();
    let text = std::fs::read_to_string(SNAPSHOT).expect("snapshot").replace("\"status\": \"usable\"", "\"status\": \"blocked\"");
    let path = format!("{}/blocked.json", sandbox.text());
    std::fs::write(&path, text).expect("write");
    let failed = run(&json(&format!("{{\"catalogue\":\"{path}\"}}")), &runtime).expect("a result");
    let object = failed.as_object().expect("an object");
    assert_eq!(object.get("status").and_then(Value::as_str), Some("failed"));
    assert!(matches!(object.get("problems"), Some(Value::Arr(problems)) if !problems.is_empty()));
}

#[test]
fn check_routing_refuses_input_it_cannot_read() {
    let runtime = runtime("");
    let sandbox = Sandbox::new();
    let broken = format!("{}/broken.json", sandbox.text());
    std::fs::write(&broken, "{").expect("write");
    let not_catalogue = format!("{}/array.json", sandbox.text());
    std::fs::write(&not_catalogue, "[]").expect("write");
    let error = |input: String| run(&json(&input), &runtime).expect_err("an error");
    assert_eq!(error("{}".to_string()).code, "invalid-input");
    assert_eq!(error(format!("{{\"catalogue\":\"{}/missing.json\"}}", sandbox.text())).code, "unexpected");
    assert_eq!(error(format!("{{\"catalogue\":\"{broken}\"}}")).code, "invalid-input");
    assert_eq!(error(format!("{{\"catalogue\":\"{not_catalogue}\"}}")), CritiqueError::new("invalid-input", format!("check-routing input: {not_catalogue}: not a JSON object")));
}

#[test]
fn a_shipped_file_that_is_not_what_it_should_be_is_named() {
    let found = catalogue(&[("good", "usable")]);
    let defaults = model_gateway::SHIPPED_DEFAULTS;
    let routing = model_gateway::SHIPPED_ROUTING;
    assert_eq!(check(&found, "{", routing).expect_err("bad defaults").code, "unexpected");
    assert_eq!(check(&found, defaults, "{").expect_err("bad routing").code, "unexpected");
    assert_eq!(check(&found, defaults, "{}").expect_err("incomplete routing").code, "invalid-routing");
}

/// A catalogue with one route, `openrouter`, probing each model with its free flag, and one `free/<model>` group per
/// model that does not name its access groups, so its probes decide; a model marked `None` has a group but no probe.
fn free_catalogue(models: &[(&str, Option<bool>)]) -> LiteLlmCatalogue {
    let listed: Vec<String> = models
        .iter()
        .filter_map(|(id, free)| free.map(|free| format!("{{\"id\":\"openrouter/{id}\",\"metadata\":{{\"status\":\"usable\",\"free\":{free}}}}}")))
        .collect();
    let groups: Vec<String> = models
        .iter()
        .map(|(id, _)| format!("\"free/{id}\":{{\"public\":true,\"deployments\":[{{\"route\":\"openrouter\",\"upstream_id\":\"{id}\",\"order\":1}}]}}"))
        .collect();
    let text = format!("{{\"updated_at\":\"2026-11-15T21:17:08Z\",\"providers\":{{\"openrouter\":{{\"models\":[{}]}}}},\"groups\":{{{}}}}}", listed.join(","), groups.join(","));
    parse_catalogue(&json(&text)).expect("a catalogue")
}

/// Routing with three free clients, each on a model named after it, in the spec stage's free roster or a fallback.
fn free_routing() -> crate::routing::Routing {
    let client = |model: &str, family: &str| format!("{{\"kind\":\"pi\",\"provider\":\"litellm-free\",\"model\":\"free/{model}\",\"family\":\"{family}\",\"data_scope\":\"public\",\"free\":true}}");
    let slot = |client: &str| format!("{{\"client\":\"{client}\",\"role\":\"gap-critic\",\"lenses\":[],\"max_price_usd\":0.05,\"time_limit_s\":240}}");
    let text = format!(
        "{{\"version\":1,\"clients\":{{\"pi-paid\":{{\"kind\":\"pi\",\"provider\":\"litellm-public\",\"model\":\"public/good\",\"family\":\"google\",\"data_scope\":\"public\"}},\"pi-free\":{},\"pi-billed\":{},\"pi-unprobed\":{}}},\
         \"fallbacks\":{{\"pi-free\":[\"pi-unprobed\"]}},\
         \"stages\":{{\"spec\":{{\"distinct_families\":true,\"base\":[{}],\"free\":[{},{}]}}}}}}",
        client("free", "nvidia"),
        client("billed", "cohere"),
        client("unprobed", "reflection"),
        slot("pi-paid"),
        slot("pi-free"),
        slot("pi-billed"),
    );
    parse_routing(&json(&text)).expect("routing")
}

#[test]
fn a_free_client_must_be_a_model_the_catalogue_calls_free() {
    let found = free_catalogue(&[("free", Some(true)), ("billed", Some(false)), ("unprobed", None)]);
    assert_eq!(
        routing_problems(&found, &free_routing()),
        vec![
            "routing client pi-billed: free/billed on litellm-free is marked free but the LiteLLM catalogue does not mark every deployment of it free".to_string(),
            "routing client pi-unprobed: free/unprobed on litellm-free is marked free but the LiteLLM catalogue has no probe of it, so nothing says it is free".to_string(),
        ]
    );
    // A model the catalogue has no group for is no more free than one it never probed; the paid client's unknown model passes.
    let empty = free_catalogue(&[]);
    assert_eq!(routing_problems(&empty, &free_routing()).len(), 3);
}

#[test]
fn the_shipped_free_clients_are_free_in_the_snapshot() {
    let routing = parse_routing(&json(model_gateway::SHIPPED_ROUTING)).expect("the shipped routing");
    let free: Vec<&str> = routing.clients.iter().filter(|(_, client)| matches!(client, crate::routing::CriticClient::Pi(pi) if pi.free)).map(|(name, _)| name.as_str()).collect();
    assert_eq!(free, ["pi-free-gemini-flash", "pi-free-nemotron-ultra", "pi-free-gemma", "pi-free-north-mini-code", "pi-free-beam"]);
    // Every free client is a free/<model> on the free route, and no other client is on it.
    for (name, client) in &routing.clients {
        if let crate::routing::CriticClient::Pi(pi) = client {
            assert_eq!(pi.free, pi.provider == "litellm-free" && pi.model.starts_with("free/"), "{name}");
        }
    }
    assert!(routing.stages.iter().all(|(_, stage)| !stage.free.is_empty()), "every stage has a free roster");
}
