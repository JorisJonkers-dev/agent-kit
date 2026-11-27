//! The network reach of a sandboxed run: the profile table, the rules each reach becomes, the package proxy's
//! settings and credential, the task's own hosts and the refusal while an sbx service secret is stored.

use super::*;
use crate::config::GatewayConfig;
use crate::config_parse::parse_defaults;
use crate::extras::RunExtras;
use crate::network::{self, Egress, NetworkScope, Reach};

fn shipped() -> GatewayConfig {
    parse_defaults(&json::parse(crate::SHIPPED_DEFAULTS).expect("JSON")).expect("the shipped defaults")
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_string()).collect()
}

const ROUTE: [&str; 2] = ["host.docker.internal:4000", "localhost:4000"];

fn route() -> Vec<String> {
    strs(&ROUTE)
}

fn global() -> Vec<String> {
    strs(&["api.anthropic.com:443", "registry.npmjs.org:443", "**.github.com:443", "pypi.org:443"])
}

fn scoped(reach: Reach) -> RunExtras {
    RunExtras { network: Some(NetworkScope { reach, ..NetworkScope::default() }), ..RunExtras::default() }
}

fn run_with(sbx: &Sbx, env: &HashMap<String, String>, extras: RunExtras, disk: &Disk, notes: &Notes) -> Result<crate::client_run::ClientRunOutcome, ClientRunError> {
    let request = ClientRunRequest { extras, ..pi_request(RunClient::Pi, Some(RunMode::ReadOnly)) };
    ClientRun { process: sbx, files: disk, clock: &Fixed, env }.launch_watched(&request, None, notes)
}

fn env_with_key(key: &str) -> HashMap<String, String> {
    let mut env = sbx_env();
    env.insert(network::KEY_VARIABLE.to_string(), key.to_string());
    env
}

fn env_file(disk: &Disk) -> String {
    disk.files.lock().expect("files").get("/run/export/pi-agent/sbx.env").cloned().expect("the env file was written")
}

#[test]
fn the_profile_table_gives_each_role_its_reach_and_the_narrowest_is_the_default() {
    let config = shipped();
    let reach = |name: &str| config.profiles.get(name).and_then(|profile| profile.network);
    assert_eq!(reach("research"), Some(Reach::OpenWeb));
    assert_eq!(reach("cheap-research"), Some(Reach::OpenWeb));
    assert_eq!(reach("implement"), Some(Reach::Packages));
    assert_eq!(reach("implement-cheap"), Some(Reach::Packages));
    for name in ["critique", "red-team", "refute", "triage", "cross-critic", "public-gemini"] {
        assert_eq!(reach(name), Some(Reach::ProxyOnly), "{name}");
    }
    // A tier profile states none, which is the narrowest; so is a run with no profile at all.
    assert_eq!(reach("tier-free"), None);
    assert_eq!(NetworkScope::of(config.profiles.get("tier-free"), true).reach, Reach::ProxyOnly);
    assert_eq!(NetworkScope::of(None, true), NetworkScope::default());
    assert_eq!(Reach::NAMES.map(|name| Reach::parse(name).map(Reach::as_str)), Reach::NAMES.map(Some));
    assert_eq!(Reach::parse("everything"), None);
}

#[test]
fn the_open_web_is_cut_to_the_packages_for_a_run_whose_repository_is_not_public() {
    let config = shipped();
    let open = NetworkScope::of(config.profiles.get("research"), true);
    assert_eq!((open.reach, open.cut_from), (Reach::OpenWeb, None));
    let cut = NetworkScope::of(config.profiles.get("research"), false);
    assert_eq!((cut.reach, cut.cut_from), (Reach::Packages, Some(Reach::OpenWeb)));
    // Only the open web is cut; the narrower reaches are the same for a private repository.
    assert_eq!(NetworkScope::of(config.profiles.get("implement"), false).reach, Reach::Packages);
    let text = network::describe(&cut, &Egress::default(), "anonymous");
    assert!(text.contains("cut from open-web because the repository is not public"), "{text}");
}

#[test]
fn a_proxy_only_reach_allows_the_route_and_the_proxy_and_denies_every_other_global_allow() {
    let egress = network::plan(Reach::ProxyOnly, &route(), &[], &global()).expect("a plan");
    assert_eq!(egress.allow, ["host.docker.internal:4000", "localhost:4000", "packages.jorisjonkers.dev:443"]);
    assert_eq!(egress.deny, global());
    assert!(egress.kept.is_empty());
}

