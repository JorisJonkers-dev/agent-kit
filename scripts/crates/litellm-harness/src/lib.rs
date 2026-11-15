//! Run a harness on a LiteLLM route, and write each harness's provider entries (agent-kit#737).
//!
//! Two subcommands, both driven by the table `scripts/render_registry.py` renders
//! (`registry/generated/litellm/harnesses.json`): the proxy URL and, per route, the
//! provider name (`litellm-<agent>`), the route's key variable and its models.
//!
//! ```text
//! litellm-harness [--table T] configure <codex|hermes|opencode|vibe> --config PATH [--check]
//! litellm-harness [--table T] run <claude|codex|hermes|opencode|vibe> <route> [--model ID]
//!                 [--secret-helper PATH] [--vibe-home DIR] [-- ARGS...]
//! ```
//!
//! `configure` writes one provider per route into the harness's own config file: Codex
//! `[model_providers.litellm-<agent>]` in config.toml, Hermes `providers:` in config.yaml,
//! OpenCode `provider` in opencode.json, Vibe `[[providers]]` and `[[models]]` in the
//! config.toml of a Vibe home that is neither `~/.vibe` nor `~/.vibe-private`. Setup owns
//! every entry named `litellm-*`: it replaces those the table names, removes the rest, and
//! keeps everything else the owner wrote. No file holds a key, only the *name* of the
//! variable that does.
//!
//! `run` reads the route's key from the secret store (`agent-kit-secret get <KEY_ENV>`,
//! the value on the helper's stdout) and starts the harness with it in the child's
//! environment, never in argv and never printed. Claude Code has no config entry: the
//! launcher sets `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` for that one process, so
//! the default profile never holds either. Exit code 2 for a missing key, table or route,
//! or a config file this will not touch.

use serde_json::Value;
use serde_yaml::{Mapping, Value as Yaml};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

pub const NAME: &str = "litellm-harness";
const PREFIX: &str = "litellm-";
const CONFIGURABLE: [&str; 4] = ["codex", "hermes", "opencode", "vibe"];
const HARNESSES: [&str; 5] = ["claude", "codex", "hermes", "opencode", "vibe"];
const SCHEMA: &str = "https://opencode.ai/config.json";
/// Homes the Vibe provider entries must never be written into.
const FORBIDDEN_VIBE_HOMES: [&str; 2] = [".vibe", ".vibe-private"];
const TABLE_REL: &str = "registry/generated/litellm/harnesses.json";

const USAGE: &str = "usage: litellm-harness [--table T] configure <codex|hermes|opencode|vibe> --config PATH [--check]\n       litellm-harness [--table T] run <claude|codex|hermes|opencode|vibe> <route> [--model ID] [--secret-helper PATH] [--vibe-home DIR] [-- ARGS...]";

/// A refusal the caller reports as one line and exit status 2.
#[derive(Debug, PartialEq, Eq)]
pub struct HarnessError(pub String);

type Res<T> = Result<T, HarnessError>;

fn err<T>(message: impl Into<String>) -> Res<T> {
    Err(HarnessError(message.into()))
}

/// The environment a harness starts with.
pub type Env = BTreeMap<OsString, OsString>;

/// What the process runs in: its environment and the executable's own path.
pub struct Ctx {
    pub environ: Env,
    pub exe: PathBuf,
}

/// Starts the harness: program, argv (program first) and environment. It returns
/// only when it could not (the real one replaces the process).
pub type ExecFn<'a> = &'a mut dyn FnMut(&str, &[String], &Env) -> Option<std::io::Error>;

// ----- the table --------------------------------------------------------------

/// A string field of a table node: a missing one is a refusal, a non-string one is shown as JSON.
fn field(node: &Value, name: &str) -> Res<String> {
    match node.get(name) {
        None | Some(Value::Null) => err(format!("the route table lacks {name:?}")),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(other) => Ok(other.to_string()),
    }
}

fn routes(table: &Value) -> &[Value] {
    table
        .get("routes")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn provider_name(route: &Value) -> Res<String> {
    Ok(format!("{PREFIX}{}", field(route, "agent")?))
}

fn route_named<'a>(table: &'a Value, agent: &str) -> Res<&'a Value> {
    let list = routes(table);
    if let Some(found) = list
        .iter()
        .find(|r| r.get("agent").and_then(Value::as_str) == Some(agent))
    {
        return Ok(found);
    }
    let known: Vec<String> = list.iter().filter_map(|r| field(r, "agent").ok()).collect();
    err(format!(
        "no LiteLLM route {agent:?}; the registry has: {}",
        known.join(", ")
    ))
}

