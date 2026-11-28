//! What the goldens cannot reach in the LiteLLM catalogue reader: a machine with no home to find the
//! catalogue in, and a file system that cannot even look at the path.

use super::config::{runtime, StubFiles, StubGit};
use crate::litellm_catalogue::{read_catalogue, require_usable, CatalogueReading};
use crate::ports::FsError;
use std::collections::HashMap;

#[test]
fn no_home_and_no_absolute_cache_home_reads_no_catalogue_and_says_nothing() {
    let env = HashMap::from([("XDG_CACHE_HOME".to_string(), "relative".to_string())]);
    let reading = read_catalogue(&runtime(StubFiles::new(), StubGit::new(None, &[]), env));
    assert_eq!(reading, CatalogueReading::default());
}

#[test]
fn a_path_the_files_cannot_look_at_is_a_warning() {
    let mut files = StubFiles::new();
    files.exists_error = Some(FsError { code: "EACCES".into(), message: "EACCES: permission denied".into() });
    let env = HashMap::from([("XDG_CACHE_HOME".to_string(), "/cache".to_string())]);
    let reading = read_catalogue(&runtime(files, StubGit::new(None, &[]), env));
    assert_eq!(reading.catalogue, None);
    assert_eq!(
        reading.warning.as_deref(),
        Some("the LiteLLM catalogue at /cache/agent-kit/litellm-catalogue.json cannot be read, so the models are listed unchecked")
    );
}

#[test]
fn a_client_with_no_route_is_never_refused_on_the_catalogue() {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), super::config::home_env("/home/someone"));
    let config = crate::config_parse::load_config(&runtime);
    let config = config.expect("the shipped defaults load");
    assert!(require_usable(&runtime, &config, "claude", "claude-opus-5-5").is_ok());
    assert!(require_usable(&runtime, &config, "nobody", "x").is_ok());
}

fn catalogue_of(models: &[(&str, &str)], groups: &str) -> crate::litellm_catalogue::LiteLlmCatalogue {
    let listed: Vec<String> = models.iter().map(|(id, meta)| format!("{{\"id\":\"opencode-go/{id}\",\"metadata\":{meta}}}")).collect();
    let text = format!("{{\"updated_at\":\"2026-11-03T11:17:01Z\",\"providers\":{{\"opencode-go\":{{\"models\":[{}]}}}},\"groups\":{groups}}}", listed.join(","));
    crate::litellm_catalogue::parse_catalogue(&crate::json::parse(&text).expect("json")).expect("catalogue")
}

fn shipped() -> crate::config::GatewayConfig {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), super::config::home_env("/home/someone"));
    crate::config_parse::load_config(&runtime).expect("the shipped defaults load")
}

fn usable(extra: &str) -> String {
    format!("{{\"status\":\"usable\",\"free\":false,\"modality\":\"chat\"{extra}}}")
}

#[test]
fn a_model_with_no_defaults_entry_takes_its_family_from_its_canonical_id_and_its_tier_from_its_name() {
    use crate::config::Tier;
    use crate::litellm_catalogue::place_from_catalogue;
    let config = shipped();
    let catalogue = catalogue_of(
        &[
            ("zzz", &usable(",\"canonical_id\":\"moonshotai/zzz\"")),
            ("vendorless", &usable(",\"canonical_id\":\"newco/vendorless\"")),
            ("tiny-lite", &usable(",\"canonical_id\":\"acme/tiny-lite\",\"context_length\":8000")),
            ("big-pro", &usable("")),
            ("9", &usable("")),
            ("claude-fast", &usable("")),
            ("note", "{\"status\":\"usable\",\"free\":false,\"modality\":\"embedding\"}"),
        ],
        "{\"g\":{\"deployments\":[{\"route\":\"opencode-go\",\"upstream_id\":\"zzz\",\"order\":1}]}}",
    );
    let place = |model: &str| place_from_catalogue(&config, &catalogue, "opencode", model);
    let zzz = place("opencode-go/zzz").expect("known").expect("chat");
    assert_eq!((zzz.family.as_str(), zzz.tier), ("moonshot", Tier::Routine));
    assert_eq!(place("opencode-go/vendorless").expect("known").expect("chat").family, "newco");
    // A group that names no modality takes its deployment's.
    let grouped = place_from_catalogue(&config, &catalogue, "pi-opencode-go", "g").expect("known").expect("chat");
    assert_eq!(grouped.family, "moonshot");
    let tiny = place("opencode-go/tiny-lite").expect("known").expect("chat");
    assert_eq!((tiny.family.as_str(), tiny.tier), ("acme", Tier::Trivial));
    assert!(tiny.notes.contains("8000 tokens of context"));
    let big = place("opencode-go/big-pro").expect("known").expect("chat");
    assert_eq!((big.family.as_str(), big.tier), ("big", Tier::Hard));
    assert_eq!(place("opencode-go/9").expect("known").expect("chat").family, "unknown");
    assert_eq!(place("opencode-go/claude-fast").expect("known").expect("chat").family, "anthropic");
    assert_eq!(place("opencode-go/note"), Some(Err("the LiteLLM catalogue lists it as embedding, not a chat model".to_string())));
    assert_eq!(place("opencode-go/never-heard-of"), None);
}