#[test]
fn a_packages_reach_adds_the_registries_and_github_keeps_the_wider_global_it_sits_under_and_denies_the_rest() {
    let egress = network::plan(Reach::Packages, &route(), &[], &global()).expect("a plan");
    for host in ["registry.npmjs.org:443", "pypi.org:443", "crates.io:443", "proxy.golang.org:443", "ghcr.io:443", "api.github.com:443", "registry-1.docker.io:443", "packages.jorisjonkers.dev:443"] {
        assert!(egress.allow.iter().any(|rule| rule == host), "{host}");
    }
    // An exact global allow is not denied; a wildcard that covers a needed host is kept and said; a provider is denied.
    assert_eq!(egress.deny, ["api.anthropic.com:443"]);
    assert_eq!(egress.kept, ["**.github.com:443"]);
    let text = network::describe(&NetworkScope { reach: Reach::Packages, ..NetworkScope::default() }, &egress, "anonymous");
    assert!(text.contains("denied 1 global allows") && text.contains("kept wider global allows **.github.com:443"), "{text}");
}

#[test]
fn the_open_web_allows_every_host_and_denies_nothing() {
    let egress = network::plan(Reach::OpenWeb, &route(), &[], &global()).expect("a plan");
    assert!(egress.allow.iter().any(|rule| rule == "**"));
    assert!(egress.deny.is_empty() && egress.kept.is_empty());
    assert!(network::describe(&NetworkScope { reach: Reach::OpenWeb, ..NetworkScope::default() }, &egress, "key injected").contains("nothing of the machine's global policy is denied"));
}

#[test]
fn a_global_policy_that_allows_every_host_cannot_be_narrowed_and_refuses_a_narrow_reach() {
    for everything in ["**:443", "**", "*"] {
        let refused = network::plan(Reach::ProxyOnly, &route(), &[], &strs(&[everything])).expect_err("cannot be narrowed");
        assert_eq!(refused.code, "sandbox-unsafe");
        assert!(refused.message.contains("allows every host"), "{}", refused.message);
    }
    assert!(network::plan(Reach::OpenWeb, &route(), &[], &strs(&["**"])).is_ok());
}

#[test]
fn the_tasks_hosts_are_added_with_port_443_unless_named_and_a_pattern_is_refused() {
    let extras = [network::extra_rule("docs.example.org").expect("host"), network::extra_rule("api.example.org:8443").expect("host and port")];
    assert_eq!(extras, ["docs.example.org:443", "api.example.org:8443"]);
    let egress = network::plan(Reach::ProxyOnly, &route(), &extras, &global()).expect("a plan");
    assert!(egress.allow.ends_with(&strs(&["docs.example.org:443", "api.example.org:8443"])), "{:?}", egress.allow);
    for bad in ["", "*", "**.example.org", "Example.org", ".example.org", "example.org.", "example.org:", "example.org:x", "exa mple.org"] {
        let refused = network::extra_rule(bad).expect_err(bad);
        assert_eq!(refused.code, "invalid-extras", "{bad}");
    }
}

#[test]
fn a_pattern_covers_a_host_by_label_and_port() {
    assert!(network::rule_covers("**.github.com:443", "api.github.com:443"));
    assert!(network::rule_covers("**.github.com:443", "a.b.github.com:443"));
    assert!(!network::rule_covers("**.github.com:443", "github.com:443"));
    assert!(!network::rule_covers("**.github.com:443", "api.github.com:80"));
    assert!(network::rule_covers("*.example.com", "a.example.com:443"));
    assert!(!network::rule_covers("*.example.com", "a.b.example.com:443"));
    assert!(network::rule_covers("api?.example.com:443", "api1.example.com:443"));
    assert!(!network::rule_covers("api?.example.com:443", "api.example.com:443"));
    assert!(!network::rule_covers("a?.example.com:443", "a..example.com:443"));
    assert!(network::rule_covers("example.com:443", "example.com:443"));
    assert!(!network::rule_covers("example.com:443", "example.org:443"));
}

