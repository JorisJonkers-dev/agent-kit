//! The live run home: the personal instance's agents plus a pinned shim agent
//! for every gate, copier and verdict id a workflow binds (issue #455).
//!
//! A live run uses the instance's own agent map (the real Pi, Claude and Vibe
//! adapters setup converged) and its generated profiles, untouched. What the
//! instance cannot hold is the shim agents: one per gate node, each pinned to
//! its own command, which belongs to the repository under test, not to setup.
//! So `gold-band-proof run --pins FILE` builds a run home next to the proof
//! records: a `settings.json` that is the instance's with the shim agents added,
//! and a `context` link to the instance's profiles. The instance's own
//! `settings.json` is never written.
//!
//! The pins file names each shim's command:
//!
//! ```json
//! {"version": 1, "timeout_secs": 3600, "shims": {"build": ["cargo", "build"]}}
//! ```
//!
//! A shim is looked up by its own name (`build-r1`), then without a repair
//! suffix (`build`), then without a round number (`findings-2` -> `findings`).
//! Placeholders in a command: `{repo}` (the node's working directory),
//! `{report_dir}`, `{run_id}`, `{attempt}`, `{tools}` (the directory of this
//! binary, where the workspace's other tools are built), `{base}` (the commit
//! the repository was at when the run started, so a gate measures the run's
//! own change) and `{requirement}` (the run's requirement file).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::harness::WorkflowInfo;
use crate::pinned;
use crate::stub::Role;

/// The pins file version this launcher reads.
const VERSION: u64 = 1;

/// The seconds a gate may run when the pins file names no limit.
const DEFAULT_TIMEOUT_SECS: u64 = 3600;

/// The settings schema version the pinned Gold Band writes. Setup's
/// `settings.json` carries none, and Gold Band reads a file without one as an
/// older schema whose custom agent ids are "not configured", so the run home
/// names it when the instance does not.
const SETTINGS_SCHEMA_VERSION: u64 = 12;

/// The pinned commands of one repository's shims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    /// The seconds each gate may run.
    pub timeout_secs: u64,
    /// The command of each shim, by shim name.
    pub shims: BTreeMap<String, Vec<String>>,
}

/// The pins a pins file holds, or why it is refused.
pub fn parse_pins(text: &str) -> Result<Pins, String> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| format!("the pins file is not JSON: {}", error))?;
    let Some(object) = value.as_object() else {
        return Err("the pins file is not a JSON object".to_string());
    };
    if let Some(key) = object
        .keys()
        .find(|key| !["version", "timeout_secs", "shims"].contains(&key.as_str()))
    {
        return Err(format!("the pins file has an unknown key `{}`", key));
    }
    if object.get("version").and_then(Value::as_u64) != Some(VERSION) {
        return Err(format!("the pins file must be version {}", VERSION));
    }
    let timeout_secs = match object.get("timeout_secs") {
        None => DEFAULT_TIMEOUT_SECS,
        Some(value) => value
            .as_u64()
            .filter(|secs| *secs > 0)
            .ok_or("`timeout_secs` must be a positive number")?,
    };
    let Some(shims) = object.get("shims").and_then(Value::as_object) else {
        return Err("`shims` must be an object".to_string());
    };
    let mut out = BTreeMap::new();
    for (name, command) in shims {
        let argv: Vec<String> = serde_json::from_value(command.clone())
            .map_err(|_| format!("shim `{}` must be an array of strings", name))?;
        if argv.is_empty() {
            return Err(format!("shim `{}` names no command", name));
        }
        out.insert(name.clone(), argv);
    }
    Ok(Pins {
        timeout_secs,
        shims: out,
    })
}

/// `name` without a trailing `-<suffix><digits>`, when it has one.
fn strip_numbered<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    let (base, tail) = name.rsplit_once('-')?;
    let digits = tail.strip_prefix(suffix)?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(base)
}

/// The pinned command of one shim: its own name, then the name without a
/// repair suffix, then without a round number.
pub fn shim_command<'a>(pins: &'a Pins, shim: &str) -> Option<&'a Vec<String>> {
    if let Some(command) = pins.shims.get(shim) {
        return Some(command);
    }
    let base = strip_numbered(shim, "r").or_else(|| strip_numbered(shim, ""))?;
    pins.shims.get(base)
}

/// What a live run home is built from.
pub struct LiveInputs<'a> {
    /// The workflow being run.
    pub info: &'a WorkflowInfo,
    /// The repository's pinned shim commands.
    pub pins: &'a Pins,
    /// The instance's Gold Band home (the directory holding `.gold-band`).
    pub instance_home: &'a Path,
    /// The run home to build; it must not exist yet.
    pub run_home: &'a Path,
    /// The pinned agent's executable, this binary.
    pub exe: &'a Path,
    /// The launcher's placeholder values.
    pub values: &'a BTreeMap<String, String>,
}

