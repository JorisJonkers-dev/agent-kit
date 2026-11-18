//! The per-run home of a client the gateway launches (#903): a directory under the run's own, made for one
//! launch and removed when it ends, that holds only what the run needs. The client is pointed at it through
//! `HOME`, the XDG variables and its own variables, so the owner's home never reaches the delegate: no MCP
//! server, skill, `AGENTS.md`, memory or session history the profile did not grant, and no path of the owner's
//! in its context. Pi has its own (`pi_launch`); this is the same design for every other client, as far as the
//! client's sign-in allows:
//!
//! - OpenCode and Hermes (on a LiteLLM route) hold the provider and a key the launch's wrapper reads from the
//!   secret store: nothing of the owner's.
//! - Codex's home holds a link to the one file its ChatGPT login rotates its tokens in, which the confinement
//!   lets it write; a copy would sign the owner out.
//! - Vibe's home holds a copy of the owner's `config.toml` and agent files; its key is asked of the keychain.
//! - Claude Code, Antigravity and Vibe's keys are in the macOS keychain, which is found through the real
//!   `HOME`, so those launches keep it and the home is a temporary directory only.
//!
//! Every home has a `tmp` directory, which `TMPDIR` names: a confined client may write nowhere else.

use crate::json::{self, Value};
use crate::launchers::LaunchPaths;
use crate::ports::Files;
use crate::run::RunMode;
use crate::text::join;
use std::collections::{BTreeMap, HashMap};

/// The directory of a launch's own home, under the run's directory.
pub const RUN_HOME: &str = "client-home";

/// The temporary directory inside a home, which `TMPDIR` names.
const TMP: &str = "tmp";

/// What the variables of a run home point a client at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunHome {
    /// The home's directory.
    pub dir: String,
    /// The variables that make the client use it; the launcher sets them last, so a caller's own never win.
    pub env: BTreeMap<String, String>,
    /// What the confinement of the launch grants for this home: paths of the owner's the client reads, paths it
    /// writes (a database, the file a login's tokens rotate in), and the TCP ports its provider is on.
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    pub ports: Vec<u16>,
    /// The client serves itself on a loopback port chosen at run time: loopback is open at any port.
    pub loopback: bool,
    /// A login the run holds a copy of, as `(the owner's file, the copy)`: what the run refreshed in it goes back
    /// to the owner's after the launch (`sync_login`).
    pub login: Option<(String, String)>,
}

/// The variables OpenCode's launch sets itself, which a caller's `extras.env` may not name.
pub const OPENCODE_PROTECTED_ENV: [&str; 13] = [
    "HOME",
    "OPENCODE_DB",
    "OPENCODE_CONFIG",
    "OPENCODE_CONFIG_CONTENT",
    "OPENCODE_CONFIG_DIR",
    "OPENCODE_DISABLE_AUTOUPDATE",
    "OPENCODE_DISABLE_CLAUDE_CODE",
    "OPENCODE_DISABLE_PROJECT_CONFIG",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "TMPDIR",
    "XDG_STATE_HOME",
];

/// The variables Hermes' launch sets itself.
pub const HERMES_PROTECTED_ENV: [&str; 7] = ["HERMES_HOME", "HOME", "TMPDIR", "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"];

/// The variables Codex's, Claude Code's and Antigravity's launches set themselves.
pub const CODEX_PROTECTED_ENV: [&str; 2] = ["CODEX_HOME", "TMPDIR"];
pub const CLAUDE_PROTECTED_ENV: [&str; 3] = ["CLAUDE_CODE_DISABLE_AUTO_MEMORY", "CLAUDE_CODE_TMPDIR", "TMPDIR"];
pub const AGY_PROTECTED_ENV: [&str; 1] = ["TMPDIR"];

/// The directory a launch's home is made in: the run's own directory, else the launch's export directory.
pub fn run_home_dir(paths: &LaunchPaths) -> String {
    join(&[paths.run_dir.as_deref().unwrap_or(&paths.export_dir), RUN_HOME])
}