#[test]
fn the_global_allows_are_read_from_the_policy_listing_and_an_unreadable_one_refuses() {
    let listing = GLOBAL_POLICY;
    assert_eq!(network::global_allows(listing).expect("rules"), global());
    for bad in ["", "not json", "[]", r#"{"rules":"x"}"#, r#"{"other":[]}"#] {
        let refused = network::global_allows(bad).expect_err(bad);
        assert_eq!(refused.code, "sandbox-unsafe", "{bad}");
    }
    // A rule that is not an object is skipped.
    assert!(network::global_allows(r#"{"rules":[1,{"scope":"global","decision":"allow","resource_type":"network"}]}"#).expect("rules").is_empty());
}

#[test]
fn a_stored_global_service_secret_refuses_unless_the_profile_names_it() {
    let github = r#"{"secrets":[{"scope":"global","type":"service","name":"github","secret":"(stored)"}],"custom_secrets":[]}"#;
    let refused = network::require_no_service_secrets(github, &[]).expect_err("a stored secret");
    assert_eq!(refused.code, "sandbox-unsafe");
    assert!(refused.message.contains("(github)") && refused.message.contains("sbx secret rm github") && refused.message.contains("sbx_secrets"), "{}", refused.message);
    assert!(network::require_no_service_secrets(github, &strs(&["github"])).is_ok());
    let untyped = r#"{"secrets":[{"scope":"global","name":"openai"}]}"#;
    assert!(network::require_no_service_secrets(untyped, &strs(&["github"])).is_err());
    // A registry credential is never injected, a custom secret is not a service secret, and none is fine.
    assert!(network::require_no_service_secrets(r#"{"secrets":[{"type":"registry","name":"ghcr.io"}],"custom_secrets":[{"scope":"x"}]}"#, &[]).is_ok());
    assert!(network::require_no_service_secrets(r#"{"secrets":[]}"#, &[]).is_ok());
    for bad in ["", "{}", r#"{"secrets":"x"}"#] {
        assert_eq!(network::require_no_service_secrets(bad, &[]).expect_err(bad).code, "sandbox-unsafe", "{bad}");
    }
}

#[test]
fn the_package_managers_are_pointed_at_the_proxy_with_only_a_placeholder_for_a_credential() {
    let placeholder = network::placeholder("mg-1-abc");
    assert!(placeholder.starts_with("mgpk-") && placeholder.len() == 29, "{placeholder}");
    assert_ne!(placeholder, network::placeholder("mg-2-abc"));
    let env = network::package_env(Some(&placeholder), "/run/packages.npmrc");
    for expected in [
        format!("GOPROXY=https://packages:{placeholder}@packages.jorisjonkers.dev/go"),
        format!("PIP_INDEX_URL=https://packages:{placeholder}@packages.jorisjonkers.dev/pypi/simple/"),
        format!("UV_DEFAULT_INDEX=https://packages:{placeholder}@packages.jorisjonkers.dev/pypi/simple/"),
        "NPM_CONFIG_REGISTRY=https://packages.jorisjonkers.dev/npm/".to_string(),
        "NPM_CONFIG_USERCONFIG=/run/packages.npmrc".to_string(),
        "CARGO_REGISTRIES_JORISJONKERS_PACKAGES_INDEX=sparse+https://packages.jorisjonkers.dev/cargo/".to_string(),
        format!("CARGO_REGISTRIES_JORISJONKERS_PACKAGES_TOKEN={placeholder}"),
    ] {
        assert!(env.contains(&expected), "{expected} in {env:?}");
    }
    assert_eq!(network::npmrc_text(Some(&placeholder)), format!("registry=https://packages.jorisjonkers.dev/npm/\n//packages.jorisjonkers.dev/npm/:_authToken={placeholder}\n"));
    // Without a key every manager goes anonymously: no credential in any setting.
    let anonymous = network::package_env(None, "/run/packages.npmrc");
    assert!(anonymous.iter().all(|line| !line.contains("packages:") && !line.contains("TOKEN")), "{anonymous:?}");
    assert!(anonymous.contains(&"GOPROXY=https://packages.jorisjonkers.dev/go".to_string()));
    assert_eq!(network::npmrc_text(None), "registry=https://packages.jorisjonkers.dev/npm/\n");
}

#[test]
fn the_proxy_key_is_the_variable_then_the_stored_file_and_otherwise_none() {
    let disk = Disk::default();
    let mut env = sbx_env();
    assert_eq!(network::proxy_key(&disk, &env), None);
    disk.files.lock().expect("files").insert("/home/me/.config/jorisjonkers/packages-token".to_string(), "from-home\n".to_string());
    assert_eq!(network::proxy_key(&disk, &env).as_deref(), Some("from-home"));
    disk.files.lock().expect("files").insert("/xdg/jorisjonkers/packages-token".to_string(), " from-xdg ".to_string());
    env.insert("XDG_CONFIG_HOME".to_string(), "/xdg".to_string());
    assert_eq!(network::proxy_key(&disk, &env).as_deref(), Some("from-xdg"));
    env.insert(network::KEY_VARIABLE.to_string(), " from-env \n".to_string());
    assert_eq!(network::proxy_key(&disk, &env).as_deref(), Some("from-env"));
    // An empty variable and an empty file are no key.
    env.insert(network::KEY_VARIABLE.to_string(), "  ".to_string());
    disk.files.lock().expect("files").insert("/xdg/jorisjonkers/packages-token".to_string(), "\n".to_string());
    assert_eq!(network::proxy_key(&disk, &env), None);
    assert_eq!(network::proxy_key(&disk, &HashMap::new()), None);
}

#[test]
fn a_run_with_no_profile_gets_the_proxy_alone_and_records_what_it_denied_with_no_credential_where_there_is_no_key() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx::new(healthy);
    let notes = Notes::default();
    run_with(&sbx, &sbx_env(), RunExtras::default(), &disk, &notes).expect("a run");
    let calls = sbx.calls();
    let name = sbx_name_in(&calls);
    assert_eq!(calls[0], "sbx version");
    assert_eq!(calls[1], "sbx secret ls -g --json");
    assert_eq!(calls[2], "sbx ls -q");
    assert_eq!(calls[4], "sbx policy ls --json");
    assert_eq!(calls[6], format!("sbx policy allow network --sandbox {name} host.docker.internal:4000,localhost:4000,packages.jorisjonkers.dev:443"));
    assert_eq!(calls[7], format!("sbx policy deny network --sandbox {name} api.anthropic.com:443,registry.npmjs.org:443,**.github.com:443,pypi.org:443"));
    assert!(!calls.iter().any(|call| call.starts_with("sbx secret set-custom")), "no key, no secret: {calls:?}");
    let file = env_file(&disk);
    assert!(file.contains("GOPROXY=https://packages.jorisjonkers.dev/go\n") && file.contains("NPM_CONFIG_USERCONFIG=/run/export/pi-agent/packages.npmrc\n"), "{file}");
    assert_eq!(disk.files.lock().expect("files").get("/run/export/pi-agent/packages.npmrc").map(String::as_str), Some("registry=https://packages.jorisjonkers.dev/npm/\n"));
    let notes = notes.0.lock().expect("notes");
    let reach = notes.iter().find(|note| note.starts_with("network reach proxy-only")).expect("the reach is recorded");
    assert!(reach.contains("denied 4 global allows") && reach.contains("no key on this machine") && reach.contains("anonymously"), "{reach}");
}

#[test]
fn the_proxy_key_reaches_the_sandbox_only_as_an_sbx_custom_secret_scoped_to_the_proxy_host() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx::new(healthy);
    let notes = Notes::default();
    run_with(&sbx, &env_with_key("the-real-key"), scoped(Reach::Packages), &disk, &notes).expect("a run");
    let calls = sbx.calls();
    let name = sbx_name_in(&calls);
    let placeholder = network::placeholder(&name);
    let secret = calls.iter().position(|call| call.starts_with("sbx secret set-custom")).expect("the secret is stored");
    assert_eq!(calls[secret], format!("sbx secret set-custom --sandbox {name} --host packages.jorisjonkers.dev --placeholder {placeholder} --value-stdin"));
    // The secret is stored after the sandbox and its rules exist, and before the client runs.
    assert!(calls[secret - 1].starts_with("sbx policy deny network") && calls[secret + 1].starts_with("sbx exec"), "{calls:?}");
    assert_eq!(sbx.stdins()[0], "the-real-key", "the key is the stdin of the secret call");
    // The key is in no argument, no env file and no npm config; the sandbox holds the placeholder.
    assert!(calls.iter().all(|call| !call.contains("the-real-key")), "{calls:?}");
    let file = env_file(&disk);
    assert!(!file.contains("the-real-key") && file.contains(&format!("GOPROXY=https://packages:{placeholder}@packages.jorisjonkers.dev/go")), "{file}");
    let files = disk.files.lock().expect("files");
    assert!(files.values().all(|text| !text.contains("the-real-key")));
    assert!(files.get("/run/export/pi-agent/packages.npmrc").expect("npmrc").contains(&format!("_authToken={placeholder}")));
    let recorded = notes.0.lock().expect("notes").iter().find(|note| note.starts_with("network reach packages")).cloned().expect("the reach");
    assert!(recorded.contains("key injected by the host proxy") && !recorded.contains("the-real-key"), "{recorded}");
}

#[test]
fn a_research_run_gets_the_open_web_and_a_critique_run_gets_the_proxy_alone() {
    let disk = pi_disk("http://localhost:4000/v1");
    let research = Sbx::new(healthy);
    run_with(&research, &sbx_env(), scoped(Reach::OpenWeb), &disk, &Notes::default()).expect("a run");
    let calls = research.calls();
    assert!(calls.iter().any(|call| call.starts_with("sbx policy allow network") && call.contains(",**,") || call.ends_with(",**")), "{calls:?}");
    assert!(!calls.iter().any(|call| call.starts_with("sbx policy deny") || call == "sbx policy ls --json"), "the open web denies nothing and reads no global policy: {calls:?}");

    let critique = Sbx::new(healthy);
    run_with(&critique, &sbx_env(), scoped(Reach::ProxyOnly), &disk, &Notes::default()).expect("a run");
    let calls = critique.calls();
    assert!(calls.iter().any(|call| call.starts_with("sbx policy deny network") && call.contains("registry.npmjs.org:443") && call.contains("**.github.com:443")), "{calls:?}");
    assert!(!calls.iter().any(|call| call.contains("**,") || call.ends_with(",**")), "{calls:?}");
}

#[test]
fn the_tasks_hosts_reach_the_sandbox_as_allow_rules_and_a_bad_one_is_refused_before_anything_is_made() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx::new(healthy);
    let extras = RunExtras { network_allow: Some(strs(&["docs.example.org", "api.example.org:8443"])), ..scoped(Reach::Packages) };
    run_with(&sbx, &sbx_env(), extras, &disk, &Notes::default()).expect("a run");
    let calls = sbx.calls();
    let allow = calls.iter().find(|call| call.starts_with("sbx policy allow network")).expect("an allow");
    assert!(allow.ends_with(",docs.example.org:443,api.example.org:8443"), "{allow}");

    let refused = Sbx::new(healthy);
    let extras = RunExtras { network_allow: Some(strs(&["**"])), ..scoped(Reach::Packages) };
    let (code, message) = gateway_code(run_with(&refused, &sbx_env(), extras, &disk, &Notes::default()).expect_err("refused"));
    assert_eq!(code, "invalid-extras");
    assert!(message.contains("network_allow"), "{message}");
    assert!(!refused.calls().iter().any(|call| call.starts_with("sbx create")), "{:?}", refused.calls());
}

#[test]
fn network_allow_needs_the_sbx_backend_and_a_client_that_applies_it() {
    let disk = pi_disk("http://localhost:4000/v1");
    let sbx = Sbx::new(healthy);
    let env = HashMap::from([("HOME".to_string(), "/home/me".to_string())]);
    let extras = RunExtras { network_allow: Some(strs(&["docs.example.org"])), ..RunExtras::default() };
    let request = ClientRunRequest { extras: extras.clone(), ..pi_request(RunClient::Pi, Some(RunMode::ReadOnly)) };
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&request).expect_err("refused"));
    assert_eq!((code.as_str(), message.contains("needs the sbx backend")), ("extras-not-supported", true));
    let request = ClientRunRequest { extras, ..pi_request(RunClient::Claude, Some(RunMode::ReadOnly)) };
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &sbx_env() }.launch(&request).expect_err("refused"));
    assert_eq!((code.as_str(), message.as_str()), ("extras-not-supported", "claude cannot apply the network_allow extra"));
}

