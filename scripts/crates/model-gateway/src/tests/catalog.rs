//! The catalog view: one row per canonical model, with its cost, its deployments and what can run it.

use super::config::{runtime, StubFiles, StubGit};
use super::scenarios::{FixedClock, Sandbox, FIXED_NOW};
use crate::adapters::fs::SystemFiles;
use crate::adapters::git::{SystemGit, TIMEOUT_MS};
use crate::adapters::process::SystemProcess;
use crate::catalog::{build, provider_name, read_filter, Filter};
use crate::cli::run_cli;
use crate::commands::Gathered;
use crate::config::{CostClass, ModelSource, Tier};
use crate::json::{parse, Value};
use crate::litellm_catalogue::{parse_catalogue, CatalogueReading};
use crate::models::{ListedModel, MissingClient, SkippedModel};
use crate::policy::{PermittedModel, PolicyListing, SendPermission};
use crate::ports::{LockTimings, Runtime};
use std::collections::HashMap;

const CATALOGUE: &str = r#"{"updated_at":"2026-11-03T11:17:01Z","providers":{
  "opencode-go":{"metadata":{"key_env":"OPENCODE_GO_API_KEY"},"models":[
    {"id":"opencode-go/glm-5.3","metadata":{"status":"usable","free":false,"modality":"chat","context_length":200000}},
    {"id":"opencode-go/beam-free","metadata":{"status":"usable","free":true,"modality":"chat","canonical_id":"reflection/beam"}}]},
  "vibe":{"metadata":{"key_env":"VIBE_API_KEY"},"models":[
    {"id":"vibe/zai-glm-5-3","metadata":{"status":"usable","free":false,"modality":"chat"}},
    {"id":"vibe/embed-1","metadata":{"status":"not-chat","reason":"an embedding model","free":false,"modality":"embedding"}}]},
  "nokey":{"models":[]}},
 "groups":{
  "glm-5.3":{"modality":"chat","deployments":[{"route":"opencode-go","upstream_id":"glm-5.3","order":2},{"route":"vibe","upstream_id":"zai-glm-5-3","order":1}]},
  "embed-1":{"modality":"embedding","deployments":[{"route":"vibe","upstream_id":"embed-1","order":1}]},
  "gone":{"deployments":[{"route":"nokey","upstream_id":"nope","order":1}]},
  "beam":{"deployments":[{"route":"opencode-go","upstream_id":"beam-free","order":1}]},
  "claude-opus-5-5":{"modality":"chat","deployments":[{"route":"anthropic","upstream_id":"claude-opus-5-5","order":1}]},
  "claude-sonnet-5-5":{"modality":"chat","deployments":[{"route":"anthropic","upstream_id":"claude-sonnet-5-5","order":1}]}}}"#;

fn config() -> crate::config::GatewayConfig {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), super::config::home_env("/home/someone"));
    crate::config_parse::load_config(&runtime).expect("the shipped defaults load")
}

fn listed(client: &str, model: &str, family: &str, class: CostClass, tier: Option<Tier>, free: bool) -> PermittedModel {
    let listed = ListedModel {
        client: client.into(),
        model: model.into(),
        family: family.into(),
        cost_class: class,
        tier,
        notes: String::new(),
        source: ModelSource::Live,
        free,
    };
    PermittedModel { listed, inline_only: false, may_send: SendPermission { code: true, history: true } }
}

fn skipped(client: &str, model: &str, reason: &str) -> SkippedModel {
    SkippedModel { client: client.into(), model: model.into(), reason: reason.into() }
}

