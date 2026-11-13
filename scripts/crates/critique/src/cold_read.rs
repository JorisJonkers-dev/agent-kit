//! `cold-read`: the cold read run after `/critique-adrs` rewrote records and check-rewrite confirmed the decisions
//! survived. Each rewritten record goes alone into a one-file packet, and the `cold-reader` role answers what was
//! decided, what was rejected and why, and when to revisit. The command reports the answers without judging them; a
//! client that may not run, fails, or gives no readable answer leaves its record `unavailable`, never a pass. A run
//! is cold-read once. The node runner's `application/cold-read.ts` and `domain/cold-read.ts`.

use crate::assets::read_asset;
use crate::clients::{client_readiness, ModelAnswer, ModelQuestion};
use crate::critic_tools::{tools_section, READ_ONLY_TOOLSET};
use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::flow_budget::{FlowBudget, GatewayClient, InvocationContext, LedgerClient, LedgerRun};
use crate::locations::{cold_read_path, manifest_path};
use crate::manifest::{parse_manifest, spec_reference, RunManifest};
use crate::paths::inside_repository;
use crate::plan::plan_limit;
use crate::routing::{ColdReadRouting, CriticClient};
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use crate::vibe_client::vibe_ask;
use model_gateway::json::{self, Object, Value};
use crate::paths::join;
use model_gateway::text::{basename, js_trim};

/// The role the cold reader runs, under the shipped assets.
const COLD_READER_ROLE: &str = "roles/cold-reader.md";

/// The three questions the cold reader answers, in the order the role asks them.
pub const COLD_READ_ANSWERS: [&str; 3] = ["decided", "rejected", "revisit"];

/// The answers to the three questions, in the order the role asks them.
pub type ColdReadAnswers = [String; 3];

/// What opens a fenced `json` block.
const OPEN: &str = "```json";

/// The text of the last fenced `json` block of a reply, found as ``/```json[ \t]*\r?\n([\s\S]*?)\r?\n?```/g`` finds it.
fn last_fenced_json(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut from = 0;
    let mut last = None;
    while let Some(found) = text[from..].find(OPEN) {
        let at = from + found;
        let mut start = at + OPEN.len();
        while matches!(bytes.get(start), Some(b' ' | b'\t')) {
            start += 1;
        }
        if bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if bytes.get(start) != Some(&b'\n') {
            from = at + 1;
            continue;
        }
        start += 1;
        match closing_fence(text, start) {
            Some((end, after)) => {
                last = Some(&text[start..end]);
                from = after;
            }
            None => from = at + 1,
        }
    }
    last
}

/// The lazy body's end: the first position from `start` after which `\r?\n?` and a closing fence follow, with the
/// position after the fence. Greedy options are tried first, as the engine backtracks.
fn closing_fence(text: &str, start: usize) -> Option<(usize, usize)> {
    (start..=text.len()).filter(|end| text.is_char_boundary(*end)).find_map(|end| {
        let rest = &text[end..];
        let r = usize::from(rest.starts_with('\r'));
        let n_after_r = usize::from(rest[r..].starts_with('\n'));
        let n_without_r = usize::from(rest.starts_with('\n'));
        [r + n_after_r, r, n_without_r, 0].into_iter().find(|skipped| rest[*skipped..].starts_with("```")).map(|skipped| (end, end + skipped + 3))
    })
}

/// Reads the last ```json block of the cold reader's reply. `None` when the reply holds no such block, the block is not
/// JSON, or a value is not a string: the reader gave no answer the runner can read. A missing key and an empty string
/// are both an empty answer, which is an omission.
pub fn parse_cold_read_reply(text: &str) -> Option<ColdReadAnswers> {
    let value = json::parse(last_fenced_json(text)?).ok()?;
    let object = value.as_object()?;
    let mut answers = [String::new(), String::new(), String::new()];
    for (slot, name) in answers.iter_mut().zip(COLD_READ_ANSWERS) {
        match object.get(name) {
            None => {}
            Some(Value::Str(answer)) => *slot = js_trim(answer).to_string(),
            Some(_) => return None,
        }
    }
    Some(answers)
}

/// The answers left empty, in the order the role asks them.
pub fn missing_answers(answers: &ColdReadAnswers) -> Vec<&'static str> {
    COLD_READ_ANSWERS.iter().zip(answers).filter(|(_, answer)| answer.is_empty()).map(|(name, _)| *name).collect()
}

fn names(names: &[&str]) -> Value {
    Value::Arr(names.iter().map(|name| Value::str(name)).collect())
}

/// One record the reader answered.
pub fn answered_file(path: &str, answers: &ColdReadAnswers) -> Value {
    let mut said = Object::new();
    for (name, answer) in COLD_READ_ANSWERS.iter().zip(answers) {
        said.set(name, Value::str(answer));
    }
    let mut file = Object::new();
    file.set("answers", Value::Obj(said));
    file.set("missing", names(&missing_answers(answers)));
    file.set("path", Value::str(path));
    file.set("status", Value::str("answered"));
    Value::Obj(file)
}