#[test]
fn a_stored_service_secret_stops_the_launch_before_anything_is_made_unless_the_profile_names_it() {
    let github = r#"{"secrets":[{"scope":"global","type":"service","name":"github","secret":"(stored)"}],"custom_secrets":[],"shadowed_services":[],"env_only_count":0}"#;
    let stored = |args: &[&str]| if args.starts_with(&["secret", "ls"]) { answer(github, "", 0) } else { healthy(args) };
    let disk = pi_disk("http://localhost:4000/v1");
    let refused = Sbx::new(stored);
    let (code, message) = gateway_code(run_with(&refused, &sbx_env(), RunExtras::default(), &disk, &Notes::default()).expect_err("refused"));
    assert_eq!(code, "sandbox-unsafe");
    assert!(message.contains("global service secret (github)"), "{message}");
    assert_eq!(refused.calls(), ["sbx version", "sbx secret ls -g --json"]);

    let named = Sbx::new(stored);
    let extras = RunExtras { network: Some(NetworkScope { secrets: strs(&["github"]), ..NetworkScope::default() }), ..RunExtras::default() };
    run_with(&named, &sbx_env(), extras, &disk, &Notes::default()).expect("the profile names the secret");

    // A secret listing that cannot be read is as unsafe as one that names a secret.
    for failing in [Sbx::new(|args| if args.starts_with(&["secret", "ls"]) { answer("", "no", 1) } else { healthy(args) }), Sbx::new(|args| if args.starts_with(&["secret", "ls"]) { Err("gone".to_string()) } else { healthy(args) })] {
        let (code, message) = gateway_code(run_with(&failing, &sbx_env(), RunExtras::default(), &disk, &Notes::default()).expect_err("refused"));
        assert_eq!(code, "sandbox-unsafe");
        assert!(message.contains("cannot be checked for stored sbx service secrets"), "{message}");
    }
}