fn model_ids(route: &Value) -> Res<Vec<String>> {
    let models = route
        .get("models")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    models.iter().map(|model| field(model, "id")).collect()
}

// ----- the entry each harness gets, by provider name ---------------------------

fn text(value: &str) -> Yaml {
    Yaml::String(value.to_string())
}

fn map(pairs: Vec<(&str, Yaml)>) -> Yaml {
    let mut mapping = Mapping::new();
    for (key, entry) in pairs {
        mapping.insert(text(key), entry);
    }
    Yaml::Mapping(mapping)
}

fn hermes_entries(table: &Value) -> Res<Vec<(String, Yaml)>> {
    let mut out = Vec::new();
    for route in routes(table) {
        let name = provider_name(route)?;
        let mut pairs = vec![
            ("name", text(&name)),
            ("base_url", text(&field(table, "base_url")?)),
            ("key_env", text(&field(route, "key_env")?)),
            ("api_mode", text("chat_completions")),
        ];
        let ids = model_ids(route)?;
        if let Some(first) = ids.first() {
            pairs.push(("default_model", text(first)));
            let models = ids.iter().map(|id| (text(id), map(vec![]))).collect();
            pairs.push(("models", Yaml::Mapping(models)));
        }
        out.push((name, map(pairs)));
    }
    Ok(out)
}

fn opencode_entries(table: &Value) -> Res<Vec<(String, Yaml)>> {
    let mut out = Vec::new();
    for route in routes(table) {
        let key_env = field(route, "key_env")?;
        let options = map(vec![
            ("baseURL", text(&field(table, "base_url")?)),
            ("apiKey", text(&format!("{{env:{key_env}}}"))),
        ]);
        let models = model_ids(route)?
            .iter()
            .map(|id| (text(id), map(vec![("name", text(id))])))
            .collect();
        let entry = map(vec![
            ("npm", text("@ai-sdk/openai-compatible")),
            ("name", text(&format!("LiteLLM {}", field(route, "agent")?))),
            ("options", options),
            ("models", Yaml::Mapping(models)),
        ]);
        out.push((provider_name(route)?, entry));
    }
    Ok(out)
}

fn codex_entries(table: &Value) -> Res<Vec<(String, Table)>> {
    let mut out = Vec::new();
    for route in routes(table) {
        let mut entry = Table::new();
        entry["name"] = value(format!("LiteLLM {}", field(route, "agent")?));
        entry["base_url"] = value(field(table, "base_url")?);
        entry["env_key"] = value(field(route, "key_env")?);
        entry["wire_api"] = value("responses");
        out.push((provider_name(route)?, entry));
    }
    Ok(out)
}

type Tables = Vec<Table>;

fn vibe_entries(table: &Value) -> Res<(Tables, Tables)> {
    let (mut providers, mut models) = (Vec::new(), Vec::new());
    for route in routes(table) {
        let name = provider_name(route)?;
        let mut provider = Table::new();
        provider["name"] = value(&name);
        provider["api_base"] = value(field(table, "base_url")?);
        provider["api_key_env_var"] = value(field(route, "key_env")?);
        provider["api_style"] = value("openai");
        provider["backend"] = value("generic");
        providers.push(provider);
        for id in model_ids(route)? {
            let mut model = Table::new();
            model["name"] = value(&id);
            model["provider"] = value(&name);
            model["alias"] = value(format!("{name}/{id}"));
            models.push(model);
        }
    }
    Ok((providers, models))
}

// ----- merging -----------------------------------------------------------------

/// `existing` with every `litellm-*` key replaced by `desired`; other keys kept in place.
fn merge_named(existing: &Mapping, desired: Vec<(String, Yaml)>) -> Mapping {
    let mut merged = Mapping::new();
    for (key, entry) in existing {
        if !key.as_str().is_some_and(|k| k.starts_with(PREFIX)) {
            merged.insert(key.clone(), entry.clone());
        }
    }
    for (name, entry) in desired {
        merged.insert(Yaml::String(name), entry);
    }
    merged
}

