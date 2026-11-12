//! Tests for the configuration reader, this machine's paths, the family
//! allowlist and the remote detector, ported from the node runner's own tests.

use super::scenarios::FixedClock;
use crate::allowlist::{load_allowlist, parse_allowlist, AllowlistEntry, FamilyAllowlist};
use crate::config::{CostClass, ModelSource, PlanLimit, PlanWindow, Tier};
use crate::config_parse::{apply_override, load_config, parse_defaults};
use crate::error::GatewayError;
use crate::git_remote::{identify_repository, parse_remote, RemoteLocation, RepositoryIdentity, DENIED_OWNER};
use crate::json::{parse, Value};
use crate::locations::{allowlist_path, config_dir, grants_path, ledger_path, override_path, vibe_available_cache_path, vibe_config_path};
use crate::ports::{Files, FsError, Git, GitRemote, LockTimings, Process, ProcessResult, Runtime};
use std::collections::{BTreeMap, HashMap};

/// A small defaults file: three declared clients, one configured, two models.
const DEFAULTS: &str = r#"{
  "version": 1,
  "clients": {
    "claude": {"binary": "claude", "cost_class": "subsidised", "model_source": "declared"},
    "codex": {"binary": "codex", "cost_class": "metered-employer", "model_source": "declared"},
    "vibe": {"binary": "vibe", "cost_class": "subsidised", "model_source": "config"}
  },
  "models": [
    {"client": "claude", "family": "anthropic", "model": "claude-sonnet-5-5", "notes": "general", "tier": "hard"},
    {"client": "codex", "family": "openai", "model": "gpt-5.5", "notes": "implementation", "tier": "frontier"}
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

fn json(text: &str) -> Value {
    parse(text).expect("test JSON")
}

fn shipped() -> crate::config::GatewayConfig {
    parse_defaults(&json(DEFAULTS)).expect("the fixture defaults")
}

fn error_of<T>(result: Result<T, GatewayError>) -> GatewayError {
    match result {
        Ok(_) => panic!("the call did not fail"),
        Err(error) => error,
    }
}

fn message_of<T>(result: Result<T, GatewayError>) -> String {
    error_of(result).message
}

/// The files a test hands the runtime: text by path, and the failures it should give.
pub struct StubFiles {
    pub texts: HashMap<String, String>,
    pub exists_error: Option<FsError>,
    pub read_error: Option<FsError>,
}

impl StubFiles {
    pub fn new() -> StubFiles {
        StubFiles { texts: HashMap::new(), exists_error: None, read_error: None }
    }

    pub fn with(mut self, path: &str, text: &str) -> StubFiles {
        self.texts.insert(path.to_string(), text.to_string());
        self
    }
}

impl Files for StubFiles {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        if let Some(error) = &self.read_error {
            return Err(error.clone());
        }
        match self.texts.get(path) {
            Some(text) => Ok(text.clone()),
            None => Err(FsError { code: "ENOENT".into(), message: format!("ENOENT: no such file or directory, open '{path}'") }),
        }
    }

    fn write_atomic(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn create_exclusive(&self, _path: &str) -> Result<bool, FsError> {
        Ok(true)
    }

    fn remove_file(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn modified_ms(&self, _path: &str) -> Option<i64> {
        None
    }

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        if let Some(error) = &self.exists_error {
            return Err(error.clone());
        }
        Ok(self.texts.contains_key(path))
    }

    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn stamp(&self, _path: &str) -> Option<crate::ports::FileStamp> {
        None
    }

    fn write_private(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
}

/// The git a test hands the runtime: one answer for the root and one for the remotes.
pub struct StubGit {
    pub root: Result<Option<String>, String>,
    pub remotes: Result<Vec<GitRemote>, String>,
}

impl StubGit {
    pub fn new(root: Option<&str>, remotes: &[(&str, &str)]) -> StubGit {
        StubGit {
            root: Ok(root.map(str::to_string)),
            remotes: Ok(remotes.iter().map(|(name, url)| GitRemote { name: name.to_string(), url: url.to_string() }).collect()),
        }
    }
}

impl Git for StubGit {
    fn root(&self, _path: &str) -> Result<Option<String>, String> {
        self.root.clone()
    }

    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        self.remotes.clone()
    }

    fn remove_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        Ok(())
    }

    fn delete_branch(&self, _repo: &str, _branch: &str) -> Result<(), String> {
        Ok(())
    }
}

struct StubProcess;

impl Process for StubProcess {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Err("the stub starts nothing".to_string())
    }
}

/// A runtime over the stubs, with the shipped defaults as its defaults text.
pub fn runtime(files: StubFiles, git: StubGit, env: HashMap<String, String>) -> Runtime {
    Runtime {
        clock: Box::new(FixedClock(0)),
        files: Box::new(files),
        lock: LockTimings::default(),
        process: Box::new(StubProcess),
        git: Box::new(git),
        env,
        defaults: crate::SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
    }
}

pub fn home_env(home: &str) -> HashMap<String, String> {
    HashMap::from([("HOME".to_string(), home.to_string())])
}

// ---------------------------------------------------------------- git remotes

fn location(host: &str, owner: &str, repo: &str) -> RemoteLocation {
    RemoteLocation { host: host.to_string(), owner: owner.to_string(), repo: repo.to_string() }
}

#[test]
fn the_remote_detector_reads_every_url_shape_the_node_runner_reads() {
    let cases: [(&str, Option<RemoteLocation>); 24] = [
        ("git@github.com:owner/repo.git", Some(location("github.com", "owner", "repo"))),
        ("github.com:owner/repo.git", Some(location("github.com", "owner", "repo"))),
        ("github.com:owner/repo", Some(location("github.com", "owner", "repo"))),
        ("git@github-work:nedap/x.git", Some(location("github-work", "nedap", "x"))),
        ("git@github.com:NeDaP/X.git", Some(location("github.com", "nedap", "x"))),
        ("git@github.com:a/b/c.git", Some(location("github.com", "a/b", "c"))),
        ("git@github.com:owner/repo/", Some(location("github.com", "owner", "repo"))),
        ("git@github.com:owner/repo.git/", Some(location("github.com", "owner", "repo"))),
        ("ssh://git@github.com/owner/repo", Some(location("github.com", "owner", "repo"))),
        ("ssh://github.com/owner/repo", Some(location("github.com", "owner", "repo"))),
        ("ssh://git@github.com:22/Nedap/x/", Some(location("github.com", "nedap", "x"))),
        ("https://github.com/owner/repo", Some(location("github.com", "owner", "repo"))),
        ("https://github.com/owner/repo.git", Some(location("github.com", "owner", "repo"))),
        ("https://github.com/owner/repo/", Some(location("github.com", "owner", "repo"))),
        ("https://user:pass@github.com/owner/repo", Some(location("github.com", "owner", "repo"))),
        ("HTTPS://GitHub.com/Owner/Repo", Some(location("github.com", "owner", "repo"))),
        ("ssh://a/b@host/x", Some(location("a", "b@host", "x"))),
        ("", None),
        ("not a url", None),
        ("file:///path", None),
        ("https://github.com", None),
        ("https://github.com/", None),
        ("https://github.com/owner", None),
        ("C:/foo", None),
    ];
    for (url, expected) in cases {
        assert_eq!(parse_remote(url), expected, "{url:?}");
    }
}

