use super::*;

#[path = "../../../test-support/fake_exe.rs"]
mod fake_exe;
use std::sync::atomic::{AtomicUsize, Ordering};

const KEY: &str = "sk-test-route-key-0123456789";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Tmp {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("litellm-harness-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        path
    }
    fn script(&self, name: &str, body: &str) -> String {
        let path = self.path(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        fake_exe::write_script(&path, &format!("#!/bin/sh\n{body}\n"));
        path.display().to_string()
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn table() -> Value {
    serde_json::json!({
        "base_url": "https://litellm.example/v1",
        "routes": [
            {"agent": "vibe", "provider": "litellm-vibe", "key_env": "LITELLM_VIBE_KEY", "data_scope": "any",
             "models": [{"id": "glm-5-3"}, {"id": "codestral", "contextWindow": 256000}]},
            {"agent": "public", "provider": "litellm-public", "key_env": "LITELLM_PUBLIC_KEY", "data_scope": "public",
             "models": [{"id": "gemini-flash"}]},
        ],
    })
}

fn table_file(tmp: &Tmp) -> String {
    tmp.file("harnesses.json", &table().to_string())
        .display()
        .to_string()
}

fn ctx(tmp: &Tmp) -> Ctx {
    let mut environ = Env::new();
    environ.insert("HOME".into(), tmp.0.clone().into_os_string());
    environ.insert("PATH".into(), "/usr/bin:/bin".into());
    environ.insert("ANTHROPIC_API_KEY".into(), "other".into());
    Ctx {
        environ,
        exe: tmp.path("bin/litellm-harness"),
    }
}

type Call = (String, Vec<String>, Env);

fn main_run(tmp: &Tmp, args: &[&str]) -> (i32, String, String, Vec<Call>) {
    main_in(&ctx(tmp), args)
}

fn main_in(ctx: &Ctx, args: &[&str]) -> (i32, String, String, Vec<Call>) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut calls: Vec<Call> = Vec::new();
    let mut exec = |program: &str, argv: &[String], env: &Env| {
        calls.push((program.to_string(), argv.to_vec(), env.clone()));
        None
    };
    let code = main_with(&args, ctx, &mut exec, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
        calls,
    )
}

fn get<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    env.get(OsStr::new(name)).and_then(|v| v.to_str())
}

fn yaml(text: &str) -> Yaml {
    serde_yaml::from_str(text).unwrap()
}

// ----- the rendered table -------------------------------------------------------

#[test]
fn the_rendered_table_has_routes_with_providers_and_holds_no_key() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../registry/generated/litellm/harnesses.json");
    let body = std::fs::read_to_string(path).unwrap();
    let table: Value = serde_json::from_str(&body).unwrap();
    assert!(!routes(&table).is_empty());
    for route in routes(&table) {
        let agent = route["agent"].as_str().unwrap();
        assert_eq!(route["provider"], format!("litellm-{agent}"));
        assert!(route["key_env"].as_str().unwrap().starts_with("LITELLM_"));
        assert!(
            // paid holds only paid/<model> groups, which the published catalogue lists once the sync plans them.
            !model_ids(route).unwrap().is_empty() || matches!(agent, "local" | "free" | "paid"),
            "only local, free and paid have no usable model yet"
        );
    }
    assert!(!body.contains("sk-"));
}

// ----- configure ----------------------------------------------------------------

