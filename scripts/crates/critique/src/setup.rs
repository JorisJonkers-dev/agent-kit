//! `setup`: the bookkeeping of `/flow setup`'s machine checklist. `list` says, per item, whether it is current or due;
//! `check` runs one item and records its result per machine, with the inputs it depended on; `quota` stores the owner's
//! Antigravity plan limit in the per-machine routing override; `kill-switch` turns the critique hooks off or on in the
//! per-machine critique configuration. A check that costs Vibe money or Antigravity quota runs only when the input
//! confirms it. The one-item-at-a-time walk with the owner is the skill's. The node runner's `application/setup.ts`.

use crate::agent_hooks::{check_installed, check_kill_switch, installed_files_hash, kill_switch_files_hash, set_kill_switch, MachineCheckOutcome};
use crate::clients::{client_binary, client_readiness, client_sign_in, client_version, review_with, CriticLaunch, CriticOutcome};
use crate::critic_run::FINDING_SCHEMA;
use crate::critic_tools::{gateway_problems, CriticToolset, GATEWAY_PROOF};
use crate::decode::{read_input, read_json_file};
use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::flow_budget::FlowBudget;
use crate::limits::AntigravityQuota;
use crate::locations::{home_dir, ledger_path, setup_probe_dir, setup_results_path};
use crate::prepared_json::client_value;
use crate::repository::{read_allowlist, resolve, Resolution};
use crate::roster::RosterCritic;
use crate::routing::{CriticClient, Routing};
use crate::routing_source::{load_routing, override_path};
use crate::runtime::Runtime;
use crate::search_client::{prove_search_reach, server_launch, session, stop, SearchCommand, SearchSession};
use crate::setup_checks::{parse_setup_results, setup_checklist, setup_item, SetupCheck, SetupCheckResult, SetupInputs, SetupItem, WRITE_PROOF_PENDING};
use crate::vibe_search::search_blocked;
use model_gateway::caps::to_fixed;
use model_gateway::json::{self, Object, Value};
use model_gateway::sha256::sha256_hex;
use crate::paths::join;
use model_gateway::timestamps::to_iso;

/// Setup's trivial prompt is a one-turn answer: a few cents at most, and never long.
const PROBE_PRICE_USD: f64 = 0.05;
const PROBE_TIME_LIMIT_S: f64 = 120.0;

const PROBE_NOTE: &str = "This note is agent-kit's setup check. It has nothing in it to review.\n";

const PROBE_PROMPT: &str = "This is agent-kit's setup check, not a review. Your working directory holds one file, note.md, which has nothing in it to review. Reply with exactly this findings object in a json code block:\n\n```json\n{\"findings\": [], \"no_findings_reason\": \"setup check: nothing to review\"}\n```\n";

/// The ledger's repository and flow for a setup check, which belongs to no repository and no flow.
const SETUP_LEDGER_SCOPE: &str = "machine-setup";

/// The live read-only proof's prompt: one `run_typescript` call that tries what a critic must never do, and one search,
/// then the empty findings.
fn gateway_prompt() -> String {
    format!(
        "This is agent-kit's setup check, not a review. Make exactly one tool call: run_typescript with this code, unchanged. Some of its calls are meant to be refused.\n\n```typescript\n{}\n```\n\nThen reply with exactly this findings object in a json code block:\n\n```json\n{{\"findings\": [], \"no_findings_reason\": \"setup check: nothing to review\"}}\n```\n",
        GATEWAY_PROOF.code
    )
}

/// What one check found.
struct Outcome {
    result: &'static str,
    detail: String,
}

fn outcome(result: &'static str, detail: impl Into<String>) -> Outcome {
    Outcome { result, detail: detail.into() }
}

impl From<MachineCheckOutcome> for Outcome {
    fn from(found: MachineCheckOutcome) -> Outcome {
        Outcome { result: found.result, detail: found.detail }
    }
}

/// What running a check came to: its outcome, or the need for the owner to confirm what it spends.
enum Ran {
    Outcome(Outcome),
    NeedsConfirmation,
}

