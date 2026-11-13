//! Whether the three critique hooks are installed and trusted on this machine, and the kill switch that turns them off.
//! The node runner's `domain/agent-hooks.ts` and `application/agent-hooks.ts`.

use crate::error::CritiqueError;
use crate::gate_command;
use crate::locations::{critique_config_path, home_dir};
use crate::runtime::Runtime;
use model_gateway::json::{self, Object, Value};
use model_gateway::sha256::sha256_hex;
use crate::paths::join;
use model_gateway::text::{dirname, is_js_whitespace, js_trim};
use std::collections::HashMap;

/// What a machine check found, in one line: `pass` or `fail`, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineCheckOutcome {
    pub result: &'static str,
    pub detail: String,
}

/// One critique hook ADR-0002 allows: the event it runs on, the matcher of its group (none for the session start), and
/// the name of the `<name>.sh` script of the critique skill.
struct Hook {
    event: &'static str,
    matcher: Option<&'static str>,
    name: &'static str,
}

const CRITIQUE_HOOKS: [Hook; 3] = [
    Hook { event: "PreToolUse", matcher: Some("Bash"), name: "critique-commit-gate" },
    Hook { event: "PreToolUse", matcher: Some("Bash"), name: "critique-pr-gate" },
    Hook { event: "SessionStart", matcher: None, name: "critique-session-start" },
];

/// One critique hook as an agent's hook file wires it.
#[derive(Clone, Debug)]
pub struct WiredHook {
    name: &'static str,
    event: &'static str,
    matcher: Option<String>,
    /// The position of its matcher group under its event, and of its handler in the group: Codex keys its trust by both.
    group_index: usize,
    handler_index: usize,
    handler: Object,
    /// The script the command runs, as the command names it.
    script: String,
}

/// The token of a handler's command whose last path segment is `<name>.sh`, quotes removed.
fn script_of(handler: Option<&Value>, name: &str) -> Option<String> {
    let command = handler?.as_object()?.get("command")?.as_str()?;
    let wanted = format!("{name}.sh");
    command
        .replace('"', " ")
        .split(is_js_whitespace)
        .find(|token| token.rsplit('/').next() == Some(wanted.as_str()))
        .map(str::to_string)
}

/// Where an agent's hook file wires each critique hook: the first handler under the hook's event whose command names
/// `<name>.sh`, in a group that matches `Bash` for a `PreToolUse` hook. A hook it does not wire is absent.
pub fn wired_hooks(file: &Value) -> Vec<WiredHook> {
    let hooks = file.as_object().and_then(|file| file.get("hooks")).and_then(Value::as_object);
    let mut found = Vec::new();
    for hook in &CRITIQUE_HOOKS {
        let groups = match hooks.and_then(|hooks| hooks.get(hook.event)) {
            Some(Value::Arr(groups)) => groups.as_slice(),
            _ => &[],
        };
        for (group_index, group) in groups.iter().enumerate() {
            let Some(group) = group.as_object() else { continue };
            let matcher = group.get("matcher").and_then(Value::as_str);
            if hook.matcher.is_some() && matcher != hook.matcher {
                continue;
            }
            let handlers: &[Value] = match group.get("hooks") {
                Some(Value::Arr(handlers)) => handlers,
                _ => &[],
            };
            let Some(handler_index) = handlers.iter().position(|handler| script_of(Some(handler), hook.name).is_some()) else { continue };
            let handler = &handlers[handler_index];
            let (Some(script), Some(handler)) = (script_of(Some(handler), hook.name), handler.as_object()) else { continue };
            found.push(WiredHook {
                name: hook.name,
                event: hook.event,
                matcher: matcher.map(str::to_string),
                group_index,
                handler_index,
                handler: handler.clone(),
                script,
            });
            break;
        }
    }
    found
}

fn event_label(event: &str) -> &'static str {
    if event == "PreToolUse" {
        "pre_tool_use"
    } else {
        "session_start"
    }
}

/// Codex's default handler timeout, in seconds.
const CODEX_DEFAULT_TIMEOUT_S: f64 = 600.0;
/// Codex's default `additionalContextLimit`, which it leaves out of the hash.
const CODEX_DEFAULT_CONTEXT_LIMIT: f64 = 2500.0;

/// The key Codex records a user hook's state under in config.toml's `[hooks.state]`: the hook file, the event, and the
/// hook's position. A hook that moves to another position is a new hook to Codex.
fn codex_hook_key(hooks_file: &str, hook: &WiredHook) -> String {
    format!("{hooks_file}:{}:{}:{}", event_label(hook.event), hook.group_index, hook.handler_index)
}

