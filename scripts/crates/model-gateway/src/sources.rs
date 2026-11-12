//! Each client's own model listing, and the Vibe probe with its cache.
//!
//! A client that answers for itself is asked, never guessed at: Antigravity and
//! OpenCode print their own listings, Hermes' configuration names its default
//! model, and Vibe is probed with a model id it does not know, whose refusal
//! names every alias it accepts. A listing that fails leaves the client out with
//! the reason rather than taking the whole listing down.

use crate::json::{self, Object, Value};
use crate::locations::{vibe_available_cache_path, vibe_config_path};
use crate::models::{
    hermes_default_model, is_vibe_model_id, parse_antigravity_models, parse_opencode_models, parse_vibe_available,
    vibe_models, ClientListing, ClientModel,
};
use crate::ports::{ProcessResult, Runtime};

/// Antigravity's own listing, as its reasons name it.
const AGY_COMMAND: &str = "agy models";

/// OpenCode's own listing, as its reasons name it.
const OPENCODE_COMMAND: &str = "opencode models";

/// Hermes' own configuration reader, as its reasons name it.
const HERMES_COMMAND: &str = "hermes config get model --json";

/// The model id the gateway probes Vibe with: reserved for the gateway, never a
/// model, so Vibe refuses it before it makes any request and its refusal answers
/// with the aliases it accepts.
const VIBE_PROBE_MODEL: &str = "model-gateway-probe";

/// How long the probe waits for Vibe's refusal, which arrives within seconds.
const PROBE_TIMEOUT_MS: u64 = 30_000;

/// The probe's own command, probed with the strictest caps Vibe takes, so even a
/// Vibe that somehow accepted the probe id as a model could buy nothing with it:
/// one turn, one token, no price. The outer timeout already stops a Vibe that
/// never answers, so the probe takes no `--time-limit` of its own — a small one
/// races the refusal the answer is read from.
const PROBE_ARGS: [&str; 10] =
    ["--model", VIBE_PROBE_MODEL, "--max-turns", "1", "--max-tokens", "1", "--max-price", "0", "-p", "ping"];

/// The probe as its reasons name it.
const PROBE_COMMAND: &str = "vibe --model model-gateway-probe --max-turns 1 --max-tokens 1 --max-price 0 -p ping";

/// How long one probe's answer holds, so the `models` calls inside it cost nothing.
const PROBE_CACHE_TTL_MS: f64 = 600_000.0;

/// The one cache file shape the gateway reads; a later one re-probes.
const CACHE_VERSION: f64 = 2.0;

/// The models a client lists for itself, or why it could not; `None` when the
/// gateway has no source for that client id.
pub fn list_client(runtime: &Runtime, id: &str) -> Option<ClientListing> {
    match id {
        "agy" => Some(antigravity_listing(runtime)),
        "hermes" => Some(hermes_listing(runtime)),
        "opencode" => Some(opencode_listing(runtime)),
        "vibe" => Some(vibe_listing(runtime)),
        _ => None,
    }
}

/// Antigravity's own listing: `agy models` prints a `Fetching available models...`
/// line, then one `<id>\t<display name>` per model. The gateway keeps the ids;
/// which families it may offer is the domain's decision.
fn antigravity_listing(runtime: &Runtime) -> ClientListing {
    let result = match runtime.process.exec("agy", &["models"], None) {
        Ok(result) => result,
        Err(message) => return ClientListing::Reason(format!("listing failed: {AGY_COMMAND}: {message}")),
    };
    if result.exit_code != 0 {
        return ClientListing::Reason(failure_reason(AGY_COMMAND, &result));
    }
    ClientListing::Models(
        parse_antigravity_models(&result.stdout)
            .into_iter()
            .map(|model| ClientModel { model, names: Vec::new(), provider: None })
            .collect(),
    )
}

/// OpenCode's own listing: `opencode models` prints one `<provider>/<model>` per
/// line, and the provider is what its cost class is set by.
fn opencode_listing(runtime: &Runtime) -> ClientListing {
    let result = match runtime.process.exec("opencode", &["models"], None) {
        Ok(result) => result,
        Err(message) => return ClientListing::Reason(format!("listing failed: {OPENCODE_COMMAND}: {message}")),
    };
    if result.exit_code != 0 {
        return ClientListing::Reason(failure_reason(OPENCODE_COMMAND, &result));
    }
    ClientListing::Models(parse_opencode_models(&result.stdout))
}