/// Atomically replaces `path` with `body`, keeping its mode (0600 for a new file).
pub fn write_atomically(path: &Path, body: &str) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mode = std::fs::metadata(path).map_or(0o600, |m| m.permissions().mode() & 0o777);
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let tmp = parent.join(format!(".{file_name}.{}.{stamp}", std::process::id()));
    let result = (|| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?
            .write_all(body.as_bytes())?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn parse_toml(old: &str) -> Res<DocumentMut> {
    old.parse().map_err(|e| HarnessError(format!("{e}")))
}

fn merge_codex(old: &str, table: &Value) -> Res<String> {
    let mut doc = parse_toml(old)?;
    if doc.get("model_providers").is_none() {
        let mut providers = Table::new();
        providers.set_implicit(true);
        doc["model_providers"] = Item::Table(providers);
    }
    let Some(providers) = doc["model_providers"].as_table_like_mut() else {
        return err("model_providers is not a table");
    };
    let stale: Vec<String> = providers
        .iter()
        .filter(|(key, _)| key.starts_with(PREFIX))
        .map(|(key, _)| key.to_string())
        .collect();
    for key in stale {
        providers.remove(&key);
    }
    for (name, entry) in codex_entries(table)? {
        providers.insert(&name, Item::Table(entry));
    }
    Ok(doc.to_string())
}

/// The array of tables under `key` without the tables whose `field` starts with
/// `litellm-`, then `added` appended.
fn replace_tables(doc: &mut DocumentMut, key: &str, field: &str, added: Tables) -> Res<()> {
    if doc.get(key).is_none() {
        doc[key] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    let Some(array) = doc[key].as_array_of_tables_mut() else {
        return err(format!("{key} is not an array of tables"));
    };
    let stale: Vec<usize> = array
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.get(field)
                .and_then(Item::as_str)
                .is_some_and(|name| name.starts_with(PREFIX))
        })
        .map(|(index, _)| index)
        .collect();
    for index in stale.into_iter().rev() {
        array.remove(index);
    }
    for entry in added {
        array.push(entry);
    }
    Ok(())
}

fn merge_vibe(old: &str, table: &Value) -> Res<String> {
    let mut doc = parse_toml(old)?;
    let (providers, models) = vibe_entries(table)?;
    replace_tables(&mut doc, "providers", "name", providers)?;
    replace_tables(&mut doc, "models", "provider", models)?;
    Ok(doc.to_string())
}

fn yaml_error(error: impl std::fmt::Display) -> HarnessError {
    HarnessError(error.to_string())
}

fn merge_hermes(old: &str, table: &Value) -> Res<String> {
    let mut data = if old.trim().is_empty() {
        Yaml::Mapping(Mapping::new())
    } else {
        serde_yaml::from_str(old).map_err(yaml_error)?
    };
    let Some(root) = data.as_mapping_mut() else {
        return err("the Hermes config is not a mapping");
    };
    let existing = match root.get("providers") {
        None | Some(Yaml::Null) => Mapping::new(),
        Some(Yaml::Mapping(providers)) => providers.clone(),
        Some(_) => return err("its providers field is not a mapping"),
    };
    let merged = merge_named(&existing, hermes_entries(table)?);
    root.insert(text("providers"), Yaml::Mapping(merged));
    serde_yaml::to_string(&data).map_err(yaml_error)
}

fn merge_opencode(old: &str, table: &Value) -> Res<String> {
    let shape = "opencode.json is not an object with an object provider field";
    let mut data = if old.trim().is_empty() {
        Yaml::Mapping(Mapping::new())
    } else {
        // Checked as JSON first; read as YAML (a superset) only to keep the key order.
        serde_json::from_str::<Value>(old).map_err(yaml_error)?;
        serde_yaml::from_str(old).map_err(yaml_error)?
    };
    let Some(root) = data.as_mapping_mut() else {
        return err(shape);
    };
    let existing = match root.get("provider") {
        None => Mapping::new(),
        Some(Yaml::Mapping(provider)) => provider.clone(),
        Some(_) => return err(shape),
    };
    if !root.contains_key("$schema") {
        root.insert(text("$schema"), text(SCHEMA));
    }
    let merged = merge_named(&existing, opencode_entries(table)?);
    root.insert(text("provider"), Yaml::Mapping(merged));
    let mut body = serde_json::to_string_pretty(&data).map_err(yaml_error)?;
    body.push('\n');
    Ok(body)
}

