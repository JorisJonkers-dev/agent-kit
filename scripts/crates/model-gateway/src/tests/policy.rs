//! Tests for the data policy: the decision per client model, the listing it
//! permits, the scope a repository gives, and the JSON the `models` command
//! prints.

use super::config::{home_env, runtime, StubFiles, StubGit};
use crate::allowlist::AllowlistEntry;
use crate::config::{CostClass, GatewayConfig, ModelSource, Tier};
use crate::config_parse::parse_defaults;
use crate::error::GatewayError;
use crate::json::{parse, pretty, Value};
use crate::models::{ListedModel, MissingClient, ModelListing, SkippedModel};
use crate::policy::{decide_policy, models_result_json, permitted_model_json, permitted_models, resolve_scope, DataScope, PolicyDecision, PolicySubject};
use std::collections::HashMap;

/// Four clients, one of each cost class plus a configured one, with a model each.
const CONFIG: &str = r#"{
  "version": 1,
  "clients": {
    "claude": {"binary": "claude", "cost_class": "subsidised", "model_source": "declared"},
    "codex": {"binary": "codex", "cost_class": "metered-employer", "model_source": "declared"},
    "hermes": {"binary": "hermes", "cost_class": "metered-personal", "model_source": "declared"},
    "vibe": {"binary": "vibe", "cost_class": "subsidised", "model_source": "declared"}
  },
  "models": [
    {"client": "claude", "family": "anthropic", "model": "claude-sonnet-5-5", "notes": "general", "tier": "hard"},
    {"client": "codex", "family": "openai", "model": "gpt-5.5", "notes": "implementation", "tier": "frontier"},
    {"client": "hermes", "family": "mistral", "model": "hermes-4", "notes": "metered", "tier": "routine"},
    {"client": "vibe", "family": "zhipu", "model": "zai-glm-5-3", "notes": "review", "tier": "hard"}
  ],
  "settings": {
    "availability_window_minutes": 60,
    "max_batch_delegations": 50,
    "max_parallel_per_client": 3,
    "metered_personal_caps": {"per_day_usd": 5, "per_workflow_usd": 2},
    "output_allowance_tokens": {"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500},
    "plan_warning_ratio": 0.8,
    "summary_max_chars": 1500
  }
}"#;

fn config() -> GatewayConfig {
    parse_defaults(&parse(CONFIG).expect("the fixture")).expect("the fixture configuration")
}

fn entry(families: &[&str], code: bool, history: bool) -> AllowlistEntry {
    AllowlistEntry { families: families.iter().map(|family| family.to_string()).collect(), code, history }
}

fn subjects(config: &GatewayConfig) -> Vec<PolicySubject> {
    config
        .models
        .iter()
        .map(|model| PolicySubject { client: model.client.clone(), model: model.model.clone(), family: model.family.clone() })
        .collect()
}

/// The permitted client models of a scope, as `<client>:<model>` keys.
fn permitted(scope: &DataScope) -> Vec<String> {
    let config = config();
    decide_policy(&config, scope, &subjects(&config))
        .permissions
        .into_iter()
        .filter(|permission| permission.permitted)
        .map(|permission| format!("{}:{}", permission.client, permission.model))
        .collect()
}

fn listed(client: &str, model: &str, family: &str, cost_class: CostClass, tier: Option<Tier>, notes: &str, source: ModelSource) -> ListedModel {
    ListedModel {
        client: client.to_string(),
        model: model.to_string(),
        family: family.to_string(),
        cost_class,
        tier,
        notes: notes.to_string(),
        source,
        free: false,
    }
}

#[test]
fn the_policy_permits_only_the_families_the_repository_entry_lists() {
    let scope = DataScope::Repository { key: "github.com/acme/widgets".to_string(), entry: Some(entry(&["anthropic", "zhipu"], false, false)) };
    assert_eq!(permitted(&scope), vec!["claude:claude-sonnet-5-5", "vibe:zai-glm-5-3"]);
}

#[test]
fn the_policy_reports_the_entry_code_and_history_permissions_for_every_model() {
    let config = config();
    let scope = DataScope::Repository { key: "github.com/acme/widgets".to_string(), entry: Some(entry(&["anthropic"], true, false)) };
    let decision = decide_policy(&config, &scope, &subjects(&config));
    let claude = decision.permissions.iter().find(|permission| permission.client == "claude").expect("claude");
    assert_eq!((claude.permitted, claude.inline_only, claude.may_send.code, claude.may_send.history), (true, false, true, false));
    let codex = decision.permissions.iter().find(|permission| permission.client == "codex").expect("codex");
    assert_eq!((codex.permitted, codex.may_send.code, codex.may_send.history), (false, true, false));
}