enum Input {
    List { repo: Option<String> },
    Check { check: String, confirmed: bool, force: bool },
    Quota { daily_cap: f64, reserve_threshold: f64 },
    KillSwitch { disabled: bool },
}

fn read_setup_input(raw: &Value) -> Result<Input, crate::error::ShapeError> {
    let fields = Fields::of(raw, "")?;
    let action = if fields.has("action") { fields.one_of("action", &["list", "check", "quota", "kill-switch"])? } else { "list".to_string() };
    Ok(match action.as_str() {
        "list" => Input::List { repo: fields.optional_string("repo")? },
        "check" => Input::Check {
            check: fields.string("check")?,
            confirmed: if fields.has("confirmed") { fields.boolean("confirmed")? } else { false },
            force: if fields.has("force") { fields.boolean("force")? } else { false },
        },
        "kill-switch" => Input::KillSwitch { disabled: fields.boolean("disabled")? },
        _ => Input::Quota { daily_cap: fields.positive_integer("daily_cap")?, reserve_threshold: fields.positive_integer("reserve_threshold")? },
    })
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    match read_input("setup", read_setup_input(raw))? {
        Input::List { repo } => list(runtime, repo.as_deref()),
        Input::Check { check, confirmed, force } => Ok(check_json(&run_check(runtime, &check, confirmed, force)?)),
        Input::KillSwitch { disabled } => set_switch(runtime, disabled),
        Input::Quota { daily_cap, reserve_threshold } => set_quota(runtime, daily_cap, reserve_threshold),
    }
}

fn set_switch(runtime: &Runtime, disabled: bool) -> Result<Value, CritiqueError> {
    let config_path = set_kill_switch(runtime, disabled)?;
    let checked = run_check(runtime, "kill-switch", false, true)?;
    Ok(json::object(vec![
        ("config_path", Value::Str(config_path)),
        ("disabled", Value::Bool(disabled)),
        ("item", checked.item.to_json()),
        ("status", Value::str("kill-switch-set")),
    ]))
}

fn list(runtime: &Runtime, repo: Option<&str>) -> Result<Value, CritiqueError> {
    let routing = load_routing(runtime)?;
    let results = results(runtime)?;
    let mut items = Vec::new();
    for check in setup_checklist(&routing.clients) {
        let inputs = inputs_of(runtime, &check, &routing)?;
        items.push(setup_item(&check, inputs, results.iter().find(|(id, _)| *id == check.id).map(|(_, last)| last)).to_json());
    }
    let notes = stale_notes(runtime)?;
    let mut pairs = vec![("items", Value::Arr(items))];
    if !notes.is_empty() {
        pairs.push(("notes", Value::Arr(notes.into_iter().map(Value::Str).collect())));
    }
    pairs.push(("results_path", Value::Str(setup_results_path(&runtime.env)?)));
    pairs.push(("status", Value::str("listed")));
    if let Some(repo) = repo {
        pairs.push(("repository", repository(runtime, repo, &routing)?));
    }
    Ok(json::object(pairs))
}

/// What this machine's routing override still names that routing no longer reads. The per-flow Vibe cap is gone: Vibe is
/// counted against its plan limit, which the model gateway reads, so the runner ignores the key and the owner should
/// delete it.
fn stale_notes(runtime: &Runtime) -> Result<Vec<String>, CritiqueError> {
    let named = override_object(runtime)?.get("budget").and_then(Value::as_object).is_some_and(|budget| budget.get("vibe_flow_cap_usd").is_some());
    if !named {
        return Ok(Vec::new());
    }
    Ok(vec![format!(
        "your routing override still names budget.vibe_flow_cap_usd in {}: it is ignored, since Vibe is counted against its plan limit; delete it",
        override_path(runtime)?
    )])
}

struct Checked {
    status: &'static str,
    results_path: String,
    item: SetupItem,
}

fn check_json(checked: &Checked) -> Value {
    json::object(vec![("item", checked.item.to_json()), ("results_path", Value::str(&checked.results_path)), ("status", Value::str(checked.status))])
}

