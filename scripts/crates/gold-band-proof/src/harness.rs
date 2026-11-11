//! The headless proof itself: a throw-away Gold Band home whose every agent id
//! is the stub ACP agent, one `gold-band run` per scenario, and the record.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use crate::record::{Outcome, Record, ScenarioResult};
use crate::refuse;

/// What the harness needs to know about a workflow to stand the stub in for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkflowInfo {
    /// The workflow id (`workflow.id` in the authoring wrapper, else `id`).
    pub id: String,
    /// Every agent id the workflow names: node `provider`s and `modelBindings` `agentId`s.
    pub agent_ids: BTreeSet<String>,
    /// For each agent id, the model ids bound to it (node `model` of the nodes it runs, and `modelId`s of its bindings).
    pub models: BTreeMap<String, BTreeSet<String>>,
    /// Every profile id a node names.
    pub profiles: BTreeSet<String>,
    /// Whether any node has `"manual_check": true`.
    pub has_manual_check: bool,
}

/// One run of the workflow: the stub's verdict and the exit code expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioSpec {
    /// The scenario name, such as `clean`.
    pub name: String,
    /// The stub verdict: `clean` (or empty), `needs-human` or `fail`.
    pub verdict: String,
    /// The exit code `gold-band run` must return.
    pub expected_exit: i32,
}

/// Everything `prove` needs.
#[derive(Clone, Debug)]
pub struct ProveOptions {
    /// The workflow file.
    pub workflow: PathBuf,
    /// The gold-band binary, or `None` when none was supplied (the proof is skipped).
    pub gold_band: Option<PathBuf>,
    /// The Gold Band commit the binary was built from, when known.
    pub gold_band_commit: Option<String>,
    /// The executable that answers `stub-acp` (the harness itself).
    pub stub_exe: PathBuf,
    /// A directory the proof may create its temporary homes and workspaces in.
    pub scratch: PathBuf,
    /// Scenarios to run in addition to the defaults.
    pub extra_scenarios: Vec<ScenarioSpec>,
    /// Seconds one `gold-band run` may take before it is killed.
    pub timeout_secs: u64,
    /// The timestamp to record, RFC 3339 UTC.
    pub recorded_at: String,
}

/// The message printed when no Gold Band binary is supplied.
pub const SKIP_NO_BINARY: &str =
    "no Gold Band binary supplied (set GOLD_BAND_BIN or pass --gold-band); nothing was proved";

/// The requirement document the proof's workspace holds.
const REQUIREMENT: &str =
    "# Requirement\n\nThe headless proof runs this requirement through the workflow.\n";

/// How long the harness waits between two looks at a running child.
const POLL_MILLIS: u64 = 50;

/// How many characters of stderr a failing scenario's note keeps.
const STDERR_TAIL: usize = 300;

/// Reads the facts about a workflow from its JSON text.
pub fn inspect(workflow_json: &str) -> Result<WorkflowInfo, String> {
    let value: Value = serde_json::from_str(workflow_json)
        .map_err(|error| format!("the workflow is not valid JSON: {}", error))?;
    let definition = match value.get("workflow") {
        Some(wrapper) if wrapper.is_object() => wrapper,
        _ => &value,
    };
    let id = definition
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "the workflow has no id".to_string())?
        .to_string();
    let mut agent_ids = BTreeSet::new();
    let mut models: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut profiles = BTreeSet::new();
    let mut has_manual_check = false;
    if let Some(Value::Array(nodes)) = definition.get("nodes") {
        for node in nodes {
            let Some(object) = node.as_object() else {
                continue;
            };
            if let Some(provider) = object.get("provider").and_then(Value::as_str) {
                agent_ids.insert(provider.to_string());
                let entry = models.entry(provider.to_string()).or_default();
                entry.extend(
                    object
                        .get("model")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                );
            }
            if let Some(profile) = object.get("profile").and_then(Value::as_str) {
                profiles.insert(profile.to_string());
            }
            if object.get("manual_check").and_then(Value::as_bool) == Some(true) {
                has_manual_check = true;
            }
        }
    }
    if let Some(bindings) = value
        .get("modelBindings")
        .and_then(|wrapper| wrapper.get("bindings"))
        .and_then(Value::as_array)
    {
        for binding in bindings {
            let Some(object) = binding.as_object() else {
                continue;
            };
            if let Some(agent_id) = object.get("agentId").and_then(Value::as_str) {
                agent_ids.insert(agent_id.to_string());
                let entry = models.entry(agent_id.to_string()).or_default();
                entry.extend(
                    object
                        .get("modelId")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                );
            }
        }
    }
    Ok(WorkflowInfo {
        id,
        agent_ids,
        models,
        profiles,
        has_manual_check,
    })
}