#[test]
fn the_remote_detector_refuses_the_shapes_neither_expression_matches() {
    for url in [
        "git@github.com:",
        "host://x",
        "ssh:///x",
        "ssh://host:/x",
        "ssh://host:22x/y",
        "https://host//",
        ":x",
        "ssh://@host/x",
        "http:/x",
        "://x",
        "abc",
        "host:a\nb",
        "https://host/a\nb",
    ] {
        assert_eq!(parse_remote(url), None, "{url:?}");
    }
}

#[test]
fn a_nedap_remote_denies_the_repository_on_any_host_or_alias() {
    for url in [
        "git@github.com:nedap/x.git",
        "github.com:nedap/x.git",
        "github.com:nedap/x",
        "ssh://git@github.com/nedap/x",
        "ssh://github.com/nedap/x",
        "https://github.com/nedap/x",
        "https://github.com/nedap/x.git",
        "https://github.com/nedap/x/",
        "https://github.com/nedap/x.git/",
        "git@github.com:NeDaP/x.git",
        "https://github.com/NeDaP/X.git",
        "ssh://git@github.com:22/Nedap/x/",
        "git@github-work:nedap/x.git",
    ] {
        let remotes = [GitRemote { name: "origin".to_string(), url: url.to_string() }];
        match identify_repository(&remotes) {
            RepositoryIdentity::DeniedOwner { remote, owner, location } => {
                assert_eq!((remote.as_str(), owner.as_str()), ("origin", DENIED_OWNER), "{url}");
                assert_eq!(location.owner, DENIED_OWNER, "{url}");
            }
            other => panic!("{url} is not denied: {other:?}"),
        }
    }
}

#[test]
fn a_repository_is_known_by_its_origin_else_its_first_remote() {
    let origin = [
        GitRemote { name: "upstream".to_string(), url: "git@github.com:me/up.git".to_string() },
        GitRemote { name: "origin".to_string(), url: "git@github.com:me/origin.git".to_string() },
    ];
    assert_eq!(
        identify_repository(&origin),
        RepositoryIdentity::Identified { key: "github.com/me/origin".to_string(), location: location("github.com", "me", "origin") }
    );

    let first = [GitRemote { name: "upstream".to_string(), url: "git@github.com:me/up.git".to_string() }];
    assert_eq!(
        identify_repository(&first),
        RepositoryIdentity::Identified { key: "github.com/me/up".to_string(), location: location("github.com", "me", "up") }
    );

    // A nested owner is not the denied organisation, which is only the first segment.
    let nested = [GitRemote { name: "origin".to_string(), url: "git@github.com:a/nedap/x.git".to_string() }];
    assert_eq!(
        identify_repository(&nested),
        RepositoryIdentity::Identified { key: "github.com/a/nedap/x".to_string(), location: location("github.com", "a/nedap", "x") }
    );

    // A denied remote wins over origin, wherever it sits in the list.
    let denied = [
        GitRemote { name: "origin".to_string(), url: "git@github.com:me/x.git".to_string() },
        GitRemote { name: "upstream".to_string(), url: "git@github.com:nedap/y.git".to_string() },
    ];
    assert_eq!(
        identify_repository(&denied),
        RepositoryIdentity::DeniedOwner {
            remote: "upstream".to_string(),
            owner: "nedap".to_string(),
            location: location("github.com", "nedap", "y"),
        }
    );

    assert_eq!(identify_repository(&[]), RepositoryIdentity::NoRemote);
    let junk = [GitRemote { name: "origin".to_string(), url: "not a url".to_string() }];
    assert_eq!(identify_repository(&junk), RepositoryIdentity::NoRemote);
}

// ------------------------------------------------------------------ locations

#[test]
fn the_machine_paths_follow_the_environment() {
    let home = HashMap::from([("HOME".to_string(), "/home/me".to_string())]);
    assert_eq!(config_dir(&home), Ok("/home/me/.config/agent-kit".to_string()));
    assert_eq!(override_path(&home), Ok("/home/me/.config/agent-kit/model-gateway.json".to_string()));
    assert_eq!(allowlist_path(&home), Ok("/home/me/.config/agent-kit/critique-allowlist.json".to_string()));
    assert_eq!(grants_path(&home), Ok("/home/me/.config/agent-kit/model-gateway-grants.json".to_string()));
    assert_eq!(vibe_config_path(&home), Ok("/home/me/.vibe/config.toml".to_string()));
    assert_eq!(vibe_available_cache_path(&home), Ok("/home/me/.cache/model-gateway/vibe-available.json".to_string()));
    assert_eq!(ledger_path(&home), Ok("/home/me/.local/state/agent-kit/model-gateway/ledger.jsonl".to_string()));

    // An absolute XDG directory wins; a relative one is not one.
    let xdg = HashMap::from([
        ("HOME".to_string(), "/home/me".to_string()),
        ("XDG_CONFIG_HOME".to_string(), "/cfg".to_string()),
        ("XDG_CACHE_HOME".to_string(), "/cache".to_string()),
        ("XDG_STATE_HOME".to_string(), "/state".to_string()),
        ("VIBE_HOME".to_string(), "/vibe".to_string()),
    ]);
    assert_eq!(config_dir(&xdg), Ok("/cfg/agent-kit".to_string()));
    assert_eq!(override_path(&xdg), Ok("/cfg/agent-kit/model-gateway.json".to_string()));
    assert_eq!(vibe_config_path(&xdg), Ok("/vibe/config.toml".to_string()));
    assert_eq!(vibe_available_cache_path(&xdg), Ok("/cache/model-gateway/vibe-available.json".to_string()));
    assert_eq!(ledger_path(&xdg), Ok("/state/agent-kit/model-gateway/ledger.jsonl".to_string()));

    let relative = HashMap::from([
        ("HOME".to_string(), "/home/me".to_string()),
        ("XDG_CONFIG_HOME".to_string(), "cfg".to_string()),
        ("XDG_CACHE_HOME".to_string(), "cache".to_string()),
        ("XDG_STATE_HOME".to_string(), "state".to_string()),
        ("VIBE_HOME".to_string(), "vibe".to_string()),
    ]);
    assert_eq!(config_dir(&relative), Ok("/home/me/.config/agent-kit".to_string()));
    assert_eq!(vibe_config_path(&relative), Ok("/home/me/.vibe/config.toml".to_string()));
    assert_eq!(vibe_available_cache_path(&relative), Ok("/home/me/.cache/model-gateway/vibe-available.json".to_string()));
    assert_eq!(ledger_path(&relative), Ok("/home/me/.local/state/agent-kit/model-gateway/ledger.jsonl".to_string()));
}

#[test]
fn a_machine_without_a_home_has_no_paths() {
    let missing = GatewayError::new("unexpected", "required environment variable is missing: HOME");
    let empty = HashMap::new();
    for result in [
        config_dir(&empty),
        override_path(&empty),
        allowlist_path(&empty),
        grants_path(&empty),
        vibe_config_path(&empty),
        vibe_available_cache_path(&empty),
        ledger_path(&empty),
    ] {
        assert_eq!(error_of(result), missing);
    }
    // A relative XDG directory still needs HOME.
    let relative = HashMap::from([("XDG_CONFIG_HOME".to_string(), "cfg".to_string())]);
    assert_eq!(error_of(config_dir(&relative)), missing);
}

