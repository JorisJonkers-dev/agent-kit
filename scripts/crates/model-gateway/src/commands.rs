//! The commands the Rust gateway carries: `models`, `usage`, `grant write`,
//! `grant reserve`, `grant list`, `worktrees clean` and the three workflow
//! commands, and `run`, read-only and writing. `batch` answers `not-ported`.

use crate::availability;
use crate::caps;
use crate::config::GatewayConfig;
use crate::config_parse::load_config;
use crate::error::GatewayError;
use crate::grants::{self, NewGrant};
use crate::grants_file::{GrantChange, GrantsStore};
use crate::json::{Object, Value};
use crate::ledger::LedgerEntry;
use crate::ledger_file::{settle_stale, LedgerStore};
use crate::launchers::DELEGATION_MARK;
use crate::locations;
use crate::models::{self, ClientListing};
use crate::policy::{self, PolicySubject};
use crate::ports::Runtime;
use crate::sources;
use crate::state_file::Workflows;
use crate::usage::{self, ClientReading};
use crate::workflows;
use std::collections::{BTreeMap, BTreeSet};

/// The subcommands the gateway has, as the command line names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Batch,
    GrantList,
    GrantReserve,
    GrantWrite,
    Models,
    Run,
    Usage,
    WorkflowEnd,
    WorkflowList,
    WorkflowStart,
    WorktreesClean,
}

/// Runs one subcommand over its parsed input.
pub fn run(command: Command, name: &str, input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    match command {
        Command::Models => models_command(input, runtime),
        Command::Usage => usage_command(input, runtime),
        Command::GrantWrite => grant_write(input, runtime),
        Command::GrantList => grant_list(input, runtime),
        Command::GrantReserve => crate::reserve::reserve(input, runtime),
        Command::WorktreesClean => crate::worktrees::clean(input, runtime),
        Command::Run => crate::run_command::RunCommand { runtime }.run(&Value::Obj(input.clone())),
        Command::WorkflowStart => workflow_start(input, runtime),
        Command::WorkflowEnd => workflow_end(input, runtime),
        Command::WorkflowList => workflow_list(runtime),
        _ => Err(GatewayError::new("not-ported", format!("{name} is not ported to the Rust gateway yet; use the node runner"))),
    }
}

/// Starts a workflow, or reports the one that is already running.
fn workflow_start(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let name = read_workflow_input(input)?;
    let now = runtime.clock.now_ms();
    let workflow = Workflows { runtime }.update(|current| workflows::start(current, &name, now))?;
    Ok(ok_object("workflow", workflows::view(&workflow, now)))
}

/// Ends a workflow at this moment; one that already ended keeps the moment it ended.
fn workflow_end(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let name = read_workflow_input(input)?;
    let now = runtime.clock.now_ms();
    let workflow = Workflows { runtime }.update(|current| workflows::end(current, &name, now))?;
    Ok(ok_object("workflow", workflows::view(&workflow, now)))
}

/// Lists every workflow this machine has started, with the 24-hour end worked out from the clock.
fn workflow_list(runtime: &Runtime) -> Result<Value, GatewayError> {
    let records = Workflows { runtime }.read()?;
    let now = runtime.clock.now_ms();
    Ok(ok_object("workflows", Value::Arr(records.iter().map(|record| workflows::view(record, now)).collect())))
}

/// `workflow start` and `workflow end` take the name of the workflow they act on.
fn read_workflow_input(input: &Object) -> Result<String, GatewayError> {
    workflows::require_name(input.get("name"), "a workflow command must name its workflow")
}

/// `{"status": "ok", <key>: <value>}`.
fn ok_object(key: &str, value: Value) -> Value {
    let mut object = Object::new();
    object.set("status", Value::str("ok"));
    object.set(key, value);
    Value::Obj(object)
}