fn run_check(runtime: &Runtime, id: &str, confirmed: bool, force: bool) -> Result<Checked, CritiqueError> {
    let routing = load_routing(runtime)?;
    let checklist = setup_checklist(&routing.clients);
    let Some(check) = checklist.iter().find(|entry| entry.id == id) else {
        return Err(CritiqueError::new("invalid-input", format!("setup input: check {id} is not on this machine's checklist")));
    };
    let inputs = inputs_of(runtime, check, &routing)?;
    let results = results(runtime)?;
    let item = setup_item(check, inputs.clone(), results.iter().find(|(entry, _)| entry == id).map(|(_, last)| last));
    let results_path = setup_results_path(&runtime.env)?;
    if item.status == "not-available" {
        return Ok(Checked { status: "not-available", results_path, item });
    }
    if item.status == "current" && !force {
        return Ok(Checked { status: "current", results_path, item });
    }
    match ran(runtime, check, &routing, confirmed)? {
        Ran::NeedsConfirmation => Ok(Checked { status: "needs-confirmation", results_path, item }),
        Ran::Outcome(found) => Ok(Checked { status: "checked", results_path, item: record(runtime, check, inputs, found)? }),
    }
}

fn set_quota(runtime: &Runtime, daily_cap: f64, reserve: f64) -> Result<Value, CritiqueError> {
    if reserve > daily_cap {
        return Err(CritiqueError::new(
            "invalid-input",
            format!("setup input: the reserve of {} is above the daily cap of {}", json::number_text(reserve), json::number_text(daily_cap)),
        ));
    }
    load_routing(runtime)?;
    let path = override_path(runtime)?;
    let mut next = override_object(runtime)?;
    let mut quota = match next.get("antigravity_quota") {
        Some(Value::Obj(quota)) => quota.clone(),
        _ => Object::new(),
    };
    quota.set("daily_cap", Value::Num(daily_cap));
    quota.set("reserve_threshold", Value::Num(reserve));
    next.set("antigravity_quota", Value::Obj(quota));
    runtime.files.write_text(&path, &format!("{}\n", json::pretty(&Value::Obj(next)))).map_err(CritiqueError::from_fs)?;
    let checked = run_check(runtime, "agy-quota", false, true)?;
    Ok(json::object(vec![
        ("antigravity_quota", json::object(vec![("daily_cap", Value::Num(daily_cap)), ("reserve_threshold", Value::Num(reserve))])),
        ("item", checked.item.to_json()),
        ("override_path", Value::Str(path)),
        ("status", Value::str("quota-set")),
    ]))
}

fn sha256_of(value: &Value) -> String {
    sha256_hex(json::compact(value).as_bytes())
}

/// What the check's result depends on, read without launching a model.
fn inputs_of(runtime: &Runtime, check: &SetupCheck, routing: &Routing) -> Result<SetupInputs, CritiqueError> {
    let Some(config) = &check.client_config else {
        return match check.kind {
            "hooks" => Ok(SetupInputs::of(vec![("files_hash", Some(installed_files_hash(runtime)?))])),
            "kill-switch" => Ok(SetupInputs::of(vec![("files_hash", Some(kill_switch_files_hash(runtime)?))])),
            _ => Ok(SetupInputs::default()),
        };
    };
    if check.kind == "read-only" {
        let CriticClient::Vibe(vibe) = config else { return Ok(SetupInputs::default()) };
        return Ok(SetupInputs::of(vec![
            ("agent_hash", agent_hash(runtime, &vibe.agent)?),
            ("cli_version", client_version(runtime, config)),
            ("files_hash", Some(runner_hash(runtime)?)),
        ]));
    }
    if check.kind == "agy-quota" {
        let quota = override_object(runtime)?.get("antigravity_quota").filter(|quota| **quota != Value::Null).cloned().unwrap_or(Value::Null);
        let AntigravityQuota { daily_cap, reserve_threshold, reserved_stages } = &routing.limits.antigravity_quota;
        let routed = json::object(vec![
            ("daily_cap", Value::Num(*daily_cap)),
            ("reserve_threshold", Value::Num(*reserve_threshold)),
            ("reserved_stages", Value::Arr(reserved_stages.iter().map(|stage| Value::str(stage)).collect())),
        ]);
        return Ok(SetupInputs::of(vec![("routing_hash", Some(sha256_of(&json::object(vec![("override", quota), ("quota", routed)]))))]));
    }
    let version = client_version(runtime, config);
    if check.kind != "critic" {
        return Ok(SetupInputs::of(vec![("cli_version", version)]));
    }
    let mut inputs = vec![("cli_version", version)];
    if let CriticClient::Vibe(vibe) = config {
        inputs.push(("agent_hash", agent_hash(runtime, &vibe.agent)?));
    }
    inputs.push(("routing_hash", Some(sha256_of(&client_value(config)))));
    Ok(SetupInputs::of(inputs))
}