#[test]
fn the_policy_permits_nothing_in_a_repository_with_a_remote_but_no_entry() {
    let config = config();
    let scope = DataScope::Repository { key: "github.com/acme/widgets".to_string(), entry: None };
    let decision = decide_policy(&config, &scope, &subjects(&config));
    assert!(decision.permissions.iter().all(|permission| !permission.permitted));
    assert!(decision.permissions.iter().all(|permission| !permission.may_send.code && !permission.may_send.history));
    assert_eq!(decision.reason, "github.com/acme/widgets has no allowlist entry, so nothing is sent");
}

#[test]
fn the_policy_permits_nothing_in_a_repository_with_no_remote() {
    let config = config();
    let decision = decide_policy(&config, &DataScope::NoRemote, &subjects(&config));
    assert!(decision.permissions.iter().all(|permission| !permission.permitted));
    assert_eq!(decision.reason, "the repository has no remote, so nothing is sent");
}

#[test]
fn the_policy_permits_only_codex_and_claude_code_in_a_nedap_repository() {
    let config = config();
    let decision = decide_policy(&config, &DataScope::Nedap, &subjects(&config));
    assert_eq!(permitted(&DataScope::Nedap), vec!["claude:claude-sonnet-5-5", "codex:gpt-5.5"]);
    assert!(decision.permissions.iter().all(|permission| permission.may_send.code && permission.may_send.history));
    assert_eq!(decision.reason, "a nedap repository: only Codex and Claude Code, with code and history");
}

#[test]
fn the_policy_counts_the_client_not_the_family_in_a_nedap_repository() {
    let elsewhere = parse_defaults(&parse(
        r#"{
          "version": 1,
          "clients": {
            "agy": {"binary": "agy", "cost_class": "subsidised", "model_source": "declared"},
            "claude": {"binary": "claude", "cost_class": "subsidised", "model_source": "declared"},
            "codex": {"binary": "codex", "cost_class": "metered-employer", "model_source": "declared"},
            "opencode": {"binary": "opencode", "cost_class": "metered-personal", "model_source": "declared"}
          },
          "models": [
            {"client": "agy", "family": "anthropic", "model": "gemini-claude", "notes": "Claude through Antigravity", "tier": "hard"},
            {"client": "claude", "family": "anthropic", "model": "claude-sonnet-5-5", "notes": "general", "tier": "hard"},
            {"client": "codex", "family": "openai", "model": "gpt-5.5", "notes": "implementation", "tier": "frontier"},
            {"client": "opencode", "family": "openai", "model": "gpt-5.5", "notes": "OpenAI through OpenCode", "tier": "frontier"}
          ],
          "settings": {
            "availability_window_minutes": 60,
            "max_batch_delegations": 50,
            "max_parallel_per_client": 3,
            "metered_personal_caps": {"per_day_usd": 5, "per_workflow_usd": 2},
            "output_allowance_tokens": {"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500},
            "plan_warning_ratio": 0.8,
            "summary_max_chars": 1500
          }
        }"#,
    )
    .expect("the fixture JSON"))
    .expect("the fixture");
    let decision = decide_policy(&elsewhere, &DataScope::Nedap, &subjects(&elsewhere));
    let clients: Vec<&str> = decision.permissions.iter().filter(|permission| permission.permitted).map(|permission| permission.client.as_str()).collect();
    assert_eq!(clients, vec!["claude", "codex"]);
    assert!(decision.permissions.iter().filter(|permission| permission.client == "agy").all(|permission| !permission.permitted));
    assert!(decision.permissions.iter().filter(|permission| permission.client == "opencode").all(|permission| !permission.permitted));
}

#[test]
fn the_policy_permits_only_subsidised_and_metered_personal_models_outside_any_repository() {
    let config = config();
    let decision = decide_policy(&config, &DataScope::Outside, &subjects(&config));
    assert_eq!(permitted(&DataScope::Outside), vec!["claude:claude-sonnet-5-5", "hermes:hermes-4", "vibe:zai-glm-5-3"]);
    assert!(decision.permissions.iter().all(|permission| permission.inline_only));
    assert!(decision.permissions.iter().all(|permission| !permission.may_send.code && !permission.may_send.history));
    assert_eq!(decision.reason, "no repository: only inline content, to subsidised and metered-personal clients");
}