#[test]
fn codex_gets_a_model_provider_per_route_and_keeps_the_owners_config() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.file(
        "codex/config.toml",
        "model = \"gpt-5\"\n[model_providers.mine]\nname = \"mine\"\n[model_providers.litellm-old]\nname=\"x\"\n",
    );
    let config_arg = config.to_str().unwrap();
    let (code, out, _, _) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "codex",
            "--config",
            config_arg,
        ],
    );
    assert_eq!(code, 0);
    assert!(out.contains("written"), "{out}");
    let doc: DocumentMut = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    assert_eq!(doc["model"].as_str(), Some("gpt-5"));
    assert_eq!(
        doc["model_providers"]["mine"]["name"].as_str(),
        Some("mine")
    );
    assert!(doc["model_providers"].get("litellm-old").is_none());
    let vibe = &doc["model_providers"]["litellm-vibe"];
    assert_eq!(vibe["name"].as_str(), Some("LiteLLM vibe"));
    assert_eq!(
        vibe["base_url"].as_str(),
        Some("https://litellm.example/v1")
    );
    assert_eq!(vibe["env_key"].as_str(), Some("LITELLM_VIBE_KEY"));
    assert_eq!(vibe["wire_api"].as_str(), Some("responses"));
    assert!(doc["model_providers"].get("litellm-public").is_some());
    assert!(tmp.path("codex/config.toml.bak").exists());
    let (_, again, _, _) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "codex",
            "--config",
            config_arg,
        ],
    );
    assert!(again.ends_with("in place\n"), "{again}");
}

#[test]
fn codex_config_without_providers_gets_the_table_and_a_private_mode() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.path("config.toml");
    let (code, ..) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "codex",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    let doc: DocumentMut = std::fs::read_to_string(&config).unwrap().parse().unwrap();
    assert!(doc["model_providers"].get("litellm-vibe").is_some());
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn an_existing_files_mode_is_kept() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.file("config.toml", "a = 1\n");
    std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o640)).unwrap();
    main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "codex",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(
        std::fs::metadata(&config).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn codex_refuses_what_it_cannot_merge() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    for (body, why) in [
        ("model_providers = 3\n", "not a table"),
        ("= broken", "TOML"),
    ] {
        let config = tmp.file("config.toml", body);
        let (code, _, err, _) = main_run(
            &tmp,
            &[
                "--table",
                &table,
                "configure",
                "codex",
                "--config",
                config.to_str().unwrap(),
            ],
        );
        assert_eq!(code, 2, "{why}");
        assert!(err.contains("left as it is"), "{err}");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), body);
    }
    // an inline table of providers is merged too
    let config = tmp.file(
        "config.toml",
        "model_providers = { mine = { name = \"m\" } }\n",
    );
    let (code, ..) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "codex",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    assert!(std::fs::read_to_string(&config)
        .unwrap()
        .contains("litellm-vibe"));
}

#[test]
fn hermes_gets_a_custom_provider_with_a_base_url_and_a_key_variable() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.file(
        "config.yaml",
        "model: x\nproviders:\n  mine: {base_url: http://a}\n  litellm-old: {}\n  1: numeric\n",
    );
    let (code, ..) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "hermes",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    let data = yaml(&std::fs::read_to_string(&config).unwrap());
    assert_eq!(data["model"], "x");
    assert_eq!(data["providers"]["mine"]["base_url"], "http://a");
    assert_eq!(data["providers"][1], "numeric");
    let entry = &data["providers"]["litellm-vibe"];
    assert_eq!(entry["base_url"], "https://litellm.example/v1");
    assert_eq!(entry["key_env"], "LITELLM_VIBE_KEY");
    assert_eq!(entry["default_model"], "glm-5-3");
    assert_eq!(entry["api_mode"], "chat_completions");
    assert!(entry["models"]["codestral"].is_mapping());
    assert!(data["providers"].get("litellm-old").is_none());
}

#[test]
fn hermes_handles_an_empty_file_a_null_providers_field_and_a_route_without_models() {
    let tmp = Tmp::new();
    let mut with_empty = table();
    with_empty["routes"][1]["models"] = serde_json::json!([]);
    let path = tmp.file("t.json", &with_empty.to_string());
    for body in ["", "providers:\n"] {
        let config = tmp.file("config.yaml", body);
        let args = [
            "--table",
            path.to_str().unwrap(),
            "configure",
            "hermes",
            "--config",
            config.to_str().unwrap(),
        ];
        assert_eq!(main_run(&tmp, &args).0, 0);
        let data = yaml(&std::fs::read_to_string(&config).unwrap());
        assert!(data["providers"]["litellm-vibe"].is_mapping());
        assert!(data["providers"]["litellm-public"]
            .get("default_model")
            .is_none());
    }
}

