//! `check-routing`: the shipped profiles and routing against the catalogue snapshot, and what it reports when a
//! roster names a model the catalogue refuses.

use super::cli::runtime;
use super::scenarios::Sandbox;
use crate::check_routing::{profile_problems, routing_problems, run};
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
fn the_snapshot_still_calls_the_retired_gemini_model_unusable() {
    // The reason the profile and the routing no longer name it: if this stops holding, the model may come back.
    let found = snapshot();
    let defaults = json("{\"profiles\":{\"p\":{\"models\":[\"pi-public:public/gemini-3.8-flash\"]}}}");
    assert_eq!(
        profile_problems(&found, &defaults),
        vec!["profile p: pi-public:public/gemini-3.8-flash cannot run: the LiteLLM catalogue marks it error on route google".to_string()]
    );
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