#[test]
fn the_policy_leaves_out_a_model_the_owner_disabled() {
    let mut disabled = config();
    disabled.disabled = vec!["claude:claude-sonnet-5-5".to_string()];
    let decision = decide_policy(&disabled, &DataScope::Nedap, &subjects(&disabled));
    let clients: Vec<&str> = decision.permissions.iter().map(|permission| permission.client.as_str()).collect();
    assert_eq!(clients, vec!["codex", "hermes", "vibe"]);
}

#[test]
fn the_policy_permits_nothing_for_a_model_whose_client_the_configuration_does_not_name() {
    let orphan = PolicySubject { client: "ghost".to_string(), model: "ghost-1".to_string(), family: "mistral".to_string() };
    let decision = decide_policy(&config(), &DataScope::Outside, &[orphan]);
    assert_eq!(decision.permissions.len(), 1);
    let permission = &decision.permissions[0];
    assert_eq!(
        (permission.client.as_str(), permission.model.as_str(), permission.permitted, permission.inline_only),
        ("ghost", "ghost-1", false, true)
    );
    assert!(!permission.may_send.code && !permission.may_send.history);
}

#[test]
fn the_policy_says_an_entry_that_lists_no_family_allows_none() {
    let config = config();
    let scope = DataScope::Repository { key: "github.com/acme/widgets".to_string(), entry: Some(entry(&[], false, false)) };
    assert_eq!(decide_policy(&config, &scope, &subjects(&config)).reason, "github.com/acme/widgets: the entry allows no family");
}

#[test]
fn the_policy_says_which_families_an_entry_allows() {
    let config = config();
    let scope = DataScope::Repository { key: "github.com/acme/widgets".to_string(), entry: Some(entry(&["anthropic", "zhipu"], false, false)) };
    assert_eq!(decide_policy(&config, &scope, &subjects(&config)).reason, "github.com/acme/widgets: the entry allows anthropic, zhipu");
}

#[test]
fn the_listing_keeps_the_models_the_policy_permits_and_the_missing_clients() {
    let listing = ModelListing {
        models: vec![
            listed("claude", "claude-sonnet-5-5", "anthropic", CostClass::Subsidised, Some(Tier::Hard), "general", ModelSource::Declared),
            listed("codex", "gpt-5.5", "openai", CostClass::MeteredEmployer, Some(Tier::Frontier), "implementation", ModelSource::Declared),
        ],
        missing: vec![
            MissingClient { client: "hermes".to_string(), reason: "not installed: hermes is not on PATH".to_string() },
            MissingClient { client: "vibe".to_string(), reason: "not installed: vibe is not on PATH".to_string() },
        ],
        skipped: vec![SkippedModel { client: "vibe".to_string(), model: "mystery".to_string(), reason: "no family".to_string() }],
    };
    let config = config();
    let scope = DataScope::Repository { key: "github.com/acme/widgets".to_string(), entry: Some(entry(&["anthropic"], true, true)) };
    let decision = decide_policy(&config, &scope, &subjects(&config));
    let permitted = permitted_models(listing, &decision);
    assert_eq!(permitted.models.len(), 1);
    assert_eq!(permitted.models[0].listed.client, "claude");
    assert_eq!((permitted.models[0].inline_only, permitted.models[0].may_send.code, permitted.models[0].may_send.history), (false, true, true));
    assert_eq!(permitted.missing.len(), 2);
    assert_eq!(permitted.skipped.len(), 1);
}

#[test]
fn the_listing_leaves_out_a_listed_model_the_policy_does_not_decide_about() {
    let listing = ModelListing {
        models: vec![listed("claude", "claude-sonnet-5-5", "anthropic", CostClass::Subsidised, Some(Tier::Hard), "general", ModelSource::Declared)],
        missing: Vec::new(),
        skipped: Vec::new(),
    };
    let nothing = PolicyDecision { reason: "nothing".to_string(), permissions: Vec::new() };
    assert!(permitted_models(listing.clone(), &nothing).models.is_empty());
    let config = config();
    let decision = decide_policy(&config, &DataScope::Nedap, &subjects(&config));
    let kept = permitted_models(listing, &decision);
    let clients: Vec<&str> = kept.models.iter().map(|model| model.listed.client.as_str()).collect();
    assert_eq!(clients, vec!["claude"]);
}