/// `HOME` and the four XDG directories, all inside the home, so a client that looks for its files by either finds only these.
fn home_vars(dir: &str) -> BTreeMap<String, String> {
    [
        ("HOME", dir.to_string()),
        ("XDG_CACHE_HOME", join(&[dir, ".cache"])),
        ("XDG_CONFIG_HOME", join(&[dir, ".config"])),
        ("XDG_DATA_HOME", join(&[dir, ".local", "share"])),
        ("XDG_STATE_HOME", join(&[dir, ".local", "state"])),
        ("TMPDIR", join(&[dir, TMP])),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect()
}

/// A leading `~` or `$HOME` is never expanded here: the real home is `env`'s `HOME`.
fn real_dir(env: &HashMap<String, String>, xdg: &str, under_home: &[&str]) -> String {
    match env.get(xdg).filter(|dir| !dir.is_empty()) {
        Some(dir) => dir.clone(),
        None => {
            let mut parts = vec![env.get("HOME").map_or("", String::as_str)];
            parts.extend_from_slice(under_home);
            join(&parts)
        }
    }
}

fn rule(action: &str, effect: &str) -> Value {
    json::object(vec![("action", Value::str(action)), ("resource", Value::str("*")), ("effect", Value::str(effect))])
}

/// The tools an OpenCode delegate may use: read, search and the web. A writing run also edits files.
const OPENCODE_READ_ACTIONS: [&str; 5] = ["read", "grep", "glob", "webfetch", "websearch"];

/// What an OpenCode run's tools may do. Everything is denied first, the read tools and the web are allowed, and
/// the tools that act outside the run (a shell, code execution, subagents, a path outside the working
/// directory) are denied last, since the last rule that matches wins. A read-only run also denies `edit`, which
/// covers `write`, `edit` and `patch`. The v1 `permission` and `tools` keys say the same, for an OpenCode that
/// reads them in place of `permissions`. No `mcp` key is written: the run's home declares no server.
pub fn opencode_config(mode: Option<RunMode>, provider: Option<(&str, Value)>) -> Value {
    let write = mode == Some(RunMode::Write);
    let mut rules = vec![rule("*", "deny")];
    rules.extend(OPENCODE_READ_ACTIONS.iter().map(|action| rule(action, "allow")));
    if write {
        rules.push(rule("edit", "allow"));
    }
    let mut denied = vec!["shell", "execute", "subagent", "external_directory"];
    if !write {
        denied.push("edit");
    }
    rules.extend(denied.into_iter().map(|action| rule(action, "deny")));
    let mut permission = vec![("*", Value::str("deny"))];
    permission.extend(OPENCODE_READ_ACTIONS.iter().map(|action| (*action, Value::str("allow"))));
    permission.extend([("edit", Value::str(if write { "allow" } else { "deny" })), ("bash", Value::str("deny")), ("external_directory", Value::str("deny")), ("task", Value::str("deny"))]);
    // The v1 `tools` map agrees with `permission`: the edit tools are on for a writing run and off otherwise.
    let mut tools = vec![("bash", Value::Bool(false)), ("task", Value::Bool(false))];
    tools.extend(["write", "edit", "patch"].into_iter().map(|tool| (tool, Value::Bool(write))));
    let mut members = vec![
        ("$schema", Value::str("https://opencode.ai/config.json")),
        ("autoupdate", Value::Bool(false)),
        ("share", Value::str("disabled")),
        ("permissions", Value::Arr(rules)),
        ("permission", json::object(permission)),
        ("tools", json::object(tools)),
    ];
    if let Some((name, entry)) = provider {
        members.push(("provider", json::object(vec![(name, entry)])));
    }
    json::object(members)
}

/// Whether a member's name says it holds a secret: a key, token, password or authorization header.
fn secret_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    ["key", "token", "secret", "password", "authorization", "bearer"].iter().any(|word| lower.contains(word))
}

/// A copy of a config value without a secret written into it: a member that says it holds one and has a literal
/// string for a value is left out. A reference (`{env:NAME}` or `{file:PATH}`) stays, so the key still comes from
/// the secret store through the launch's wrapper and no key is written into the run home.
pub fn without_inline_secrets(value: &Value) -> Value {
    match value {
        Value::Obj(object) => {
            let mut kept = json::Object::new();
            for (name, member) in object.entries() {
                let literal = matches!(member, Value::Str(text) if !text.contains("{env:") && !text.contains("{file:"));
                if !(secret_name(name) && literal) {
                    kept.set(name, without_inline_secrets(member));
                }
            }
            Value::Obj(kept)
        }
        Value::Arr(items) => Value::Arr(items.iter().map(without_inline_secrets).collect()),
        other => other.clone(),
    }
}

