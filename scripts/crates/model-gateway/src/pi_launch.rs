//! What a confined Pi launch needs before it starts, the port of
//! `application/pi-launch.ts`: the route's provider from the home's
//! `models.json`, the key from the secret store, the launcher, and the launch's
//! own agent directory. Pi has no sandbox of its own, so a launch that cannot be
//! confined never starts.

use crate::config::{ToolList, ToolScope};
use crate::json::{self, Value};
use crate::launchers::PiConfinement;
use crate::ports::{Files, Process};
use crate::text::{join, js_trim};
use std::collections::HashMap;

/// The longest the secret store may take to give the key; it reads a local file and calls no model.
const KEY_TIMEOUT_MS: u64 = 20_000;

/// Why a Pi run cannot start without the launcher.
pub const NO_LAUNCHER: &str =
    "pi: agent-launcher is not on PATH, and a Pi delegate only runs confined to its working directory; re-run agent-kit setup, which installs it";

const FIX: &str = "re-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json";

/// The home's Pi directory: `PI_CODING_AGENT_DIR`, else `~/.pi/agent`.
pub fn home_agent_dir(env: &HashMap<String, String>) -> String {
    match env.get("PI_CODING_AGENT_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => dir.clone(),
        None => join(&[env.get("HOME").map_or("", String::as_str), ".pi", "agent"]),
    }
}

/// The port of an `http` or `https` URL: the one after the host, else the scheme's own.
pub fn url_port(url: &str) -> Option<u16> {
    let (rest, default) = match (url.strip_prefix("https://"), url.strip_prefix("http://")) {
        (Some(rest), _) => (rest, 443),
        (None, Some(rest)) => (rest, 80),
        (None, None) => return None,
    };
    let authority = rest.split('/').next().unwrap_or("");
    let Some((_, port)) = authority.rsplit_once(':') else {
        return Some(default);
    };
    let digits = !port.is_empty() && port.len() <= 5 && port.bytes().all(|byte| byte.is_ascii_digit());
    port.parse::<u16>().ok().filter(|port| digits && *port > 0)
}

/// What a confined launch needs of the route's provider in the home's `models.json`.
struct PiRoute {
    base_url: String,
    api: String,
    api_key: String,
    model: Value,
    port: u16,
}

/// The route's provider as the home's `models.json` holds it, or why a confined launch cannot use it.
fn route_of(files: &dyn Files, env: &HashMap<String, String>, provider: &str, model: &str) -> Result<PiRoute, String> {
    let path = join(&[&home_agent_dir(env), "models.json"]);
    let parsed = files.read_text(&path).ok().and_then(|text| json::parse(&text).ok());
    let Some(parsed) = parsed else {
        return Err(format!("pi: {path} cannot be read; {FIX}"));
    };
    let entry = parsed.as_object().and_then(|object| object.get("providers")).and_then(Value::as_object).and_then(|providers| providers.get(provider)).and_then(Value::as_object);
    let Some(entry) = entry else {
        return Err(format!("pi: {path} has no {provider} provider; {FIX}"));
    };
    let text = |key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    let (Some(base_url), Some(api), Some(api_key)) = (text("baseUrl"), text("api"), text("apiKey")) else {
        return Err(format!("pi: the {provider} provider in {path} lacks a baseUrl, api or apiKey; {FIX}"));
    };
    let Some(port) = url_port(&base_url) else {
        return Err(format!("pi: the {provider} provider's baseUrl is not an http or https URL with a port the confinement can allow"));
    };
    let models = match entry.get("models") {
        Some(Value::Arr(models)) => models.clone(),
        _ => Vec::new(),
    };
    let routed = models.into_iter().find(|candidate| candidate.as_object().and_then(|object| object.get("id")).and_then(Value::as_str) == Some(model));
    Ok(PiRoute { base_url, api, api_key, model: routed.unwrap_or_else(|| json::object(vec![("id", Value::str(model))])), port })
}

/// The key: a literal as it is, or `!<program> get <NAME>` run without a shell; never a shell command.
pub(crate) fn key_of(process: &dyn Process, provider: &str, api_key: &str) -> Result<String, String> {
    let Some(command) = api_key.strip_prefix('!') else {
        return Ok(api_key.to_string());
    };
    let words: Vec<&str> = command.split_whitespace().collect();
    let [program, "get", name] = words.as_slice() else {
        return Err(format!("pi: the {provider} key is a shell command the gateway does not run; setup writes it as !agent-kit-secret get <NAME>"));
    };
    let read = process.exec(program, &["get", name], Some(KEY_TIMEOUT_MS));
    let value = match &read {
        Ok(result) if !result.timed_out && result.exit_code == 0 => js_trim(&result.stdout).to_string(),
        _ => String::new(),
    };
    if !value.is_empty() {
        return Ok(value);
    }
    // Only the error stream is shown: a failed read never puts anything of the key in a message.
    let said = match &read {
        Err(reason) => reason.clone(),
        Ok(result) => first_line(&result.stderr).unwrap_or_else(|| format!("exit {}", result.exit_code)),
    };
    Err(format!("pi: the {provider} key could not be read from the secret store ({name}): {said}"))
}

fn first_line(text: &str) -> Option<String> {
    text.split('\n').map(js_trim).find(|line| !line.is_empty()).map(str::to_string)
}

/// The launcher that confines a launch: `AGENT_KIT_LAUNCHER_BIN`, else `agent-launcher` on `PATH`.
fn launcher_of(files: &dyn Files, env: &HashMap<String, String>) -> Option<String> {
    if let Some(named) = env.get("AGENT_KIT_LAUNCHER_BIN").filter(|named| !named.is_empty()) {
        return Some(named.clone());
    }
    let path = env.get("PATH").map_or("", String::as_str);
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .any(|dir| files.exists(&join(&[dir, "agent-launcher"])).unwrap_or(false))
        .then(|| "agent-launcher".to_string())
}