#[test]
fn hermes_refuses_a_config_that_is_not_a_mapping() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    for body in ["- a\n", "providers: [1]\n", "a: [\n", "# only a comment\n"] {
        let config = tmp.file("config.yaml", body);
        let (code, _, err, _) = main_run(
            &tmp,
            &[
                "--table",
                &table,
                "configure",
                "hermes",
                "--config",
                config.to_str().unwrap(),
            ],
        );
        assert_eq!(code, 2, "{body}");
        assert!(err.contains("left as it is"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), body);
    }
}

#[test]
fn opencode_gets_a_provider_that_reads_its_key_from_the_environment() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.file(
        "opencode.json",
        &serde_json::json!({"mcp": {"servers": {}}, "provider": {"mine": {"npm": "x"}, "litellm-old": {}}}).to_string(),
    );
    let (code, ..) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "opencode",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(&config).unwrap();
    assert!(body.ends_with("}\n"));
    let data: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(data["$schema"], SCHEMA);
    assert_eq!(data["mcp"], serde_json::json!({"servers": {}}));
    assert_eq!(data["provider"]["mine"], serde_json::json!({"npm": "x"}));
    assert!(data["provider"].get("litellm-old").is_none());
    let entry = &data["provider"]["litellm-public"];
    assert_eq!(
        entry["options"],
        serde_json::json!({"baseURL": "https://litellm.example/v1", "apiKey": "{env:LITELLM_PUBLIC_KEY}"})
    );
    assert_eq!(
        entry["models"],
        serde_json::json!({"gemini-flash": {"name": "gemini-flash"}})
    );
    // the owner's key order survives: mcp, then provider, then the added schema
    assert!(body.find("\"mcp\"").unwrap() < body.find("\"provider\"").unwrap());
}

#[test]
fn opencode_reads_compact_json_and_keeps_a_schema_it_has() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.file(
        "opencode.json",
        "{\"$schema\":\"https://own\",\"theme\":\"x\"}",
    );
    let (code, ..) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "opencode",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(data["$schema"], "https://own");
    assert_eq!(data["theme"], "x");
    assert!(data["provider"]["litellm-vibe"].is_object());
}

#[test]
fn opencode_refuses_a_file_that_is_not_an_object_with_an_object_provider() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    for body in [
        "[1, 2]",
        "{\"provider\": 3}",
        "{\"provider\": null}",
        "{not json",
        "{\"a\": 1,}",
    ] {
        let config = tmp.file("opencode.json", body);
        let (code, _, err, _) = main_run(
            &tmp,
            &[
                "--table",
                &table,
                "configure",
                "opencode",
                "--config",
                config.to_str().unwrap(),
            ],
        );
        assert_eq!(code, 2, "{body}");
        assert!(err.contains("left as it is"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), body);
    }
}

