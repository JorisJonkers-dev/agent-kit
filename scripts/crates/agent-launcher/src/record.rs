//! The support record: which agent can hold which reach level on which OS,
//! the tool configuration that makes it hold it, and the agent ids that name
//! one (agent, level, skill set) each.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The reach levels (ADR-0013's table), weakest first.
pub const LEVELS: [&str; 7] = [
    "read",
    "read-web",
    "research",
    "write-scoped",
    "run-one-command",
    "write",
    "full",
];

/// The operating systems a cell is recorded for.
pub const OSES: [&str; 2] = ["linux", "mac"];

/// Levels that write to one declared output directory, and so must name it.
const OUTPUT_LEVELS: [&str; 2] = ["research", "write-scoped"];

/// The whole file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    #[serde(default)]
    pub agents: BTreeMap<String, Agent>,
    #[serde(default)]
    pub ids: BTreeMap<String, AgentId>,
}

/// One agent: the command that starts it and what it holds at each level.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub levels: BTreeMap<String, Level>,
}

/// One level of one agent: its tool configuration (arguments) and the
/// per-OS state of the cell.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub support: BTreeMap<String, String>,
}

/// One agent id.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentId {
    pub agent: Option<String>,
    pub reach: Option<String>,
    pub skill_set: Option<String>,
    pub output_dir: Option<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
}

/// The state a cell may hold.
pub const STATES: [&str; 3] = ["verified", "unverified", "unsupported"];

/// Parses the file, or says why it is not a support record.
pub fn parse(text: &str) -> Result<Record, String> {
    serde_yaml::from_str(text).map_err(|error| error.to_string())
}

/// The state of one cell, `unverified` when the record holds none.
pub fn cell<'a>(record: &'a Record, agent: &str, level: &str, os: &str) -> &'a str {
    record
        .agents
        .get(agent)
        .and_then(|a| a.levels.get(level))
        .and_then(|l| l.support.get(os))
        .map_or("unverified", String::as_str)
}

/// The id grammar: lowercase letters and digits in dash-separated groups.
fn id_ok(id: &str) -> bool {
    id.split('-').all(|group| {
        !group.is_empty()
            && group
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

/// Every rule problem, in a stable order.
pub fn problems(record: &Record) -> Vec<String> {
    let mut out = Vec::new();
    for (name, agent) in &record.agents {
        if !id_ok(name) {
            out.push(format!("agent {name:?}: bad agent name"));
        }
        if agent.command.is_empty() || agent.command[0].is_empty() {
            out.push(format!("agent {name}: no command"));
        }
        for (level, entry) in &agent.levels {
            if !LEVELS.contains(&level.as_str()) {
                out.push(format!("agent {name}: unknown reach level {level:?}"));
            }
            for os in OSES {
                match entry.support.get(os).map(String::as_str) {
                    None => out.push(format!("agent {name} {level}: no support state for {os}")),
                    Some(state) if !STATES.contains(&state) => out.push(format!(
                        "agent {name} {level}: bad support state {state:?} for {os}"
                    )),
                    Some(_) => {}
                }
            }
            for os in entry.support.keys() {
                if !OSES.contains(&os.as_str()) {
                    out.push(format!("agent {name} {level}: unknown OS {os:?}"));
                }
            }
        }
    }
    for id in record.ids.keys() {
        out.extend(id_rule_problems(record, id));
    }
    duplicate_ids(record, &mut out);
    out
}

/// The rule problems of one id alone (not of its neighbours).
pub fn id_rule_problems(record: &Record, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(entry) = record.ids.get(id) {
        id_problems(record, id, entry, &mut out);
    }
    out
}

fn id_problems(record: &Record, id: &str, entry: &AgentId, out: &mut Vec<String>) {
    if !id_ok(id) {
        out.push(format!("id {id:?}: bad agent id"));
    }
    let Some(agent) = entry.agent.as_deref() else {
        out.push(format!("id {id}: no agent"));
        return;
    };
    if !record.agents.contains_key(agent) {
        out.push(format!("id {id}: unknown agent {agent:?}"));
    }
    let Some(level) = entry.reach.as_deref() else {
        out.push(format!("id {id}: no reach level"));
        return;
    };
    if !LEVELS.contains(&level) {
        out.push(format!("id {id}: unknown reach level {level:?}"));
        return;
    }
    match entry.output_dir.as_deref() {
        None if OUTPUT_LEVELS.contains(&level) => {
            out.push(format!("id {id}: reach level {level} needs an output_dir"));
        }
        Some(_) if !OUTPUT_LEVELS.contains(&level) => {
            out.push(format!("id {id}: reach level {level} takes no output_dir"));
        }
        Some(dir) if dir.is_empty() || dir.split('/').any(|part| part == "..") => {
            out.push(format!("id {id}: output_dir {dir:?} is empty or leaves its parent"));
        }
        _ => {}
    }
    if entry.hosts.is_empty() {
        out.push(format!("id {id}: declares no hosts"));
    }
    for os in &entry.hosts {
        if !OSES.contains(&os.as_str()) {
            out.push(format!("id {id}: unknown host {os:?}"));
        } else if record.agents.contains_key(agent) {
            let state = cell(record, agent, level, os);
            if state != "verified" {
                out.push(format!(
                    "id {id}: {agent} at {level} is {state} on {os}, and only a verified cell may be used"
                ));
            }
        }
    }
}

/// One id for each agent, level and skill set.
fn duplicate_ids(record: &Record, out: &mut Vec<String>) {
    let mut seen: BTreeMap<(&str, &str, &str), &str> = BTreeMap::new();
    for (id, entry) in &record.ids {
        let (Some(agent), Some(level)) = (entry.agent.as_deref(), entry.reach.as_deref()) else {
            continue;
        };
        let key = (agent, level, entry.skill_set.as_deref().unwrap_or(""));
        if let Some(first) = seen.insert(key, id) {
            out.push(format!(
                "id {id}: same agent, level and skill set as {first}; one id for each"
            ));
        }
    }
}
