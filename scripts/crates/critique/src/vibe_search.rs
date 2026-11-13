//! What decides whether a Vibe critic searches, and what its journal says of the searches it made: the installed agent
//! file and the home's configuration (read as TOML), the extras a searching launch asks the gateway for, and the tool
//! calls the journal records. The node runner's `adapters/vibe/index.ts`.

use crate::critic_tools::{SearchServerLaunch, ToolCall, VIBE_SEARCH_SERVER};
use crate::error::CritiqueError;
use crate::packet_search::SearchToolName;
use crate::runtime::Runtime;
use crate::vibe_client::{journal_entries, js_string, vibe_home};
use model_gateway::extras::{AttachedServer, RunExtras};
use model_gateway::json::{Object, Value};
use model_gateway::text::join;
use model_gateway::toml::read_toml;
use std::collections::BTreeMap;

/// What a Vibe critic asks the gateway for. Every critic runs its own agent, which pins the model and its prices, with
/// `read_file` and, when it searches (ADR-0006), the runner's own server in its packet copy: the gateway switches the
/// home's servers off by name, so Vibe's joining of every layer's servers leaves only the runner's live, and Mistral's
/// hosted connectors stay off.
pub fn vibe_extras(agent: &str, search: Option<(&[SearchToolName], &SearchServerLaunch)>) -> RunExtras {
    let mut tools = vec!["read_file".to_string()];
    let Some((names, server)) = search else {
        return RunExtras { agent: Some(agent.to_string()), tools: Some(tools), ..RunExtras::default() };
    };
    tools.extend(names.iter().map(|tool| format!("{VIBE_SEARCH_SERVER}_{}", tool.name())));
    RunExtras {
        agent: Some(agent.to_string()),
        disable_home_mcp: Some(true),
        env: Some(BTreeMap::from([("VIBE_ENABLE_CONNECTORS".to_string(), "false".to_string())])),
        mcp_servers: Some(vec![AttachedServer { name: VIBE_SEARCH_SERVER.to_string(), command: server.command.clone(), args: server.args.clone() }]),
        tools: Some(tools),
        ..RunExtras::default()
    }
}

/// Why a critic of this agent cannot search, read from the installed agent file before any launch. The agent file sits
/// above the launch's environment, so one that sets `enable_mcp = false` switches the search server off whatever the
/// launch asks, and one that leaves the search tools unapproved makes headless Vibe refuse every call.
pub fn search_blocked(runtime: &Runtime, agent: &str) -> Result<Option<String>, CritiqueError> {
    let home = vibe_home(runtime)?;
    let path = join(&[&home, "agents", &format!("{agent}.toml")]);
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Ok(Some(format!("agent-file-missing: ~/.vibe/agents/{agent}.toml is not installed; re-run agent-kit setup")));
    }
    let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
    if let Some(problem) = agent_file_search_problem(&text, agent) {
        return Ok(Some(problem));
    }
    let config = join(&[&home, "config.toml"]);
    if !runtime.files.exists(&config).unwrap_or(false) {
        return Ok(None);
    }
    Ok(home_config_problem(&runtime.files.read_text(&config).map_err(CritiqueError::from_fs)?))
}

/// Why an agent file keeps its critics from searching, read as TOML: it cannot be read, it switches MCP off, it declares
/// MCP servers of its own (they would run beside the search server, since Vibe joins every layer's servers), or it
/// leaves a search tool out of `enabled_tools` or without `permission = "always"` (headless Vibe refuses every call that
/// asks). None when it lets them search.
pub fn agent_file_search_problem(toml: &str, agent: &str) -> Option<String> {
    let file = format!("~/.vibe/agents/{agent}.toml");
    let fix = "re-run agent-kit setup to install the current critic agents";
    let document = match read_toml(toml) {
        // A TOML document is a table.
        Ok(document) => document.as_object().cloned().unwrap_or_else(Object::new),
        Err(message) => return Some(format!("agent-file-unreadable: {file} is not TOML the runner can read ({message}); {fix}")),
    };
    if document.get("enable_mcp") == Some(&Value::Bool(false)) {
        return Some(format!("agent-file-disables-mcp: {file} sets enable_mcp = false, which the launch cannot override; {fix}"));
    }
    let mut declared = Vec::new();
    declared_servers(&Value::Obj(document.clone()), &mut declared);
    if !declared.is_empty() {
        return Some(format!(
            "agent-file-declares-servers: {file} declares MCP servers ({}) that would run beside the search server; {fix}",
            declared.join(", ")
        ));
    }
    let enabled: &[Value] = match document.get("enabled_tools") {
        Some(Value::Arr(items)) => items,
        _ => &[],
    };
    let settings = document.get("tools").and_then(Value::as_object);
    let unapproved: Vec<String> = ["grep", "list", "symbol"]
        .iter()
        .map(|tool| format!("{VIBE_SEARCH_SERVER}_{tool}"))
        .filter(|tool| {
            let permission = settings.and_then(|settings| settings.get(tool)).and_then(Value::as_object).and_then(|setting| setting.get("permission"));
            !enabled.iter().any(|name| name.as_str() == Some(tool)) || permission.and_then(Value::as_str) != Some("always")
        })
        .collect();
    if unapproved.is_empty() {
        return None;
    }
    Some(format!("agent-file-lacks-search-tools: {file} does not enable and approve {}; {fix}", unapproved.join(", ")))
}