#[test]
fn a_global_policy_that_cannot_be_read_or_narrowed_stops_a_narrow_run_and_removes_what_it_made() {
    let disk = pi_disk("http://localhost:4000/v1");
    for answer_for_policy in [answer("", "no", 1), answer("not json", "", 0), answer(r#"{"rules":[{"scope":"global","decision":"allow","resource_type":"network","resources":["**"]}]}"#, "", 0)] {
        let sbx = Sbx::new(move |args| if args.starts_with(&["policy", "ls"]) { answer_for_policy.clone() } else { healthy(args) });
        let (code, _) = gateway_code(run_with(&sbx, &sbx_env(), scoped(Reach::ProxyOnly), &disk, &Notes::default()).expect_err("refused"));
        assert_eq!(code, "sandbox-unsafe");
        assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")), "{:?}", sbx.calls());
    }
    let unrun = Sbx::new(|args| if args.starts_with(&["policy", "ls"]) { Err("gone".to_string()) } else { healthy(args) });
    assert_eq!(gateway_code(run_with(&unrun, &sbx_env(), scoped(Reach::Packages), &disk, &Notes::default()).expect_err("refused")).0, "sandbox-unsafe");
}

#[test]
fn a_rule_or_secret_sbx_refuses_is_the_runs_error_and_the_sandbox_is_removed() {
    let disk = pi_disk("http://localhost:4000/v1");
    for (step, env) in [("deny", sbx_env()), ("set-custom", env_with_key("k"))] {
        let sbx = Sbx { run: Box::new(move |args| if args.get(1) == Some(&"set-custom") { answer("", "refused", 1) } else { answer("ok\n", "", 0) }), ..Sbx::new(move |args| if args.get(1) == Some(&step) { answer("", "refused", 1) } else { healthy(args) }) };
        let error = run_with(&sbx, &env, scoped(Reach::ProxyOnly), &disk, &Notes::default()).expect_err("the step failed");
        let ClientRunError::Spawn(message) = error else { panic!("a spawn error") };
        assert!(message.contains("refused"), "{message}");
        let calls = sbx.calls();
        assert!(calls.last().expect("calls").starts_with("sbx rm --force mg-"), "{calls:?}");
        assert!(!calls.iter().any(|call| call.starts_with("sbx exec")), "{calls:?}");
    }
    // A secret call that cannot start, and an exit the process reports as an error, are the run's error too.
    let unrun = Sbx { run: Box::new(|args| if args.get(1) == Some(&"set-custom") { Err("no sbx".to_string()) } else { answer("ok\n", "", 0) }), ..Sbx::new(healthy) };
    let error = run_with(&unrun, &env_with_key("k"), scoped(Reach::ProxyOnly), &disk, &Notes::default()).expect_err("failed");
    assert!(matches!(error, ClientRunError::Spawn(message) if message.contains("sbx secret did not run: no sbx")));
}

#[test]
fn the_runs_npm_config_that_cannot_be_written_stops_the_launch() {
    struct Refusing(Disk);
    impl Files for Refusing {
        fn read_text(&self, path: &str) -> Result<String, FsError> {
            self.0.read_text(path)
        }
        fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
            self.0.write_atomic(path, content)
        }
        fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
            self.0.mkdir_all(path)
        }
        fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
            self.0.create_exclusive(path)
        }
        fn remove_file(&self, path: &str) -> Result<(), FsError> {
            self.0.remove_file(path)
        }
        fn modified_ms(&self, path: &str) -> Option<i64> {
            self.0.modified_ms(path)
        }
        fn exists(&self, path: &str) -> Result<bool, FsError> {
            self.0.exists(path)
        }
        fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
            self.0.append(path, content)
        }
        fn stamp(&self, path: &str) -> Option<FileStamp> {
            self.0.stamp(path)
        }
        fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
            if path.ends_with("packages.npmrc") {
                return Err(FsError { code: "EACCES".into(), message: format!("EACCES: {path}") });
            }
            self.0.write_private(path, content)
        }
        fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
            self.0.write_text(path, content)
        }
        fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
            self.0.temp_dir(parent, prefix)
        }
        fn remove(&self, path: &str) -> Result<(), FsError> {
            self.0.remove(path)
        }
    }
    let disk = Refusing(pi_disk("http://localhost:4000/v1"));
    let sbx = Sbx::new(healthy);
    let request = ClientRunRequest { extras: scoped(Reach::ProxyOnly), ..pi_request(RunClient::Pi, Some(RunMode::ReadOnly)) };
    let env = sbx_env();
    let (code, message) = gateway_code(ClientRun { process: &sbx, files: &disk, clock: &Fixed, env: &env }.launch(&request).expect_err("refused"));
    assert_eq!(code, "sandbox-unsupported");
    assert!(message.contains("the run's npm config was not written"), "{message}");
    assert!(!sbx.calls().iter().any(|call| call.starts_with("sbx create")), "{:?}", sbx.calls());
}

