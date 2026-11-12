//! Tests for the model listing: the catalogue joined to what each client says
//! it offers, the parsers of the clients' own answers, and the provider rules.

use crate::config::{ClientConfig, CostClass, GatewayConfig, MeteredPersonalCaps, ModelEntry, ModelSource, Settings, Tier};
use crate::models::{
    cost_class_for, family_of, family_of_model_id, hermes_default_model, is_provider_allowed, is_vibe_model_id, list_models,
    model_key, model_part, open_code_provider, parse_antigravity_models, parse_opencode_models, parse_vibe_available,
    sorted_clients, strip_ansi, vibe_models, ClientListing, ClientModel, ListedModel, SkippedModel, FAMILY_UNKNOWN,
    NOT_ACCEPTED_BY_CLIENT,
};
use std::collections::{BTreeMap, BTreeSet};

fn client(binary: &str, cost_class: CostClass, model_source: ModelSource) -> ClientConfig {
    ClientConfig {
        binary: binary.to_string(),
        cost_class,
        model_source,
        families: None,
        provider_cost_classes: None,
        providers: None,
        plan_limits: None,
    }
}

fn entry(client: &str, model: &str, family: &str, tier: Tier, notes: &str) -> ModelEntry {
    ModelEntry {
        client: client.to_string(),
        model: model.to_string(),
        names: None,
        family: family.to_string(),
        tier,
        notes: notes.to_string(),
        prices: None,
    }
}

fn named(entry: ModelEntry, names: &[&str]) -> ModelEntry {
    ModelEntry { names: Some(names.iter().map(|name| name.to_string()).collect()), ..entry }
}

fn settings() -> Settings {
    Settings {
        plan_warning_ratio: 0.8,
        summary_max_chars: 1500.0,
        max_parallel_per_client: 3.0,
        max_batch_delegations: 50.0,
        availability_window_minutes: 60.0,
        metered_personal_caps: MeteredPersonalCaps { per_workflow_usd: 2.0, per_day_usd: 5.0 },
        output_allowance_tokens: [500.0, 2000.0, 6000.0, 16000.0],
    }
}

/// Two declared clients, one live, one configured and one live with providers, with models for each.
fn config() -> GatewayConfig {
    let mut clients = BTreeMap::new();
    clients.insert("agy".to_string(), ClientConfig { families: Some(vec!["google".to_string()]), ..client("agy", CostClass::Subsidised, ModelSource::Live) });
    clients.insert("claude".to_string(), client("claude", CostClass::Subsidised, ModelSource::Declared));
    clients.insert("codex".to_string(), client("codex", CostClass::MeteredEmployer, ModelSource::Declared));
    clients.insert("hermes".to_string(), client("hermes", CostClass::MeteredPersonal, ModelSource::Live));
    clients.insert(
        "opencode".to_string(),
        ClientConfig {
            provider_cost_classes: Some(BTreeMap::from([("opencode-go".to_string(), CostClass::Subsidised)])),
            ..client("opencode", CostClass::MeteredPersonal, ModelSource::Live)
        },
    );
    clients.insert("vibe".to_string(), client("vibe", CostClass::Subsidised, ModelSource::Live));
    GatewayConfig {
        version: 1.0,
        clients,
        models: vec![
            entry("claude", "claude-sonnet-5-5", "anthropic", Tier::Hard, "general"),
            entry("claude", "claude-haiku-5-5", "anthropic", Tier::Routine, "cheap"),
            entry("codex", "gpt-5.5", "openai", Tier::Frontier, "implementation"),
            named(entry("vibe", "mistral-large-latest", "mistral", Tier::Hard, "broad"), &["mistral-large-4-0"]),
            named(entry("vibe", "glm-5-3", "zhipu", Tier::Hard, "review"), &["zai-glm-5-3"]),
            entry("agy", "gemini-3.8-flash-high", "google", Tier::Hard, "long context"),
        ],
        settings: settings(),
        disabled: Vec::new(),
    }
}

/// The configuration with one client's settings changed.
fn with_client(base: GatewayConfig, id: &str, patch: impl FnOnce(&mut ClientConfig)) -> GatewayConfig {
    let mut config = base;
    patch(config.clients.get_mut(id).expect("the client"));
    config
}

/// The configuration with the override's `disabled` list set.
fn disabled(base: GatewayConfig, keys: &[&str]) -> GatewayConfig {
    GatewayConfig { disabled: keys.iter().map(|key| key.to_string()).collect(), ..base }
}

/// The listings a client's own source gave, by client id.
fn listings(entries: Vec<(&str, ClientListing)>) -> BTreeMap<String, ClientListing> {
    entries.into_iter().map(|(id, listing)| (id.to_string(), listing)).collect()
}

fn models_of(models: Vec<ClientModel>) -> ClientListing {
    ClientListing::Models(models)
}

fn reason(text: &str) -> ClientListing {
    ClientListing::Reason(text.to_string())
}

fn model(id: &str) -> ClientModel {
    ClientModel { model: id.to_string(), names: Vec::new(), provider: None }
}

fn provider_model(id: &str, provider: &str) -> ClientModel {
    ClientModel { model: id.to_string(), names: Vec::new(), provider: Some(provider.to_string()) }
}

fn named_model(id: &str, names: &[&str]) -> ClientModel {
    ClientModel { model: id.to_string(), names: names.iter().map(|name| name.to_string()).collect(), provider: None }
}

fn installed(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| id.to_string()).collect()
}

fn listed_model(
    client: &str,
    model: &str,
    family: &str,
    cost_class: CostClass,
    tier: Option<Tier>,
    notes: &str,
    source: ModelSource,
) -> ListedModel {
    ListedModel {
        client: client.to_string(),
        model: model.to_string(),
        family: family.to_string(),
        cost_class,
        tier,
        notes: notes.to_string(),
        source,
    }
}