/// One record the reader could not read: every answer is missing, and `reason` says why.
pub fn unavailable_file(path: &str, reason: &str) -> Value {
    let mut file = Object::new();
    file.set("missing", names(&COLD_READ_ANSWERS));
    file.set("path", Value::str(path));
    file.set("reason", Value::str(reason));
    file.set("status", Value::str("unavailable"));
    Value::Obj(file)
}

struct Input {
    files: Vec<String>,
    run_dir: String,
}

fn read_input(raw: &Value) -> Result<Input, ShapeError> {
    let fields = Fields::of(raw, "")?;
    Ok(Input { files: fields.strings("files")?, run_dir: fields.string("run_dir")? })
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input(raw).map_err(|error| error.into_error("invalid-input", "cold-read input"))?;
    if input.files.is_empty() {
        return Err(CritiqueError::new("invalid-input", "cold-read input: files names no record"));
    }
    let manifest = read_manifest(runtime, &input.run_dir)?;
    let path = cold_read_path(&manifest.run_dir);
    if runtime.files.exists(&path).unwrap_or(false) {
        return Err(CritiqueError::new("already-cold-read", format!("run {} was already cold-read", manifest.run_id)));
    }
    let Some(cold_read) = load_routing(runtime)?.cold_read else {
        return Err(CritiqueError::new("invalid-routing", "routing has no cold_read, so a rewrite cannot be cold-read"));
    };
    let blocked = blocked(runtime, &manifest, &cold_read)?;
    // One launch at a time, in the order the caller named the records, so the ledger reads in that order. Every record
    // is read before the first launch, so a bad path costs nothing and a retry re-charges nothing.
    let texts = working_tree_texts(runtime, &manifest, &input.files)?;
    let mut files = Vec::new();
    let mut ledger_warnings = Vec::new();
    for (index, (file, text)) in input.files.iter().zip(&texts).enumerate() {
        let record = Record { path: file, text, n: index + 1 };
        files.push(read(runtime, &manifest, &cold_read, &record, blocked.as_deref(), &mut ledger_warnings)?);
    }
    let mut result = Object::new();
    result.set("files", Value::Arr(files));
    if !ledger_warnings.is_empty() {
        result.set("ledger_warnings", Value::Arr(ledger_warnings.iter().map(|warning| Value::str(warning)).collect()));
    }
    result.set("run_id", Value::str(&manifest.run_id));
    result.set("status", Value::str("checked"));
    let result = Value::Obj(result);
    runtime.files.write_text(&path, &format!("{}\n", json::pretty(&result))).map_err(CritiqueError::from_fs)?;
    Ok(result)
}

/// The run's manifest; a missing or malformed one is an `invalid-run`.
pub fn read_manifest(runtime: &Runtime, run_dir: &str) -> Result<RunManifest, CritiqueError> {
    let path = manifest_path(run_dir);
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Err(CritiqueError::new("invalid-run", format!("{path} does not exist")));
    }
    let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
    parse_manifest(&text).map_err(|message| CritiqueError::new("invalid-run", format!("{path}: {message}")))
}

/// One rewritten record: the path the caller named, its text, and its place in the order (from 1).
struct Record<'a> {
    path: &'a str,
    text: &'a str,
    n: usize,
}