#[test]
fn a_skipped_model_the_client_may_not_reach_or_nobody_knows_stays_out_of_the_listing() {
    use crate::config::ModelSource;
    use crate::litellm_catalogue::judge_listing;
    use crate::models::{ModelListing, SkippedModel, FAMILY_UNKNOWN};
    let mut config = shipped();
    let skipped = |client: &str, model: &str| SkippedModel { client: client.into(), model: model.into(), reason: FAMILY_UNKNOWN.into() };
    let catalogue = catalogue_of(&[("hy3", &usable("")), ("kimi-x", &usable(""))], "{}");
    let listing = || ModelListing { models: vec![], missing: vec![], skipped: vec![skipped("opencode", "opencode-go/hy3"), skipped("nobody", "opencode-go/hy3")] };
    let judged = judge_listing(&config, listing(), Some(&catalogue));
    // OpenCode lists it on the subsidised provider, with the tier its name suggests.
    assert_eq!(judged.models.len(), 1);
    assert_eq!((judged.models[0].client.as_str(), judged.models[0].source), ("opencode", ModelSource::Live));
    assert_eq!(judged.models[0].cost_class, crate::config::CostClass::Subsidised);
    assert_eq!(judged.skipped, vec![skipped("nobody", "opencode-go/hy3")]);
    // A client restricted to other families never sees it.
    config.clients.get_mut("opencode").expect("client").families = Some(vec!["google".to_string()]);
    let barred = judge_listing(&config, listing(), Some(&catalogue));
    assert!(barred.models.is_empty());
    // Skipped for another reason, it stays skipped for that reason.
    let other = ModelListing { skipped: vec![SkippedModel { reason: "other".into(), ..skipped("opencode", "opencode-go/hy3") }], ..listing() };
    assert_eq!(judge_listing(&config, other, Some(&catalogue)).skipped[0].reason, "other");
}

#[test]
fn a_run_on_a_model_only_the_catalogue_knows_is_placed_or_refused_with_the_reason() {
    use crate::litellm_catalogue::place_unlisted;
    let config = shipped();
    let catalogue = "{\"updated_at\":\"2026-11-03T11:17:01Z\",\"providers\":{\"opencode-go\":{\"models\":[\
        {\"id\":\"opencode-go/beam-free\",\"metadata\":{\"status\":\"usable\",\"free\":true,\"modality\":\"chat\",\"canonical_id\":\"reflection/beam-1.0-501b-a23b\"}},\
        {\"id\":\"opencode-go/embed-1\",\"metadata\":{\"status\":\"usable\",\"free\":false,\"modality\":\"embedding\"}}]}},\
        \"groups\":{\"beam-1.0-501b-a23b\":{\"modality\":\"chat\",\"deployments\":[{\"route\":\"opencode-go\",\"upstream_id\":\"beam-free\",\"order\":1}]}}}";
    let path = "/cache/agent-kit/litellm-catalogue.json";
    let env = HashMap::from([("XDG_CACHE_HOME".to_string(), "/cache".to_string())]);
    let with = |files: StubFiles| runtime(files, StubGit::new(None, &[]), env.clone());
    let known = with(StubFiles::new().with(path, catalogue));
    assert_eq!(place_unlisted(&known, &config, "pi-opencode-go", "beam-1.0-501b-a23b"), Ok("reflection".to_string()));
    assert_eq!(place_unlisted(&known, &config, "opencode", "opencode-go/beam-free"), Ok("reflection".to_string()));
    let embedding = place_unlisted(&known, &config, "opencode", "opencode-go/embed-1").expect_err("not chat");
    assert_eq!(embedding.code, "unknown-model");
    assert!(embedding.message.ends_with("the LiteLLM catalogue lists it as embedding, not a chat model"), "{}", embedding.message);
    let alias = place_unlisted(&known, &config, "pi-opencode-go", "beam-free").expect_err("an upstream id");
    assert!(alias.message.ends_with("machine override; Pi names it beam-1.0-501b-a23b, the group that serves it"), "{}", alias.message);
    let nowhere = place_unlisted(&known, &config, "pi-opencode-go", "nothing").expect_err("unknown");
    assert!(nowhere.message.ends_with("machine override"), "{}", nowhere.message);
    let unread = place_unlisted(&with(StubFiles::new()), &config, "pi-opencode-go", "beam-free").expect_err("no catalogue");
    assert!(unread.message.ends_with("machine override"), "{}", unread.message);
}