/// The hash Codex trusts a command hook by (Codex 0.166.0, `hook_hash`): the sha256 of the event, the matcher and the
/// handler as Codex normalizes it, serialized as JSON with sorted keys. A hook whose hash differs from the trusted one is
/// "modified" and does not run until it is trusted again.
fn codex_hook_hash(hook: &WiredHook) -> String {
    let handler = &hook.handler;
    let timeout = match handler.get("timeout") {
        Some(Value::Num(timeout)) => timeout.max(1.0),
        _ => CODEX_DEFAULT_TIMEOUT_S,
    };
    // Every level's keys in sorted order, which is the order they are written here.
    let mut normalized = Object::new();
    if let Some(Value::Num(limit)) = handler.get("additionalContextLimit") {
        if *limit != CODEX_DEFAULT_CONTEXT_LIMIT {
            normalized.set("additionalContextLimit", Value::Num(*limit));
        }
    }
    normalized.set("async", Value::Bool(handler.get("async") == Some(&Value::Bool(true))));
    normalized.set("command", handler.get("command").cloned().unwrap_or(Value::Null));
    if let Some(Value::Str(message)) = handler.get("statusMessage") {
        normalized.set("statusMessage", Value::str(message));
    }
    normalized.set("timeout", Value::Num(timeout));
    normalized.set("type", Value::str("command"));
    let mut identity = Object::new();
    identity.set("event_name", Value::str(event_label(hook.event)));
    identity.set("hooks", Value::Arr(vec![Value::Obj(normalized)]));
    if let Some(matcher) = &hook.matcher {
        identity.set("matcher", Value::str(matcher));
    }
    format!("sha256:{}", sha256_hex(json::compact(&Value::Obj(identity)).as_bytes()))
}

/// One hook's state in Codex's config.toml: the hash the owner trusted, and whether `/hooks` turned it off.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CodexHookState {
    pub trusted_hash: Option<String>,
    pub enabled: Option<bool>,
}

/// The key of a `[hooks.state."<key>"]` line, as Codex writes it when the owner trusts a hook in `/hooks`: the text
/// between the quotes, escapes left as they are.
pub(crate) fn state_table_key(line: &str) -> Option<&str> {
    let rest = js_trim_start_ws(line.strip_prefix('[')?);
    let rest = rest.strip_prefix("hooks.state.\"")?;
    let mut end = None;
    let mut chars = rest.char_indices();
    while let Some((at, c)) = chars.next() {
        if c == '"' {
            end = Some(at);
            break;
        }
        if c == '\\' {
            // An escape takes the next character, whatever it is, but a line break.
            match chars.next() {
                Some((_, '\n' | '\r' | '\u{2028}' | '\u{2029}')) | None => return None,
                Some(_) => {}
            }
        }
    }
    let end = end?;
    (js_trim_start_ws(&rest[end + 1..]) == "]").then(|| &rest[..end])
}

fn js_trim_start_ws(text: &str) -> &str {
    text.trim_start_matches(is_js_whitespace)
}

/// The value of `<key>\s*=\s*` at the start of the line, and what follows it.
fn assignment<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = js_trim_start_ws(line.strip_prefix(key)?);
    Some(js_trim_start_ws(rest.strip_prefix('=')?))
}