#[test]
fn a_permitted_model_prints_its_keys_in_the_order_the_node_runner_writes_them() {
    let model = crate::policy::PermittedModel {
        listed: listed("claude", "claude-sonnet-5-5", "anthropic", CostClass::Subsidised, Some(Tier::Hard), "general", ModelSource::Declared),
        inline_only: false,
        may_send: crate::policy::SendPermission { code: true, history: false },
    };
    assert_eq!(
        pretty(&permitted_model_json(&model)),
        "{\n  \"client\": \"claude\",\n  \"cost_class\": \"subsidised\",\n  \"family\": \"anthropic\",\n  \"model\": \"claude-sonnet-5-5\",\n  \"notes\": \"general\",\n  \"source\": \"declared\",\n  \"tier\": \"hard\",\n  \"inline_only\": false,\n  \"may_send\": {\n    \"code\": true,\n    \"history\": false\n  }\n}"
    );
    // A model the catalogue does not know prints a null tier.
    let unknown = crate::policy::PermittedModel {
        listed: listed("vibe", "mystery", "zhipu", CostClass::Subsidised, None, "", ModelSource::Live),
        inline_only: true,
        may_send: crate::policy::SendPermission { code: false, history: false },
    };
    assert_eq!(
        pretty(&permitted_model_json(&unknown)),
        "{\n  \"client\": \"vibe\",\n  \"cost_class\": \"subsidised\",\n  \"family\": \"zhipu\",\n  \"model\": \"mystery\",\n  \"notes\": \"\",\n  \"source\": \"live\",\n  \"tier\": null,\n  \"inline_only\": true,\n  \"may_send\": {\n    \"code\": false,\n    \"history\": false\n  }\n}"
    );
}

#[test]
fn the_models_result_prints_the_policy_the_status_and_the_listing() {
    let listing = crate::policy::PolicyListing {
        models: vec![crate::policy::PermittedModel {
            listed: listed("claude", "claude-sonnet-5-5", "anthropic", CostClass::Subsidised, Some(Tier::Hard), "general", ModelSource::Declared),
            inline_only: false,
            may_send: crate::policy::SendPermission { code: true, history: true },
        }],
        missing: vec![MissingClient { client: "hermes".to_string(), reason: "not installed: hermes is not on PATH".to_string() }],
        skipped: vec![SkippedModel { client: "vibe".to_string(), model: "mystery".to_string(), reason: "no family".to_string() }],
    };
    let printed = models_result_json("a nedap repository: only Codex and Claude Code, with code and history", &listing, None);
    let keys: Vec<&str> = printed.as_object().expect("an object").entries().into_iter().map(|(key, _)| key).collect();
    assert_eq!(keys, vec!["policy", "status", "missing", "models", "skipped"]);
    assert_eq!(
        pretty(&printed),
        "{\n  \"policy\": \"a nedap repository: only Codex and Claude Code, with code and history\",\n  \"status\": \"ok\",\n  \"missing\": [\n    {\n      \"client\": \"hermes\",\n      \"reason\": \"not installed: hermes is not on PATH\"\n    }\n  ],\n  \"models\": [\n    {\n      \"client\": \"claude\",\n      \"cost_class\": \"subsidised\",\n      \"family\": \"anthropic\",\n      \"model\": \"claude-sonnet-5-5\",\n      \"notes\": \"general\",\n      \"source\": \"declared\",\n      \"tier\": \"hard\",\n      \"inline_only\": false,\n      \"may_send\": {\n        \"code\": true,\n        \"history\": true\n      }\n    }\n  ],\n  \"skipped\": [\n    {\n      \"client\": \"vibe\",\n      \"model\": \"mystery\",\n      \"reason\": \"no family\"\n    }\n  ]\n}"
    );
}

// ---------------------------------------------------------------- resolveScope

const REPO: &str = "/work/agent-kit";
const KEY: &str = "github.com/jorisjonkers-dev/agent-kit";
const ALLOWLIST: &str = "/home/me/.config/agent-kit/critique-allowlist.json";