fn skipped(client: &str, model: &str, reason: &str) -> SkippedModel {
    SkippedModel { client: client.to_string(), model: model.to_string(), reason: reason.to_string() }
}

/// The Vibe models its own answer listed, leaving the catalogue's built-ins aside.
fn probed(gateway: &GatewayConfig, listed: &BTreeMap<String, ClientListing>) -> Vec<ListedModel> {
    list_models(gateway, &installed(&["vibe"]), listed, &[]).models.into_iter().filter(|entry| entry.client == "vibe").collect()
}

/// A listed model as the tuple a test compares.
fn row(entry: &ListedModel) -> (&str, &str, &str, CostClass, Option<Tier>, &str, ModelSource) {
    (&entry.client, &entry.model, &entry.family, entry.cost_class, entry.tier, &entry.notes, entry.source)
}

#[test]
fn list_models_lists_a_model_a_listing_names_twice_once() {
    let listed = listings(vec![(
        "opencode",
        models_of(vec![provider_model("opencode-go/glm-5.3", "opencode-go"), provider_model("opencode-go/glm-5.3", "opencode-go")]),
    )]);

    assert_eq!(list_models(&config(), &installed(&["opencode"]), &listed, &[]).models.len(), 1);
}

#[test]
fn list_models_lists_an_accepted_vibe_alias_the_catalogue_does_not_know() {
    let listed = listings(vec![("vibe", models_of(vec![model("zai-glm-6")]))]);

    let found: Vec<(String, String)> = probed(&config(), &listed).into_iter().map(|entry| (entry.model, entry.family)).collect();
    assert_eq!(found, [("zai-glm-6".to_string(), "zhipu".to_string())]);
}

#[test]
fn list_models_lists_the_declared_models_of_an_installed_client_with_its_cost_class_and_source() {
    let listing = list_models(&config(), &installed(&["claude"]), &listings(vec![]), &[]);

    assert_eq!(
        listing.models,
        vec![
            listed_model("claude", "claude-haiku-5-5", "anthropic", CostClass::Subsidised, Some(Tier::Routine), "cheap", ModelSource::Declared),
            listed_model("claude", "claude-sonnet-5-5", "anthropic", CostClass::Subsidised, Some(Tier::Hard), "general", ModelSource::Declared),
        ]
    );
}

#[test]
fn list_models_leaves_out_a_client_that_is_not_installed_and_names_it_with_the_reason() {
    let listing = list_models(&config(), &installed(&["claude"]), &listings(vec![]), &[]);

    let missing: Vec<(&str, &str)> = listing.missing.iter().map(|entry| (entry.client.as_str(), entry.reason.as_str())).collect();
    assert_eq!(
        missing,
        [
            ("agy", "not installed: agy is not on PATH"),
            ("codex", "not installed: codex is not on PATH"),
            ("hermes", "not installed: hermes is not on PATH"),
            ("opencode", "not installed: opencode is not on PATH"),
            ("vibe", "not installed: vibe is not on PATH"),
        ]
    );
}

#[test]
fn list_models_lists_nothing_for_an_installed_client_whose_listing_the_gateway_does_not_have() {
    let listing = list_models(&config(), &installed(&["hermes"]), &listings(vec![]), &[]);

    assert_eq!(listing.models, vec![]);
    let missing: Vec<&str> = listing.missing.iter().map(|entry| entry.client.as_str()).collect();
    assert_eq!(missing, ["agy", "claude", "codex", "opencode", "vibe"]);
}

#[test]
fn list_models_joins_a_live_listing_to_the_catalogue_and_marks_it_live() {
    let listed = listings(vec![("agy", models_of(vec![model("gemini-3.8-flash-high")]))]);

    let listing = list_models(&config(), &installed(&["agy"]), &listed, &[]);
    assert_eq!(
        listing.models,
        vec![listed_model("agy", "gemini-3.8-flash-high", "google", CostClass::Subsidised, Some(Tier::Hard), "long context", ModelSource::Live)]
    );
}

#[test]
fn list_models_lists_a_model_the_catalogue_does_not_know_with_no_tier_and_no_notes() {
    let listed = listings(vec![("agy", models_of(vec![model("gemini-4-pro")]))]);

    let listing = list_models(&config(), &installed(&["agy"]), &listed, &[]);
    assert_eq!(
        listing.models,
        vec![listed_model("agy", "gemini-4-pro", "google", CostClass::Subsidised, None, "", ModelSource::Live)]
    );
}

#[test]
fn list_models_leaves_out_a_model_of_a_family_the_client_may_not_reach() {
    let listed = listings(vec![(
        "agy",
        models_of(vec![model("gemini-4-pro"), model("claude-sonnet-4-6"), model("gpt-oss-120b-medium")]),
    )]);

    let listing = list_models(&config(), &installed(&["agy"]), &listed, &[]);
    let models: Vec<&str> = listing.models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(models, ["gemini-4-pro"]);
}

#[test]
fn list_models_reports_every_model_whose_family_it_cannot_work_out_in_skipped() {
    let listed = listings(vec![(
        "opencode",
        models_of(vec![provider_model("opencode-go/longcat-2.0", "opencode-go"), provider_model("opencode-go/hy3", "opencode-go")]),
    )]);

    let listing = list_models(&config(), &installed(&["opencode"]), &listed, &[]);
    assert_eq!(listing.models, vec![]);
    assert_eq!(
        listing.skipped,
        vec![skipped("opencode", "opencode-go/hy3", FAMILY_UNKNOWN), skipped("opencode", "opencode-go/longcat-2.0", FAMILY_UNKNOWN)]
    );
}