fn gathered(catalogue: Option<&str>, profiles: Value) -> Gathered {
    let catalogue = catalogue.map(|text| parse_catalogue(&parse(text).expect("json")).expect("catalogue"));
    let permitted = PolicyListing {
        models: vec![
            listed("pi-vibe", "glm-5.3", "zhipu", CostClass::Subsidised, Some(Tier::Routine), false),
            listed("pi-opencode-go", "glm-5.3", "zhipu", CostClass::Subsidised, Some(Tier::Routine), false),
            listed("claude", "claude-opus-5-5", "anthropic", CostClass::Subsidised, Some(Tier::Frontier), false),
            listed("codex", "gpt-5.5", "openai", CostClass::MeteredEmployer, None, false),
            listed("pi-free", "free-one", "acme", CostClass::Free, Some(Tier::Trivial), true),
            listed("pi-vibe", "free-one", "acme", CostClass::Free, Some(Tier::Trivial), true),
        ],
        missing: vec![MissingClient { client: "hermes".into(), reason: "no model".into() }],
        skipped: vec![
            skipped("pi-vibe", "embed-1", "r1"),
            skipped("pi-opencode-go", "embed-1", "r1"),
            skipped("pi-public", "embed-1", "r2"),
            skipped("hermes", "mystery", "no"),
        ],
    };
    Gathered {
        config: config(),
        reading: CatalogueReading { catalogue, warning: Some("stale".into()) },
        permitted,
        profiles,
        policy_reason: "policy".into(),
    }
}

fn profiles() -> Value {
    parse(
        r#"[{"name":"b-profile","models":[{"client":"pi-vibe","model":"glm-5.3"}]},
            {"name":"a-profile","models":[{"client":"pi-opencode-go","model":"glm-5.3"},{"client":"claude","model":"claude-opus-5-5"}]},
            {"name":"other","models":[{"client":"pi-vibe","model":"unrelated"}]},
            {"name":"empty"},7]"#,
    )
    .expect("json")
}

fn row<'a>(view: &'a crate::catalog::View, model: &str) -> &'a crate::catalog::Row {
    view.rows.iter().find(|row| row.model == model).unwrap_or_else(|| panic!("no row {model}"))
}