#[test]
fn a_listed_model_carries_its_cost_and_the_providers_that_serve_it_in_fallback_order() {
    use crate::json::{parse, Value};
    use crate::model_facts::annotate;
    let config = shipped();
    let catalogue = crate::litellm_catalogue::parse_catalogue(
        &parse(
            r#"{"updated_at":"2026-11-03T11:17:01Z","providers":{},"groups":{"glm-5.3":{"deployments":[
                {"route":"opencode-go","upstream_id":"glm-5.3","order":2},{"route":"vibe","upstream_id":"zai-glm-5-3","order":1}]}}}"#,
        )
        .expect("json"),
    )
    .expect("catalogue");
    let listed = parse(
        r#"{"status":"ok","models":[
            {"client":"pi-vibe","model":"glm-5.3","cost_class":"subsidised"},
            {"client":"pi-vibe","model":"free-one","cost_class":"free","free":true},
            {"client":"pi-vibe","model":"nobody-knows","cost_class":"subsidised"},
            {"client":"codex","model":"gpt-5.5","cost_class":"metered-employer"},
            {"client":"opencode","model":"opencode-go/kimi-k2.7-code","cost_class":"subsidised"},
            {"client":"claude","model":"claude-opus-5-5","cost_class":"subsidised"},
            7,
            {"client":"pi-auto","model":"auto/glm-5.3","cost_class":"subsidised"}]}"#,
    )
    .expect("json");
    let out = annotate(&config, Some(&catalogue), listed.clone());
    let models = match out.as_object().and_then(|object| object.get("models")) {
        Some(Value::Arr(models)) => models.clone(),
        _ => panic!("models"),
    };
    let field = |index: usize, key: &str| crate::json::compact(models[index].as_object().and_then(|object| object.get(key)).expect(key));
    // A Pi group takes the price of the model its first deployment serves, and lists every deployment in order.
    assert_eq!(field(0, "cost"), r#"{"class":"subsidised","input_per_mtok":1.4,"output_per_mtok":4.4}"#);
    assert_eq!(
        field(0, "providers"),
        r#"[{"provider":"vibe","upstream_id":"zai-glm-5-3","route":"vibe"},{"provider":"opencode-go","upstream_id":"glm-5.3","route":"vibe"}]"#
    );
    // Free says zero, explicitly; a model nobody prices says unknown, and one no group serves has no provider.
    assert_eq!(field(1, "cost"), r#"{"class":"free","input_per_mtok":0,"output_per_mtok":0}"#);
    assert_eq!(field(2, "cost"), r#"{"class":"subsidised","input_per_mtok":"unknown","output_per_mtok":"unknown"}"#);
    assert_eq!(field(2, "providers"), "[]");
    // Another client's model is priced by its own defaults entry and served by its route or its model's provider.
    assert_eq!(field(3, "cost"), r#"{"class":"metered-employer","input_per_mtok":1.25,"output_per_mtok":10}"#);
    assert_eq!(field(3, "providers"), r#"[{"provider":"codex","upstream_id":"gpt-5.5","route":"codex"}]"#);
    assert_eq!(field(4, "providers"), r#"[{"provider":"opencode-go","upstream_id":"opencode-go/kimi-k2.7-code","route":"opencode-go"}]"#);
    assert_eq!(field(5, "cost"), r#"{"class":"subsidised","input_per_mtok":"unknown","output_per_mtok":"unknown"}"#);
    // An auto group tries the free copies and then the paid ones, and says so; its client's class still decides the caps.
    assert!(field(7, "cost").starts_with(r#"{"class":"free, then paid","#), "{}", field(7, "cost"));
    // Without a catalogue a Pi model has no provider to name; a result that lists no models, or an odd entry, is left as it is.
    let bare = annotate(&config, None, listed);
    assert!(crate::json::compact(&bare.as_object().and_then(|object| object.get("models")).map(|models| match models { Value::Arr(items) => items[0].clone(), _ => Value::Null }).expect("first")).contains(r#""providers":[]"#));
    assert_eq!(annotate(&config, None, Value::Null), Value::Null);
    assert_eq!(crate::json::compact(&annotate(&config, None, parse(r#"{"status":"ok"}"#).expect("json"))), r#"{"status":"ok"}"#);
}

#[test]
fn an_opencode_model_on_a_litellm_provider_is_judged_by_the_group_a_pi_route_names() {
    let config = shipped();
    let catalogue = "{\"updated_at\":\"2026-11-03T11:17:01Z\",\"providers\":{\"opencode-go\":{\"models\":[\
        {\"id\":\"opencode-go/beam-free\",\"metadata\":{\"status\":\"blocked\",\"reason\":\"trains on request data\",\"free\":true,\"modality\":\"chat\"}}]}},\
        \"groups\":{\"beam\":{\"modality\":\"chat\",\"deployments\":[{\"route\":\"opencode-go\",\"upstream_id\":\"beam-free\",\"order\":1}]}}}";
    let env = HashMap::from([("XDG_CACHE_HOME".to_string(), "/cache".to_string())]);
    let known = runtime(StubFiles::new().with("/cache/agent-kit/litellm-catalogue.json", catalogue), StubGit::new(None, &[]), env);
    let refused = require_usable(&known, &config, "opencode", "litellm-opencode-go/beam").expect_err("the group is blocked");
    assert_eq!(refused.code, "model-not-usable");
    assert!(require_usable(&known, &config, "opencode", "litellm-opencode-go/unknown-group").is_ok());
}