#[test]
fn vibe_gets_providers_and_models_in_a_non_private_home() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.file(
        ".config/agent-kit/vibe-litellm/config.toml",
        "active_model = \"m\"\n[[providers]]\nname = \"mistral\"\napi_base = \"https://a\"\n\
         [[models]]\nname = \"x\"\nprovider = \"mistral\"\nalias = \"x\"\n\
         [[models]]\nname = \"old\"\nprovider = \"litellm-old\"\nalias = \"o\"\n",
    );
    let args = [
        "--table",
        &table,
        "configure",
        "vibe",
        "--config",
        config.to_str().unwrap(),
    ];
    let (code, ..) = main_run(&tmp, &args);
    assert_eq!(code, 0);
    let body = std::fs::read_to_string(&config).unwrap();
    let doc: DocumentMut = body.parse().unwrap();
    assert_eq!(doc["active_model"].as_str(), Some("m"));
    let names = |key: &str, field: &str| -> Vec<String> {
        doc[key]
            .as_array_of_tables()
            .unwrap()
            .iter()
            .map(|t| t[field].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        names("providers", "name"),
        ["mistral", "litellm-vibe", "litellm-public"]
    );
    let provider = doc["providers"]
        .as_array_of_tables()
        .unwrap()
        .get(1)
        .unwrap();
    assert_eq!(
        provider["api_base"].as_str(),
        Some("https://litellm.example/v1")
    );
    assert_eq!(
        provider["api_key_env_var"].as_str(),
        Some("LITELLM_VIBE_KEY")
    );
    assert_eq!(provider["api_style"].as_str(), Some("openai"));
    assert_eq!(provider["backend"].as_str(), Some("generic"));
    assert_eq!(
        names("models", "alias"),
        [
            "x",
            "litellm-vibe/glm-5-3",
            "litellm-vibe/codestral",
            "litellm-public/gemini-flash"
        ]
    );
    assert!(main_run(&tmp, &args).1.ends_with("in place\n"));
}

#[test]
fn vibe_creates_the_arrays_it_lacks_and_refuses_inline_arrays() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.path("agent-kit/vibe-litellm/config.toml");
    let args = [
        "--table",
        &table,
        "configure",
        "vibe",
        "--config",
        config.to_str().unwrap(),
    ];
    assert_eq!(main_run(&tmp, &args).0, 0);
    assert!(std::fs::read_to_string(&config)
        .unwrap()
        .contains("[[models]]"));
    let inline = tmp.file(
        "other/vibe-litellm/config.toml",
        "providers = [{ name = \"x\" }]\n",
    );
    let args = [
        "--table",
        &table,
        "configure",
        "vibe",
        "--config",
        inline.to_str().unwrap(),
    ];
    let (code, _, err, _) = main_run(&tmp, &args);
    assert!(code == 2 && err.contains("not an array of tables"), "{err}");
}

#[test]
fn vibe_providers_are_never_written_into_the_default_or_private_home() {
    for home_name in [".vibe", ".vibe-private"] {
        let tmp = Tmp::new();
        let table = table_file(&tmp);
        let config = tmp.path(&format!("{home_name}/config.toml"));
        let args = [
            "--table",
            &table,
            "configure",
            "vibe",
            "--config",
            config.to_str().unwrap(),
        ];
        let (code, _, err, _) = main_run(&tmp, &args);
        assert!(code == 2 && err.contains("never writes"), "{err}");
        assert!(!config.exists());
    }
}

#[test]
fn a_symlink_or_dot_dot_into_the_default_home_is_refused() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    std::fs::create_dir_all(tmp.path(".vibe")).unwrap();
    std::os::unix::fs::symlink(tmp.path(".vibe"), tmp.path("alias")).unwrap();
    for config in [
        tmp.path("alias/config.toml"),
        tmp.path("x/../.vibe/config.toml"),
    ] {
        let args = [
            "--table",
            &table,
            "configure",
            "vibe",
            "--config",
            config.to_str().unwrap(),
        ];
        let (code, _, err, _) = main_run(&tmp, &args);
        assert!(code == 2 && err.contains("never writes"), "{err}");
    }
}

#[test]
fn a_home_named_private_is_refused_too() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.path("my-private-home/config.toml");
    let args = [
        "--table",
        &table,
        "configure",
        "vibe",
        "--config",
        config.to_str().unwrap(),
    ];
    let (code, _, err, _) = main_run(&tmp, &args);
    assert!(code == 2 && err.contains("private"));
}