/// `path` made absolute and normalised, with the part that exists resolved through symlinks.
fn resolve(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or(path.to_path_buf());
    let mut clean = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                clean.pop();
            }
            other => clean.push(other),
        }
    }
    let mut tail: Vec<&OsStr> = Vec::new();
    let mut head = clean.as_path();
    while let (Err(_), Some(name), Some(parent)) =
        (head.canonicalize(), head.file_name(), head.parent())
    {
        tail.push(name);
        head = parent;
    }
    let base = head.canonicalize().unwrap_or(head.to_path_buf());
    tail.iter().rev().fold(base, |acc, part| acc.join(part))
}

fn check_vibe_home(config: &Path, home: &Path) -> Res<()> {
    let resolved = resolve(config)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    for name in FORBIDDEN_VIBE_HOMES {
        if resolved == resolve(&home.join(name)) {
            return err(format!(
                "{} is a Vibe home this never writes to; use a separate one",
                resolved.display()
            ));
        }
    }
    if resolved
        .file_name()
        .is_some_and(|n| n.to_string_lossy().contains("private"))
    {
        return err(format!(
            "{} looks like a private Vibe home; use a separate one",
            resolved.display()
        ));
    }
    Ok(())
}

/// Merges the table into one harness's config file; the result says what happened.
pub fn configure(
    harness: &str,
    table: &Value,
    config: &Path,
    check: bool,
    home: &Path,
) -> Res<String> {
    if harness == "vibe" {
        check_vibe_home(config, home)?;
    }
    let old = if config.exists() {
        std::fs::read_to_string(config)
            .map_err(|e| HarnessError(format!("{} could not be read: {e}", config.display())))?
    } else {
        String::new()
    };
    let merged = match harness {
        "codex" => merge_codex(&old, table),
        "hermes" => merge_hermes(&old, table),
        "opencode" => merge_opencode(&old, table),
        _ => merge_vibe(&old, table),
    };
    let new = merged
        .map_err(|e| HarnessError(format!("{} was left as it is: {}", config.display(), e.0)))?;
    if new == old {
        return Ok("in place".into());
    }
    if !check {
        if config.exists() {
            let mut backup = config.as_os_str().to_os_string();
            backup.push(".bak");
            std::fs::copy(config, &backup).map_err(|e| {
                HarnessError(format!("could not back up {}: {e}", config.display()))
            })?;
        }
        write_atomically(config, &new)
            .map_err(|e| HarnessError(format!("could not write {}: {e}", config.display())))?;
    }
    let verb = if check { "would be written" } else { "written" };
    Ok(format!("{verb} ({} routes)", routes(table).len()))
}

// ----- running -----------------------------------------------------------------

fn set(env: &mut Env, name: &str, to: &str) {
    env.insert(name.into(), to.into());
}

/// `flag value` when there is a value, else nothing.
fn flag(name: &str, given: Option<String>) -> Vec<String> {
    given.map_or(vec![], |m| vec![name.to_string(), m])
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|w| w.to_string()).collect()
}

/// What `plan` is given about the launch.
pub struct Launch<'a> {
    pub key: &'a str,
    pub args: &'a [String],
    pub model: Option<&'a str>,
    pub environ: &'a Env,
    pub vibe_home: &'a Path,
}

/// The argv and environment to start the harness with. The key is only ever in the environment.
pub fn plan(
    harness: &str,
    route: &Value,
    table: &Value,
    launch: &Launch,
) -> Res<(Vec<String>, Env)> {
    let mut env = launch.environ.clone();
    let name = provider_name(route)?;
    let key_env = field(route, "key_env")?;
    let model = launch.model.map(String::from);
    let mut argv = match harness {
        "claude" => {
            env.remove(OsStr::new("ANTHROPIC_API_KEY"));
            env.remove(OsStr::new("CLAUDE_CODE_OAUTH_TOKEN"));
            let base = field(table, "base_url")?;
            let root = base.strip_suffix("/v1").unwrap_or(&base);
            set(&mut env, "ANTHROPIC_BASE_URL", root);
            set(&mut env, "ANTHROPIC_AUTH_TOKEN", launch.key);
            [words(&["claude"]), flag("--model", model)].concat()
        }
        "codex" => {
            set(&mut env, &key_env, launch.key);
            let provider = format!("model_provider=\"{name}\"");
            [words(&["codex", "-c", &provider]), flag("-m", model)].concat()
        }
        "hermes" => {
            set(&mut env, &key_env, launch.key);
            [
                words(&["hermes", "chat", "--provider", &name]),
                flag("-m", model),
            ]
            .concat()
        }
        "opencode" => {
            set(&mut env, &key_env, launch.key);
            let qualified = model.map(|m| format!("{name}/{m}"));
            [words(&["opencode"]), flag("-m", qualified)].concat()
        }
        _ => {
            set(&mut env, &key_env, launch.key);
            set(&mut env, "VIBE_HOME", &launch.vibe_home.to_string_lossy());
            if let Some(model) = model {
                set(&mut env, "VIBE_ACTIVE_MODEL", &format!("{name}/{model}"));
            }
            words(&["vibe"])
        }
    };
    argv.extend(launch.args.iter().cloned());
    Ok((argv, env))
}