/// Hermes' own configuration: `hermes model` is an interactive picker, so the
/// only non-interactive source is the configuration itself, read through Hermes'
/// own reader — `hermes config get model --json` prints the default model and the
/// provider it runs on. That names one model, not the many providers Hermes can
/// reach; a listing that fails or names no model leaves Hermes out with the
/// reason rather than taking the whole listing down.
fn hermes_listing(runtime: &Runtime) -> ClientListing {
    let result = match runtime.process.exec("hermes", &["config", "get", "model", "--json"], None) {
        Ok(result) => result,
        Err(message) => return ClientListing::Reason(format!("listing failed: {HERMES_COMMAND}: {message}")),
    };
    if result.exit_code != 0 {
        return ClientListing::Reason(failure_reason(HERMES_COMMAND, &result));
    }
    match hermes_default_model(&result.stdout) {
        Some(model) => ClientListing::Models(vec![model]),
        None => ClientListing::Reason(format!("listing failed: {HERMES_COMMAND} named no model")),
    }
}

/// Vibe's own answer, the only measure of what it accepts: the gateway runs
/// `vibe --model <probe id> -p ping`, which Vibe refuses before any request, and
/// reads the aliases from the `Available:` line of its refusal. Vibe gets those
/// aliases from its built-in catalogue too, so the home's `config.toml` never
/// decides what is offered: it is read only to join each accepted alias to the
/// model name the configuration gives it, which the catalogue may know the model
/// by. A probe that cannot run, times out or says nothing the gateway can read
/// leaves Vibe out with the reason — it offers no model rather than one a run
/// would be refused on.
///
/// A configuration that declares the probe id as a model of its own would let the
/// probe run a model instead of being refused, so it fails closed before anything
/// is launched: the listing is left unreadable with the reason, and no probe runs
/// at all.
///
/// The probe costs a Vibe start every time it runs, so its answer is kept for ten
/// minutes beside the Vibe that gave it: before probing, the gateway looks for an
/// answer the same Vibe binary and the same `config.toml` gave within the window,
/// and only probes when there is none. A probe that failed is never kept, so a bad
/// moment does not stick for ten minutes; an answer that does not read — a corrupt
/// file, a key that no longer matches — is a plain miss, and the probe runs.
fn vibe_listing(runtime: &Runtime) -> ClientListing {
    let declared = declared_models(runtime);
    if let Some(clash) = declared.clash {
        return ClientListing::Reason(clash);
    }
    let key = probe_key(runtime);
    let now = runtime.clock.now_ms();
    if let Some(cached) = read_cache(runtime, &key, now) {
        return listing_of(&cached, &declared.names);
    }
    let result = match runtime.process.exec("vibe", &PROBE_ARGS, Some(PROBE_TIMEOUT_MS)) {
        Ok(result) => result,
        Err(message) => return ClientListing::Reason(format!("listing failed: {PROBE_COMMAND}: {message}")),
    };
    if result.timed_out {
        return ClientListing::Reason(format!(
            "listing failed: {PROBE_COMMAND} did not answer within {}s",
            PROBE_TIMEOUT_MS / 1000
        ));
    }
    let accepted = match parse_vibe_available(&format!("{}\n{}", result.stderr, result.stdout)) {
        Some(accepted) => accepted,
        None => return ClientListing::Reason(failure_reason(PROBE_COMMAND, &result)),
    };
    // Only an answer worth keeping is kept: a failed probe is never cached.
    write_cache(runtime, &key, &accepted, now);
    listing_of(&accepted, &declared.names)
}

/// The accepted aliases as the gateway lists them, each joined to the names the
/// home's configuration gives it.
fn listing_of(accepted: &[String], names: &[(String, Vec<String>)]) -> ClientListing {
    ClientListing::Models(
        accepted
            .iter()
            .map(|alias| ClientModel { model: alias.clone(), names: names_of(names, alias), provider: None })
            .collect(),
    )
}