#[test]
fn check_changes_nothing_and_a_foreign_file_is_refused() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let config = tmp.path("opencode.json");
    let c = config.to_str().unwrap();
    let (code, out, ..) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "opencode",
            "--config",
            c,
            "--check",
        ],
    );
    assert!(
        code == 0 && out.contains("would be written (2 routes)"),
        "{out}"
    );
    assert!(!config.exists());
    std::fs::write(&config, "[1, 2]").unwrap();
    let (code, _, err, _) = main_run(
        &tmp,
        &["--table", &table, "configure", "opencode", "--config", c],
    );
    assert!(code == 2 && err.contains("left as it is"));
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "[1, 2]");
}

#[test]
fn no_config_file_holds_a_key() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    for (name, ext) in [("codex", "toml"), ("hermes", "yaml"), ("opencode", "json")] {
        let config = tmp.path(&format!("{name}.{ext}"));
        main_run(
            &tmp,
            &[
                "--table",
                &table,
                "configure",
                name,
                "--config",
                config.to_str().unwrap(),
            ],
        );
        assert!(!std::fs::read_to_string(&config).unwrap().contains("sk-"));
    }
}

#[test]
fn a_write_that_fails_leaves_no_temporary_file() {
    let tmp = Tmp::new();
    let target = tmp.path("out.json");
    std::fs::create_dir(&target).unwrap();
    assert!(write_atomically(&target, "x").is_err());
    let left: Vec<_> = std::fs::read_dir(&tmp.0).unwrap().collect();
    assert_eq!(left.len(), 1);
}

#[test]
fn unreadable_unbackable_and_unwritable_configs_are_refused() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    // a directory where the file should be
    let dir = tmp.path("config.toml");
    std::fs::create_dir(&dir).unwrap();
    let (code, _, err, _) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "codex",
            "--config",
            dir.to_str().unwrap(),
        ],
    );
    assert!(code == 2 && err.contains("could not be read"), "{err}");
    // a backup that cannot be made
    let config = tmp.file("c2/config.toml", "a = 1\n");
    std::fs::create_dir(tmp.path("c2/config.toml.bak")).unwrap();
    let (code, _, err, _) = main_run(
        &tmp,
        &[
            "--table",
            &table,
            "configure",
            "codex",
            "--config",
            config.to_str().unwrap(),
        ],
    );
    assert!(code == 2 && err.contains("could not back up"), "{err}");
    // a folder that cannot be written (skipped when the user may write anyway)
    let locked = tmp.path("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(locked.join("probe"), "").is_err() {
        let config = locked.join("config.toml");
        let (code, _, err, _) = main_run(
            &tmp,
            &[
                "--table",
                &table,
                "configure",
                "codex",
                "--config",
                config.to_str().unwrap(),
            ],
        );
        assert!(code == 2 && err.contains("could not write"), "{err}");
    }
}

// ----- run ------------------------------------------------------------------------

fn helper(tmp: &Tmp) -> String {
    tmp.script(
        "agent-kit-secret",
        &format!("[ \"$1 $2\" = \"get LITELLM_VIBE_KEY\" ] || exit 9\necho {KEY}"),
    )
}

fn run_args<'a>(table: &'a str, helper: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "--table",
        table,
        "run",
        rest[0],
        rest[1],
        "--secret-helper",
        helper,
    ];
    args.extend(&rest[2..]);
    args
}

#[test]
fn claude_code_gets_the_base_url_and_token_for_that_process_only() {
    let tmp = Tmp::new();
    let (table, helper) = (table_file(&tmp), helper(&tmp));
    let args = run_args(
        &table,
        &helper,
        &["claude", "vibe", "--model", "glm-5-3", "--", "-p", "hi"],
    );
    let (code, _, _, calls) = main_run(&tmp, &args);
    let [(program, argv, env)] = &calls[..] else {
        panic!("{calls:?}")
    };
    assert_eq!(code, 0);
    assert_eq!(program, "claude");
    assert_eq!(argv, &["claude", "--model", "glm-5-3", "-p", "hi"]);
    assert_eq!(
        get(env, "ANTHROPIC_BASE_URL"),
        Some("https://litellm.example")
    );
    assert_eq!(get(env, "ANTHROPIC_AUTH_TOKEN"), Some(KEY));
    assert!(get(env, "ANTHROPIC_API_KEY").is_none());
    assert_eq!(get(env, "PATH"), Some("/usr/bin:/bin"));
}