/// The cause of an I/O error without the "(os error N)" suffix.
fn reason(error: &std::io::Error) -> String {
    let full = error.to_string();
    full.split(" (os error")
        .next()
        .unwrap_or_default()
        .to_string()
}

fn read_key(helper: &str, key_env: &str) -> Res<String> {
    let done = Command::new(helper)
        .args(["get", key_env])
        .output()
        .map_err(|e| {
            HarnessError(format!(
                "the secret store helper {helper} did not run: {}",
                reason(&e)
            ))
        })?;
    let key = String::from_utf8_lossy(&done.stdout).trim().to_string();
    if !done.status.success() || key.is_empty() {
        return err(format!(
            "no key {key_env} in the secret store; run agent-kit setup to mint it"
        ));
    }
    Ok(key)
}

fn load_table(path: &Path) -> Res<Value> {
    let read = std::fs::read_to_string(path)
        .map_err(|e| e.to_string())
        .and_then(|body| serde_json::from_str::<Value>(&body).map_err(|e| e.to_string()));
    let table = read.map_err(|e| {
        HarnessError(format!(
            "cannot read the route table {}: {e}",
            path.display()
        ))
    })?;
    let usable = table
        .get("base_url")
        .and_then(Value::as_str)
        .is_some_and(|u| !u.is_empty())
        && table.get("routes").is_some_and(Value::is_array);
    if !usable {
        return err(format!("{} is not a LiteLLM route table", path.display()));
    }
    Ok(table)
}

/// The first `rel` found in `start` or one of its ancestors.
pub fn find_up(start: &Path, rel: &str) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(rel))
        .find(|candidate| candidate.exists())
}

/// The table beside a kit checkout, else the copy setup puts in ~/.config/agent-kit.
pub fn default_table(exe: &Path, config_home: &Path) -> PathBuf {
    let installed = config_home.join("agent-kit").join("litellm-harnesses.json");
    match find_up(exe, TABLE_REL) {
        Some(kit) => kit,
        None if installed.exists() => installed,
        None => PathBuf::from(TABLE_REL),
    }
}

// ----- the command line ----------------------------------------------------------

#[derive(Default)]
struct Args {
    table: Option<PathBuf>,
    config: Option<PathBuf>,
    check: bool,
    model: Option<String>,
    secret_helper: Option<String>,
    vibe_home: Option<PathBuf>,
    positional: Vec<String>,
    passthrough: Vec<String>,
}

/// None for `--help`; a usage problem as the message.
fn parse(args: &[String]) -> Result<Option<Args>, String> {
    let split = args.iter().position(|a| a == "--").unwrap_or(args.len());
    let mut parsed = Args {
        passthrough: args.get(split + 1..).unwrap_or_default().to_vec(),
        ..Args::default()
    };
    let mut own = args[..split].iter();
    while let Some(arg) = own.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((name, given)) if arg.starts_with("--") => (name, Some(given.to_string())),
            _ => (arg.as_str(), None),
        };
        match name {
            "--help" | "-h" => return Ok(None),
            "--check" => parsed.check = true,
            "--table" | "--config" | "--model" | "--secret-helper" | "--vibe-home" => {
                let given = inline
                    .or_else(|| own.next().cloned())
                    .ok_or(format!("{name} needs a value"))?;
                match name {
                    "--table" => parsed.table = Some(given.into()),
                    "--config" => parsed.config = Some(given.into()),
                    "--model" => parsed.model = Some(given),
                    "--secret-helper" => parsed.secret_helper = Some(given),
                    _ => parsed.vibe_home = Some(given.into()),
                }
            }
            _ if arg.starts_with("--") => return Err(format!("unknown argument {arg}")),
            _ => parsed.positional.push(arg.clone()),
        }
    }
    Ok(Some(parsed))
}