fn ran(runtime: &Runtime, check: &SetupCheck, routing: &Routing, confirmed: bool) -> Result<Ran, CritiqueError> {
    let found = |outcome: Outcome| Ok(Ran::Outcome(outcome));
    match check.kind {
        "hooks" => return found(check_installed(runtime)?.into()),
        "kill-switch" => return found(check_kill_switch(runtime)?.into()),
        "agy-quota" => return found(quota_check(runtime, routing)?),
        _ => {}
    }
    let Some(config) = &check.client_config else {
        return found(outcome("unproven", format!("{}.", check.reason.clone().unwrap_or_default())));
    };
    let client = check.client.clone().unwrap_or_default();
    match check.kind {
        "read-only" => {
            let reach = search_reach(runtime, &client, routing)?;
            // Without a search server the reach proof has already failed.
            if reach.result == "fail" {
                return found(reach);
            }
            let Some(own) = &runtime.search else { return found(reach) };
            let blocked = match config { CriticClient::Vibe(vibe) => search_blocked(runtime, &vibe.agent)?, _ => None };
            if let Some(blocked) = blocked {
                return found(outcome("fail", blocked));
            }
            if !confirmed {
                return Ok(Ran::NeedsConfirmation);
            }
            found(gateway_proof(runtime, own, &client, config, routing)?)
        }
        "critic" => {
            let readiness = client_readiness(runtime, config)?;
            if !readiness.ready {
                return found(outcome("fail", readiness.detail.unwrap_or_default()));
            }
            if !confirmed {
                return Ok(Ran::NeedsConfirmation);
            }
            found(probe(runtime, &client, config)?)
        }
        _ => {
            let sign_in = client_sign_in(runtime, config)?;
            found(if sign_in.ready {
                outcome("pass", format!("{} is installed and signed in", client_binary(config)))
            } else {
                outcome("fail", sign_in.detail.unwrap_or_default())
            })
        }
    }
}

/// The part of a Vibe client's read-only proof that needs no model: the runner's search server, started as a critic's
/// client starts it, finds nothing past a packet with every way out a critic could try.
fn search_reach(runtime: &Runtime, client: &str, routing: &Routing) -> Result<Outcome, CritiqueError> {
    let dir = join(&[&setup_probe_dir(&runtime.env, client)?, "search-proof"]);
    Ok(match prove_search_reach(runtime, &dir, &routing.limits.search).map_err(CritiqueError::unexpected)? {
        None => outcome("pass", "search cannot reach past the packet"),
        Some(problem) => outcome("fail", format!("search is not proven to stay inside the packet: {problem}")),
    })
}

/// The critic a setup check launches.
fn setup_critic(client: &str, config: &CriticClient) -> RosterCritic {
    RosterCritic {
        client: client.to_string(),
        client_config: config.clone(),
        critic_id: format!("setup-{client}"),
        fallbacks: Vec::new(),
        lenses: Vec::new(),
        max_price_usd: PROBE_PRICE_USD,
        role: "setup-check".to_string(),
        time_limit_s: PROBE_TIME_LIMIT_S,
    }
}