#[test]
fn list_models_takes_the_cost_class_of_the_provider_a_model_runs_on() {
    let listed = listings(vec![(
        "opencode",
        models_of(vec![provider_model("opencode-go/glm-5.3", "opencode-go"), provider_model("other/gemini-4-argon", "other")]),
    )]);
    let named = with_client(config(), "opencode", |client| client.providers = Some(vec!["opencode-go".to_string(), "other".to_string()]));

    let listing = list_models(&named, &installed(&["opencode"]), &listed, &[]);
    let models: Vec<(&str, CostClass)> = listing.models.iter().map(|entry| (entry.model.as_str(), entry.cost_class)).collect();
    assert_eq!(models, [("opencode-go/glm-5.3", CostClass::Subsidised), ("other/gemini-4-argon", CostClass::MeteredPersonal)]);
}

#[test]
fn list_models_lists_the_aliases_vibe_itself_accepts_marked_live() {
    let listed = listings(vec![("vibe", models_of(vec![model("mistral-large-latest"), model("mistral-medium-4-1")]))]);

    assert_eq!(
        probed(&config(), &listed),
        vec![
            listed_model("vibe", "mistral-large-latest", "mistral", CostClass::Subsidised, Some(Tier::Hard), "broad", ModelSource::Live),
            listed_model("vibe", "mistral-medium-4-1", "mistral", CostClass::Subsidised, None, "", ModelSource::Live),
        ]
    );
}

#[test]
fn list_models_joins_a_vibe_model_to_the_catalogue_on_a_name_and_lists_it_once_by_its_alias() {
    let listed = listings(vec![(
        "vibe",
        models_of(vec![named_model("glm-5-3", &["zai-glm-5-3"]), named_model("glm-5-3", &["zai-glm-5-3"])]),
    )]);

    assert_eq!(
        probed(&config(), &listed),
        vec![listed_model("vibe", "glm-5-3", "zhipu", CostClass::Subsidised, Some(Tier::Hard), "review", ModelSource::Live)]
    );
}

#[test]
fn list_models_joins_a_vibe_model_on_its_name_when_its_alias_is_not_the_catalogue_id() {
    let listed = listings(vec![("vibe", models_of(vec![named_model("large", &["mistral-large-4-0"])]))]);

    let found = probed(&config(), &listed);
    let rows: Vec<(&str, Option<Tier>, &str)> =
        found.iter().map(|entry| (entry.model.as_str(), entry.tier, entry.notes.as_str())).collect();
    assert_eq!(rows, [("large", Some(Tier::Hard), "broad")]);
}

#[test]
fn list_models_joins_a_vibe_model_on_an_alias_the_catalogue_lists_among_its_names() {
    let listed = listings(vec![("vibe", models_of(vec![model("zai-glm-5-3")]))]);

    let found = probed(&config(), &listed);
    let rows: Vec<(&str, Option<Tier>)> = found.iter().map(|entry| (entry.model.as_str(), entry.tier)).collect();
    assert_eq!(rows, [("zai-glm-5-3", Some(Tier::Hard))]);
}

#[test]
fn list_models_lists_a_vibe_model_whose_alias_and_names_both_miss_the_catalogue() {
    let listed = listings(vec![("vibe", models_of(vec![named_model("fast", &["mistral-medium-4-1"])]))]);

    let found = probed(&config(), &listed);
    let rows: Vec<(&str, &str, Option<Tier>, &str)> = found
        .iter()
        .map(|entry| (entry.model.as_str(), entry.family.as_str(), entry.tier, entry.notes.as_str()))
        .collect();
    assert_eq!(rows, [("fast", "mistral", None, "")]);
}

#[test]
fn list_models_reports_a_vibe_model_whose_alias_and_names_name_no_family() {
    let listed = listings(vec![("vibe", models_of(vec![named_model("fast", &["mystery"])]))]);

    let listing = list_models(&config(), &installed(&["vibe"]), &listed, &[]);
    assert_eq!(
        listing.skipped,
        vec![
            skipped("vibe", "fast", FAMILY_UNKNOWN),
            skipped("vibe", "glm-5-3", NOT_ACCEPTED_BY_CLIENT),
            skipped("vibe", "mistral-large-latest", NOT_ACCEPTED_BY_CLIENT),
        ]
    );
}

#[test]
fn list_models_leaves_out_a_vibe_model_whose_alias_or_catalogue_id_the_override_disabled() {
    let listed = listings(vec![(
        "vibe",
        models_of(vec![named_model("glm-5-3", &["zai-glm-5-3"]), model("mistral-large-latest")]),
    )]);
    let by_alias = disabled(config(), &[&model_key("vibe", "glm-5-3")]);
    let by_name = disabled(config(), &[&model_key("vibe", "mistral-large-latest")]);

    let by_alias_models = probed(&by_alias, &listed);
    let aliased: Vec<&str> = by_alias_models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(aliased, ["mistral-large-latest"]);
    let by_name_models = probed(&by_name, &listed);
    let named: Vec<&str> = by_name_models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(named, ["glm-5-3"]);
}

#[test]
fn list_models_leaves_out_a_vibe_model_whose_recorded_catalogue_name_the_override_disabled() {
    let listed = listings(vec![("vibe", models_of(vec![named_model("glm-5-3", &["zai-glm-5-3"])]))]);
    let off = disabled(config(), &[&model_key("vibe", "zai-glm-5-3")]);

    assert_eq!(probed(&off, &listed), vec![]);
}