fn allowlist_file(families: &str, code: bool, history: bool) -> StubFiles {
    StubFiles::new().with(
        ALLOWLIST,
        &format!(r#"{{"repositories": {{"{KEY}": {{"code": {code}, "families": [{families}], "history": {history}, "updated_at": "2026-11-04T00:00:00.000Z"}}}}, "version": 1}}"#),
    )
}

#[test]
fn a_delegation_with_no_repository_is_outside_any() {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(resolve_scope(&runtime, None), Ok(DataScope::Outside));
}

#[test]
fn a_path_in_no_repository_is_outside_any() {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(resolve_scope(&runtime, Some("/tmp/nowhere")), Ok(DataScope::Outside));
}

#[test]
fn a_repository_is_decided_by_its_allowlist_entry() {
    let files = allowlist_file(r#""anthropic""#, true, false);
    let git = StubGit::new(Some(REPO), &[("origin", "git@github.com:JorisJonkers-dev/agent-kit.git")]);
    let runtime = runtime(files, git, home_env("/home/me"));
    assert_eq!(
        resolve_scope(&runtime, Some(REPO)),
        Ok(DataScope::Repository { key: KEY.to_string(), entry: Some(entry(&["anthropic"], true, false)) })
    );
}

#[test]
fn a_repository_with_no_allowlist_entry_is_still_a_repository() {
    let git = StubGit::new(Some(REPO), &[("origin", "git@github.com:JorisJonkers-dev/agent-kit.git")]);
    let runtime = runtime(StubFiles::new(), git, home_env("/home/me"));
    assert_eq!(resolve_scope(&runtime, Some(REPO)), Ok(DataScope::Repository { key: KEY.to_string(), entry: None }));
}

#[test]
fn a_nedap_repository_is_its_own_scope() {
    let git = StubGit::new(Some(REPO), &[("origin", "git@github.com:nedap/agent-kit.git")]);
    let runtime = runtime(StubFiles::new(), git, home_env("/home/me"));
    assert_eq!(resolve_scope(&runtime, Some(REPO)), Ok(DataScope::Nedap));
}

#[test]
fn a_repository_with_no_remote_is_its_own_scope() {
    let runtime = runtime(StubFiles::new(), StubGit::new(Some(REPO), &[]), home_env("/home/me"));
    assert_eq!(resolve_scope(&runtime, Some(REPO)), Ok(DataScope::NoRemote));
}

#[test]
fn a_path_git_cannot_read_is_an_error_naming_the_path() {
    let mut git = StubGit::new(Some(REPO), &[]);
    git.root = Err("fatal: not a git repository".to_string());
    let broken = runtime(StubFiles::new(), git, home_env("/home/me"));
    let error = resolve_scope(&broken, Some(REPO)).expect_err("git failed");
    assert_eq!(error, GatewayError::new("repository-unreadable", format!("{REPO}: fatal: not a git repository")));

    let mut git = StubGit::new(Some(REPO), &[]);
    git.remotes = Err("fatal: bad config".to_string());
    let broken = runtime(StubFiles::new(), git, home_env("/home/me"));
    let error = resolve_scope(&broken, Some(REPO)).expect_err("git failed");
    assert_eq!(error, GatewayError::new("repository-unreadable", format!("{REPO}: fatal: bad config")));
}

#[test]
fn an_allowlist_that_cannot_be_read_stops_the_scope() {
    let mut files = StubFiles::new();
    files.exists_error = Some(crate::ports::FsError { code: "EACCES".into(), message: "EACCES: permission denied, stat".into() });
    let git = StubGit::new(Some(REPO), &[("origin", "git@github.com:JorisJonkers-dev/agent-kit.git")]);
    let runtime = runtime(files, git, home_env("/home/me"));
    assert_eq!(
        resolve_scope(&runtime, Some(REPO)),
        Err(GatewayError::new("invalid-config", format!("{ALLOWLIST}: cannot read the family allowlist: EACCES: permission denied, stat")))
    );
}

#[test]
fn a_machine_without_a_home_cannot_resolve_a_scope() {
    let git = StubGit::new(Some(REPO), &[("origin", "git@github.com:JorisJonkers-dev/agent-kit.git")]);
    let runtime = runtime(StubFiles::new(), git, HashMap::new());
    assert_eq!(resolve_scope(&runtime, Some(REPO)), Err(GatewayError::new("unexpected", "required environment variable is missing: HOME")));
}

#[test]
fn the_scope_decides_the_models_the_way_the_node_runner_does() {
    let files = allowlist_file(r#""anthropic""#, true, false);
    let git = StubGit::new(Some(REPO), &[("origin", "git@github.com:JorisJonkers-dev/agent-kit.git")]);
    let runtime = runtime(files, git, home_env("/home/me"));
    let scope = resolve_scope(&runtime, Some(REPO)).expect("the scope");
    let config = config();
    let decision = decide_policy(&config, &scope, &subjects(&config));
    assert_eq!(decision.reason, format!("{KEY}: the entry allows anthropic"));
    let permitted: Vec<&str> = decision.permissions.iter().filter(|permission| permission.permitted).map(|permission| permission.client.as_str()).collect();
    assert_eq!(permitted, vec!["claude"]);
    let claude = decision.permissions.iter().find(|permission| permission.client == "claude").expect("claude");
    assert_eq!((claude.may_send.code, claude.may_send.history), (true, false));
}

#[test]
fn a_scope_is_compared_whole() {
    assert_ne!(DataScope::Outside, DataScope::NoRemote);
    assert_eq!(Value::Null.as_object(), None);
}