#[test]
fn each_other_harness_is_started_on_its_route_with_the_key_variable_set() {
    let cases: [(&str, &[&str]); 4] = [
        (
            "codex",
            &[
                "codex",
                "-c",
                "model_provider=\"litellm-vibe\"",
                "-m",
                "glm-5-3",
            ],
        ),
        (
            "hermes",
            &[
                "hermes",
                "chat",
                "--provider",
                "litellm-vibe",
                "-m",
                "glm-5-3",
            ],
        ),
        ("opencode", &["opencode", "-m", "litellm-vibe/glm-5-3"]),
        ("vibe", &["vibe"]),
    ];
    for (name, argv) in cases {
        let tmp = Tmp::new();
        let (table, helper) = (table_file(&tmp), helper(&tmp));
        let (code, _, _, calls) = main_run(
            &tmp,
            &run_args(&table, &helper, &[name, "vibe", "--model", "glm-5-3"]),
        );
        let [(program, got, env)] = &calls[..] else {
            panic!()
        };
        assert_eq!(code, 0);
        assert_eq!(got, argv);
        assert_eq!(program, argv[0]);
        assert_eq!(get(env, "LITELLM_VIBE_KEY"), Some(KEY));
        if name == "vibe" {
            assert_eq!(get(env, "VIBE_ACTIVE_MODEL"), Some("litellm-vibe/glm-5-3"));
            assert!(get(env, "VIBE_HOME")
                .unwrap()
                .ends_with("agent-kit/vibe-litellm"));
        }
    }
}

#[test]
fn without_a_model_no_model_flag_is_passed() {
    for name in HARNESSES {
        let tmp = Tmp::new();
        let (table, helper) = (table_file(&tmp), helper(&tmp));
        let (_, _, _, calls) = main_run(&tmp, &run_args(&table, &helper, &[name, "vibe"]));
        assert!(!calls[0].1.iter().any(|a| a == "-m" || a == "--model"));
        assert!(get(&calls[0].2, "VIBE_ACTIVE_MODEL").is_none());
    }
}

#[test]
fn the_key_is_never_in_argv_or_output() {
    for name in HARNESSES {
        let tmp = Tmp::new();
        let (table, helper) = (table_file(&tmp), helper(&tmp));
        let args = run_args(
            &table,
            &helper,
            &[name, "vibe", "--model", "glm-5-3", "--", "extra"],
        );
        let (_, out, err, calls) = main_run(&tmp, &args);
        assert!(!format!("{out}{err}").contains(KEY));
        assert!(!calls[0].1.iter().any(|a| a.contains(KEY)));
    }
}

#[test]
fn a_missing_key_a_missing_route_and_an_unlisted_model_are_refused_without_launching() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let failing = tmp.script("failing", "exit 1");
    let empty = tmp.script("empty", "echo");
    let gone = tmp.path("gone").display().to_string();
    for (helper, why) in [
        (&failing, "LITELLM_VIBE_KEY"),
        (&empty, "LITELLM_VIBE_KEY"),
        (&gone, "did not run"),
    ] {
        let args = [
            "--table",
            &table,
            "run",
            "codex",
            "vibe",
            "--secret-helper",
            helper,
        ];
        let (code, _, err, calls) = main_run(&tmp, &args);
        assert!(code == 2 && err.contains(why) && calls.is_empty(), "{err}");
    }
    let (code, _, err, calls) = main_run(&tmp, &["--table", &table, "run", "codex", "nope"]);
    assert!(
        code == 2
            && err.contains("no LiteLLM route")
            && err.contains("vibe, public")
            && calls.is_empty()
    );
    let (code, _, err, calls) = main_run(
        &tmp,
        &["--table", &table, "run", "codex", "vibe", "--model", "nope"],
    );
    assert!(code == 2 && err.contains("does not list") && calls.is_empty());
}