/// The live part of a Vibe client's read-only proof (ADR-0006, amended): a critic launched exactly as a searching critic
/// is, in a packet of one note, calls a shell command, a file write and a process start through `run_typescript`, then
/// one search. Each of the three must come back `tool_denied`, the search must complete, and the packet must be as it was.
fn gateway_proof(runtime: &Runtime, own: &SearchCommand, client: &str, config: &CriticClient, routing: &Routing) -> Result<Outcome, CritiqueError> {
    let dir = join(&[&setup_probe_dir(&runtime.env, client)?, "read-only"]);
    let packet_dir = join(&[&dir, "packet"]);
    let critic_dir = join(&[&dir, "critic"]);
    let prompt_path = join(&[&dir, "prompt.md"]);
    let fs = |result: Result<(), model_gateway::ports::FsError>| result.map_err(CritiqueError::from_fs);
    for path in runtime.dirs.list_tree(&packet_dir) {
        fs(runtime.files.remove(&join(&[&packet_dir, &path])))?;
    }
    fs(runtime.files.write_text(&join(&[&packet_dir, "note.md"]), PROBE_NOTE))?;
    fs(runtime.files.write_text(&prompt_path, &gateway_prompt()))?;
    // A previous check's output must never be read as this one's.
    fs(runtime.files.remove(&join(&[&critic_dir, "export", "export.json"])))?;
    let limits = &routing.limits.search;
    let tools = match session(runtime, own, &packet_dir, limits, &[]) {
        SearchSession::Failed { detail } => return Ok(outcome("fail", format!("search is not proven to stay inside the packet: {detail}"))),
        SearchSession::Started { tools, .. } => tools,
    };
    let critic = setup_critic(client, config);
    let report = review_with(
        runtime,
        &CriticLaunch {
            critic: &critic,
            packet_dir: packet_dir.clone(),
            prompt_path,
            critic_dir,
            schema_path: join(&[&runtime.assets_root, FINDING_SCHEMA]),
            toolset: CriticToolset::VibeSearch { search: tools, limits: limits.clone(), server: server_launch(own, &packet_dir, limits) },
        },
    );
    stop(runtime, &packet_dir);
    let report = report?;
    let status = outcome_status(&report.outcome);
    log_invocation(runtime, client, config, status, report.charged_usd)?;
    let charged = format!(" (charged ${})", to_fixed(report.charged_usd, 4));
    if let CriticOutcome::Failed { failure, detail, .. } = &report.outcome {
        return Ok(outcome("fail", format!("{failure}: {detail}{charged}")));
    }
    let after = runtime.dirs.list_tree(&packet_dir);
    let untouched = after == ["note.md"] && runtime.files.read_text(&join(&[&packet_dir, "note.md"])).map_err(CritiqueError::from_fs)? == PROBE_NOTE;
    let mut problems = gateway_problems(&report.tool_calls);
    if !untouched {
        problems.push("the critic changed its packet".to_string());
    }
    Ok(if problems.is_empty() {
        outcome(
            "pass",
            format!(
                "search cannot reach past the packet, and through run_typescript {} came back tool_denied while {} completed{charged}; {WRITE_PROOF_PENDING}",
                GATEWAY_PROOF.refused.join(", "),
                GATEWAY_PROOF.searched
            ),
        )
    } else {
        outcome("fail", format!("run_typescript is not proven to reach only the search tools: {}{charged}", problems.join("; ")))
    })
}

fn outcome_status(outcome: &CriticOutcome) -> &'static str {
    match outcome {
        CriticOutcome::Ok { .. } => "ok",
        CriticOutcome::Failed { .. } => "failed",
    }
}

/// The bundled runner, which is the search server: a new one must be proven again.
fn runner_hash(runtime: &Runtime) -> Result<String, CritiqueError> {
    let path = join(&[&runtime.assets_root, "critique.mjs"]);
    hash_of_file(runtime, &path).map(|hash| hash.unwrap_or_else(|| "none".to_string()))
}