/// The scenarios every workflow gets: `clean` (verdict `clean`, exit 0) always, and `needs-human` (exit 2) when the workflow has a manual-check node.
pub fn default_scenarios(info: &WorkflowInfo) -> Vec<ScenarioSpec> {
    let mut scenarios = vec![ScenarioSpec {
        name: "clean".to_string(),
        verdict: "clean".to_string(),
        expected_exit: 0,
    }];
    if info.has_manual_check {
        scenarios.push(ScenarioSpec {
            name: "needs-human".to_string(),
            verdict: "needs-human".to_string(),
            expected_exit: 2,
        });
    }
    scenarios
}

/// Parses `NAME:VERDICT:EXIT` (verdict one of `clean`, `needs-human`, `fail`; exit an integer).
pub fn parse_scenario(text: &str) -> Result<ScenarioSpec, String> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 3 {
        return Err(format!(
            "the scenario `{}` is not NAME:VERDICT:EXIT (three colon separated parts)",
            text
        ));
    }
    let name = parts[0];
    if name.is_empty() {
        return Err(format!("the scenario `{}` has an empty name", text));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(format!(
            "the scenario name `{}` may only hold letters, digits, `.`, `_` and `-`",
            name
        ));
    }
    let verdict = parts[1];
    if !matches!(verdict, "clean" | "needs-human" | "fail") {
        return Err(format!(
            "the scenario `{}` has the unknown verdict `{}` (use clean, needs-human or fail)",
            name, verdict
        ));
    }
    let exit_text = parts[2];
    let expected_exit = exit_text.parse::<i32>().map_err(|_| {
        format!(
            "the scenario `{}` has the exit code `{}`, which is not an integer",
            name, exit_text
        )
    })?;
    Ok(ScenarioSpec {
        name: name.to_string(),
        verdict: verdict.to_string(),
        expected_exit,
    })
}

/// Gold Band's exit code for a printed run state: completed and success is 0, completed otherwise 1, paused 2, running 4.
pub fn exit_for_state(status: &str, outcome: Option<&str>) -> Option<i32> {
    match status {
        "completed" => Some(if outcome == Some("success") { 0 } else { 1 }),
        "paused" => Some(2),
        "running" => Some(4),
        _ => None,
    }
}

/// Finds the run state `gold-band run` printed (a JSON object that starts at a line holding only `{`) and returns its `status` and `outcome`.
pub fn parse_run_state(output: &str) -> Option<(String, Option<String>)> {
    if let Some(state) = state_from_json(output) {
        return Some(state);
    }
    let mut start = 0usize;
    while start <= output.len() {
        let end = match output[start..].find('\n') {
            Some(index) => start + index,
            None => output.len(),
        };
        if &output[start..end] == "{" {
            return state_from_json(&output[start..]);
        }
        if end == output.len() {
            break;
        }
        start = end + 1;
    }
    None
}

/// The `status` and `outcome` of a JSON object, when the text is one and holds a status.
fn state_from_json(text: &str) -> Option<(String, Option<String>)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let status = object.get("status")?.as_str()?.to_string();
    let outcome = object
        .get("outcome")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some((status, outcome))
}

/// Builds `root/home` (a Gold Band home) and `root/workspace` for one scenario and returns `(home, workspace)`.
pub fn prepare(
    root: &Path,
    info: &WorkflowInfo,
    stub_exe: &Path,
    verdict: &str,
) -> Result<(PathBuf, PathBuf), String> {
    let home = root.join("home");
    let workspace = root.join("workspace");
    let gold_band_dir = home.join(".gold-band");
    let profiles_dir = gold_band_dir.join("context").join("profiles");
    fs::create_dir_all(&profiles_dir).map_err(|error| {
        format!(
            "cannot create the Gold Band home {}: {}",
            gold_band_dir.display(),
            error
        )
    })?;
    let settings_path = gold_band_dir.join("settings.json");
    fs::write(&settings_path, settings_json(info, stub_exe, verdict)).map_err(|error| {
        format!(
            "cannot write the Gold Band settings {}: {}",
            settings_path.display(),
            error
        )
    })?;
    for profile in &info.profiles {
        let path = profiles_dir.join(format!("{}.md", profile));
        fs::write(&path, profile_text(profile))
            .map_err(|error| format!("cannot write the profile {}: {}", path.display(), error))?;
    }
    fs::create_dir_all(&workspace).map_err(|error| {
        format!(
            "cannot create the workspace {}: {}",
            workspace.display(),
            error
        )
    })?;
    let requirement = workspace.join("requirement.md");
    fs::write(&requirement, REQUIREMENT).map_err(|error| {
        format!(
            "cannot write the requirement {}: {}",
            requirement.display(),
            error
        )
    })?;
    git_init(&workspace, &home)?;
    Ok((home, workspace))
}