/// Lists every client model this machine can use now for the repository the caller works in, and the clients it cannot.
fn models_command(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let repo = read_repo(input)?;
    let config = load_config(runtime)?;
    let mut installed = BTreeSet::new();
    for (id, client) in &config.clients {
        if runtime.process.resolve(&client.binary).is_some() {
            installed.insert(id.clone());
        }
    }
    let mut listings: BTreeMap<String, ClientListing> = BTreeMap::new();
    for (id, client) in &config.clients {
        if !installed.contains(id) || client.model_source == crate::config::ModelSource::Declared {
            continue;
        }
        if let Some(listing) = sources::list_client(runtime, id) {
            listings.insert(id.clone(), listing);
        }
    }
    let now = runtime.clock.now_ms();
    let unavailable = standing_refusals(runtime, now);
    let listing = models::list_models(&config, &installed, &listings, &unavailable);
    let scope = policy::resolve_scope(runtime, repo.as_deref())?;
    let subjects: Vec<PolicySubject> = listing
        .models
        .iter()
        .map(|model| PolicySubject { client: model.client.clone(), model: model.model.clone(), family: model.family.clone() })
        .collect();
    let decision = policy::decide_policy(&config, &scope, &subjects);
    Ok(policy::models_result_json(&decision.reason, &policy::permitted_models(listing, &decision)))
}

/// `models` takes the path of the repository the caller works in, when it has one.
fn read_repo(input: &Object) -> Result<Option<String>, GatewayError> {
    match input.get("repo") {
        None => Ok(None),
        Some(Value::Str(repo)) if !repo.is_empty() => Ok(Some(repo.clone())),
        Some(_) => Err(GatewayError::invalid_input("repo must be a non-empty string")),
    }
}

/// The client models this machine cannot run now, as their own answers said: read from the machine-wide ledger.
/// A ledger the gateway cannot read knows nothing, so it lists none and the listing still works.
fn standing_refusals(runtime: &Runtime, now_ms: i64) -> Vec<models::SkippedModel> {
    let Ok(entries) = (LedgerStore { runtime }).read() else {
        return Vec::new();
    };
    usage::standing_refusals(&caps::run_records(&entries), now_ms).iter().map(usage::StandingRefusal::to_skipped_model).collect()
}

/// What every client can do now and what it has spent; it settles the reservations whose runs died first.
fn usage_command(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let input = usage::read_usage_input(&Value::Obj(input.clone()))?;
    let config = load_config(runtime)?;
    let now = runtime.clock.now_ms();
    settle_stale(runtime, now)?;
    let readings = read_clients(runtime, &config);
    let entries = (LedgerStore { runtime }).read()?;
    Ok(usage::usage_report(&input, &config, &readings, &entries, now))
}

/// What the gateway finds out about each client without sending a prompt: PATH, and the client's own status command.
fn read_clients(runtime: &Runtime, config: &GatewayConfig) -> Vec<ClientReading> {
    availability::read_clients(runtime, config.clients.iter())
}

/// A field a grant must name, refused when it is missing or blank.
fn require_text(value: Option<&Value>, name: &str) -> Result<String, GatewayError> {
    match value {
        Some(Value::Str(text)) if !text.trim().is_empty() => Ok(text.clone()),
        _ => Err(GatewayError::invalid_input(format!("a grant must name its {name}"))),
    }
}

/// Writes the grant the owner gave: refuses a call from inside a delegation, a bad input, a client that is
/// not metered-employer, a workflow that ended, a ledger it cannot read and a live grant already standing.
fn grant_write(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    if runtime.env.contains_key(DELEGATION_MARK) {
        return Err(GatewayError::new(
            "nested-delegation",
            "a delegate may not write a grant: only the agent the owner works in may, after asking the owner",
        ));
    }
    let cap_usd = match input.get("cap_usd") {
        Some(Value::Num(cap)) if cap.is_finite() && *cap > 0.0 => *cap,
        _ => return Err(GatewayError::invalid_input("cap_usd must be a number above 0")),
    };
    let max_starts = match input.get("max_starts") {
        None => None,
        Some(value) => Some(grants::start_count(value).ok_or_else(|| GatewayError::invalid_input("max_starts must be a whole number above 0"))?),
    };
    let answer = require_text(input.get("answer"), "answer")?;
    let client_name = require_text(input.get("client"), "client")?;
    let question = require_text(input.get("question"), "question")?;
    let workflow = workflows::require_name(input.get("workflow"), "a grant must name its workflow")?;
    let config = load_config(runtime)?;
    let Some(client) = config.clients.get(&client_name) else {
        return Err(GatewayError::new("unknown-model", format!("the gateway knows no client {client_name}")));
    };
    if client.cost_class != crate::config::CostClass::MeteredEmployer {
        return Err(GatewayError::invalid_input(format!(
            "{client_name} is {}: a grant is for a metered-employer client, which is the one that spends someone else's money",
            client.cost_class.as_str()
        )));
    }
    let now = runtime.clock.now_ms();
    let record = workflows::require_open(&(Workflows { runtime }).read()?, &workflow, now)?;
    let entries = read_ledger_for_grant(runtime, now)?;
    let id = grants::grant_id(now, &random_suffix());
    let grant = grants::new_grant(&NewGrant {
        id: &id,
        workflow: &workflow,
        client: &client_name,
        cap_usd,
        max_starts,
        question: &question,
        answer: &answer,
        written_at: now,
        workflow_record: record.as_ref(),
    })?;
    // The duplicate check runs inside the grants lock, so two concurrent writes cannot both pass it.
    let written = (GrantsStore { runtime }).update(|existing| {
        if let Some(standing) = grants::live_grant(existing, &workflow, &client_name, &entries, record.as_ref(), now) {
            let room = grants::grant_room(&entries, &standing);
            return Err(GatewayError::new(
                "grant-exists",
                format!(
                    "workflow {workflow} already has live grant {} of {} with {} left: the owner re-approves only after it lapsed",
                    standing.id,
                    caps::usd(standing.cap_usd),
                    caps::usd(room.remaining_usd)
                ),
            ));
        }
        let mut all = existing.to_vec();
        all.push(grant.clone());
        Ok(GrantChange { grants: all, grant: grant.clone() })
    })?;
    Ok(grant_report(runtime, &written))
}