/// The sha256 of the critic agent file; none when it is not installed.
fn agent_hash(runtime: &Runtime, agent: &str) -> Result<Option<String>, CritiqueError> {
    hash_of_file(runtime, &join(&[home_dir(&runtime.env)?, ".vibe", "agents", &format!("{agent}.toml")]))
}

/// A file that exists and cannot be read.
fn unreadable(path: &str) -> CritiqueError {
    CritiqueError::unexpected(format!("cannot read {path}"))
}

fn hash_of_file(runtime: &Runtime, path: &str) -> Result<Option<String>, CritiqueError> {
    if !runtime.files.exists(path).unwrap_or(false) {
        return Ok(None);
    }
    let bytes = runtime.dirs.read_bytes(path).ok_or_else(|| unreadable(path))?;
    Ok(Some(sha256_hex(&bytes)))
}

/// Antigravity reports no quota, so the check passes once the owner has stored the plan's limit in the override.
fn quota_check(runtime: &Runtime, routing: &Routing) -> Result<Outcome, CritiqueError> {
    let quota = &routing.limits.antigravity_quota;
    let used = FlowBudget::new(runtime).agy_invocations_today()?;
    let stored = override_object(runtime)?.get("antigravity_quota").and_then(Value::as_object).is_some_and(|stored| matches!(stored.get("daily_cap"), Some(Value::Num(_))));
    let figures = format!(
        "daily cap {}, reserve threshold {}, {used} used today",
        json::number_text(quota.daily_cap),
        json::number_text(quota.reserve_threshold)
    );
    Ok(if stored {
        outcome("pass", format!("{figures}; set in {}", override_path(runtime)?))
    } else {
        outcome("fail", format!("Antigravity cannot report its quota, so routing's default applies ({figures}); give your plan's daily limit"))
    })
}

/// Puts the trivial prompt to one critic client, exactly as a critic is launched, and logs the launch to the ledger.
fn probe(runtime: &Runtime, client: &str, config: &CriticClient) -> Result<Outcome, CritiqueError> {
    let dir = setup_probe_dir(&runtime.env, client)?;
    let packet_dir = join(&[&dir, "packet"]);
    let critic_dir = join(&[&dir, "critic"]);
    let prompt_path = join(&[&dir, "prompt.md"]);
    let fs = |result: Result<(), model_gateway::ports::FsError>| result.map_err(CritiqueError::from_fs);
    fs(runtime.files.write_text(&join(&[&packet_dir, "note.md"]), PROBE_NOTE))?;
    fs(runtime.files.write_text(&prompt_path, PROBE_PROMPT))?;
    // A previous check's output must never be read as this one's.
    fs(runtime.files.remove(&join(&[&critic_dir, "export", "export.json"])))?;
    fs(runtime.files.remove(&join(&[&critic_dir, "result.json"])))?;
    let critic = setup_critic(client, config);
    let report = review_with(
        runtime,
        &CriticLaunch {
            critic: &critic,
            packet_dir,
            prompt_path,
            critic_dir,
            schema_path: join(&[&runtime.assets_root, FINDING_SCHEMA]),
            toolset: crate::critic_tools::READ_ONLY_TOOLSET,
        },
    )?;
    log_invocation(runtime, client, config, outcome_status(&report.outcome), report.charged_usd)?;
    let charged = if config.kind() == "vibe" { format!(" (charged ${})", to_fixed(report.charged_usd, 4)) } else { String::new() };
    Ok(match &report.outcome {
        CriticOutcome::Ok { .. } => outcome("pass", format!("{client} answered the trivial prompt{charged}")),
        CriticOutcome::Failed { failure, detail, .. } => outcome("fail", format!("{failure}: {detail}{charged}")),
    })
}