/// The names the configuration gives an alias; the last declaration of an alias
/// wins, as a map's does.
fn names_of(names: &[(String, Vec<String>)], alias: &str) -> Vec<String> {
    names.iter().rev().find(|(name, _)| name == alias).map(|(_, names)| names.clone()).unwrap_or_default()
}

/// The name the home's configuration gives each alias it declares, and the clash
/// when the configuration declares the probe id itself as a model.
struct Declared {
    names: Vec<(String, Vec<String>)>,
    clash: Option<String>,
}

/// The name the home's configuration gives each alias it declares, which joins an
/// accepted alias to the catalogue entry that knows the model by that name — and
/// the clash, when the configuration declares the probe id itself as a model.
/// Nothing about the configuration can fail the listing — it never decides what is
/// offered — so a home the environment does not name, a file that is not there and
/// one that is not TOML read no names. The one exception is the clash: a
/// configuration that makes the probe id a model has made the probe's refusal
/// impossible to rely on.
fn declared_models(runtime: &Runtime) -> Declared {
    let none = || Declared { names: Vec::new(), clash: None };
    let Ok(path) = vibe_config_path(&runtime.env) else {
        return none();
    };
    if !runtime.files.exists(&path).unwrap_or(false) {
        return none();
    }
    let Ok(text) = runtime.files.read_text(&path) else {
        return none();
    };
    let Ok(declared) = vibe_models(&text) else {
        return none();
    };
    if declared.iter().any(|model| model.model == VIBE_PROBE_MODEL) {
        return Declared {
            names: Vec::new(),
            clash: Some(format!(
                "listing failed: {path} declares the gateway's probe id '{VIBE_PROBE_MODEL}' as a model, so the probe could run it instead of being refused: rename or remove the alias"
            )),
        };
    }
    Declared { names: declared.into_iter().map(|model| (model.model, model.names)).collect(), clash: None }
}

/// What one probe's answer is keyed by: the Vibe that gave it, by the file its
/// name on PATH resolves to — through every symlink to the file itself — with what
/// makes that file the one it is: its change time, its size and its inode; and the
/// home's `config.toml` by its path and the SHA-256 of its content, so a retargeted
/// symlink or a rewrite within the same modification time is still another
/// configuration. A file that is not there, or whose stamp cannot be read, has no
/// key but its own: null, which no stamp equals.
struct VibeProbeKey {
    binary: Option<String>,
    binary_ctime_ms: Option<f64>,
    binary_size: Option<f64>,
    binary_ino: Option<f64>,
    config_path: Option<String>,
    config_sha256: Option<String>,
}

/// The probe's cache key, built from the Vibe it would run and the home's
/// configuration. A machine the environment cannot name a home for has no answer
/// it can keep either: nothing matches, so the probe runs and nothing is written.
fn probe_key(runtime: &Runtime) -> VibeProbeKey {
    let Ok(config_path) = vibe_config_path(&runtime.env) else {
        return VibeProbeKey {
            binary: None,
            binary_ctime_ms: None,
            binary_size: None,
            binary_ino: None,
            config_path: None,
            config_sha256: None,
        };
    };
    let binary = runtime.process.resolve("vibe").and_then(|resolved| runtime.files.stamp(&resolved));
    let config = runtime.files.stamp(&config_path);
    VibeProbeKey {
        binary: binary.as_ref().map(|stamp| stamp.realpath.clone()),
        binary_ctime_ms: binary.as_ref().map(|stamp| stamp.ctime_ms),
        binary_size: binary.as_ref().map(|stamp| stamp.size),
        binary_ino: binary.as_ref().map(|stamp| stamp.ino),
        config_path: Some(config.as_ref().map_or(config_path, |stamp| stamp.realpath.clone())),
        config_sha256: config.as_ref().map(|stamp| stamp.sha256.clone()),
    }
}

/// The answer a previous probe kept, when it is this Vibe's and this
/// configuration's and still inside its window; every failure is a plain miss, so
/// the cache can never make a listing fail.
fn read_cache(runtime: &Runtime, key: &VibeProbeKey, now: i64) -> Option<Vec<String>> {
    let path = vibe_available_cache_path(&runtime.env).ok()?;
    if !runtime.files.exists(&path).unwrap_or(false) {
        return None;
    }
    let text = runtime.files.read_text(&path).ok()?;
    let entry = json::parse(&text).ok()?;
    cache_entry_models(&entry, key, now)
}