/// Runs one scenario: prepares a fresh home under `root`, runs `gold-band run`, and reports what happened.
pub fn run_scenario(
    gold_band: &Path,
    workflow: &Path,
    root: &Path,
    info: &WorkflowInfo,
    stub_exe: &Path,
    spec: &ScenarioSpec,
    timeout_secs: u64,
) -> ScenarioResult {
    let mut result = ScenarioResult {
        name: spec.name.clone(),
        expected_exit: spec.expected_exit,
        exit_code: None,
        run_status: None,
        run_outcome: None,
        note: None,
    };
    let (home, workspace) = match prepare(root, info, stub_exe, &spec.verdict) {
        Ok(paths) => paths,
        Err(message) => {
            result.note = Some(message);
            return result;
        }
    };
    let stdout_path = root.join("stdout.txt");
    let stderr_path = root.join("stderr.txt");
    let mut child = match spawn_run(
        gold_band,
        workflow,
        &workspace,
        &home,
        &stdout_path,
        &stderr_path,
    ) {
        Ok(child) => child,
        Err(message) => {
            result.note = Some(message);
            return result;
        }
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().ok().flatten() {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            timed_out = true;
            break None;
        }
        sleep(Duration::from_millis(POLL_MILLIS));
    };
    let mut note = None;
    let exit_code = if timed_out {
        note = Some(format!("timed out after {} seconds", timeout_secs));
        None
    } else {
        match status.and_then(|status| status.code()) {
            Some(code) => Some(code),
            None => {
                note = Some("killed by a signal".to_string());
                None
            }
        }
    };
    let output = fs::read_to_string(&stdout_path).unwrap_or_default();
    if let Some((run_status, run_outcome)) = parse_run_state(&output) {
        result.run_status = Some(run_status);
        result.run_outcome = run_outcome;
    }
    if note.is_none() {
        note = disagreement_note(
            exit_code,
            spec.expected_exit,
            &stderr_path,
            result.run_status.as_deref(),
            result.run_outcome.as_deref(),
        );
    }
    result.exit_code = exit_code;
    result.note = note;
    result
}

/// Runs the whole proof and returns the record (not yet written).
pub fn prove(options: &ProveOptions) -> Result<Record, String> {
    let text = fs::read_to_string(&options.workflow).map_err(|error| {
        format!(
            "cannot read the workflow {}: {}",
            options.workflow.display(),
            error
        )
    })?;
    let workflow = refuse::load_workflow(&options.workflow)?;
    let info = inspect(&text)?;
    let mut record = Record {
        workflow_id: workflow.id,
        workflow_sha256: workflow.sha256,
        gold_band_commit: options.gold_band_commit.clone(),
        outcome: Outcome::Skipped,
        scenarios: Vec::new(),
        skip_reason: Some(SKIP_NO_BINARY.to_string()),
        recorded_at: options.recorded_at.clone(),
    };
    let Some(gold_band) = options.gold_band.as_deref() else {
        return Ok(record);
    };
    // Gold Band runs inside the scenario's workspace, so the workflow path must not depend on the caller's directory.
    let workflow_path = fs::canonicalize(&options.workflow).unwrap_or(options.workflow.clone());
    let scenarios = scenario_list(&info, &options.extra_scenarios);
    let mut results = Vec::with_capacity(scenarios.len());
    for (index, spec) in scenarios.iter().enumerate() {
        let root = options.scratch.join(format!("{}-{}", index, spec.name));
        results.push(run_scenario(
            gold_band,
            &workflow_path,
            &root,
            &info,
            &options.stub_exe,
            spec,
            options.timeout_secs,
        ));
    }
    let passed = results
        .iter()
        .all(|result| result.passed() && result.note.is_none());
    record.outcome = if passed {
        Outcome::Passed
    } else {
        Outcome::Failed
    };
    record.scenarios = results;
    record.skip_reason = None;
    Ok(record)
}

/// The default scenarios with the extra ones folded in: an extra replaces a default of the same name, others append.
fn scenario_list(info: &WorkflowInfo, extra: &[ScenarioSpec]) -> Vec<ScenarioSpec> {
    let mut scenarios = default_scenarios(info);
    for spec in extra {
        match scenarios
            .iter_mut()
            .find(|existing| existing.name == spec.name)
        {
            Some(existing) => *existing = spec.clone(),
            None => scenarios.push(spec.clone()),
        }
    }
    scenarios
}