/// What a scope puts in a Pi launch's home: each skill and plugin the scope names, with the place in the
/// operator's home it is copied from. Nothing the scope does not list is here, so nothing else is copied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PiScope {
    /// `(name, folder)`: a folder holding a `SKILL.md`.
    pub skills: Vec<(String, String)>,
    /// `(name, path)`: a Pi extension, a file or a folder.
    pub plugins: Vec<(String, String)>,
}

/// Where the operator's skills are: Pi's own `skills` folder, then the shared `~/.agents/skills`.
fn skill_homes(env: &HashMap<String, String>) -> [String; 2] {
    [join(&[&home_agent_dir(env), "skills"]), join(&[env.get("HOME").map_or("", String::as_str), ".agents", "skills"])]
}

/// The skills and plugins a scope lists, found in the operator's home, or the one that is not there. The scope
/// has been checked by `unenforceable`, so every entry is a plain name; only the names it lists are looked for.
pub fn resolve_scope(files: &dyn Files, env: &HashMap<String, String>, tools: &ToolScope) -> Result<PiScope, String> {
    let names = |list: &ToolList| -> Vec<String> { if let ToolList::Only(items) = list { items.clone() } else { Vec::new() } };
    let found = |candidates: &[String]| candidates.iter().find(|path| files.exists(path).unwrap_or(false)).cloned();
    let mut scope = PiScope::default();
    let homes = skill_homes(env);
    for name in names(&tools.skills) {
        let candidates: Vec<String> = homes.iter().map(|home| join(&[home, &name])).collect();
        let marked: Vec<String> = candidates.iter().map(|folder| join(&[folder, "SKILL.md"])).collect();
        let Some(at) = found(&marked) else {
            return Err(format!("skill {name} is not in Pi's home: looked for {}", marked.join(" and ")));
        };
        scope.skills.push((name, at.trim_end_matches("/SKILL.md").to_string()));
    }
    let extensions = join(&[&home_agent_dir(env), "extensions"]);
    for name in names(&tools.plugins) {
        let candidates = [join(&[&extensions, &name]), join(&[&extensions, &format!("{name}.ts")]), join(&[&extensions, &format!("{name}.js")])];
        let Some(at) = found(&candidates) else {
            return Err(format!("plugin {name} is not in Pi's home: looked for {}, {} and {}", candidates[0], candidates[1], candidates[2]));
        };
        scope.plugins.push((name, at));
    }
    Ok(scope)
}

/// The launch's own agent directory: a `models.json` holding only the route's
/// provider, its key read from the secret store just now and written in plain
/// (mode 0600), and an empty `settings.json`; and what the confined launch runs
/// through. An error is why no launch can start: the home has no provider for
/// the route, the key cannot be read, or the operating system has no launcher
/// to confine with, and a Pi delegate never runs unconfined. With a `scope`, the
/// directory also holds a copy of each skill and plugin it lists and nothing
/// else of the operator's, and the launch loads only those.
pub fn prepare_pi(
    files: &dyn Files,
    process: &dyn Process,
    env: &HashMap<String, String>,
    provider: &str,
    model: &str,
    agent_dir: &str,
    scope: Option<&ToolScope>,
) -> Result<PiConfinement, String> {
    let route = route_of(files, env, provider, model)?;
    let launcher = launcher_of(files, env).ok_or_else(|| NO_LAUNCHER.to_string())?;
    let listed = match scope {
        Some(scope) => resolve_scope(files, env, scope)?,
        None => PiScope::default(),
    };
    let key = key_of(process, provider, &route.api_key)?;
    let entry = json::object(vec![
        ("api", Value::str(&route.api)),
        ("apiKey", Value::str(&key)),
        ("baseUrl", Value::str(&route.base_url)),
        ("models", Value::Arr(vec![route.model.clone()])),
    ]);
    let models = json::object(vec![("providers", json::object(vec![(provider, entry)]))]);
    let write_failed = |error: crate::ports::FsError| format!("pi: cannot write the launch's agent directory {agent_dir}: {}", error.message);
    files.write_private(&join(&[agent_dir, "settings.json"]), "{}\n").map_err(write_failed)?;
    let copy = |from: &str, kind: &str, name: &str, folder: &str| {
        let to = join(&[agent_dir, folder, from.rsplit('/').next().unwrap_or(name)]);
        files.copy_tree(from, &to).map(|()| to).map_err(|error| format!("pi: cannot copy {kind} {name} into the launch's agent directory {agent_dir}: {}", error.message))
    };
    let skills = listed.skills.iter().map(|(name, folder)| copy(folder, "skill", name, "skills")).collect::<Result<Vec<_>, _>>()?;
    let extensions = listed.plugins.iter().map(|(name, path)| copy(path, "plugin", name, "extensions")).collect::<Result<Vec<_>, _>>()?;
    // The key goes in last, so a write or a copy that fails leaves no key behind.
    files.write_private(&join(&[agent_dir, "models.json"]), &format!("{}\n", json::pretty(&models))).map_err(write_failed)?;
    let bin = join(&[&home_agent_dir(env), "bin"]);
    Ok(PiConfinement {
        launcher,
        agent_dir: agent_dir.to_string(),
        bin: files.exists(&bin).unwrap_or(false).then_some(bin),
        port: route.port,
        path: env.get("PATH").cloned(),
        skills,
        extensions,
    })
}