#[test]
fn list_models_hides_a_listed_model_a_ledger_answer_refused_under_another_name() {
    // The ledger recorded the answer under `mistral-large-4-0`, a name of the
    // catalogue entry `mistral-large-latest`, so the answer hides both.
    let listed = listings(vec![("vibe", models_of(vec![model("mistral-large-latest")]))]);
    let refused = vec![skipped("vibe", "mistral-large-4-0", "rate-limited at 2026-11-09T10:00:00.000Z")];

    let listing = list_models(&config(), &installed(&["vibe"]), &listed, &refused);
    assert_eq!(listing.models, vec![]);
    assert_eq!(
        listing.skipped,
        vec![
            skipped("vibe", "glm-5-3", NOT_ACCEPTED_BY_CLIENT),
            skipped("vibe", "mistral-large-latest", "rate-limited at 2026-11-09T10:00:00.000Z"),
        ]
    );
}

#[test]
fn list_models_leaves_out_a_model_the_override_disabled() {
    let off = disabled(config(), &[&model_key("claude", "claude-haiku-5-5"), &model_key("agy", "gemini-4-pro")]);
    let listed = listings(vec![("agy", models_of(vec![model("gemini-4-pro")]))]);

    let listing = list_models(&off, &installed(&["agy", "claude"]), &listed, &[]);
    let models: Vec<&str> = listing.models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(models, ["claude-sonnet-5-5"]);
}

#[test]
fn list_models_reports_a_listing_that_failed_in_missing_with_its_reason() {
    let listed = listings(vec![("agy", reason("listing failed: agy models exited 1"))]);

    let listing = list_models(&config(), &installed(&["agy", "claude", "codex", "hermes", "opencode", "vibe"]), &listed, &[]);
    assert_eq!(listing.missing, vec![crate::models::MissingClient { client: "agy".to_string(), reason: "listing failed: agy models exited 1".to_string() }]);
    let models: Vec<&str> = listing.models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(models, ["claude-haiku-5-5", "claude-sonnet-5-5", "gpt-5.5"]);
}

#[test]
fn list_models_sorts_by_client_then_model_id() {
    let listed = listings(vec![
        ("agy", models_of(vec![model("gemini-4-pro"), model("gemini-3.8-flash-high")])),
        ("vibe", models_of(vec![model("glm-5-3"), model("mistral-large-latest")])),
    ]);

    let listing = list_models(&config(), &installed(&["agy", "claude", "codex", "vibe"]), &listed, &[]);
    let keys: Vec<String> = listing.models.iter().map(|entry| model_key(&entry.client, &entry.model)).collect();
    assert_eq!(
        keys,
        [
            "agy:gemini-3.8-flash-high",
            "agy:gemini-4-pro",
            "claude:claude-haiku-5-5",
            "claude:claude-sonnet-5-5",
            "codex:gpt-5.5",
            "vibe:glm-5-3",
            "vibe:mistral-large-latest",
        ]
    );
}

#[test]
fn family_of_names_the_family_a_model_id_starts_with() {
    let cases = [
        ("gemini-3.8-flash-high", "google"),
        ("claude-sonnet-4-6", "anthropic"),
        ("gpt-oss-120b-medium", "openai"),
        ("deepseek-v4.1-flash", "deepseek"),
        ("glm-5.3", "zhipu"),
        ("zai-glm-5-3", "zhipu"),
        ("zai-glm-6", "zhipu"),
        ("kimi-k3", "moonshot"),
        ("minimax-m2", "minimax"),
        ("grok-4.7", "xai"),
        ("qwen3-coder", "alibaba"),
        ("mistral-large-4-0", "mistral"),
        ("codestral-2508", "mistral"),
    ];
    for (model, family) in cases {
        assert_eq!(family_of(model), Some(family), "{model}");
    }
    assert_eq!(family_of("hy3"), None);
    assert_eq!(family_of("longcat-2.0"), None);
}

#[test]
fn family_of_model_id_names_the_family_of_the_model_part() {
    assert_eq!(family_of_model_id("anthropic/claude-sonnet-4.6"), Some("anthropic"));
    assert_eq!(family_of_model_id("google/gemini-3.8-flash-high"), Some("google"));
    assert_eq!(family_of_model_id("glm-5-3"), Some("zhipu"));
    assert_eq!(family_of_model_id("anthropic/hy3"), None);
}

#[test]
fn hermes_default_model_reads_the_model_and_provider_its_configuration_names() {
    assert_eq!(
        hermes_default_model("{\"default\":\"anthropic/claude-sonnet-4.6\",\"provider\":\"anthropic\"}"),
        Some(ClientModel { model: "anthropic/claude-sonnet-4.6".to_string(), names: vec![], provider: Some("anthropic".to_string()) })
    );
    assert_eq!(
        hermes_default_model("{\"default\":\"glm-5.3\"}"),
        Some(ClientModel { model: "glm-5.3".to_string(), names: vec![], provider: None })
    );
    assert_eq!(
        hermes_default_model("\"glm-5.3\""),
        Some(ClientModel { model: "glm-5.3".to_string(), names: vec![], provider: None })
    );
}

#[test]
fn hermes_default_model_says_nothing_when_the_configuration_names_no_model() {
    assert_eq!(hermes_default_model("{}"), None);
    assert_eq!(hermes_default_model("{\"default\":\"\"}"), None);
    assert_eq!(hermes_default_model("{\"default\":\"glm-5.3\",\"provider\":\"\"}").map(|model| model.provider), Some(None));
    assert_eq!(hermes_default_model("\"\""), None);
    assert_eq!(hermes_default_model("not json"), None);
    assert_eq!(hermes_default_model("[]"), None);
    assert_eq!(hermes_default_model("1"), None);
}

