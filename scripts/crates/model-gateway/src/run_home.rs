//! The per-run home of a client the gateway launches (#903): a directory under the run's own, made for one
//! launch and removed when it ends, that holds only the provider the run needs. The client is pointed at it
//! through `HOME`, the XDG variables and its own variables, so the owner's home never reaches the delegate:
//! no MCP server, skill, `AGENTS.md`, memory or session history the profile did not grant, and no path of the
//! owner's in its context. Pi has its own (`pi_launch`); this is the same design for OpenCode and Hermes.
//! OpenCode's own providers sign in through the owner's data directory, which stays (see `prepare_opencode`).
//!
//! Claude Code, Codex, Antigravity and Vibe keep the home they have, because each one's sign-in lives there
//! (a subscription login, rotating tokens, a keychain item named after the config directory). They are
//! closed against MCP and project context by flags instead: see their launchers.

use crate::json::{self, Value};
use crate::launchers::LaunchPaths;
use crate::ports::Files;
use crate::run::RunMode;
use crate::text::join;
use std::collections::{BTreeMap, HashMap};

/// The directory of a launch's own home, under the run's directory.
pub const RUN_HOME: &str = "client-home";

/// What the variables of a run home point a client at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunHome {
    /// The home's directory.
    pub dir: String,
    /// The variables that make the client use it; the launcher sets them last, so a caller's own never win.
    pub env: BTreeMap<String, String>,
}

/// The variables OpenCode's launch sets itself, which a caller's `extras.env` may not name.
pub const OPENCODE_PROTECTED_ENV: [&str; 11] = [
    "HOME",
    "OPENCODE_CONFIG",
    "OPENCODE_CONFIG_CONTENT",
    "OPENCODE_CONFIG_DIR",
    "OPENCODE_DISABLE_AUTOUPDATE",
    "OPENCODE_DISABLE_CLAUDE_CODE",
    "OPENCODE_DISABLE_PROJECT_CONFIG",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
];

/// The variables Hermes' launch sets itself.
pub const HERMES_PROTECTED_ENV: [&str; 6] = ["HERMES_HOME", "HOME", "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"];

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
    let mut vars = home_vars(dir);
    if litellm.is_none() {
        vars.insert("XDG_DATA_HOME".to_string(), real_dir(env, "XDG_DATA_HOME", &[".local", "share"]));
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
    Ok(RunHome { dir: dir.to_string(), env: vars })
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
    let path = join(&[&owner, "config.yaml"]);
    let text = files.read_text(&path).map_err(|_| format!("hermes: {path} cannot be read; re-run agent-kit setup so it writes the LiteLLM providers there"))?;
    let Some(block) = yaml_block(&text, "providers", provider) else {
        return Err(format!("hermes: {path} has no {provider} provider; re-run agent-kit setup so it writes the LiteLLM providers there"));
    };
    // A provider's key is named by `key_env` and read by the launch's wrapper; a literal one is not copied.
    let block = block.lines().filter(|line| !literal_secret(line)).collect::<Vec<_>>().join("\n");
    let config = format!("providers:\n{block}\nmcp_servers: {{}}\n");
    files.write_private(&join(&[dir, ".hermes", "config.yaml"]), &config).map_err(|error| format!("hermes: cannot write the launch's home {dir}: {}", error.message))?;
    let mut vars = home_vars(dir);
    vars.insert("HERMES_HOME".to_string(), join(&[dir, ".hermes"]));
    Ok(RunHome { dir: dir.to_string(), env: vars })
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