/// The names of every `mcp_servers` the value declares, at any depth, in the order met.
fn declared_servers(value: &Value, names: &mut Vec<String>) {
    match value {
        Value::Arr(items) => items.iter().for_each(|item| declared_servers(item, names)),
        Value::Obj(object) => {
            for (key, child) in object.entries() {
                if key == "mcp_servers" {
                    names.extend(server_names(child));
                }
                declared_servers(child, names);
            }
        }
        _ => {}
    }
}

/// The `name` of each server in an `mcp_servers` value, `?` for one without.
fn server_names(servers: &Value) -> Vec<String> {
    let name = |server: &Value| match server.as_object().and_then(|server| server.get("name")) {
        Some(Value::Str(name)) => name.clone(),
        _ => "?".to_string(),
    };
    match servers {
        Value::Arr(items) => items.iter().map(name).collect(),
        other => vec![name(other)],
    }
}

/// Why the home's `config.toml` keeps a critic from searching: the runner must read its MCP server names to switch them
/// off, so a file it cannot read blocks search. None when it reads.
pub fn home_config_problem(toml: &str) -> Option<String> {
    read_toml(toml).err().map(|message| {
        format!("home-config-unreadable: ~/.vibe/config.toml is not TOML the runner can read ({message}), so its MCP servers cannot be switched off by name")
    })
}

/// Every tool call the journal records, in order: its `effect` entries, each with the tool's name (`file_system.bash`,
/// `mcp_packet.grep`), its status and, for a failed call, the error code (`tool_denied`).
pub fn tool_calls(stdout: &str) -> Vec<ToolCall> {
    journal_entries(stdout)
        .iter()
        .filter(|entry| entry.as_ref().and_then(|entry| entry.get("type")).and_then(Value::as_str) == Some("effect"))
        .map(|entry| {
            let state = state_of(entry.as_ref());
            let code = state.and_then(|state| state.get("error")).and_then(Value::as_object).and_then(|error| error.get("code")).and_then(Value::as_str);
            ToolCall {
                tool: js_string(entry.as_ref().and_then(|entry| entry.get("detail")).and_then(Value::as_object).and_then(|detail| detail.get("toolName"))),
                status: js_string(state.and_then(|state| state.get("status"))),
                code: code.map(str::to_string),
            }
        })
        .collect()
}

fn state_of(entry: Option<&Object>) -> Option<&Object> {
    entry.and_then(|entry| entry.get("state")).and_then(Value::as_object)
}

/// Why a searching critic's searches failed, as its journal shows them: each search call that did not complete, such as
/// one made after the server died. None when every call completed.
pub fn search_failure(stdout: &str) -> Option<String> {
    let prefix = format!("mcp_{VIBE_SEARCH_SERVER}.");
    let entries = journal_entries(stdout);
    let calls: Vec<&Object> = entries
        .iter()
        .flatten()
        .filter(|entry| {
            entry.get("type").and_then(Value::as_str) == Some("effect")
                && js_string(entry.get("detail").and_then(Value::as_object).and_then(|detail| detail.get("toolName"))).starts_with(&prefix)
        })
        .collect();
    let failed: Vec<&Object> =
        calls.iter().filter_map(|entry| state_of(Some(entry))).filter(|state| state.get("status").and_then(Value::as_str) == Some("failed")).collect();
    let first = failed.first()?;
    let error = first.get("error").and_then(Value::as_object);
    Some(format!(
        "{} of {} search calls failed, the first with {}: {}",
        failed.len(),
        calls.len(),
        js_string(error.and_then(|error| error.get("code"))),
        js_string(error.and_then(|error| error.get("message")))
    ))
}