fn env_path(ctx: &Ctx, name: &str) -> Option<PathBuf> {
    ctx.environ
        .get(OsStr::new(name))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn refuse(error: HarnessError) -> String {
    error.0
}

/// The positionals checked against the command: (command, harness, route).
fn shape(args: &Args) -> Result<(&str, &str, &str), String> {
    let list: Vec<&str> = args.positional.iter().map(String::as_str).collect();
    match list.as_slice() {
        ["configure", harness] if CONFIGURABLE.contains(harness) => Ok(("configure", harness, "")),
        ["run", harness, route] if HARNESSES.contains(harness) => Ok(("run", harness, route)),
        ["configure" | "run", ..] => Err("bad harness or wrong number of arguments".into()),
        _ => Err("expected the command configure or run".into()),
    }
}

fn execute(args: &Args, ctx: &Ctx, exec: ExecFn, out: &mut dyn Write) -> Result<i32, String> {
    let (command, harness, route_name) = shape(args)?;
    let home = env_path(ctx, "HOME").unwrap_or_else(|| PathBuf::from("/"));
    let config_home = env_path(ctx, "XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
    let table_path = args
        .table
        .clone()
        .unwrap_or_else(|| default_table(&ctx.exe, &config_home));
    let table = load_table(&table_path).map_err(refuse)?;
    if command == "configure" {
        let config = args.config.as_deref().ok_or("configure needs --config")?;
        let said = configure(harness, &table, config, args.check, &home).map_err(refuse)?;
        let _ = writeln!(out, "{harness}: LiteLLM providers {said}");
        return Ok(0);
    }
    let route = route_named(&table, route_name).map_err(refuse)?;
    if let Some(model) = &args.model {
        if !model_ids(route).map_err(refuse)?.contains(model) {
            return Err(format!(
                "route {route_name} does not list the model {model:?}"
            ));
        }
    }
    let vibe_home = args
        .vibe_home
        .clone()
        .unwrap_or_else(|| config_home.join("agent-kit").join("vibe-litellm"));
    if harness == "vibe" {
        check_vibe_home(&vibe_home.join("config.toml"), &home).map_err(refuse)?;
    }
    let helper = args.secret_helper.clone().unwrap_or_else(|| {
        home.join(".local/bin/agent-kit-secret")
            .to_string_lossy()
            .into_owned()
    });
    let key_env = field(route, "key_env").map_err(refuse)?;
    let key = read_key(&helper, &key_env).map_err(refuse)?;
    let launch = Launch {
        key: &key,
        args: &args.passthrough,
        model: args.model.as_deref(),
        environ: &ctx.environ,
        vibe_home: &vibe_home,
    };
    let (argv, env) = plan(harness, route, &table, &launch).map_err(refuse)?;
    match exec(&argv[0], &argv, &env) {
        None => Ok(0),
        Some(failure) => Err(format!("cannot start {}: {}", argv[0], reason(&failure))),
    }
}

/// The command with its surroundings supplied; returns the exit status.
pub fn main_with(
    args: &[String],
    ctx: &Ctx,
    exec: ExecFn,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let parsed = match parse(args) {
        Ok(Some(parsed)) => parsed,
        Ok(None) => {
            let _ = writeln!(out, "{USAGE}");
            return 0;
        }
        Err(message) => {
            let _ = writeln!(err, "{NAME}: {message}\n{USAGE}");
            return 2;
        }
    };
    match execute(&parsed, ctx, exec, out) {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(err, "{NAME}: {message}");
            2
        }
    }
}

/// Replaces this process with the harness; returns only the failure.
pub fn real_exec(program: &str, argv: &[String], env: &Env) -> Option<std::io::Error> {
    Some(
        Command::new(program)
            .args(&argv[1..])
            .env_clear()
            .envs(env)
            .exec(),
    )
}

/// The command as the binary runs it.
pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let ctx = Ctx {
        environ: std::env::vars_os().collect(),
        exe: std::env::current_exe().unwrap_or_default(),
    };
    main_with(
        &args,
        &ctx,
        &mut real_exec,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

#[cfg(test)]
mod tests;