/// The provider of the owner's OpenCode config a litellm route runs on, cut down to the model of the run, or why it is not there.
fn opencode_provider(files: &dyn Files, env: &HashMap<String, String>, provider: &str, model: &str) -> Result<Value, String> {
    let path = join(&[&real_dir(env, "XDG_CONFIG_HOME", &[".config"]), "opencode", "opencode.json"]);
    let parsed = files.read_text(&path).ok().and_then(|text| json::parse(&text).ok());
    let Some(parsed) = parsed else {
        return Err(format!("opencode: {path} cannot be read; re-run agent-kit setup so it writes the LiteLLM providers there"));
    };
    let entry = parsed.as_object().and_then(|config| config.get("provider")).and_then(Value::as_object).and_then(|providers| providers.get(provider));
    let Some(Value::Obj(entry)) = entry else {
        return Err(format!("opencode: {path} has no {provider} provider; re-run agent-kit setup so it writes the LiteLLM providers there"));
    };
    let Value::Obj(mut entry) = without_inline_secrets(&Value::Obj(entry.clone())) else { unreachable!("an object stays an object") };
    if let Some(Value::Obj(models)) = entry.get("models") {
        let id = model.split_once('/').map_or(model, |(_, id)| id);
        if let Some(chosen) = models.get(id) {
            entry.set("models", json::object(vec![(id, chosen.clone())]));
        }
    }
    Ok(Value::Obj(entry))
}

/// A launch's OpenCode home: the run-local config (its tools per the run's mode, the one provider the model
/// names, no MCP server) and the variables that put OpenCode on it. The owner's config directory, skills,
/// `AGENTS.md` and plugins are never read. OpenCode also reads `.claude`'s skills and the project's own
/// `opencode.json`, which can declare servers: both are switched off.
///
/// A LiteLLM provider's key is read from the secret store by the launch's wrapper, so its data directory is the
/// run's own and holds nothing. OpenCode's own providers (`opencode`, `opencode-go`, and the free models) have
/// their sign-in in `opencode.db` of the owner's data directory, where a copy of one credential is not
/// possible: a fresh data directory signs in as nobody and the free models answer 429 or no route. Those launches
/// keep `XDG_DATA_HOME` as it is, and only the config, state, cache and `HOME` are the run's own.
pub fn prepare_opencode(files: &dyn Files, env: &HashMap<String, String>, model: &str, mode: Option<RunMode>, dir: &str) -> Result<RunHome, String> {
    let provider = crate::models::open_code_provider(model);
    let litellm = provider.as_deref().filter(|name| name.starts_with("litellm-"));
    let entry = match litellm {
        Some(name) => Some((name, opencode_provider(files, env, name, model)?)),
        None => None,
    };
    let config = join(&[dir, "opencode.json"]);
    files.write_private(&config, &format!("{}\n", json::pretty(&opencode_config(mode, entry)))).map_err(|error| format!("opencode: cannot write the launch's home {dir}: {}", error.message))?;
    made_tmp(files, dir, "opencode")?;
    let mut vars = home_vars(dir);
    let mut writes = Vec::new();
    if litellm.is_none() {
        // The sign-in is in `opencode.db` of the owner's data directory, which a fresh one cannot hold: the run's
        // own data directory stays its own, and the database is the owner's, so the confinement grants that one
        // directory (OpenCode writes the database's journal beside it). Everything else it writes is the run's.
        let owner = join(&[&real_dir(env, "XDG_DATA_HOME", &[".local", "share"]), "opencode"]);
        if files.exists(&owner).unwrap_or(false) {
            vars.insert("OPENCODE_DB".to_string(), join(&[&owner, "opencode.db"]));
            writes.push(owner);
        }
    }
    for (name, value) in [
        ("OPENCODE_CONFIG", config.as_str()),
        ("OPENCODE_CONFIG_CONTENT", "{}"),
        ("OPENCODE_DISABLE_AUTOUPDATE", "1"),
        ("OPENCODE_DISABLE_CLAUDE_CODE", "1"),
        ("OPENCODE_DISABLE_PROJECT_CONFIG", "1"),
    ] {
        vars.insert(name.to_string(), value.to_string());
    }
    vars.insert("OPENCODE_CONFIG_DIR".to_string(), join(&[dir, ".config", "opencode"]));
    // `opencode run --standalone` serves itself on a loopback port chosen at run time and connects to it: loopback is
    // open at any port, and the rest of the network only at 443.
    Ok(RunHome { dir: dir.to_string(), env: vars, writes, ports: vec![443], loopback: true, ..RunHome::default() })
}

/// The home's `tmp` directory, which a confined client may write and nothing else is.
fn made_tmp(files: &dyn Files, dir: &str, client: &str) -> Result<(), String> {
    files.mkdir_all(&join(&[dir, TMP])).map_err(|error| format!("{client}: cannot make the launch's home {dir}: {}", error.message))
}