#[test]
fn parse_antigravity_models_reads_one_model_per_line_by_its_id() {
    let stdout = "Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\nclaude-sonnet-4-6\tClaude Sonnet 4.6\n";
    assert_eq!(parse_antigravity_models(stdout), ["gemini-3.8-flash-high", "claude-sonnet-4-6"]);
}

#[test]
fn parse_antigravity_models_reads_a_listing_that_ends_without_a_newline_and_skips_lines_that_name_no_model() {
    assert_eq!(parse_antigravity_models("Fetching available models...\ngemini-4-pro\tGemini 4 Pro"), ["gemini-4-pro"]);
    assert_eq!(parse_antigravity_models("Fetching available models...\n\n\t\n"), Vec::<String>::new());
}

#[test]
fn parse_opencode_models_reads_one_provider_model_per_line() {
    assert_eq!(
        parse_opencode_models("opencode-go/glm-5.3\nother/gemini-4-argon\n"),
        vec![provider_model("opencode-go/glm-5.3", "opencode-go"), provider_model("other/gemini-4-argon", "other")]
    );
}

#[test]
fn parse_opencode_models_skips_a_line_that_names_no_provider_or_no_model() {
    assert_eq!(parse_opencode_models("\nno-provider\n/provider-only\nprovider/\n"), vec![]);
}

#[test]
fn vibe_models_reads_no_model_from_an_entry_that_is_not_a_table() {
    assert_eq!(vibe_models("models = { oops = \"x\" }\n"), Ok(vec![]));
}

#[test]
fn vibe_models_reads_the_alias_and_name_of_every_model_the_configuration_declares() {
    let toml = "[models.mistral-large-4-0]\nname = \"mistral-large-4-0\"\nprovider = \"mistral\"\nalias = \"mistral-large-latest\"\n\n[models.zai-glm-5-3]\nname = \"zai-glm-5-3\"\nprovider = \"mistral\"\nalias = \"glm-5-3\"\n";

    assert_eq!(
        vibe_models(toml),
        Ok(vec![
            ClientModel { model: "mistral-large-latest".to_string(), names: vec!["mistral-large-4-0".to_string()], provider: None },
            ClientModel { model: "glm-5-3".to_string(), names: vec!["zai-glm-5-3".to_string()], provider: None },
        ])
    );
}

#[test]
fn vibe_models_names_a_model_by_its_alias_alone_when_the_alias_is_its_name() {
    assert_eq!(
        vibe_models("models = { small = { name = \"small\", alias = \"small\", provider = \"mistral\" } }\n"),
        Ok(vec![ClientModel { model: "small".to_string(), names: vec![], provider: None }])
    );
}

#[test]
fn vibe_models_reads_the_models_an_array_of_tables_declares() {
    let toml = "[[models]]\nname = \"mistral-large-4-0-3\"\nprovider = \"mistral\"\nalias = \"mistral-large-4-0-3\"\n\n[[models]]\nname = \"glm-5-3\"\nprovider = \"mistral\"\nalias = \"glm-5-3\"\n\n[[models]]\nprovider = \"mistral\"\n";

    assert_eq!(
        vibe_models(toml),
        Ok(vec![
            ClientModel { model: "mistral-large-4-0-3".to_string(), names: vec![], provider: None },
            ClientModel { model: "glm-5-3".to_string(), names: vec![], provider: None },
        ])
    );
}

#[test]
fn vibe_models_declares_no_model_an_array_entry_names_no_alias_for() {
    let toml = "[[models]]\nname = \"mistral-large-latest\"\nprovider = \"mistral\"\n\n[[models]]\nprovider = \"mistral\"\n";
    assert_eq!(vibe_models(toml), Ok(vec![]));
}

#[test]
fn vibe_models_reads_an_inline_models_table_and_names_a_model_without_an_alias_by_its_key() {
    assert_eq!(
        vibe_models("models = { small = { name = \"small-1\", provider = \"mistral\" } }\n"),
        Ok(vec![ClientModel { model: "small".to_string(), names: vec!["small-1".to_string()], provider: None }])
    );
    assert_eq!(
        vibe_models("models = { small = { provider = \"mistral\" } }\n"),
        Ok(vec![ClientModel { model: "small".to_string(), names: vec![], provider: None }])
    );
    assert_eq!(
        vibe_models("models = { small = { alias = \"\", name = \"small-1\" } }\n"),
        Ok(vec![ClientModel { model: "small".to_string(), names: vec!["small-1".to_string()], provider: None }])
    );
    assert_eq!(
        vibe_models("models = { small = { name = \"\" } }\n"),
        Ok(vec![ClientModel { model: "small".to_string(), names: vec![], provider: None }])
    );
}

#[test]
fn vibe_models_reads_no_model_from_a_configuration_that_declares_none() {
    assert_eq!(vibe_models("active_model = \"mistral-large-4-0\"\n"), Ok(vec![]));
    assert_eq!(vibe_models("models = \"not a table\"\n"), Ok(vec![]));
}

#[test]
fn vibe_models_fails_on_a_file_that_is_not_toml() {
    assert!(vibe_models("this is not TOML\n").is_err());
}

#[test]
fn model_key_joins_a_client_and_a_model_id() {
    assert_eq!(model_key("codex", "gpt-5.5"), "codex:gpt-5.5");
}

#[test]
fn list_models_offers_none_of_the_catalogue_entries_when_vibe_accepts_nothing() {
    let listing = list_models(&config(), &installed(&["vibe"]), &listings(vec![("vibe", models_of(vec![]))]), &[]);

    assert_eq!(listing.models, vec![]);
    assert_eq!(
        listing.skipped,
        vec![skipped("vibe", "glm-5-3", NOT_ACCEPTED_BY_CLIENT), skipped("vibe", "mistral-large-latest", NOT_ACCEPTED_BY_CLIENT)]
    );
    assert_eq!(listing.missing.iter().filter(|entry| entry.client == "vibe").count(), 0);
}