#[test]
fn vibe_never_runs_in_the_default_or_private_home() {
    let tmp = Tmp::new();
    let (table, helper) = (table_file(&tmp), helper(&tmp));
    for home_name in [".vibe", ".vibe-private"] {
        let home = tmp.path(home_name).display().to_string();
        let args = run_args(&table, &helper, &["vibe", "vibe", "--vibe-home", &home]);
        let (code, _, err, calls) = main_run(&tmp, &args);
        assert!(
            code == 2 && err.contains("never writes") && calls.is_empty(),
            "{err}"
        );
    }
}

#[test]
fn a_bad_table_is_refused() {
    let tmp = Tmp::new();
    for body in [
        "{}",
        "[]",
        "{\"base_url\": \"\", \"routes\": []}",
        "{\"base_url\": \"u\", \"routes\": 3}",
        "not json",
    ] {
        let bad = tmp.file("t.json", body).display().to_string();
        let (code, _, err, _) = main_run(&tmp, &["--table", &bad, "run", "codex", "vibe"]);
        assert!(code == 2 && err.contains("route table"), "{err}");
    }
    let missing = tmp.path("missing.json").display().to_string();
    assert_eq!(
        main_run(&tmp, &["--table", &missing, "run", "codex", "vibe"]).0,
        2
    );
}

#[test]
fn a_table_with_a_missing_field_is_refused_and_a_number_is_shown() {
    let tmp = Tmp::new();
    let helper = helper(&tmp);
    let mut broken = table();
    broken["routes"][0]
        .as_object_mut()
        .unwrap()
        .remove("key_env");
    let path = tmp
        .file("t.json", &broken.to_string())
        .display()
        .to_string();
    let (code, _, err, _) = main_run(&tmp, &run_args(&path, &helper, &["codex", "vibe"]));
    assert!(code == 2 && err.contains("lacks \"key_env\""), "{err}");
    let mut numbered = table();
    numbered["routes"][1]["agent"] = serde_json::json!(5);
    assert_eq!(provider_name(&numbered["routes"][1]).unwrap(), "litellm-5");
    // an unreadable model entry stops the run before the key is read
    let mut modelless = table();
    modelless["routes"][0]["models"] = serde_json::json!([{}]);
    let path = tmp
        .file("m.json", &modelless.to_string())
        .display()
        .to_string();
    let (code, ..) = main_run(
        &tmp,
        &run_args(&path, &helper, &["codex", "vibe", "--model", "x"]),
    );
    assert_eq!(code, 2);
    let config = tmp.path("c.yaml").display().to_string();
    let (code, ..) = main_run(
        &tmp,
        &["--table", &path, "configure", "hermes", "--config", &config],
    );
    assert_eq!(code, 2);
}

#[test]
fn an_exec_that_fails_is_reported() {
    let tmp = Tmp::new();
    let (table, helper) = (table_file(&tmp), helper(&tmp));
    let args: Vec<String> = [
        "--table",
        &table,
        "run",
        "codex",
        "vibe",
        "--secret-helper",
        &helper,
    ]
    .iter()
    .map(|a| a.to_string())
    .collect();
    let mut failing = |_: &str, _: &[String], _: &Env| Some(std::io::Error::from_raw_os_error(2));
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = main_with(&args, &ctx(&tmp), &mut failing, &mut out, &mut err);
    let err = String::from_utf8(err).unwrap();
    assert!(
        code == 2 && err.contains("cannot start codex: No such file or directory"),
        "{err}"
    );
    // the real exec returns when the program does not exist
    let env = Env::new();
    let failure = real_exec("/nonexistent/program", &["x".to_string()], &env);
    assert!(failure.is_some());
}

// ----- the command line ---------------------------------------------------------