/// The `[hooks.state."<key>"]` tables of a Codex config.toml. Only the two keys the check needs are read; anything else
/// in the file is ignored.
pub(crate) fn codex_hook_states(toml: &str) -> HashMap<String, CodexHookState> {
    let mut states: HashMap<String, CodexHookState> = HashMap::new();
    let mut current: Option<String> = None;
    for raw in toml.split('\n') {
        let line = js_trim(raw);
        if let Some(key) = state_table_key(line) {
            states.insert(key.to_string(), CodexHookState::default());
            current = Some(key.to_string());
            continue;
        }
        if line.starts_with('[') {
            current = None;
        }
        let Some(state) = current.as_ref().and_then(|key| states.get_mut(key)) else { continue };
        if let Some(value) = assignment(line, "trusted_hash").and_then(|value| value.strip_prefix('"')) {
            if let Some(end) = value.find('"') {
                state.trusted_hash = Some(value[..end].to_string());
            }
        }
        if let Some(value) = assignment(line, "enabled") {
            let boundary = |word: &str| value.strip_prefix(word).filter(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'));
            if boundary("true").is_some() {
                state.enabled = Some(true);
            } else if boundary("false").is_some() {
                state.enabled = Some(false);
            }
        }
    }
    states
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CodexTrust {
    Trusted,
    Untrusted,
    Modified,
    Disabled,
}

/// Codex runs a user hook only while it is enabled and its trusted hash is its current one.
fn codex_trust(state: Option<&CodexHookState>, hash: &str) -> CodexTrust {
    if state.and_then(|state| state.enabled) == Some(false) {
        return CodexTrust::Disabled;
    }
    match state.and_then(|state| state.trusted_hash.as_deref()) {
        None => CodexTrust::Untrusted,
        Some(trusted) if trusted == hash => CodexTrust::Trusted,
        Some(_) => CodexTrust::Modified,
    }
}

/// "a is ..." or "a, b and c are ..."; empty for no names.
fn clause(names: &[&str], one: &str, many: &str) -> String {
    match names {
        [] => String::new(),
        [only] => format!("{only} {one}"),
        [init @ .., last] => format!("{} and {last} {many}", init.join(", ")),
    }
}

/// One agent home the hooks are installed into, and the file that wires them.
struct AgentHome {
    label: &'static str,
    home: String,
    hooks_file: String,
    /// Codex's config.toml, which holds the owner's trust per hook; none for Claude Code, which runs hooks as configured.
    trust_file: Option<String>,
}

/// Claude Code's config root, where its settings.json wires hooks: `CLAUDE_CONFIG_DIR`, or `~/.claude`.
fn claude_home(runtime: &Runtime) -> Result<String, CritiqueError> {
    match runtime.env.get("CLAUDE_CONFIG_DIR") {
        Some(dir) => Ok(dir.clone()),
        None => Ok(join(&[home_dir(&runtime.env)?, ".claude"])),
    }
}

/// Codex's home, where hooks.json wires hooks and config.toml records their trust: `CODEX_HOME`, or `~/.codex`.
fn codex_home(runtime: &Runtime) -> Result<String, CritiqueError> {
    match runtime.env.get("CODEX_HOME") {
        Some(dir) => Ok(dir.clone()),
        None => Ok(join(&[home_dir(&runtime.env)?, ".codex"])),
    }
}

fn homes(runtime: &Runtime) -> Result<Vec<AgentHome>, CritiqueError> {
    let (claude, codex) = (claude_home(runtime)?, codex_home(runtime)?);
    Ok(vec![
        AgentHome { label: "Claude Code", hooks_file: join(&[&claude, "settings.json"]), home: claude, trust_file: None },
        AgentHome { label: "Codex", hooks_file: join(&[&codex, "hooks.json"]), trust_file: Some(join(&[&codex, "config.toml"])), home: codex },
    ])
}

/// What a file holds, or nothing when it is not there.
fn text(runtime: &Runtime, path: &str) -> Result<Option<String>, CritiqueError> {
    if !runtime.files.exists(path).unwrap_or(false) {
        return Ok(None);
    }
    runtime.files.read_text(path).map(Some).map_err(CritiqueError::from_fs)
}

fn sha256_of(value: &Value) -> String {
    sha256_hex(json::compact(value).as_bytes())
}

fn optional_text(text: Option<String>) -> Value {
    text.map_or(Value::Null, Value::Str)
}

/// sha256 of every file the hook check reads, so the item is due again when one changes.
pub fn installed_files_hash(runtime: &Runtime) -> Result<String, CritiqueError> {
    let mut read = Vec::new();
    for agent in homes(runtime)? {
        read.push(Value::Bool(runtime.files.exists(&agent.home).unwrap_or(false)));
        for path in std::iter::once(&agent.hooks_file).chain(agent.trust_file.as_ref()) {
            read.push(optional_text(text(runtime, path)?));
        }
    }
    Ok(sha256_of(&Value::Arr(read)))
}

/// Whether the three critique hooks are installed: in Claude Code's settings.json and Codex's hooks.json, each naming a
/// script that exists, and in Codex also trusted by the owner in `/hooks` at the position it now holds. It reads files
/// only; no agent is launched.
pub fn check_installed(runtime: &Runtime) -> Result<MachineCheckOutcome, CritiqueError> {
    let mut parts = Vec::new();
    let mut present = 0;
    let mut failed = false;
    for agent in homes(runtime)? {
        if !runtime.files.exists(&agent.home).unwrap_or(false) {
            parts.push(format!("{}: not set up here.", agent.label));
            continue;
        }
        present += 1;
        let problems = agent_problems(runtime, &agent)?;
        failed |= !problems.is_empty();
        let fine = if agent.trust_file.is_none() { "all three hooks installed" } else { "all three hooks installed and trusted" };
        parts.push(format!("{}: {}.", agent.label, if problems.is_empty() { fine.to_string() } else { problems.join("; ") }));
    }
    if present == 0 {
        return Ok(MachineCheckOutcome { result: "fail", detail: "Neither a Claude Code home nor a Codex home is set up on this machine, so no hook can run.".to_string() });
    }
    Ok(MachineCheckOutcome { result: if failed { "fail" } else { "pass" }, detail: parts.join(" ") })
}

/// What keeps the hooks from running in one agent home, in the order the owner fixes them; none when they run.
fn agent_problems(runtime: &Runtime, agent: &AgentHome) -> Result<Vec<String>, CritiqueError> {
    let Some(file) = text(runtime, &agent.hooks_file)? else {
        return Ok(vec![format!("{} does not exist", agent.hooks_file)]);
    };
    let Ok(file) = json::parse(&file) else {
        return Ok(vec![format!("{} does not parse", agent.hooks_file)]);
    };
    let wired = wired_hooks(&file);
    let mut missing = Vec::new();
    let mut scriptless = Vec::new();
    let mut running = Vec::new();
    for hook in &CRITIQUE_HOOKS {
        match wired.iter().find(|wired| wired.name == hook.name) {
            None => missing.push(hook.name),
            Some(wired) if !runtime.files.exists(&wired.script).unwrap_or(false) => scriptless.push(hook.name),
            Some(wired) => running.push(wired),
        }
    }
    let mut problems = vec![
        clause(&missing, "is not installed", "are not installed"),
        clause(&scriptless, "names a script that does not exist", "name scripts that do not exist"),
    ];
    if let Some(trust_file) = &agent.trust_file {
        let states = codex_hook_states(&text(runtime, trust_file)?.unwrap_or_default());
        let trust = |status: CodexTrust| -> Vec<&str> {
            running
                .iter()
                .filter(|hook| codex_trust(states.get(&codex_hook_key(&agent.hooks_file, hook)), &codex_hook_hash(hook)) == status)
                .map(|hook| hook.name)
                .collect()
        };
        problems.push(clause(&trust(CodexTrust::Modified), "changed since it was trusted", "changed since they were trusted"));
        problems.push(clause(&trust(CodexTrust::Untrusted), "is not trusted yet", "are not trusted yet"));
        problems.push(clause(&trust(CodexTrust::Disabled), "is turned off in /hooks", "are turned off in /hooks"));
    }
    Ok(problems.into_iter().filter(|problem| !problem.is_empty()).collect())
}

/// sha256 of what the kill switch reads: the configuration's text and `CRITIQUE_OFF`.
pub fn kill_switch_files_hash(runtime: &Runtime) -> Result<String, CritiqueError> {
    let config = optional_text(text(runtime, &critique_config_path(&runtime.env)?)?);
    let off = runtime.env.get("CRITIQUE_OFF").map_or(Value::Null, |off| Value::str(off));
    Ok(sha256_of(&json::object(vec![("config", config), ("off", off)])))
}

/// The kill switch: `CRITIQUE_OFF=1`, or `"disabled": true` in this machine's critique configuration, makes every hook a
/// no-op. The check asks the gate itself, with a command it never gates, so it reads the switch exactly as the hooks do.
pub fn check_kill_switch(runtime: &Runtime) -> Result<MachineCheckOutcome, CritiqueError> {
    let path = critique_config_path(&runtime.env)?;
    let payload = json::object(vec![
        ("cwd", Value::str(dirname(&path))),
        ("tool_input", json::object(vec![("command", Value::str("true"))])),
        ("tool_name", Value::str("Bash")),
    ]);
    let probe = gate_command::run(&payload, runtime)?;
    let field = |key: &str| probe.as_object().and_then(|probe| probe.get(key)).and_then(Value::as_str).map(str::to_string);
    let (result, detail) = match field("reason").as_deref() {
        Some("off") => ("pass", "Off: CRITIQUE_OFF=1 is set in this environment.".to_string()),
        Some("disabled") => ("pass", format!("Off: {path} disables every hook.")),
        Some("failed-open") => ("fail", format!("{path} does not parse, so every hook fails open and logs: {}", field("detail").unwrap_or_default())),
        _ => ("pass", format!("On: the hooks run. CRITIQUE_OFF=1, or {{\"disabled\": true}} in {path}, turns every one of them off.")),
    };
    Ok(MachineCheckOutcome { result, detail })
}

/// Writes the disabled flag, keeping the configuration's other keys. A file that is not a JSON object is left as it is.
pub fn set_kill_switch(runtime: &Runtime, disabled: bool) -> Result<String, CritiqueError> {
    let path = critique_config_path(&runtime.env)?;
    let mut config = Object::new();
    if runtime.files.exists(&path).unwrap_or(false) {
        let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
        match json::parse(&text) {
            Ok(Value::Obj(object)) => config = object,
            _ => {
                return Err(CritiqueError::new(
                    "invalid-input",
                    format!("setup input: {path} is not a JSON object; fix or remove it before turning the kill switch"),
                ))
            }
        }
    }
    config.set("disabled", Value::Bool(disabled));
    runtime.files.write_text(&path, &format!("{}\n", json::pretty(&Value::Obj(config)))).map_err(CritiqueError::from_fs)?;
    Ok(path)
}