#[test]
fn list_models_lists_an_accepted_model_once_marked_live_and_skips_the_entries_its_answer_did_not_name() {
    let listed = listings(vec![("vibe", models_of(vec![named_model("glm-5-3", &["zai-glm-5-3"])]))]);

    let listing = list_models(&config(), &installed(&["vibe"]), &listed, &[]);
    let models: Vec<(&str, ModelSource)> = listing.models.iter().map(|entry| (entry.model.as_str(), entry.source)).collect();
    assert_eq!(models, [("glm-5-3", ModelSource::Live)]);
    assert_eq!(listing.skipped, vec![skipped("vibe", "mistral-large-latest", NOT_ACCEPTED_BY_CLIENT)]);
}

#[test]
fn list_models_lists_an_accepted_alias_that_names_a_catalogue_entry_once_under_the_alias() {
    let listed = listings(vec![("vibe", models_of(vec![named_model("large", &["mistral-large-4-0"])]))]);

    let listing = list_models(&config(), &installed(&["vibe"]), &listed, &[]);
    let models: Vec<(&str, ModelSource)> = listing.models.iter().map(|entry| (entry.model.as_str(), entry.source)).collect();
    assert_eq!(models, [("large", ModelSource::Live)]);
    assert_eq!(listing.skipped, vec![skipped("vibe", "glm-5-3", NOT_ACCEPTED_BY_CLIENT)]);
}

#[test]
fn list_models_does_not_bring_back_a_catalogue_entry_whose_accepted_model_the_override_disabled() {
    let listed = listings(vec![("vibe", models_of(vec![named_model("large", &["mistral-large-4-0"])]))]);
    let off = disabled(config(), &[&model_key("vibe", "large")]);

    let listing = list_models(&off, &installed(&["vibe"]), &listed, &[]);
    assert_eq!(listing.models, vec![]);
    assert_eq!(listing.skipped, vec![skipped("vibe", "glm-5-3", NOT_ACCEPTED_BY_CLIENT)]);
}

#[test]
fn list_models_leaves_out_a_catalogue_entry_the_override_disabled_offered_or_skipped() {
    let off = disabled(config(), &[&model_key("vibe", "zai-glm-5-3")]);

    let listing = list_models(&off, &installed(&["vibe"]), &listings(vec![("vibe", models_of(vec![]))]), &[]);
    assert_eq!(listing.models, vec![]);
    assert_eq!(listing.skipped, vec![skipped("vibe", "mistral-large-latest", NOT_ACCEPTED_BY_CLIENT)]);
}

#[test]
fn list_models_offers_no_vibe_model_when_its_own_answer_could_not_be_read() {
    let listed = listings(vec![("vibe", reason("listing failed: vibe --model model-gateway-probe -p ping exited 1: no such file"))]);

    let listing = list_models(&config(), &installed(&["vibe"]), &listed, &[]);
    assert_eq!(listing.models, vec![]);
    assert_eq!(listing.skipped.iter().filter(|entry| entry.client == "vibe").count(), 0);
    assert_eq!(
        listing.missing.iter().filter(|entry| entry.client == "vibe").collect::<Vec<_>>(),
        vec![&crate::models::MissingClient {
            client: "vibe".to_string(),
            reason: "listing failed: vibe --model model-gateway-probe -p ping exited 1: no such file".to_string(),
        }]
    );
}

#[test]
fn list_models_lists_no_model_for_a_client_without_built_in_models_when_its_listing_cannot_be_read() {
    let listing = list_models(&config(), &installed(&["agy"]), &listings(vec![("agy", reason("listing failed"))]), &[]);

    assert_eq!(listing.models, vec![]);
    let missing: Vec<(&str, &str)> = listing
        .missing
        .iter()
        .filter(|entry| entry.client == "agy")
        .map(|entry| (entry.client.as_str(), entry.reason.as_str()))
        .collect();
    assert_eq!(missing, [("agy", "listing failed")]);
}

#[test]
fn list_models_applies_the_client_family_restriction_to_the_built_in_models() {
    let restricted = with_client(config(), "vibe", |client| client.families = Some(vec!["mistral".to_string()]));

    let listing = list_models(&restricted, &installed(&["vibe"]), &listings(vec![("vibe", models_of(vec![]))]), &[]);
    assert_eq!(listing.models, vec![]);
    assert_eq!(listing.skipped, vec![skipped("vibe", "mistral-large-latest", NOT_ACCEPTED_BY_CLIENT)]);
}

#[test]
fn list_models_leaves_out_an_accepted_model_of_a_barred_family_without_leaking_its_catalogue_entry() {
    let restricted = with_client(config(), "vibe", |client| client.families = Some(vec!["mistral".to_string()]));
    let mut gateway = restricted;
    gateway.models.push(entry("vibe", "glm-5-3-extra", "zhipu", Tier::Hard, "x"));
    let listed = listings(vec![("vibe", models_of(vec![named_model("glm", &["zai-glm-5-3"])]))]);

    let listing = list_models(&gateway, &installed(&["vibe"]), &listed, &[]);
    assert_eq!(listing.models, vec![]);
    assert_eq!(listing.skipped, vec![skipped("vibe", "mistral-large-latest", NOT_ACCEPTED_BY_CLIENT)]);
}