/// The lines of the block `child:` (two spaces in) under the top-level key `parent:` of a YAML file: the key's
/// own line and everything indented deeper, up to the next line that is not. Hermes' `config.yaml` is written
/// by setup in this shape, so the block is copied as it is and no YAML is parsed.
fn yaml_block(text: &str, parent: &str, child: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|line| *line != format!("{parent}:"));
    lines.next()?;
    let mut block: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in lines {
        let indent = line.len() - line.trim_start().len();
        if !line.trim().is_empty() && indent == 0 {
            break;
        }
        if indent == 2 && !line.trim().is_empty() {
            if inside {
                break;
            }
            inside = line.trim_end() == format!("  {child}:");
        }
        if inside {
            block.push(line);
        }
    }
    (!block.is_empty()).then(|| block.join("\n"))
}

/// Whether a line of a Hermes provider block writes a secret in plain: `api_key: sk-...`, `token: ...`.
fn literal_secret(line: &str) -> bool {
    let Some((name, value)) = line.trim().split_once(':') else { return false };
    secret_name(name) && !name.ends_with("_env") && !value.trim().is_empty()
}

/// A launch's Hermes home: a `config.yaml` with the one provider the run names, copied from the owner's, and no
/// MCP server, skill, memory or persona. Hermes finds it by `HERMES_HOME`, and `HOME` and the XDG directories point inside it too.
pub fn prepare_hermes(files: &dyn Files, env: &HashMap<String, String>, provider: &str, dir: &str) -> Result<RunHome, String> {
    let owner = env.get("HERMES_HOME").filter(|dir| !dir.is_empty()).cloned().unwrap_or_else(|| join(&[env.get("HOME").map_or("", String::as_str), ".hermes"]));
    let mut vars = home_vars(dir);
    vars.insert("HERMES_HOME".to_string(), join(&[dir, ".hermes"]));
    let failed = |error: crate::ports::FsError| format!("hermes: cannot write the launch's home {dir}: {}", error.message);
    // A provider the owner's home must give (its key is in `.env` there) runs with that one file, linked into the
    // run's home and readable by the confinement; the rest of the owner's home is not there.
    if crate::launchers::hermes_key(provider).is_none() {
        let key_file = join(&[&owner, ".env"]);
        files.write_private(&join(&[dir, ".hermes", "config.yaml"]), "mcp_servers: {}\n").map_err(failed)?;
        // A copy, never a link: a link in the run's home could be moved to point anywhere a later launch grants.
        if let Ok(keys) = files.read_text(&key_file) {
            files.write_private(&join(&[dir, ".hermes", ".env"]), &keys).map_err(failed)?;
        }
        made_tmp(files, dir, "hermes")?;
        return Ok(RunHome { dir: dir.to_string(), env: vars, ports: vec![443], ..RunHome::default() });
    }
    let path = join(&[&owner, "config.yaml"]);
    let text = files.read_text(&path).map_err(|_| format!("hermes: {path} cannot be read; re-run agent-kit setup so it writes the LiteLLM providers there"))?;
    let Some(block) = yaml_block(&text, "providers", provider) else {
        return Err(format!("hermes: {path} has no {provider} provider; re-run agent-kit setup so it writes the LiteLLM providers there"));
    };
    // A provider's key is named by `key_env` and read by the launch's wrapper; a literal one is not copied.
    let block = block.lines().filter(|line| !literal_secret(line)).collect::<Vec<_>>().join("\n");
    // The port of the provider's `base_url`; a block that names none, or none the launcher can read, is held to 443.
    let ports = vec![block.lines().find_map(|line| line.trim().strip_prefix("base_url:")).and_then(|url| crate::pi_launch::url_port(url.trim())).unwrap_or(443)];
    let config = format!("providers:\n{block}\nmcp_servers: {{}}\n");
    files.write_private(&join(&[dir, ".hermes", "config.yaml"]), &config).map_err(failed)?;
    made_tmp(files, dir, "hermes")?;
    Ok(RunHome { dir: dir.to_string(), env: vars, ports, ..RunHome::default() })
}