/// An invocation line, as fan-out writes one per launch, so a setup check counts against today's Antigravity quota and
/// its Vibe spend stays on record. It belongs to no repository, flow, run or stage.
fn log_invocation(runtime: &Runtime, client: &str, config: &CriticClient, status: &str, charged: f64) -> Result<(), CritiqueError> {
    let line = json::object(vec![
        ("charged_usd", Value::Num(charged)),
        ("client", Value::str(client)),
        ("critic_id", Value::Str(format!("setup-{client}"))),
        ("family", Value::str(config.family())),
        ("flow", Value::str(SETUP_LEDGER_SCOPE)),
        ("kind", Value::str(config.kind())),
        ("model", Value::str(config.model())),
        ("recorded_at", Value::Str(to_iso(runtime.clock.now_ms()))),
        ("repository", Value::str(SETUP_LEDGER_SCOPE)),
        ("status", Value::str(status)),
        ("type", Value::str("invocation")),
    ]);
    let path = ledger_path(&runtime.env)?;
    runtime.files.mkdir_all(model_gateway::text::dirname(&path)).map_err(CritiqueError::from_fs)?;
    runtime.files.append(&path, &format!("{}\n", json::compact(&line))).map_err(CritiqueError::from_fs)
}

fn record(runtime: &Runtime, check: &SetupCheck, inputs: SetupInputs, found: Outcome) -> Result<SetupItem, CritiqueError> {
    let mut checks = results(runtime)?;
    let entry = SetupCheckResult { checked_at: to_iso(runtime.clock.now_ms()), detail: found.detail, inputs: inputs.clone(), result: found.result.to_string() };
    match checks.iter_mut().find(|(id, _)| *id == check.id) {
        Some((_, last)) => *last = entry.clone(),
        None => checks.push((check.id.clone(), entry.clone())),
    }
    let mut written = Object::new();
    for (id, result) in &checks {
        written.set(id, result.to_json());
    }
    let file = json::object(vec![("checks", Value::Obj(written)), ("version", Value::Num(1.0))]);
    runtime.files.write_text(&setup_results_path(&runtime.env)?, &format!("{}\n", json::pretty(&file))).map_err(CritiqueError::from_fs)?;
    Ok(setup_item(check, inputs, Some(&entry)))
}

/// Where the repository the owner is in stands with the family allowlist.
fn repository(runtime: &Runtime, repo: &str, routing: &Routing) -> Result<Value, CritiqueError> {
    let resolved = match resolve(runtime, repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(resolved) => resolved,
    };
    let mut routed: Vec<&str> = Vec::new();
    for (_, client) in routing.clients.iter().filter(|(_, client)| !matches!(client, CriticClient::Codex(_) | CriticClient::Claude(_))) {
        if !routed.contains(&client.family()) {
            routed.push(client.family());
        }
    }
    let routed_families = Value::Arr(routed.into_iter().map(Value::str).collect());
    let allowlist = read_allowlist(runtime)?;
    let entry = allowlist.get(&resolved.key);
    let (code, families, history, status) = match entry {
        None => (false, Vec::new(), false, "not-opted-in"),
        Some(entry) => (entry.code, entry.families.clone(), entry.history, "opted-in"),
    };
    Ok(json::object(vec![
        ("code", Value::Bool(code)),
        ("families", Value::Arr(families.iter().map(|family| Value::str(family)).collect())),
        ("history", Value::Bool(history)),
        ("key", Value::Str(resolved.key)),
        ("routed_families", routed_families),
        ("status", Value::str(status)),
    ]))
}

/// This machine's routing override as it is on disk; none is an empty one. Routing has already checked it is an object.
fn override_object(runtime: &Runtime) -> Result<Object, CritiqueError> {
    let path = override_path(runtime)?;
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Ok(Object::new());
    }
    let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
    Ok(json::parse(&text).map_err(CritiqueError::unexpected)?.as_object().cloned().unwrap_or_else(Object::new))
}

/// The results this machine recorded; none is an empty file.
fn results(runtime: &Runtime) -> Result<Vec<(String, SetupCheckResult)>, CritiqueError> {
    let path = setup_results_path(&runtime.env)?;
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Ok(Vec::new());
    }
    read_json_file(runtime, &path, "invalid-setup", |text| parse_setup_results(text).map_err(|error| error.0))
}