#[test]
fn list_models_lists_no_catalogue_entry_for_a_client_that_has_no_built_in_models() {
    let base = with_client(config(), "hermes", |client| client.model_source = ModelSource::Config);
    let mut hermes = base;
    hermes.models.push(entry("hermes", "claude-sonnet-5-5", "anthropic", Tier::Hard, "x"));
    let listed = listings(vec![("hermes", models_of(vec![model("gpt-5.5")]))]);

    let listing = list_models(&hermes, &installed(&["hermes"]), &listed, &[]);
    let models: Vec<(&str, ModelSource)> = listing.models.iter().map(|entry| (entry.model.as_str(), entry.source)).collect();
    assert_eq!(models, [("gpt-5.5", ModelSource::Config)]);
}

#[test]
fn parse_vibe_available_reads_the_aliases_from_the_refusal_of_a_model_id_vibe_does_not_know() {
    assert_eq!(
        parse_vibe_available("Error: Unknown model: model-gateway-probe. Available: glm-5-3, mistral-large-4-0-3, mistral-medium-3.5, zai-glm-5-3-flash\n"),
        Some(vec![
            "glm-5-3".to_string(),
            "mistral-large-4-0-3".to_string(),
            "mistral-medium-3.5".to_string(),
            "zai-glm-5-3-flash".to_string(),
        ])
    );
}

#[test]
fn parse_vibe_available_tolerates_the_connection_warnings_printed_before_the_refusal() {
    let output = [
        "Warning: MCP server memory failed to connect",
        "Warning: MCP server fetch failed to connect",
        "Error: Unknown model: zz. Available: glm-5-3, mistral-medium-3.5",
        "",
    ]
    .join("\n");

    assert_eq!(parse_vibe_available(&output), Some(vec!["glm-5-3".to_string(), "mistral-medium-3.5".to_string()]));
}

#[test]
fn parse_vibe_available_keeps_the_ids_of_the_last_refusal_only() {
    assert_eq!(
        parse_vibe_available("Available: one\nAvailable: two, three\n"),
        Some(vec!["two".to_string(), "three".to_string()])
    );
}

#[test]
fn parse_vibe_available_reads_the_aliases_through_the_terminal_colours() {
    let output = "\u{1b}[31mError:\u{1b}[0m Unknown model: probe. \u{1b}[1mAvailable:\u{1b}[0m \u{1b}[32mglm-5-3\u{1b}[0m, \u{1b}[32mmistral-medium-3.5\u{1b}[0m\n";

    assert_eq!(parse_vibe_available(output), Some(vec!["glm-5-3".to_string(), "mistral-medium-3.5".to_string()]));
}

#[test]
fn parse_vibe_available_drops_the_full_stop_of_a_refusal_that_ends_as_a_sentence() {
    assert_eq!(
        parse_vibe_available("Unknown model: probe. Available: glm-5-3, mistral-medium-3.5."),
        Some(vec!["glm-5-3".to_string(), "mistral-medium-3.5".to_string()])
    );
}

#[test]
fn parse_vibe_available_tolerates_the_sentence_that_follows_the_list() {
    assert_eq!(
        parse_vibe_available("Error: Unknown model: model-gateway-probe. Available: glm-5-3, mistral-medium-3.5. Please check the configuration.\n"),
        Some(vec!["glm-5-3".to_string(), "mistral-medium-3.5".to_string()])
    );
}

#[test]
fn parse_vibe_available_reads_the_aliases_from_a_leading_newline() {
    let stdout = "Error: Unknown model: model-gateway-probe. Available: glm-5-3, mistral-medium-3.5\n";
    assert_eq!(parse_vibe_available(&format!("\n{stdout}")), Some(vec!["glm-5-3".to_string(), "mistral-medium-3.5".to_string()]));
}

#[test]
fn parse_vibe_available_fails_closed_on_an_id_that_is_not_one_shape() {
    assert_eq!(parse_vibe_available("Available: glm-5-3, --danger-file\n"), None);
    assert_eq!(parse_vibe_available("Available: glm-5-3, two words\n"), None);
    assert_eq!(parse_vibe_available("Available: glm-5-3, , mistral-medium-3.5\n"), None);
}

#[test]
fn parse_vibe_available_fails_closed_on_an_answer_that_names_nothing_available() {
    assert_eq!(parse_vibe_available(""), None);
    assert_eq!(parse_vibe_available("Warning: MCP server memory failed to connect\nsomething went wrong\n"), None);
    assert_eq!(parse_vibe_available("Error: Unknown model: probe. Available:\n"), None);
    assert_eq!(parse_vibe_available("Error: Unknown model: probe. Available: \n"), None);
}

#[test]
fn strip_ansi_removes_the_escapes_a_terminal_answer_carries() {
    assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
    assert_eq!(strip_ansi("\u{1b}[1;38;5;9mx\u{1b}[?25hy"), "xy");
    assert_eq!(strip_ansi("\u{1b}[0 qx"), "x");
    assert_eq!(strip_ansi("\u{1b}]0;a title\u{7}text"), "text");
    assert_eq!(strip_ansi("\u{1b}]0;a title\u{1b}\\text"), "text");
    assert_eq!(strip_ansi("\u{1b}]a\u{1b}\\b\u{7}c"), "c");
}

#[test]
fn strip_ansi_keeps_an_escape_it_cannot_read() {
    assert_eq!(strip_ansi("\u{1b}]0;a title"), "\u{1b}]0;a title");
    assert_eq!(strip_ansi("\u{1b}[31"), "\u{1b}[31");
    assert_eq!(strip_ansi("\u{1b}X"), "\u{1b}X");
    assert_eq!(strip_ansi("plain"), "plain");
}

#[test]
fn is_vibe_model_id_holds_a_model_id_to_one_shape() {
    assert!(is_vibe_model_id("glm-5-3"));
    assert!(is_vibe_model_id("mistral-large-4-0-3"));
    assert!(is_vibe_model_id("a"));
    assert!(!is_vibe_model_id(""));
    assert!(!is_vibe_model_id("-glm"));
    assert!(!is_vibe_model_id("two words"));
    assert!(!is_vibe_model_id("glm/5"));
}