/// The launch's home for a client that keeps the real `HOME` (its keychain is found through it): a temporary
/// directory, named by `TMPDIR` and by the client's own variable for it, and nothing else.
pub fn prepare_tmp(files: &dyn Files, client: &str, dir: &str, own_variable: Option<&str>) -> Result<RunHome, String> {
    made_tmp(files, dir, client)?;
    let tmp = join(&[dir, TMP]);
    let mut vars: BTreeMap<String, String> = BTreeMap::from([("TMPDIR".to_string(), tmp.clone())]);
    vars.extend(own_variable.map(|name| (name.to_string(), tmp)));
    Ok(RunHome { dir: dir.to_string(), env: vars, ports: vec![443], ..RunHome::default() })
}

/// Codex's home: `auth.json` is a copy of the owner's (a link, or a write grant on the owner's file, would let a
/// path planted in the writable home move the grant), so a token Codex refreshes is in the copy; `sync_login` puts
/// its token fields back after the launch. Its sessions, logs and caches are the run's. The owner's `config.toml`,
/// plugins, skills and `AGENTS.md` are not there. A login that is not there is no run.
pub fn prepare_codex(files: &dyn Files, env: &HashMap<String, String>, dir: &str) -> Result<RunHome, String> {
    let owner = env.get("CODEX_HOME").filter(|dir| !dir.is_empty()).cloned().unwrap_or_else(|| join(&[env.get("HOME").map_or("", String::as_str), ".codex"]));
    let auth = join(&[&owner, "auth.json"]);
    let login = files.read_text(&auth).map_err(|_| format!("codex: {auth} is not there; sign in with codex login"))?;
    let mut home = prepare_tmp(files, "codex", dir, None)?;
    let copy = join(&[dir, "auth.json"]);
    files.write_private(&copy, &login).map_err(|error| format!("codex: cannot make the launch's home {dir}: {}", error.message))?;
    home.env.insert("CODEX_HOME".to_string(), dir.to_string());
    home.login = Some((auth, copy));
    Ok(home)
}

/// Puts what a launch refreshed in its copy of a login back into the owner's file: the `tokens` and `last_refresh`
/// members, when they changed, and nothing else of the copy (the run cannot write any other field of the owner's
/// file). A copy that is gone or does not parse changes nothing.
pub fn sync_login(files: &dyn Files, owner: &str, copy: &str) -> Result<(), String> {
    let parse = |path: &str| files.read_text(path).ok().and_then(|text| json::parse(&text).ok());
    let (Some(Value::Obj(held)), Some(Value::Obj(mut kept))) = (parse(copy), parse(owner)) else {
        return Ok(());
    };
    let mut changed = false;
    for member in ["tokens", "last_refresh"] {
        if let Some(fresh) = held.get(member).filter(|fresh| kept.get(member) != Some(*fresh)) {
            kept.set(member, fresh.clone());
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    files.write_private(owner, &format!("{}\n", json::pretty(&Value::Obj(kept)))).map_err(|error| format!("codex: the refreshed login could not be written back to {owner}: {}", error.message))
}

/// Vibe's home: a copy of the owner's `config.toml` (the model aliases) and its agent files (the read-only and
/// writing delegates, the critics), and nothing else of the owner's: no session history, trusted folders or key
/// file. The key is the keychain's, which the confinement lets Vibe ask. The owner's home is `~/.vibe`, whatever
/// `VIBE_HOME` the caller is in: a Study or private session never reaches the run.
pub fn prepare_vibe(files: &dyn Files, env: &HashMap<String, String>, dir: &str) -> Result<RunHome, String> {
    let owner = join(&[env.get("HOME").map_or("", String::as_str), ".vibe"]);
    let copy = |from: &str, to: &str| files.copy_tree(from, to).map_err(|error| format!("vibe: cannot copy {from} into the launch's home {dir}: {}", error.message));
    let mut home = prepare_tmp(files, "vibe", dir, None)?;
    for (name, target) in [("config.toml", "config.toml"), ("agents", "agents")] {
        let from = join(&[&owner, name]);
        if files.exists(&from).unwrap_or(false) {
            copy(&from, &join(&[dir, "vibe", target]))?;
        }
    }
    home.env.insert("VIBE_HOME".to_string(), join(&[dir, "vibe"]));
    Ok(home)
}

/// The names of the children (two spaces in) of a top-level key of a YAML file, for a file written in the shape setup writes.
pub fn yaml_children(text: &str, parent: &str) -> Vec<String> {
    let header = format!("{parent}:");
    text.lines()
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| line.trim().is_empty() || line.starts_with(' '))
        .filter(|line| line.starts_with("  ") && !line.starts_with("   "))
        .filter_map(|line| line.trim().strip_suffix(':').map(str::to_string))
        .collect()
}