#[test]
fn the_command_line_is_checked() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    let (code, out, ..) = main_run(&tmp, &["--help"]);
    assert!(code == 0 && out.contains("usage: litellm-harness"));
    assert_eq!(main_run(&tmp, &["-h"]).0, 0);
    for args in [
        &["--table"][..],
        &["--bogus"],
        &[],
        &["frobnicate"],
        &["run", "codex"],
        &["run", "nope", "vibe"],
        &["configure", "claude"],
        &["configure", "codex", "extra"],
    ] {
        let (code, _, err, calls) = main_run(&tmp, args);
        assert!(
            code == 2 && err.contains("litellm-harness:") && calls.is_empty(),
            "{args:?}: {err}"
        );
    }
    let (code, _, err, _) = main_run(&tmp, &["--table", &table, "configure", "codex"]);
    assert!(code == 2 && err.contains("needs --config"));
}

#[test]
fn options_may_use_equals_and_follow_the_positionals() {
    let tmp = Tmp::new();
    let helper = helper(&tmp);
    let table = format!("--table={}", table_file(&tmp));
    let secret = format!("--secret-helper={helper}");
    let (code, _, _, calls) = main_run(
        &tmp,
        &[
            "run",
            "codex",
            "vibe",
            &table,
            &secret,
            "--model=glm-5-3",
            "--",
            "--table",
            "x",
        ],
    );
    assert_eq!(code, 0);
    assert_eq!(calls[0].1[calls[0].1.len() - 2..], ["--table", "x"]);
}

#[test]
fn home_and_the_config_home_come_from_the_environment() {
    let tmp = Tmp::new();
    let mut c = ctx(&tmp);
    c.environ.remove(OsStr::new("HOME"));
    c.environ
        .insert("XDG_CONFIG_HOME".into(), tmp.path("xdg").into_os_string());
    tmp.file("xdg/agent-kit/litellm-harnesses.json", &table().to_string());
    let secret = helper(&tmp);
    let (code, _, _, calls) = main_in(&c, &["run", "vibe", "vibe", "--secret-helper", &secret]);
    assert_eq!(code, 0);
    let home = get(&calls[0].2, "VIBE_HOME").unwrap();
    assert!(
        home.starts_with(tmp.path("xdg").to_str().unwrap()),
        "{home}"
    );
}

#[test]
fn the_default_table_is_the_kits_then_the_installed_copy() {
    let tmp = Tmp::new();
    let exe = tmp.path("kit/scripts/target/release/litellm-harness");
    let kit = tmp.file("kit/registry/generated/litellm/harnesses.json", "{}");
    let config_home = tmp.path("cfg");
    assert_eq!(default_table(&exe, &config_home), kit);
    let installed = tmp.file("cfg/agent-kit/litellm-harnesses.json", "{}");
    assert_eq!(default_table(&exe, &config_home), kit);
    let elsewhere = tmp.path("bin/agent-kit-litellm");
    assert_eq!(default_table(&elsewhere, &config_home), installed);
    assert_eq!(
        default_table(&elsewhere, &tmp.path("none")),
        PathBuf::from(TABLE_REL)
    );
    assert_eq!(find_up(&elsewhere, "no/such/thing"), None);
}

#[test]
fn run_reads_the_process_environment() {
    assert_eq!(run(["--help".into()].into_iter()), 0);
}

#[test]
fn the_default_secret_helper_is_in_the_home_bin() {
    let tmp = Tmp::new();
    let table = table_file(&tmp);
    tmp.script(
        ".local/bin/agent-kit-secret",
        &format!("[ \"$1 $2\" = \"get LITELLM_VIBE_KEY\" ] || exit 9\necho {KEY}"),
    );
    let (code, _, _, calls) = main_run(&tmp, &["--table", &table, "run", "codex", "vibe"]);
    assert_eq!(code, 0);
    assert_eq!(get(&calls[0].2, "LITELLM_VIBE_KEY"), Some(KEY));
}