/// Whether the entry read is the answer of this Vibe, this configuration, inside
/// its window; the models it holds when it is.
fn cache_entry_models(entry: &Value, key: &VibeProbeKey, now: i64) -> Option<Vec<String>> {
    let record = entry.as_object()?;
    if !matches!(record.get("version"), Some(Value::Num(version)) if *version == CACHE_VERSION) {
        return None;
    }
    if !same_text(record.get("binary"), key.binary.as_deref()) {
        return None;
    }
    if !same_number(record.get("binary_ctime_ms"), key.binary_ctime_ms) {
        return None;
    }
    if !same_number(record.get("binary_size"), key.binary_size) {
        return None;
    }
    if !same_number(record.get("binary_ino"), key.binary_ino) {
        return None;
    }
    if !same_text(record.get("config_path"), key.config_path.as_deref()) {
        return None;
    }
    if !same_text(record.get("config_sha256"), key.config_sha256.as_deref()) {
        return None;
    }
    let written_at = match record.get("written_at_ms") {
        Some(Value::Num(written_at)) if written_at.is_finite() => *written_at,
        _ => return None,
    };
    let age = now as f64 - written_at;
    if !(age >= 0.0 && age < PROBE_CACHE_TTL_MS) {
        return None;
    }
    match record.get("models") {
        Some(Value::Arr(models)) if !models.is_empty() => models
            .iter()
            .map(|model| match model {
                Value::Str(model) if is_vibe_model_id(model) => Some(model.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// Whether a member is the text the key holds, `null` standing for a key part that
/// is not there.
fn same_text(member: Option<&Value>, key: Option<&str>) -> bool {
    match (member, key) {
        (Some(Value::Str(member)), Some(key)) => member == key,
        (Some(Value::Null), None) => true,
        _ => false,
    }
}

/// Whether a member is the number the key holds, `null` standing for a key part
/// that is not there.
fn same_number(member: Option<&Value>, key: Option<f64>) -> bool {
    match (member, key) {
        (Some(Value::Num(member)), Some(key)) => *member == key,
        (Some(Value::Null), None) => true,
        _ => false,
    }
}

/// What one probe's answer looks like on disk: a file only its owner can read,
/// written in one step so a reader never sees half of it. Best effort, because a
/// listing that worked never fails on one that could not be written.
fn write_cache(runtime: &Runtime, key: &VibeProbeKey, models: &[String], now: i64) {
    let Ok(path) = vibe_available_cache_path(&runtime.env) else {
        return;
    };
    let mut entry = Object::new();
    entry.set("binary", text_value(key.binary.as_deref()));
    entry.set("binary_ctime_ms", number_value(key.binary_ctime_ms));
    entry.set("binary_ino", number_value(key.binary_ino));
    entry.set("binary_size", number_value(key.binary_size));
    entry.set("config_path", text_value(key.config_path.as_deref()));
    entry.set("config_sha256", text_value(key.config_sha256.as_deref()));
    entry.set("models", Value::Arr(models.iter().map(|model| Value::str(model)).collect()));
    entry.set("version", Value::Num(CACHE_VERSION));
    entry.set("written_at_ms", Value::Num(now as f64));
    let _ = runtime.files.write_private(&path, &json::compact(&Value::Obj(entry)));
}

fn text_value(text: Option<&str>) -> Value {
    text.map_or(Value::Null, Value::str)
}

fn number_value(number: Option<f64>) -> Value {
    number.map_or(Value::Null, Value::Num)
}

/// Why a listing command the gateway ran gave no models, with the client's own
/// first line of complaint.
fn failure_reason(command: &str, result: &ProcessResult) -> String {
    let detail = result.stderr.trim().split('\n').next().unwrap_or_default();
    let exit = format!("{command} exited {}", result.exit_code);
    if detail.is_empty() {
        format!("listing failed: {exit}")
    } else {
        format!("listing failed: {exit}: {detail}")
    }
}