#[test]
fn a_group_row_says_what_serves_it_in_fallback_order_what_it_costs_and_what_runs_it() {
    let view = build(&gathered(Some(CATALOGUE), profiles()), &Filter::default());
    let glm = row(&view, "glm-5.3");
    assert_eq!((glm.modality.as_str(), glm.family.as_str(), glm.tier, glm.context_length), ("chat", "zhipu", Some(Tier::Routine), Some(200000)));
    assert_eq!(crate::json::compact(&glm.cost), r#"{"class":"subsidised","input_per_mtok":1.4,"output_per_mtok":4.4}"#);
    let served: Vec<(i64, &str, &str, Option<&str>, &str)> =
        glm.deployments.iter().map(|d| (d.order, d.provider.as_str(), d.route.as_str(), d.key.as_deref(), d.status.as_str())).collect();
    assert_eq!(served, vec![(1, "Mistral", "vibe", Some("VIBE_API_KEY"), "usable"), (2, "OpenCode Go", "opencode-go", Some("OPENCODE_GO_API_KEY"), "usable")]);
    assert_eq!(glm.clients, vec!["pi-opencode-go", "pi-vibe"]);
    assert_eq!(glm.profiles, vec!["a-profile", "b-profile"]);
    assert_eq!((glm.status.as_str(), glm.reason.as_str(), glm.free), ("usable", "", false));
    // A model another client lists by the id a deployment serves it under joins the group; its tier is the client's.
    let opus = row(&view, "claude-opus-5-5");
    assert_eq!((opus.clients.clone(), opus.tier, opus.family.as_str()), (vec!["claude".to_string()], Some(Tier::Frontier), "anthropic"));
    assert_eq!(opus.profiles, vec!["a-profile"]);
    assert_eq!(opus.deployments[0].key, None);
    // A group no client lists takes its family and tier from the defaults that name it.
    let sonnet = row(&view, "claude-sonnet-5-5");
    assert_eq!((sonnet.family.as_str(), sonnet.tier, sonnet.clients.is_empty()), ("anthropic", Some(Tier::Hard), true));
    assert_eq!(view.updated_at.as_deref(), Some("2026-11-03T11:17:01Z"));
    assert_eq!((view.warnings.clone(), view.policy.as_str()), (vec!["stale".to_string()], "policy"));
    assert_eq!(view.missing, vec![("hermes".to_string(), "no model".to_string())]);
}

#[test]
fn a_model_that_cannot_run_is_a_row_with_the_providers_reason_and_every_clients_reason() {
    let view = build(&gathered(Some(CATALOGUE), profiles()), &Filter::default());
    let embed = row(&view, "embed-1");
    assert_eq!((embed.modality.as_str(), embed.status.as_str(), embed.tier), ("embedding", "not-chat", None));
    assert_eq!(embed.reason, "an embedding model; pi-vibe, pi-opencode-go: r1; pi-public: r2");
    assert!(embed.clients.is_empty());
    // A group the catalogue holds no probe for is unprobed, with no modality or tier to claim.
    let gone = row(&view, "gone");
    assert_eq!((gone.modality.as_str(), gone.status.as_str(), gone.tier, gone.family.as_str()), ("unknown", "unprobed", None, "gone"));
    assert_eq!((gone.deployments[0].key.as_deref(), gone.deployments[0].reason.as_str()), (None, "the catalogue holds no probe for it"));
    assert_eq!(gone.deployments[0].provider, "nokey");
}

#[test]
fn free_is_the_catalogues_flag_or_a_clients_and_a_free_group_says_zero_explicitly() {
    let view = build(&gathered(Some(CATALOGUE), profiles()), &Filter::default());
    let beam = row(&view, "beam");
    assert!(beam.free);
    assert_eq!(crate::json::compact(&beam.cost), r#"{"class":"free","input_per_mtok":0,"output_per_mtok":0}"#);
    assert_eq!(beam.family, "reflection");
    assert!(row(&view, "free-one").free);
}

#[test]
fn a_model_no_group_covers_is_a_row_of_its_own_and_so_is_one_that_was_skipped() {
    let view = build(&gathered(Some(CATALOGUE), Value::Null), &Filter::default());
    let gpt = row(&view, "gpt-5.5");
    assert_eq!((gpt.status.as_str(), gpt.modality.as_str(), gpt.clients.clone(), gpt.family.as_str()), ("usable", "chat", vec!["codex".to_string()], "openai"));
    assert_eq!((gpt.deployments[0].provider.as_str(), gpt.deployments[0].status.as_str(), gpt.deployments[0].order), ("codex", "unprobed", 1));
    assert!(gpt.profiles.is_empty());
    assert_eq!(crate::json::compact(&gpt.cost), r#"{"class":"metered-employer","input_per_mtok":1.25,"output_per_mtok":10}"#);
    // One row for a model two clients list; a Pi group nobody serves has no provider to name.
    let free = row(&view, "free-one");
    assert_eq!(free.clients, vec!["pi-free", "pi-vibe"]);
    assert!(free.deployments.is_empty());
    let mystery = row(&view, "mystery");
    assert_eq!((mystery.status.as_str(), mystery.modality.as_str(), mystery.reason.as_str(), mystery.family.as_str()), ("unusable", "unknown", "hermes: no", "mystery"));
    assert!(mystery.deployments.is_empty() && mystery.clients.is_empty() && mystery.tier.is_none());
}

#[test]
fn without_a_catalogue_every_model_is_a_row_the_clients_gave() {
    let view = build(&gathered(None, profiles()), &Filter::default());
    let models: Vec<&str> = view.rows.iter().map(|row| row.model.as_str()).collect();
    assert_eq!(models, vec!["claude-opus-5-5", "embed-1", "free-one", "glm-5.3", "gpt-5.5", "mystery"]);
    assert_eq!(view.updated_at, None);
}

#[test]
fn filters_keep_the_rows_of_a_route_a_status_a_modality_or_the_free_ones() {
    let all = gathered(Some(CATALOGUE), profiles());
    let names = |filter: Filter| build(&all, &filter).rows.iter().map(|row| row.model.clone()).collect::<Vec<_>>();
    assert_eq!(names(Filter { route: Some("opencode-go".into()), ..Filter::default() }), vec!["beam", "glm-5.3"]);
    // A client's route counts, with or without the `pi-` prefix; so does the client itself.
    assert_eq!(names(Filter { route: Some("free".into()), ..Filter::default() }), vec!["free-one"]);
    assert_eq!(names(Filter { route: Some("codex".into()), ..Filter::default() }), vec!["gpt-5.5"]);
    assert_eq!(names(Filter { status: Some("not-chat".into()), ..Filter::default() }), vec!["embed-1"]);
    assert_eq!(names(Filter { status: Some("unprobed".into()), ..Filter::default() }).len(), 4);
    assert_eq!(names(Filter { modality: Some("embedding".into()), ..Filter::default() }), vec!["embed-1"]);
    assert_eq!(names(Filter { free: true, ..Filter::default() }), vec!["beam", "free-one"]);
    assert!(names(Filter { route: Some("nowhere".into()), ..Filter::default() }).is_empty());
}

#[test]
fn a_filter_reads_its_fields_and_refuses_the_wrong_kind() {
    let read = |text: &str| match parse(text).expect("json") {
        Value::Obj(object) => read_filter(&object),
        _ => unreachable!(),
    };
    assert_eq!(read("{}").expect("empty"), Filter::default());
    let all = read(r#"{"route":"r","free":true,"status":"s","modality":"m"}"#).expect("all");
    assert_eq!((all.route.as_deref(), all.free, all.status.as_deref(), all.modality.as_deref()), (Some("r"), true, Some("s"), Some("m")));
    for text in [r#"{"free":"yes"}"#, r#"{"route":""}"#, r#"{"status":3}"#, r#"{"modality":false}"#] {
        assert_eq!(read(text).expect_err("refused").code, "invalid-input", "{text}");
    }
}

#[test]
fn a_route_is_named_by_the_provider_it_reaches() {
    let named: Vec<String> = ["vibe", "vibe-study-api", "google", "anthropic", "openrouter", "opencode-go", "local", "other"].map(provider_name).to_vec();
    assert_eq!(named, ["Mistral", "Mistral", "Google", "Anthropic", "OpenRouter", "OpenCode Go", "local", "other"]);
}

#[test]
fn the_table_has_a_line_per_model_then_its_deployments_clients_and_reason() {
    let table = build(&gathered(Some(CATALOGUE), profiles()), &Filter::default()).to_table();
    assert!(table.starts_with("MODEL  "), "{table}");
    for expected in [
        "glm-5.3  ",
        "subsidised 1.4/4.4",
        "    1. Mistral via route vibe, key VIBE_API_KEY as zai-glm-5-3 [usable]\n",
        "    2. OpenCode Go via route opencode-go, key OPENCODE_GO_API_KEY as glm-5.3 [usable]\n",
        "    runs on: pi-opencode-go, pi-vibe; profiles: a-profile, b-profile\n",
        "    runs on: nothing; profiles: none\n",
        "    1. Mistral via route vibe, key VIBE_API_KEY as embed-1 [not-chat: an embedding model]\n",
        "    reason: an embedding model; pi-vibe, pi-opencode-go: r1; pi-public: r2\n",
        "200000",
        "free 0/0",
        "missing client hermes: no model\n",
        "warning: stale\n",
        "9 models; policy: policy\n",
    ] {
        assert!(table.contains(expected), "{expected}\n{table}");
    }
    let unknown = build(&gathered(Some(CATALOGUE), profiles()), &Filter { route: Some("codex".into()), ..Filter::default() }).to_table();
    assert!(unknown.contains("metered-employer 1.25/10"), "{unknown}");
}

pub(super) fn sandbox_runtime(sandbox: &Sandbox, catalogue: Option<&str>) -> Runtime {
    let root = sandbox.text();
    std::fs::create_dir_all(format!("{root}/home/.cache/agent-kit")).expect("cache");
    if let Some(text) = catalogue {
        std::fs::write(format!("{root}/home/.cache/agent-kit/litellm-catalogue.json"), text).expect("catalogue");
    }
    let env = HashMap::from([
        ("HOME".to_string(), format!("{root}/home")),
        ("XDG_STATE_HOME".to_string(), format!("{root}/state")),
        ("PATH".to_string(), String::new()),
    ]);
    Runtime {
        clock: Box::new(FixedClock(FIXED_NOW)),
        files: Box::new(SystemFiles { cwd: root.clone() }),
        lock: LockTimings::default(),
        process: Box::new(SystemProcess { env: env.clone() }),
        git: Box::new(SystemGit { env: env.clone(), timeout_ms: TIMEOUT_MS }),
        env,
        defaults: crate::SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    }
}

pub(super) fn cli(runtime: &Runtime, args: &[&str]) -> crate::cli::CliResult {
    run_cli(&args.iter().map(|word| word.to_string()).collect::<Vec<_>>(), runtime)
}

#[test]
fn the_command_prints_a_table_by_default_and_one_json_object_with_json() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, Some(CATALOGUE));
    let table = cli(&runtime, &["catalog", "--route", "opencode-go"]);
    assert_eq!(table.exit_code, 0, "{}", table.stderr);
    assert!(table.stdout.contains("glm-5.3") && table.stdout.contains("    1. Mistral via route vibe"), "{}", table.stdout);
    assert!(table.stdout.contains("missing client pi-vibe"), "{}", table.stdout);
    let json = cli(&runtime, &["catalog", "--json", "--status", "not-chat", "--modality", "embedding", "--free", "--repo", "/nowhere/at/all"]);
    assert_eq!(json.exit_code, 1, "a repository path git cannot read is an error: {}", json.stdout);
    let chat = cli(&runtime, &["catalog", "--json", "--route", "opencode-go", "--modality", "chat"]);
    assert!(chat.stdout.contains("\"tier\": \"routine\"") && chat.stdout.contains("\"context_length\": 200000"), "{}", chat.stdout);
    let json = cli(&runtime, &["catalog", "--json", "--status", "not-chat", "--modality", "embedding"]);
    assert_eq!(json.exit_code, 0, "{}", json.stderr);
    let parsed = parse(&json.stdout).expect("json");
    let models = match parsed.as_object().and_then(|object| object.get("models")) {
        Some(Value::Arr(models)) => models.clone(),
        _ => panic!("models"),
    };
    assert_eq!(models.len(), 1);
    assert_eq!(parsed.as_object().and_then(|object| object.get("status")).and_then(Value::as_str), Some("ok"));
    let entry = models[0].as_object().expect("a model");
    for key in ["model", "modality", "family", "tier", "context_length", "free", "cost", "status", "reason", "deployments", "clients", "profiles"] {
        assert!(entry.get(key).is_some(), "{key}");
    }
}

#[test]
fn the_command_works_without_a_catalogue_and_refuses_flags_it_does_not_have() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, None);
    let bare = cli(&runtime, &["catalog"]);
    assert_eq!(bare.exit_code, 0, "{}", bare.stderr);
    assert!(bare.stdout.contains("0 models; policy: no repository"), "{}", bare.stdout);
    for args in [&["catalog", "--bogus"][..], &["catalog", "--route"][..], &["catalog", "extra"][..]] {
        let refused = cli(&runtime, args);
        assert_eq!(refused.exit_code, 1);
        assert!(refused.stdout.contains("catalog takes [--repo PATH]"), "{}", refused.stdout);
    }
    let mut wrong = crate::json::Object::new();
    wrong.set("free", Value::str("yes"));
    assert_eq!(crate::catalog::view(&wrong, &runtime).expect_err("a filter of the wrong kind").code, "invalid-input");
    let bad = crate::commands::run(crate::commands::Command::Catalog, "catalog", &crate::json::Object::new(), &runtime).expect("the command runs");
    assert!(crate::json::compact(&bad).starts_with(r#"{"status":"ok","policy":"#));
}