/// The settings entry of one pinned shim agent.
fn pinned_entry(
    inputs: &LiveInputs,
    agent_id: &str,
    role: &Role,
    command: &[String],
    reports: &Path,
) -> Value {
    let models = inputs
        .info
        .models
        .get(agent_id)
        .map(|set| set.iter().cloned().collect::<Vec<String>>().join(","))
        .unwrap_or_default();
    json!({
        "adapter": {
            "command": inputs.exe.to_string_lossy(),
            "args": ["pinned-acp"],
            "displayName": format!("Pinned shim ({})", agent_id),
            "env": {
                pinned::ROLE_ENV: role.as_text(),
                pinned::COMMAND_ENV: Value::from(command.to_vec()).to_string(),
                pinned::REPORT_ROOT_ENV: reports.to_string_lossy(),
                pinned::TIMEOUT_ENV: inputs.pins.timeout_secs.to_string(),
                pinned::MODELS_ENV: models,
                pinned::VALUES_ENV: json!(inputs.values).to_string(),
            },
        },
        "systemPromptDelivery": "none",
    })
}

/// Builds the run home and returns its report directory, or every reason the
/// run cannot be bound.
pub fn compose(inputs: &LiveInputs) -> Result<PathBuf, Vec<String>> {
    let settings_path = inputs.instance_home.join(".gold-band/settings.json");
    let text = fs::read_to_string(&settings_path).map_err(|error| {
        vec![format!(
            "cannot read {}: {}",
            settings_path.display(),
            error
        )]
    })?;
    let mut settings: Map<String, Value> = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(object)) => object,
        _ => {
            return Err(vec![format!(
                "{} is not a JSON object",
                settings_path.display()
            )])
        }
    };
    let mut agents = match settings.remove("agents") {
        Some(Value::Object(agents)) => agents,
        _ => {
            return Err(vec![format!(
                "{} has no agents map",
                settings_path.display()
            )])
        }
    };
    let reports = inputs.run_home.join("reports");
    let mut problems = Vec::new();
    let mut added = Map::new();
    for agent_id in &inputs.info.agent_ids {
        let role = inputs.info.roles.get(agent_id);
        let shim = match role {
            Some(Role::Gate(name) | Role::Copy(name)) => Some(name.clone()),
            Some(Role::Verdict(round)) => Some(format!("findings-{}", round)),
            Some(Role::Grant(_)) => {
                problems.push(format!(
                    "agent {}: a grant check belongs to the work instance, not a personal live run",
                    agent_id
                ));
                continue;
            }
            _ => None,
        };
        let (Some(shim), Some(role)) = (shim, role) else {
            if !agents.contains_key(agent_id) {
                problems.push(format!(
                    "agent {}: not in the instance's agent map ({})",
                    agent_id,
                    settings_path.display()
                ));
            }
            continue;
        };
        if agents.contains_key(agent_id) {
            problems.push(format!(
                "agent {}: the instance already declares it, so it cannot be pinned",
                agent_id
            ));
            continue;
        }
        let command: Vec<String> = match role {
            Role::Verdict(_) => Vec::new(),
            _ => match shim_command(inputs.pins, &shim) {
                Some(command) => command.clone(),
                None => {
                    problems.push(format!(
                        "agent {}: the pins file names no command for `{}`",
                        agent_id, shim
                    ));
                    continue;
                }
            },
        };
        added.insert(
            agent_id.clone(),
            pinned_entry(inputs, agent_id, role, &command, &reports),
        );
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    agents.extend(added);
    settings.insert("agents".to_string(), Value::Object(agents));
    settings
        .entry("settingsSchemaVersion")
        .or_insert(Value::from(SETTINGS_SCHEMA_VERSION));
    let write = || -> std::io::Result<()> {
        let data = inputs.run_home.join(".gold-band");
        fs::create_dir_all(&data)?;
        fs::create_dir_all(&reports)?;
        let text = serde_json::to_string_pretty(&Value::Object(settings))? + "\n";
        fs::write(data.join("settings.json"), text)?;
        let context = inputs.instance_home.join(".gold-band/context");
        if context.is_dir() {
            std::os::unix::fs::symlink(&context, data.join("context"))?;
        }
        Ok(())
    };
    write().map_err(|error| {
        vec![format!(
            "cannot build the run home {}: {}",
            inputs.run_home.display(),
            error
        )]
    })?;
    Ok(reports)
}