#[test]
fn the_credential_never_prints_its_key() {
    let mut launch = sbx_launch::translate(RunClient::Pi, "mg-test", &ProcessCommand::new("/opt/agent-launcher", &["confine", "--write", "/run/export", "--", "pi"]), ROUTE_REMOTE, &|_| Ok(String::new())).expect("a translation");
    sbx_launch::apply_network(&mut launch, &Sbx::new(healthy), &Disk::default(), &env_with_key("the-real-key"), &NetworkScope::default(), &[], "/run/x.npmrc").expect("applied");
    assert!(!format!("{launch:?}").contains("the-real-key"));
    assert!(format!("{launch:?}").contains("<hidden>"));
}

#[test]
fn a_profile_states_its_reach_and_secrets_in_the_defaults_and_a_malformed_one_is_refused() {
    let broken = |from: &str, to: &str| {
        let text = crate::SHIPPED_DEFAULTS.replacen(from, to, 1);
        assert_ne!(text, crate::SHIPPED_DEFAULTS, "{from} is in the defaults");
        parse_defaults(&json::parse(&text).expect("JSON")).expect_err("refused").message
    };
    assert!(broken(r#""network": "open-web""#, r#""network": "everywhere""#).contains(".network must be one of open-web, packages, proxy-only"));
    assert!(broken(r#""network": "open-web""#, r#""network": 3"#).contains(".network must be one of"));
    assert!(broken(r#""network": "open-web""#, r#""sbx_secrets": []"#).contains(".sbx_secrets must not be empty"));
    let named = crate::SHIPPED_DEFAULTS.replacen(r#""network": "open-web""#, r#""network": "open-web", "sbx_secrets": ["github"]"#, 1);
    let config = parse_defaults(&json::parse(&named).expect("JSON")).expect("a profile naming a secret");
    let scope = NetworkScope::of(config.profiles.values().find(|profile| profile.sbx_secrets.is_some()), true);
    assert_eq!(scope.secrets, ["github"]);
}

#[test]
fn a_rule_the_route_and_the_task_both_name_is_allowed_once() {
    let route = strs(&["host.docker.internal:4000", network::PROXY_RULE]);
    let extras = strs(&["host.docker.internal:4000"]);
    let egress = network::plan(Reach::ProxyOnly, &route, &extras, &[]).expect("a plan");
    assert_eq!(egress.allow, ["host.docker.internal:4000", network::PROXY_RULE]);
}