/// The settings document that makes every agent id of the workflow the stub.
fn settings_json(info: &WorkflowInfo, stub_exe: &Path, verdict: &str) -> String {
    let command = stub_exe.to_string_lossy().into_owned();
    let mut agents = Map::new();
    for agent_id in &info.agent_ids {
        let models = info
            .models
            .get(agent_id)
            .map(|set| set.iter().cloned().collect::<Vec<String>>().join(","))
            .unwrap_or_default();
        agents.insert(
            agent_id.clone(),
            json!({
                "adapter": {
                    "command": &command,
                    "args": ["stub-acp"],
                    "displayName": format!("Stub ACP ({})", agent_id),
                    "env": {
                        "STUB_ACP_VERDICT": verdict,
                        "STUB_ACP_MODELS": models,
                    },
                },
            }),
        );
    }
    json!({
        "settingsSchemaVersion": 12,
        "agents": Value::Object(agents),
    })
    .to_string()
}

/// The profile file Gold Band needs for a profile id a node names.
fn profile_text(id: &str) -> String {
    format!(
        "---\nid: {}\nname: Stub profile {}\nsummary: Stub profile for the headless proof.\ncreatedAt: 2026-11-03T00:00:00Z\nupdatedAt: 2026-11-03T00:00:00Z\ndynamicTemplate: false\n---\n\nStub profile for the headless proof.\n",
        id, id
    )
}

/// Makes `workspace` a git repository with one empty commit, isolated from the user's git configuration.
fn git_init(workspace: &Path, home: &Path) -> Result<(), String> {
    let git = Path::new("git");
    git_step(git, workspace, home, "init", &["init", "-q"])?;
    git_step(
        git,
        workspace,
        home,
        "commit",
        &[
            "-c",
            "user.name=proof",
            "-c",
            "user.email=proof@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "proof",
        ],
    )
}

/// Runs one git step in `workspace` with no user or system configuration, naming the step when it fails.
pub(crate) fn git_step(
    program: &Path,
    workspace: &Path,
    home: &Path,
    step: &str,
    args: &[&str],
) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(workspace)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", home)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            format!(
                "cannot run git {} in {}: {}",
                step,
                workspace.display(),
                error
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "git {} failed in {}: {}",
        step,
        workspace.display(),
        stderr_text(&output.stderr)
    ))
}

/// The trimmed text of a command's stderr.
fn stderr_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_string()
}

/// Starts `gold-band run` with its output redirected to files, so no pipe can fill and block.
fn spawn_run(
    gold_band: &Path,
    workflow: &Path,
    workspace: &Path,
    home: &Path,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<Child, String> {
    let stdout = fs::File::create(stdout_path)
        .map_err(|error| format!("cannot create {}: {}", stdout_path.display(), error))?;
    let stderr = fs::File::create(stderr_path)
        .map_err(|error| format!("cannot create {}: {}", stderr_path.display(), error))?;
    Command::new(gold_band)
        .arg("run")
        .arg("--workflow")
        .arg(workflow)
        .arg("--requirement-file")
        .arg(workspace.join("requirement.md"))
        .current_dir(workspace)
        .env("GOLD_BAND_HOME", home)
        .env("HOME", home)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| format!("cannot run {}: {}", gold_band.display(), error))
}

/// Why a finished run does not match its scenario, when it does not.
fn disagreement_note(
    exit_code: Option<i32>,
    expected_exit: i32,
    stderr_path: &Path,
    run_status: Option<&str>,
    run_outcome: Option<&str>,
) -> Option<String> {
    let code = exit_code?;
    if code != expected_exit {
        let mut note = format!("expected exit {}, got {}", expected_exit, code);
        let stderr = fs::read_to_string(stderr_path).unwrap_or_default();
        let tail = tail_text(&stderr, STDERR_TAIL);
        if !tail.is_empty() {
            note.push_str(": ");
            note.push_str(&tail);
        }
        return Some(note);
    }
    let state_exit = run_status.and_then(|status| exit_for_state(status, run_outcome));
    if state_exit != Some(code) {
        return Some(format!(
            "exit {} is not backed by a printed run state ({}/{})",
            code,
            run_status.unwrap_or("none"),
            run_outcome.unwrap_or("none")
        ));
    }
    None
}

/// The last `limit` characters of `text`, trimmed.
fn tail_text(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    let count = trimmed.chars().count();
    if count <= limit {
        return trimmed.to_string();
    }
    trimmed.chars().skip(count - limit).collect()
}