/// Cold-reads one record: its own one-file packet, the reader's prompt beside it, and one launch, logged whether or not
/// its answer is readable.
fn read(
    runtime: &Runtime,
    manifest: &RunManifest,
    cold_read: &ColdReadRouting,
    record: &Record<'_>,
    blocked: Option<&str>,
    ledger_warnings: &mut Vec<String>,
) -> Result<Value, CritiqueError> {
    let Record { path, text, n } = *record;
    if let Some(reason) = blocked {
        return Ok(unavailable_file(path, reason));
    }
    let dir = join(&[&manifest.run_dir, "cold-read", &n.to_string()]);
    let packet_dir = join(&[&dir, "packet"]);
    let packet_path = join(&["artifact", basename(path)]);
    let write = |place: &str, content: &str| runtime.files.write_text(place, content).map_err(CritiqueError::from_fs);
    write(&join(&[&packet_dir, &packet_path]), text)?;
    let prompt_path = join(&[&dir, "prompt.md"]);
    write(&prompt_path, &prompt(runtime, &packet_path)?)?;
    let started = runtime.clock.monotonic_ms();
    let question = ModelQuestion {
        max_price_usd: cold_read.max_price_usd,
        output_dir: dir.clone(),
        prompt_path,
        time_limit_s: cold_read.time_limit_s,
        work_dir: packet_dir,
    };
    let report = match vibe_ask(runtime, &question, &cold_read.client_config) {
        Ok(report) => report,
        // A launch that fails must not lose the records already read, or a retry would charge them again.
        Err(error) => return Ok(unavailable_file(path, &format!("the launch failed: {}", error.message))),
    };
    let duration_ms = runtime.clock.monotonic_ms() - started;
    // A launch with no readable answer is a failed launch in the ledger, as for any critic.
    let (answers, cost_usd, failure) = match &report.answer {
        ModelAnswer::Answered { text, cost_usd } => (parse_cold_read_reply(text), *cost_usd, None),
        ModelAnswer::Failed { failure, detail, cost_usd } => (None, *cost_usd, Some(format!("{failure}: {detail}"))),
    };
    write(&join(&[&dir, "stdout.json"]), &report.stdout)?;
    write(&join(&[&dir, "stderr.txt"]), &report.stderr)?;
    let config = &cold_read.client_config;
    let context = InvocationContext {
        flow: manifest.flow.clone(),
        repository: manifest.repository.clone(),
        round: manifest.round,
        run_id: manifest.run_id.clone(),
        stage: manifest.stage.clone(),
    };
    let run = LedgerRun {
        charged_usd: report.charged_usd,
        client: cold_read.client.clone(),
        cost_usd,
        critic_id: format!("cold-read-{n}"),
        duration_ms: Some(duration_ms),
        exit_code: Some(f64::from(report.exit_code)),
        failure: None,
        family: config.family.clone(),
        finding_count: None,
        lens_hashes: None,
        lenses: None,
        model: config.model.clone(),
        prompt_hashes: None,
        retry_of: None,
        role: Some("cold-reader".to_string()),
        status: if answers.is_none() { "failed" } else { "ok" }.to_string(),
        tokens: None,
        vibe_cost_usd: Some(report.vibe_cost_usd),
    };
    let client = LedgerClient {
        kind: "vibe".to_string(),
        gateway: config.gateway.as_ref().map(|gateway| GatewayClient { client: gateway.client.clone(), model: gateway.model.clone() }),
    };
    if let Some(warning) = FlowBudget::new(runtime).log_invocation(&context, &run, &client, Some(&spec_reference(manifest)))? {
        ledger_warnings.push(warning);
    }
    Ok(match (failure, answers) {
        (Some(failure), _) => unavailable_file(path, &failure),
        (None, None) => unavailable_file(path, "the cold reader gave no readable answer"),
        (None, Some(answers)) => answered_file(path, &answers),
    })
}

/// Why the cold reader may not run now, or `None` while it may: the repository's allowlist does not permit its family,
/// Vibe is at its plan limit, or the client is not ready on this machine. Read once, before the records, so a client
/// that cannot run costs nothing.
fn blocked(runtime: &Runtime, manifest: &RunManifest, cold_read: &ColdReadRouting) -> Result<Option<String>, CritiqueError> {
    let config = &cold_read.client_config;
    if !manifest.allowed_families.contains(&config.family) {
        return Ok(Some(format!("the allowlist does not permit the {} family", config.family)));
    }
    if let Some(reason) = plan_limit(runtime, "vibe")?.unable_to_start {
        return Ok(Some(reason));
    }
    let ready = client_readiness(runtime, &CriticClient::Vibe(config.clone()))?;
    Ok(if ready.ready { None } else { ready.detail })
}

/// The rewritten records as they sit in the working tree; one missing there is input the command cannot read. Every
/// path is placed in the repository before any file is looked at, as the node runner's concurrent reads do.
fn working_tree_texts(runtime: &Runtime, manifest: &RunManifest, files: &[String]) -> Result<Vec<String>, CritiqueError> {
    let mut places = Vec::new();
    for file in files {
        places.push(join(&[&manifest.repo_root, &inside_repository(&manifest.repo_root, file)?]));
    }
    // Every look at whether a file is there settles before any read does, so a missing file is reported before a
    // file that cannot be read, whatever their order.
    for (file, full) in files.iter().zip(&places) {
        if !runtime.files.exists(full).unwrap_or(false) {
            return Err(CritiqueError::new("invalid-input", format!("cold-read input: {file} does not exist in the repository")));
        }
    }
    places.iter().map(|full| runtime.files.read_text(full).map_err(CritiqueError::from_fs)).collect()
}

/// The reader's prompt: its role, the one tool it has, and the one file it reads.
fn prompt(runtime: &Runtime, packet_path: &str) -> Result<String, CritiqueError> {
    let role = read_asset(runtime, COLD_READER_ROLE)?;
    let files = [
        "# Files".to_string(),
        String::new(),
        "Your working directory holds one file and no others. Read it with `read_file`, using the path exactly as written here:".to_string(),
        String::new(),
        format!("- `{packet_path}`: the decision record you answer about"),
    ]
    .join("\n");
    Ok(format!("{}\n", [js_trim(&role).to_string(), tools_section(&READ_ONLY_TOOLSET), files].join("\n\n")))
}