/// The ledger, after the reservations whose runs died are settled when the ledger can be written. A ledger that
/// cannot be read refuses the write: a standing grant cannot be ruled out, and a second grant carries a fresh full cap.
fn read_ledger_for_grant(runtime: &Runtime, now_ms: i64) -> Result<Vec<LedgerEntry>, GatewayError> {
    // Settling writes the ledger: a ledger that reads fine but cannot be written is no reason to refuse.
    let _ = settle_stale(runtime, now_ms);
    (LedgerStore { runtime }).read().map_err(|error| {
        let path = locations::ledger_path(&runtime.env).unwrap_or_default();
        GatewayError::new(
            "ledger-unreadable",
            format!(
                "{path}: the ledger cannot be read, so a standing grant cannot be ruled out and no grant is written: {}",
                error.message
            ),
        )
    })
}

/// The grant it wrote, with what it has left. The write has already happened, so a ledger that cannot be read
/// is a warning rather than a failure: an agent that saw an error would ask the owner again and write a second grant.
fn grant_report(runtime: &Runtime, grant: &grants::Grant) -> Value {
    let mut object = Object::new();
    match (LedgerStore { runtime }).read() {
        Ok(entries) => {
            let room = grants::grant_room(&entries, grant);
            let view = grants::GrantView {
                grant: grant.clone(),
                spent_usd: room.spent_usd,
                reserved_usd: room.reserved_usd,
                remaining_usd: room.remaining_usd,
            };
            object.set("grant", grants::grant_view_json(&view));
            object.set("status", Value::str("ok"));
        }
        Err(error) => {
            let mut with_room = grants::grant_members(grant);
            with_room.set("room", Value::Null);
            object.set("grant", Value::Obj(with_room));
            object.set("status", Value::str("ok"));
            object.set(
                "warning",
                Value::Str(format!(
                    "the grant is written, but the ledger could not be read, so what it has left is unknown: {}",
                    error.message
                )),
            );
        }
    }
    Value::Obj(object)
}

/// Six lowercase hex digits (three random bytes) that keep two grants of one second apart.
pub fn random_suffix() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    format!("{:06x}", RandomState::new().build_hasher().finish() & 0x00ff_ffff)
}

/// Lists every grant this machine holds: the live ones with what each has left, and the lapsed ones with why.
fn grant_list(_input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let now = runtime.clock.now_ms();
    settle_stale(runtime, now)?;
    let grants = (GrantsStore { runtime }).read()?;
    let entries = (LedgerStore { runtime }).read()?;
    let records = (Workflows { runtime }).read()?;
    let (live, lapsed) = grants::grant_views(&grants, &entries, &records, now);
    let mut object = Object::new();
    object.set("grants", Value::Arr(live.iter().map(grants::grant_view_json).collect()));
    object.set("lapsed", Value::Arr(lapsed.iter().map(grants::lapsed_grant_json).collect()));
    object.set("status", Value::str("ok"));
    Ok(Value::Obj(object))
}