#[test]
fn list_models_lists_only_the_models_of_the_default_opencode_provider() {
    let mixed = models_of(vec![
        provider_model("opencode-go/glm-5.3", "opencode-go"),
        provider_model("opencode/glm-5.3", "opencode"),
        provider_model("other/gemini-4-argon", "other"),
        provider_model("openrouter/hy3", "openrouter"),
    ]);

    let listing = list_models(&config(), &installed(&["opencode"]), &listings(vec![("opencode", mixed)]), &[]);
    let models: Vec<&str> = listing.models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(models, ["opencode-go/glm-5.3"]);
    assert_eq!(listing.skipped, vec![]);
    assert_eq!(listing.missing.iter().filter(|entry| entry.client == "opencode").count(), 0);
}

#[test]
fn list_models_lists_the_models_of_every_provider_the_client_names_but_never_the_pay_per_token_one() {
    let named = with_client(config(), "opencode", |client| {
        client.providers = Some(vec!["opencode-go".to_string(), "other".to_string(), "opencode".to_string()])
    });
    let mixed = models_of(vec![
        provider_model("opencode-go/glm-5.3", "opencode-go"),
        provider_model("opencode/glm-5.3", "opencode"),
        provider_model("other/gemini-4-argon", "other"),
        provider_model("openrouter/hy3", "openrouter"),
    ]);

    let listing = list_models(&named, &installed(&["opencode"]), &listings(vec![("opencode", mixed)]), &[]);
    let models: Vec<&str> = listing.models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(models, ["opencode-go/glm-5.3", "other/gemini-4-argon"]);
}

#[test]
fn list_models_drops_an_opencode_model_that_names_no_provider() {
    let listed = listings(vec![("opencode", models_of(vec![model("glm-5.3")]))]);

    assert_eq!(list_models(&config(), &installed(&["opencode"]), &listed, &[]).models, vec![]);
}

#[test]
fn list_models_lists_an_opencode_model_whose_provider_only_its_id_names() {
    let listed = listings(vec![("opencode", models_of(vec![model("opencode-go/glm-5.3")]))]);

    let listing = list_models(&config(), &installed(&["opencode"]), &listed, &[]);
    let models: Vec<(&str, CostClass)> = listing.models.iter().map(|entry| (entry.model.as_str(), entry.cost_class)).collect();
    assert_eq!(models, [("opencode-go/glm-5.3", CostClass::Subsidised)]);
}

#[test]
fn open_code_provider_is_the_part_of_the_model_id_before_its_first_slash() {
    assert_eq!(open_code_provider("opencode-go/glm-5-3"), Some("opencode-go".to_string()));
    assert_eq!(open_code_provider("openrouter/z-ai/glm-5"), Some("openrouter".to_string()));
    assert_eq!(open_code_provider("glm-5-3"), None);
    assert_eq!(open_code_provider("/glm-5-3"), None);
    assert_eq!(open_code_provider("opencode-go/"), None);
}

#[test]
fn is_provider_allowed_names_the_provider_the_machine_names_and_opencode_go_by_default() {
    assert!(is_provider_allowed(Some("opencode-go"), None));
    assert!(is_provider_allowed(Some("zai"), Some(&["zai".to_string()])));
    assert!(!is_provider_allowed(Some("zai"), None));
    assert!(!is_provider_allowed(Some("opencode-go"), Some(&["zai".to_string()])));
    assert!(!is_provider_allowed(None, None));
}

#[test]
fn is_provider_allowed_is_never_the_pay_per_token_provider() {
    assert!(!is_provider_allowed(Some("opencode"), Some(&["opencode".to_string(), "opencode-go".to_string()])));
}

#[test]
fn model_part_is_what_follows_the_last_slash() {
    assert_eq!(model_part("opencode-go/glm-5.3"), "glm-5.3");
    assert_eq!(model_part("openrouter/z-ai/glm-5"), "glm-5");
    assert_eq!(model_part("glm-5-3"), "glm-5-3");
}

#[test]
fn cost_class_for_takes_the_provider_class_else_the_clients_own() {
    let plain = client("opencode", CostClass::MeteredPersonal, ModelSource::Live);
    assert_eq!(cost_class_for(&plain, None), CostClass::MeteredPersonal);
    assert_eq!(cost_class_for(&plain, Some("opencode-go")), CostClass::MeteredPersonal);
    let mapped = ClientConfig {
        provider_cost_classes: Some(BTreeMap::from([("opencode-go".to_string(), CostClass::Subsidised)])),
        ..plain
    };
    assert_eq!(cost_class_for(&mapped, Some("opencode-go")), CostClass::Subsidised);
    assert_eq!(cost_class_for(&mapped, Some("other")), CostClass::MeteredPersonal);
}

#[test]
fn sorted_clients_gives_every_client_in_client_order() {
    let config = config();
    let ids: Vec<&str> = sorted_clients(&config).into_iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["agy", "claude", "codex", "hermes", "opencode", "vibe"]);
}

#[test]
fn a_listed_model_carries_the_fields_the_listing_prints() {
    let listed = listings(vec![("agy", models_of(vec![model("gemini-3.8-flash-high")]))]);
    let listing = list_models(&config(), &installed(&["agy"]), &listed, &[]);

    assert_eq!(
        listing.models.iter().map(row).collect::<Vec<_>>(),
        [("agy", "gemini-3.8-flash-high", "google", CostClass::Subsidised, Some(Tier::Hard), "long context", ModelSource::Live)]
    );
}