// ------------------------------------------------------------------ allowlist

#[test]
fn the_allowlist_reads_an_entry_families_code_and_history() {
    let allowlist = parse_allowlist(&json(
        r#"{"repositories": {"github.com/acme/widgets": {"code": true, "families": ["anthropic", "openai"], "history": false, "updated_at": "x"}}, "version": 1}"#,
    ))
    .expect("the allowlist");
    assert_eq!(
        allowlist.repositories.get("github.com/acme/widgets"),
        Some(&AllowlistEntry { families: vec!["anthropic".to_string(), "openai".to_string()], code: true, history: false })
    );
}

#[test]
fn an_entry_recorded_before_a_permission_existed_allows_neither() {
    let allowlist = parse_allowlist(&json(
        r#"{"repositories": {"github.com/acme/widgets": {"families": ["mistral"], "updated_at": "x"}}, "version": 1}"#,
    ))
    .expect("the allowlist");
    assert_eq!(
        allowlist.repositories.get("github.com/acme/widgets"),
        Some(&AllowlistEntry { families: vec!["mistral".to_string()], code: false, history: false })
    );
}

#[test]
fn a_file_with_no_repositories_is_an_empty_allowlist() {
    assert_eq!(parse_allowlist(&json(r#"{"repositories": {}, "version": 1}"#)), Ok(FamilyAllowlist::empty()));
}

#[test]
fn the_allowlist_refuses_what_it_cannot_read() {
    let cases: [(&str, &str); 8] = [
        ("[]", "the allowlist must be a JSON object"),
        (r#"{"repositories": {}, "version": 2}"#, "version must be 1"),
        (r#"{"repositories": [], "version": 1}"#, "repositories must be an object keyed by repository"),
        (r#"{"repositories": {"github.com/acme/widgets": "mistral"}, "version": 1}"#, "repositories.github.com/acme/widgets must be an object"),
        (
            r#"{"repositories": {"github.com/acme/widgets": {"families": "mistral"}}, "version": 1}"#,
            "repositories.github.com/acme/widgets.families must be a list of non-empty strings",
        ),
        (
            r#"{"repositories": {"github.com/acme/widgets": {"families": [7]}}, "version": 1}"#,
            "repositories.github.com/acme/widgets.families must be a list of non-empty strings",
        ),
        (
            r#"{"repositories": {"github.com/acme/widgets": {"code": "yes", "families": []}}, "version": 1}"#,
            "repositories.github.com/acme/widgets.code must be true or false",
        ),
        (
            r#"{"repositories": {"github.com/acme/widgets": {"families": [], "history": 1}}, "version": 1}"#,
            "repositories.github.com/acme/widgets.history must be true or false",
        ),
    ];
    for (raw, message) in cases {
        let error = error_of(parse_allowlist(&json(raw)));
        assert_eq!((error.code.as_str(), error.message.as_str()), ("invalid-config", message), "{raw}");
    }
    // A family that is the empty string is not a family.
    let empty = error_of(parse_allowlist(&json(r#"{"repositories": {"k": {"families": [""]}}, "version": 1}"#)));
    assert_eq!(empty.message, "repositories.k.families must be a list of non-empty strings");
}

#[test]
fn a_missing_allowlist_file_is_an_empty_allowlist() {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(load_allowlist(&runtime), Ok(FamilyAllowlist::empty()));
}

#[test]
fn the_allowlist_file_is_read_from_the_machine_configuration() {
    let path = "/home/me/.config/agent-kit/critique-allowlist.json";
    let files = StubFiles::new().with(path, r#"{"repositories": {"github.com/acme/widgets": {"families": ["anthropic"]}}, "version": 1}"#);
    let runtime = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    let allowlist = load_allowlist(&runtime).expect("the allowlist");
    assert_eq!(
        allowlist.repositories.get("github.com/acme/widgets"),
        Some(&AllowlistEntry { families: vec!["anthropic".to_string()], code: false, history: false })
    );
}

#[test]
fn an_allowlist_that_cannot_be_read_names_the_file() {
    let path = "/home/me/.config/agent-kit/critique-allowlist.json";
    let mut files = StubFiles::new();
    files.exists_error = Some(FsError { code: "EACCES".into(), message: "EACCES: permission denied, stat".into() });
    let unreadable = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(error_of(load_allowlist(&unreadable)).message, format!("{path}: cannot read the family allowlist: EACCES: permission denied, stat"));

    let mut files = StubFiles::new();
    files.texts.insert(path.to_string(), String::new());
    files.read_error = Some(FsError { code: "EISDIR".into(), message: "EISDIR: illegal operation on a directory, read".into() });
    let unreadable = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(error_of(load_allowlist(&unreadable)).message, format!("{path}: cannot read the family allowlist: EISDIR: illegal operation on a directory, read"));

    let files = StubFiles::new().with(path, "{");
    let broken = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(
        error_of(load_allowlist(&broken)).message,
        format!("{path}: not JSON: Expected property name or '}}' in JSON at position 1 (line 1 column 2)")
    );

    let files = StubFiles::new().with(path, "[]");
    let broken = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(error_of(load_allowlist(&broken)).message, format!("{path}: the allowlist must be a JSON object"));
}

// -------------------------------------------------------------- parseDefaults

#[test]
fn the_shipped_defaults_parse() {
    let config = parse_defaults(&json(crate::SHIPPED_DEFAULTS)).expect("the shipped defaults");
    assert_eq!(config.version, 1.0);
    assert_eq!(config.disabled, Vec::<String>::new());
    assert_eq!(config.clients.len(), 6);
    assert_eq!(config.clients.get("codex").map(|client| client.cost_class), Some(CostClass::MeteredEmployer));
    assert_eq!(config.clients.get("vibe").map(|client| client.model_source), Some(ModelSource::Live));
    assert_eq!(config.clients.get("agy").and_then(|client| client.families.clone()), Some(vec!["google".to_string()]));
    assert_eq!(
        config.clients.get("opencode").and_then(|client| client.provider_cost_classes.clone()),
        Some(BTreeMap::from([("opencode-go".to_string(), CostClass::Subsidised)]))
    );
    assert_eq!(config.models.len(), 21);
    assert_eq!(config.models[0].model, "claude-opus-5-5");
    assert_eq!(config.models[0].tier, Tier::Frontier);
    assert_eq!(config.models[5].names, Some(vec!["zai-glm-5-3".to_string()]));
    assert_eq!(config.models[3].prices.as_ref().map(|prices| prices.cached_input), Some(0.125));
    assert_eq!(config.settings.plan_warning_ratio, 0.8);
    assert_eq!(config.settings.summary_max_chars, 1500.0);
    assert_eq!(config.settings.max_parallel_per_client, 3.0);
    assert_eq!(config.settings.max_batch_delegations, 50.0);
    assert_eq!(config.settings.availability_window_minutes, 60.0);
    assert_eq!(config.settings.metered_personal_caps.per_workflow_usd, 2.0);
    assert_eq!(config.settings.metered_personal_caps.per_day_usd, 5.0);
    assert_eq!(config.settings.output_allowance_tokens, [500.0, 2000.0, 6000.0, 16000.0]);
    assert_eq!(config.settings.allowance(Tier::Hard), 6000.0);
}

#[test]
fn the_defaults_keep_the_families_providers_prices_and_plan_a_client_names() {
    let raw = json(
        r#"{
          "version": 1,
          "clients": {
            "agy": {"binary": "agy", "cost_class": "subsidised", "families": ["google"], "model_source": "live"},
            "opencode": {"binary": "opencode", "cost_class": "metered-personal", "model_source": "live", "provider_cost_classes": {"opencode-go": "subsidised"}, "providers": ["opencode-go"], "plan_limits": [{"window": "5h", "limit_usd": 5}, {"window": "day", "limit_runs": 20}]}
          },
          "models": [
            {"client": "vibe", "family": "zhipu", "model": "zai-glm-5-3", "notes": "review", "prices": {"cached_input": 0.14, "input": 1.4, "output": 4.4}, "tier": "hard"},
            {"client": "claude", "family": "anthropic", "model": "claude-sonnet-5-5", "notes": "general", "tier": "hard"}
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
    );
    let config = parse_defaults(&raw).expect("the defaults");
    assert_eq!(config.clients.get("agy").and_then(|client| client.families.clone()), Some(vec!["google".to_string()]));
    let opencode = config.clients.get("opencode").expect("opencode");
    assert_eq!(opencode.providers, Some(vec!["opencode-go".to_string()]));
    assert_eq!(opencode.provider_cost_classes, Some(BTreeMap::from([("opencode-go".to_string(), CostClass::Subsidised)])));
    assert_eq!(
        opencode.plan_limits,
        Some(vec![
            PlanLimit::Usd { window: PlanWindow::FiveHours, limit_usd: 5.0 },
            PlanLimit::Runs { window: PlanWindow::Day, limit_runs: 20.0 },
        ])
    );
    assert_eq!(config.clients.get("agy").and_then(|client| client.providers.clone()), None);
    assert_eq!(config.models[0].prices, Some(crate::config::ModelPrices { input: 1.4, cached_input: 0.14, output: 4.4 }));
    assert_eq!(config.models[1].prices, None);
    assert_eq!(config.models[1].names, None);
}

#[test]
fn the_defaults_refuse_what_they_cannot_read() {
    let cases: [(&str, &str); 36] = [
        ("[]", "the defaults must be a JSON object"),
        (r#"{"version": 2}"#, "version must be 1"),
        (r#"{"version": true}"#, "version must be 1"),
        (r#"{"version": 1, "clients": []}"#, "clients must be an object keyed by client id"),
        (r#"{"version": 1, "clients": {"claude": "claude"}}"#, "clients.claude must be an object"),
        (
            r#"{"version": 1, "clients": {"claude": {"cost_class": "subsidised", "model_source": "declared"}}}"#,
            "clients.claude is new, so it needs binary, cost_class and model_source",
        ),
        (
            r#"{"version": 1, "clients": {"claude": {"binary": "claude", "cost_class": "free", "model_source": "declared"}}}"#,
            "clients.claude.cost_class must be one of subsidised, metered-personal, metered-employer",
        ),
        (
            r#"{"version": 1, "clients": {"claude": {"binary": "claude", "cost_class": "subsidised", "model_source": "guess"}}}"#,
            "clients.claude.model_source must be one of live, config, declared",
        ),
        (
            r#"{"version": 1, "clients": {"claude": {"binary": "claude", "cost_class": "subsidised", "families": [1], "model_source": "declared"}}}"#,
            "clients.claude.families must be a list of non-empty strings",
        ),
        (
            r#"{"version": 1, "clients": {"claude": {"binary": "claude", "cost_class": "subsidised", "model_source": "declared", "provider_cost_classes": []}}}"#,
            "clients.claude.provider_cost_classes must be an object keyed by provider name",
        ),
        (
            r#"{"version": 1, "clients": {"claude": {"binary": "claude", "cost_class": "subsidised", "model_source": "declared", "provider_cost_classes": {"p": "free"}}}}"#,
            "clients.claude.provider_cost_classes.p must be one of subsidised, metered-personal, metered-employer",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "providers": [1]}}}"#,
            "clients.opencode.providers must be a list of non-empty strings",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": 5}}}"#,
            "clients.opencode.plan_limits must be a list of windows",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [5]}}}"#,
            "clients.opencode.plan_limits[0] must be an object",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "year", "limit_usd": 5}]}}}"#,
            "clients.opencode.plan_limits[0].window must be one of 5h, day, week, month",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"limit_usd": 5}]}}}"#,
            "clients.opencode.plan_limits[0].window must be one of 5h, day, week, month",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "day"}]}}}"#,
            "clients.opencode.plan_limits[0] must name exactly one of limit_usd and limit_runs",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "day", "limit_usd": 5, "limit_runs": 5}]}}}"#,
            "clients.opencode.plan_limits[0] must name exactly one of limit_usd and limit_runs",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "day", "limit_usd": 0}]}}}"#,
            "clients.opencode.plan_limits[0].limit_usd must be above 0",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "day", "limit_usd": "5"}]}}}"#,
            "clients.opencode.plan_limits[0].limit_usd must be a number",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "day", "limit_runs": 1.5}]}}}"#,
            "clients.opencode.plan_limits[0].limit_runs must be a whole number above 0",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "day", "limit_runs": 0}]}}}"#,
            "clients.opencode.plan_limits[0].limit_runs must be a whole number above 0",
        ),
        (
            r#"{"version": 1, "clients": {"opencode": {"binary": "opencode", "cost_class": "subsidised", "model_source": "live", "plan_limits": [{"window": "day", "limit_runs": "5"}]}}}"#,
            "clients.opencode.plan_limits[0].limit_runs must be a number",
        ),
        (r#"{"version": 1, "clients": {}, "models": {}}"#, "models must be a list"),
        (r#"{"version": 1, "clients": {}, "models": ["claude"]}"#, "models[0] must be an object"),
        (r#"{"version": 1, "clients": {}, "models": [{"model": "gpt-5.5"}]}"#, "models[0].client must be a non-empty string"),
        (r#"{"version": 1, "clients": {}, "models": [{"client": "codex"}]}"#, "models[0].model must be a non-empty string"),
        (
            r#"{"version": 1, "clients": {}, "models": [{"client": "claude", "family": "anthropic", "model": "x", "notes": "n", "tier": "epic"}]}"#,
            "models[0].tier must be one of trivial, routine, hard, frontier",
        ),
        (
            r#"{"version": 1, "clients": {}, "models": [{"client": "claude", "family": "anthropic", "model": "x", "notes": "n", "prices": 1, "tier": "hard"}]}"#,
            "models[0].prices must be an object",
        ),
        (
            r#"{"version": 1, "clients": {}, "models": [{"client": "claude", "family": "anthropic", "model": "x", "notes": "n", "prices": {"input": 1, "output": 2}, "tier": "hard"}]}"#,
            "models[0].prices.cached_input must be a number",
        ),
        (
            r#"{"version": 1, "clients": {}, "models": [{"client": "claude", "family": "anthropic", "model": "x", "notes": "n", "prices": {"cached_input": 0, "input": 0, "output": 2}, "tier": "hard"}]}"#,
            "models[0].prices.input and models[0].prices.output must be above 0",
        ),
        (
            r#"{"version": 1, "clients": {}, "models": [{"client": "claude", "family": "anthropic", "model": "x", "notes": "n", "prices": {"cached_input": -1, "input": 1, "output": 2}, "tier": "hard"}]}"#,
            "models[0].prices.cached_input must be at least 0",
        ),
        (r#"{"version": 1, "clients": {}, "models": [], "settings": []}"#, "settings must be an object"),
        (r#"{"version": 1, "clients": {}, "models": [], "settings": {"metered_personal_caps": 5}}"#, "settings.metered_personal_caps must be an object"),
        (
            r#"{"version": 1, "clients": {}, "models": [], "settings": {"metered_personal_caps": {}, "output_allowance_tokens": 500}}"#,
            "settings.output_allowance_tokens must be an object keyed by tier",
        ),
        (r#"{"version": 1, "clients": {}, "models": [], "settings": {}, "disabled": [1]}"#, "disabled must be a list of non-empty strings"),
    ];
    for (raw, message) in cases {
        let error = error_of(parse_defaults(&json(raw)));
        assert_eq!((error.code.as_str(), error.message.as_str()), ("invalid-config", message), "{raw}");
    }
}

#[test]
fn the_defaults_refuse_each_setting_that_is_wrong() {
    let cases: [(&str, &str); 12] = [
        (r#""availability_window_minutes": "60""#, "settings.availability_window_minutes must be a number"),
        (r#""max_batch_delegations": 0"#, "settings.max_batch_delegations must be a whole number of at least 1"),
        (r#""max_batch_delegations": 2.5"#, "settings.max_batch_delegations must be a whole number of at least 1"),
        (r#""max_parallel_per_client": 0"#, "settings.max_parallel_per_client must be a whole number of at least 1"),
        (r#""max_parallel_per_client": 1.5"#, "settings.max_parallel_per_client must be a whole number of at least 1"),
        (r#""plan_warning_ratio": "high""#, "settings.plan_warning_ratio must be a number"),
        (r#""plan_warning_ratio": 80"#, "settings.plan_warning_ratio must be between 0 and 1"),
        (r#""plan_warning_ratio": -1"#, "settings.plan_warning_ratio must be between 0 and 1"),
        (r#""summary_max_chars": "long""#, "settings.summary_max_chars must be a number"),
        (r#""metered_personal_caps": {"per_day_usd": 5}"#, "settings.metered_personal_caps.per_workflow_usd must be a number"),
        (r#""metered_personal_caps": {"per_workflow_usd": 2}"#, "settings.metered_personal_caps.per_day_usd must be a number"),
        (r#""output_allowance_tokens": {"trivial": 500}"#, "settings.output_allowance_tokens.frontier must be a number"),
    ];
    for (setting, message) in cases {
        let raw = format!(
            r#"{{"version": 1, "clients": {{}}, "models": [], "settings": {{"availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3, "metered_personal_caps": {{"per_day_usd": 5, "per_workflow_usd": 2}}, "output_allowance_tokens": {{"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500}}, "plan_warning_ratio": 0.8, "summary_max_chars": 1500, {setting}}}}}"#
        );
        let error = error_of(parse_defaults(&json(&raw)));
        assert_eq!(error.message, message, "{setting}");
    }
}

#[test]
fn an_allowance_must_be_a_whole_number_of_tokens_above_zero() {
    for (allowance, message) in [
        (r#""hard": 1.5"#, "settings.output_allowance_tokens.hard must be a whole number of tokens above 0"),
        (r#""hard": 0"#, "settings.output_allowance_tokens.hard must be a whole number of tokens above 0"),
        (r#""hard": "6000""#, "settings.output_allowance_tokens.hard must be a number"),
    ] {
        let raw = format!(
            r#"{{"version": 1, "clients": {{}}, "models": [], "settings": {{"availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3, "metered_personal_caps": {{"per_day_usd": 5, "per_workflow_usd": 2}}, "output_allowance_tokens": {{"frontier": 16000, {allowance}, "routine": 2000, "trivial": 500}}, "plan_warning_ratio": 0.8, "summary_max_chars": 1500}}}}"#
        );
        assert_eq!(error_of(parse_defaults(&json(&raw))).message, message, "{allowance}");
    }
    // The tiers are read from the frontier down, as the node runner reads them.
    let raw = json(
        r#"{"version": 1, "clients": {}, "models": [], "settings": {"availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3, "metered_personal_caps": {"per_day_usd": 5, "per_workflow_usd": 2}, "output_allowance_tokens": {"frontier": 0, "hard": 6000, "routine": 2000, "trivial": 0}, "plan_warning_ratio": 0.8, "summary_max_chars": 1500}}"#,
    );
    assert_eq!(error_of(parse_defaults(&raw)).message, "settings.output_allowance_tokens.frontier must be a whole number of tokens above 0");
    let merged = apply_override(&shipped(), &json(r#"{"settings": {"output_allowance_tokens": {"frontier": 0, "trivial": 0}}}"#));
    assert_eq!(message_of(merged), "settings.output_allowance_tokens.frontier must be a whole number of tokens above 0");
}

#[test]
fn a_number_that_is_not_finite_is_not_a_number() {
    let mut raw = json(DEFAULTS);
    if let Value::Obj(object) = &mut raw {
        if let Some(Value::Obj(settings)) = object.get_mut("settings") {
            settings.set("summary_max_chars", Value::Num(f64::INFINITY));
        }
    }
    assert_eq!(error_of(parse_defaults(&raw)).message, "settings.summary_max_chars must be a number");
}

#[test]
fn the_defaults_read_the_disabled_list_before_the_settings() {
    // The settings must be an object before anything inside them is read.
    let raw = json(r#"{"version": 1, "clients": {}, "models": [], "settings": [], "disabled": [1]}"#);
    assert_eq!(error_of(parse_defaults(&raw)).message, "settings must be an object");
    // Inside a settings object, the disabled list is read first.
    let raw = json(r#"{"version": 1, "clients": {}, "models": [], "settings": {"metered_personal_caps": {}}, "disabled": [1]}"#);
    assert_eq!(error_of(parse_defaults(&raw)).message, "disabled must be a list of non-empty strings");
}

// --------------------------------------------------------------- applyOverride

#[test]
fn an_override_that_names_nothing_keeps_the_defaults() {
    assert_eq!(apply_override(&shipped(), &json("{}")), Ok(shipped()));
}

#[test]
fn an_override_replaces_one_field_of_a_client_and_keeps_the_rest() {
    let merged = apply_override(&shipped(), &json(r#"{"clients": {"codex": {"cost_class": "subsidised"}}}"#)).expect("the override");
    let codex = merged.clients.get("codex").expect("codex");
    assert_eq!(codex.binary, "codex");
    assert_eq!(codex.cost_class, CostClass::Subsidised);
    assert_eq!(codex.model_source, ModelSource::Declared);
    assert_eq!(codex.families, None);
    assert_eq!(codex.provider_cost_classes, None);
    assert_eq!(codex.providers, None);
    assert_eq!(codex.plan_limits, None);
}

#[test]
fn an_override_sets_the_cost_class_of_one_provider_and_keeps_the_others() {
    let base = apply_override(
        &shipped(),
        &json(
            r#"{"clients": {"opencode": {"binary": "opencode", "cost_class": "metered-personal", "model_source": "live", "provider_cost_classes": {"a": "subsidised", "b": "metered-employer"}}}}"#,
        ),
    )
    .expect("the base");
    let merged = apply_override(&base, &json(r#"{"clients": {"opencode": {"provider_cost_classes": {"b": "subsidised"}}}}"#)).expect("the override");
    assert_eq!(
        merged.clients.get("opencode").and_then(|client| client.provider_cost_classes.clone()),
        Some(BTreeMap::from([("a".to_string(), CostClass::Subsidised), ("b".to_string(), CostClass::Subsidised)]))
    );
    // An empty map names no provider, so the client keeps the ones it had.
    let kept = apply_override(&base, &json(r#"{"clients": {"opencode": {"provider_cost_classes": {}}}}"#)).expect("the override");
    assert_eq!(kept.clients.get("opencode").and_then(|client| client.provider_cost_classes.clone()), base.clients.get("opencode").and_then(|client| client.provider_cost_classes.clone()));
}

#[test]
fn an_override_replaces_the_providers_a_client_may_reach() {
    let base = apply_override(
        &shipped(),
        &json(r#"{"clients": {"opencode": {"binary": "opencode", "cost_class": "metered-personal", "model_source": "live", "providers": ["a"]}}}"#),
    )
    .expect("the base");
    let merged = apply_override(&base, &json(r#"{"clients": {"opencode": {"providers": ["b", "c"]}}}"#)).expect("the override");
    assert_eq!(merged.clients.get("opencode").and_then(|client| client.providers.clone()), Some(vec!["b".to_string(), "c".to_string()]));
    let kept = apply_override(&merged, &json(r#"{"clients": {"opencode": {"cost_class": "subsidised"}}}"#)).expect("the override");
    assert_eq!(kept.clients.get("opencode").and_then(|client| client.providers.clone()), Some(vec!["b".to_string(), "c".to_string()]));
}

#[test]
fn an_override_replaces_the_plan_of_a_client() {
    let base = apply_override(
        &shipped(),
        &json(r#"{"clients": {"codex": {"plan_limits": [{"window": "5h", "limit_usd": 5}, {"window": "day", "limit_runs": 20}]}}}"#),
    )
    .expect("the base");
    assert_eq!(
        base.clients.get("codex").and_then(|client| client.plan_limits.clone()),
        Some(vec![
            PlanLimit::Usd { window: PlanWindow::FiveHours, limit_usd: 5.0 },
            PlanLimit::Runs { window: PlanWindow::Day, limit_runs: 20.0 },
        ])
    );
    let merged = apply_override(&base, &json(r#"{"clients": {"codex": {"plan_limits": [{"window": "week", "limit_usd": 50}]}}}"#)).expect("the override");
    assert_eq!(
        merged.clients.get("codex").and_then(|client| client.plan_limits.clone()),
        Some(vec![PlanLimit::Usd { window: PlanWindow::Week, limit_usd: 50.0 }])
    );
    let kept = apply_override(&merged, &json(r#"{"clients": {"codex": {"cost_class": "subsidised"}}}"#)).expect("the override");
    assert_eq!(kept.clients.get("codex").and_then(|client| client.plan_limits.clone()), merged.clients.get("codex").and_then(|client| client.plan_limits.clone()));
}

#[test]
fn an_override_adds_a_client_the_defaults_do_not_name() {
    let merged = apply_override(
        &shipped(),
        &json(r#"{"clients": {"hermes": {"binary": "hermes", "cost_class": "metered-personal", "model_source": "live", "providers": ["anthropic", "google"]}}}"#),
    )
    .expect("the override");
    let hermes = merged.clients.get("hermes").expect("hermes");
    assert_eq!(hermes.binary, "hermes");
    assert_eq!(hermes.cost_class, CostClass::MeteredPersonal);
    assert_eq!(hermes.model_source, ModelSource::Live);
    assert_eq!(hermes.providers, Some(vec!["anthropic".to_string(), "google".to_string()]));
    assert_eq!(hermes.families, None);
    assert_eq!(hermes.provider_cost_classes, None);
    assert_eq!(hermes.plan_limits, None);
}

#[test]
fn an_override_refuses_a_client_field_it_does_not_know() {
    assert_eq!(
        message_of(apply_override(&shipped(), &json(r#"{"clients": {"vibe": {"read_only_proof": "trusted"}}}"#))),
        "clients.vibe.read_only_proof is not a field a client has; choose from binary, cost_class, model_source, families, provider_cost_classes, providers, plan_limits"
    );
}

#[test]
fn an_override_refuses_a_top_level_key_it_does_not_know() {
    assert_eq!(
        message_of(apply_override(&shipped(), &json(r#"{"client": {"codex": {"cost_class": "subsidised"}}}"#))),
        "the override.client is not a key the override has; choose from version, clients, disabled, models, settings"
    );
}

#[test]
fn an_override_refuses_a_provider_list_that_is_not_one() {
    assert_eq!(
        message_of(apply_override(&shipped(), &json(r#"{"clients": {"vibe": {"providers": "anthropic"}}}"#))),
        "clients.vibe.providers must be a list of non-empty strings"
    );
}

#[test]
fn an_override_overrides_the_tier_notes_and_family_of_a_model_it_names() {
    let merged = apply_override(
        &shipped(),
        &json(r#"{"models": [{"client": "codex", "family": "openai-internal", "model": "gpt-5.5", "notes": "grant needed", "tier": "hard"}]}"#),
    )
    .expect("the override");
    assert_eq!(merged.models.len(), 2);
    assert_eq!(merged.models[1].family, "openai-internal");
    assert_eq!(merged.models[1].notes, "grant needed");
    assert_eq!(merged.models[1].tier, Tier::Hard);
}

#[test]
fn an_override_keeps_the_fields_a_model_override_does_not_name() {
    let merged = apply_override(&shipped(), &json(r#"{"models": [{"client": "claude", "model": "claude-sonnet-5-5", "tier": "frontier"}]}"#)).expect("the override");
    assert_eq!(merged.models[0].family, "anthropic");
    assert_eq!(merged.models[0].notes, "general");
    assert_eq!(merged.models[0].tier, Tier::Frontier);
}

#[test]
fn an_override_reads_the_config_names_of_a_catalogue_entry_and_keeps_them() {
    let base = parse_defaults(&json(
        r#"{"version": 1, "clients": {"vibe": {"binary": "vibe", "cost_class": "subsidised", "model_source": "config"}}, "models": [{"client": "vibe", "family": "zhipu", "model": "glm-5-3", "names": ["zai-glm-5-3"], "notes": "review", "tier": "hard"}], "settings": {"availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3, "metered_personal_caps": {"per_day_usd": 5, "per_workflow_usd": 2}, "output_allowance_tokens": {"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500}, "plan_warning_ratio": 0.8, "summary_max_chars": 1500}}"#,
    ))
    .expect("the base");
    let merged = apply_override(&base, &json(r#"{"models": [{"client": "vibe", "model": "glm-5-3", "tier": "frontier"}]}"#)).expect("the override");
    assert_eq!(merged.models[0].names, Some(vec!["zai-glm-5-3".to_string()]));
    assert_eq!(merged.models[0].tier, Tier::Frontier);
    let renamed = apply_override(&base, &json(r#"{"models": [{"client": "vibe", "model": "glm-5-3", "names": ["zai-glm-6"]}]}"#)).expect("the override");
    assert_eq!(renamed.models[0].names, Some(vec!["zai-glm-6".to_string()]));
}

#[test]
fn an_override_names_a_model_by_one_of_its_config_names_and_keeps_its_alias() {
    let base = parse_defaults(&json(
        r#"{"version": 1, "clients": {"vibe": {"binary": "vibe", "cost_class": "subsidised", "model_source": "config"}}, "models": [{"client": "vibe", "family": "zhipu", "model": "glm-5-3", "names": ["zai-glm-5-3"], "notes": "review", "tier": "hard"}], "settings": {"availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3, "metered_personal_caps": {"per_day_usd": 5, "per_workflow_usd": 2}, "output_allowance_tokens": {"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500}, "plan_warning_ratio": 0.8, "summary_max_chars": 1500}}"#,
    ))
    .expect("the base");
    let merged = apply_override(&base, &json(r#"{"models": [{"client": "vibe", "model": "zai-glm-5-3", "tier": "frontier"}]}"#)).expect("the override");
    assert_eq!(merged.models.len(), 1);
    assert_eq!(merged.models[0].model, "glm-5-3");
    assert_eq!(merged.models[0].names, Some(vec!["zai-glm-5-3".to_string()]));
    assert_eq!(merged.models[0].tier, Tier::Frontier);
}

#[test]
fn an_override_refuses_config_names_that_are_not_a_list_of_non_empty_strings() {
    assert_eq!(
        message_of(apply_override(&shipped(), &json(r#"{"models": [{"client": "claude", "model": "claude-sonnet-5-5", "names": [""]}]}"#))),
        "models[0].names must be a list of non-empty strings"
    );
}

#[test]
fn an_override_adds_a_declared_model_the_defaults_do_not_name() {
    let merged = apply_override(
        &shipped(),
        &json(r#"{"models": [{"client": "claude", "family": "anthropic", "model": "claude-opus-5-5", "notes": "frontier", "tier": "frontier"}]}"#),
    )
    .expect("the override");
    let models: Vec<&str> = merged.models.iter().map(|entry| entry.model.as_str()).collect();
    assert_eq!(models, vec!["claude-sonnet-5-5", "gpt-5.5", "claude-opus-5-5"]);
}

#[test]
fn an_override_overrides_one_setting_and_deep_merges_the_caps() {
    let merged = apply_override(&shipped(), &json(r#"{"settings": {"metered_personal_caps": {"per_workflow_usd": 1}, "summary_max_chars": 500}}"#)).expect("the override");
    assert_eq!(merged.settings.summary_max_chars, 500.0);
    assert_eq!(merged.settings.metered_personal_caps.per_workflow_usd, 1.0);
    assert_eq!(merged.settings.metered_personal_caps.per_day_usd, 5.0);
    assert_eq!(merged.settings.availability_window_minutes, 60.0);
    assert_eq!(merged.settings.plan_warning_ratio, 0.8);
}

#[test]
fn an_override_overrides_the_batch_size_limit_and_the_parallel_limit() {
    let merged = apply_override(&shipped(), &json(r#"{"settings": {"max_batch_delegations": 10, "max_parallel_per_client": 2}}"#)).expect("the override");
    assert_eq!(merged.settings.max_batch_delegations, 10.0);
    assert_eq!(merged.settings.max_parallel_per_client, 2.0);
}

#[test]
fn an_override_overrides_the_output_allowance_of_one_tier_and_keeps_the_others() {
    let merged = apply_override(&shipped(), &json(r#"{"settings": {"output_allowance_tokens": {"hard": 1000}}}"#)).expect("the override");
    assert_eq!(merged.settings.output_allowance_tokens, [500.0, 2000.0, 1000.0, 16000.0]);
}

#[test]
fn an_override_replaces_the_disabled_list_and_keeps_it_when_it_names_none() {
    assert_eq!(apply_override(&shipped(), &json(r#"{"disabled": ["codex:gpt-5.5"]}"#)).expect("the override").disabled, vec!["codex:gpt-5.5".to_string()]);
    let base = apply_override(&shipped(), &json(r#"{"disabled": ["codex:gpt-5.5"]}"#)).expect("the base");
    assert_eq!(apply_override(&base, &json(r#"{"settings": {"summary_max_chars": 500}}"#)).expect("the override").disabled, vec!["codex:gpt-5.5".to_string()]);
}

#[test]
fn an_override_refuses_a_plan_warning_ratio_outside_zero_to_one() {
    assert_eq!(
        message_of(apply_override(&shipped(), &json(r#"{"settings": {"plan_warning_ratio": 80}}"#))),
        "settings.plan_warning_ratio must be between 0 and 1"
    );
}

#[test]
fn an_override_refuses_what_it_cannot_read() {
    let cases: [(&str, &str); 28] = [
        ("[]", "the override must be a JSON object"),
        (r#"{"version": 2}"#, "version must be 1"),
        (r#"{"clients": []}"#, "clients must be an object keyed by client id"),
        (r#"{"clients": {"claude": "claude"}}"#, "clients.claude must be an object"),
        (r#"{"clients": {"hermes": {"binary": "hermes", "model_source": "live"}}}"#, "clients.hermes is new, so it needs binary, cost_class and model_source"),
        (r#"{"clients": {"claude": {"cost_class": "free"}}}"#, "clients.claude.cost_class must be one of subsidised, metered-personal, metered-employer"),
        (r#"{"clients": {"claude": {"model_source": "guess"}}}"#, "clients.claude.model_source must be one of live, config, declared"),
        (r#"{"clients": {"claude": {"families": "google"}}}"#, "clients.claude.families must be a list of non-empty strings"),
        (r#"{"clients": {"claude": {"provider_cost_classes": "free"}}}"#, "clients.claude.provider_cost_classes must be an object keyed by provider name"),
        (r#"{"clients": {"claude": {"provider_cost_classes": {"p": "free"}}}}"#, "clients.claude.provider_cost_classes.p must be one of subsidised, metered-personal, metered-employer"),
        (r#"{"models": {}}"#, "models must be a list"),
        (r#"{"models": ["claude"]}"#, "models[0] must be an object"),
        (r#"{"models": [{"model": "gpt-5.5"}]}"#, "models[0].client must be a non-empty string"),
        (r#"{"models": [{"client": "codex"}]}"#, "models[0].model must be a non-empty string"),
        (r#"{"models": [{"client": "claude", "family": "anthropic", "model": "new", "tier": "hard"}]}"#, "models[0] (claude:new) is new, so it needs family, tier and notes"),
        (r#"{"models": [{"client": "codex", "model": "gpt-5.5", "tier": "epic"}]}"#, "models[0].tier must be one of trivial, routine, hard, frontier"),
        (r#"{"models": [{"client": "codex", "model": "gpt-5.5", "family": ""}]}"#, "models[0].family must be a non-empty string"),
        (r#"{"models": [{"client": "codex", "model": "gpt-5.5", "notes": ""}]}"#, "models[0].notes must be a non-empty string"),
        (r#"{"models": [{"client": "codex", "model": "gpt-5.5", "prices": {"input": 0, "output": 2, "cached_input": 0}}]}"#, "models[0].prices.input and models[0].prices.output must be above 0"),
        (r#"{"settings": []}"#, "settings must be an object"),
        (r#"{"settings": {"summary_max_chars": "long"}}"#, "settings.summary_max_chars must be a number"),
        (r#"{"settings": {"max_parallel_per_client": 0}}"#, "settings.max_parallel_per_client must be a whole number of at least 1"),
        (r#"{"settings": {"max_parallel_per_client": 1.5}}"#, "settings.max_parallel_per_client must be a whole number of at least 1"),
        (r#"{"settings": {"max_batch_delegations": 0}}"#, "settings.max_batch_delegations must be a whole number of at least 1"),
        (r#"{"settings": {"max_batch_delegations": 2.5}}"#, "settings.max_batch_delegations must be a whole number of at least 1"),
        (r#"{"settings": {"metered_personal_caps": 5}}"#, "settings.metered_personal_caps must be an object"),
        (r#"{"settings": {"metered_personal_caps": {"per_day_usd": "five"}}}"#, "settings.metered_personal_caps.per_day_usd must be a number"),
        (r#"{"settings": {"output_allowance_tokens": 500}}"#, "settings.output_allowance_tokens must be an object keyed by tier"),
    ];
    for (raw, message) in cases {
        let error = error_of(apply_override(&shipped(), &json(raw)));
        assert_eq!((error.code.as_str(), error.message.as_str()), ("invalid-config", message), "{raw}");
    }
    // A version the override names and the defaults have is no error, and neither is a price it names.
    assert_eq!(apply_override(&shipped(), &json(r#"{"version": 1}"#)), Ok(shipped()));
    let priced = apply_override(&shipped(), &json(r#"{"models": [{"client": "codex", "model": "gpt-5.5", "prices": {"input": 1, "output": 2, "cached_input": 0}}]}"#))
        .expect("the override");
    assert_eq!(priced.models[1].prices, Some(crate::config::ModelPrices { input: 1.0, cached_input: 0.0, output: 2.0 }));
    // An allowance that is not a whole number of tokens, and a disabled list that is not strings.
    assert_eq!(
        message_of(apply_override(&shipped(), &json(r#"{"settings": {"output_allowance_tokens": {"hard": 1.5}}}"#))),
        "settings.output_allowance_tokens.hard must be a whole number of tokens above 0"
    );
    assert_eq!(
        message_of(apply_override(&shipped(), &json(r#"{"disabled": "codex:gpt-5.5"}"#))),
        "disabled must be a list of non-empty strings"
    );
}

#[test]
fn an_override_reads_the_clients_before_the_disabled_list_and_the_models() {
    let raw = json(r#"{"clients": {"claude": "claude"}, "disabled": [1], "models": {}}"#);
    assert_eq!(message_of(apply_override(&shipped(), &raw)), "clients.claude must be an object");
    let raw = json(r#"{"disabled": [1], "models": {}}"#);
    assert_eq!(message_of(apply_override(&shipped(), &raw)), "disabled must be a list of non-empty strings");
    let raw = json(r#"{"models": {}, "settings": []}"#);
    assert_eq!(message_of(apply_override(&shipped(), &raw)), "models must be a list");
}

// ------------------------------------------------------------------ loadConfig

#[test]
fn the_shipped_defaults_load_with_no_override() {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), home_env("/home/me"));
    let config = load_config(&runtime).expect("the configuration");
    assert_eq!(config.clients.len(), 6);
    assert_eq!(config.models.len(), 21);
}

#[test]
fn the_override_is_merged_over_the_shipped_defaults() {
    let path = "/home/me/.config/agent-kit/model-gateway.json";
    let files = StubFiles::new().with(path, r#"{"settings": {"summary_max_chars": 500}, "disabled": ["codex:gpt-5.5"]}"#);
    let runtime = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    let config = load_config(&runtime).expect("the configuration");
    assert_eq!(config.settings.summary_max_chars, 500.0);
    assert_eq!(config.disabled, vec!["codex:gpt-5.5".to_string()]);
    assert_eq!(config.clients.len(), 6);
}

#[test]
fn defaults_that_are_not_json_name_the_file() {
    let mut runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), home_env("/home/me"));
    runtime.defaults = "{".to_string();
    let error = error_of(load_config(&runtime));
    assert_eq!(error.code, "invalid-config");
    assert_eq!(error.message, "defaults.json: not JSON: Expected property name or '}' in JSON at position 1 (line 1 column 2)");
}

#[test]
fn defaults_that_are_invalid_name_the_file() {
    let mut runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), home_env("/home/me"));
    runtime.defaults = r#"{"version": 2}"#.to_string();
    runtime.defaults_path = "/opt/agent-kit/defaults.json".to_string();
    assert_eq!(error_of(load_config(&runtime)).message, "/opt/agent-kit/defaults.json: version must be 1");
}

#[test]
fn an_override_that_cannot_be_looked_for_names_the_file() {
    let path = "/home/me/.config/agent-kit/model-gateway.json";
    let mut files = StubFiles::new();
    files.exists_error = Some(FsError { code: "EACCES".into(), message: "EACCES: permission denied, stat".into() });
    let runtime = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    let error = error_of(load_config(&runtime));
    assert_eq!(error.code, "invalid-config");
    assert_eq!(error.message, format!("{path}: cannot read the override: EACCES: permission denied, stat"));
}

#[test]
fn an_override_that_cannot_be_read_names_the_file() {
    let path = "/home/me/.config/agent-kit/model-gateway.json";
    let mut files = StubFiles::new();
    files.texts.insert(path.to_string(), String::new());
    files.read_error = Some(FsError { code: "EISDIR".into(), message: "EISDIR: illegal operation on a directory, read".into() });
    let runtime = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(error_of(load_config(&runtime)).message, format!("{path}: cannot read the override: EISDIR: illegal operation on a directory, read"));
}

#[test]
fn an_override_that_is_not_json_names_the_file() {
    let path = "/home/me/.config/agent-kit/model-gateway.json";
    let files = StubFiles::new().with(path, "{");
    let runtime = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(
        error_of(load_config(&runtime)).message,
        format!("{path}: not JSON: Expected property name or '}}' in JSON at position 1 (line 1 column 2)")
    );
}

#[test]
fn an_override_that_is_invalid_names_the_file() {
    let path = "/home/me/.config/agent-kit/model-gateway.json";
    let files = StubFiles::new().with(path, "[]");
    let runtime = runtime(files, StubGit::new(None, &[]), home_env("/home/me"));
    assert_eq!(error_of(load_config(&runtime)).message, format!("{path}: the override must be a JSON object"));
}

#[test]
fn a_machine_without_a_home_cannot_load_its_override() {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), HashMap::new());
    assert_eq!(error_of(load_config(&runtime)), GatewayError::new("unexpected", "required environment variable is missing: HOME"));
}
