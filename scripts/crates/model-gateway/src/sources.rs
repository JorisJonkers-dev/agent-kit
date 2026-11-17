//! Each client's own model listing, and the Vibe probe with its cache.
//!
//! A client that answers for itself is asked, never guessed at: Antigravity and
//! OpenCode print their own listings, Hermes' configuration names its default
//! model, and Vibe is probed with a model id it does not know, whose refusal
//! names every alias it accepts. A listing that fails leaves the client out with
//! the reason rather than taking the whole listing down.

use crate::json::{self, Object, Value};
use crate::locations::{pi_models_path, vibe_available_cache_path, vibe_config_path};
use crate::config::{CostClass, GatewayConfig};
use crate::litellm_catalogue::LiteLlmCatalogue;
use crate::models::{
    hermes_default_model, is_vibe_model_id, parse_antigravity_models, parse_opencode_free_models, parse_opencode_models, parse_vibe_available,
    vibe_models, ClientListing, ClientModel, ListedModel, ModelListing, SkippedModel,
};
use crate::ports::{ProcessResult, Runtime};

/// Antigravity's own listing, as its reasons name it.
const AGY_COMMAND: &str = "agy models";

/// OpenCode's own listing, as its reasons name it.
const OPENCODE_COMMAND: &str = "opencode models";

/// OpenCode's server model list with prices, as its reasons name it: the source of `opencode-free`'s models.
const OPENCODE_FREE_COMMAND: &str = "opencode api GET /api/model";

/// What to check when OpenCode lists none of its own free models.
const OPENCODE_FREE_HINT: &str = "OpenCode's models catalogue may have failed to load (look for `failed to load plugin plugin.id=opencode.models.dev` in ~/.local/share/opencode/log/opencode.log), or OPENCODE_DISABLE_FREE_MODELS is set";

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
        crate::run::OPENCODE_FREE_CLIENT => Some(opencode_free_listing(runtime)),
        "vibe" => Some(vibe_listing(runtime)),
        client if crate::run::is_pi_client(client) => Some(pi_listing(runtime, client)),
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

/// OpenCode's own free models, from its server's model list: only models of OpenCode's own provider priced at zero.
/// A list with none of them names what to check, since OpenCode serves free models without any sign-in.
fn opencode_free_listing(runtime: &Runtime) -> ClientListing {
    let result = match runtime.process.exec("opencode", &["api", "GET", "/api/model"], None) {
        Ok(result) => result,
        Err(message) => return ClientListing::Reason(format!("listing failed: {OPENCODE_FREE_COMMAND}: {message}")),
    };
    if result.exit_code != 0 {
        return ClientListing::Reason(failure_reason(OPENCODE_FREE_COMMAND, &result));
    }
    match parse_opencode_free_models(&result.stdout) {
        Ok(models) if models.is_empty() => ClientListing::Reason(format!("{OPENCODE_FREE_COMMAND} names no free OpenCode model: {OPENCODE_FREE_HINT}")),
        Ok(models) => ClientListing::Models(models),
        Err(reason) => ClientListing::Reason(format!("listing failed: {OPENCODE_FREE_COMMAND}: {reason}")),
    }
}

/// Whether OpenCode lists the model as one of its own free models now. Asked at run time too, so a model that
/// stopped being free, or a paid `opencode/<id>`, is refused before it runs.
pub fn opencode_model_is_free(runtime: &Runtime, model: &str) -> bool {
    match opencode_free_listing(runtime) {
        ClientListing::Models(models) => models.iter().any(|listed| listed.model == model),
        ClientListing::Reason(_) => false,
    }
}

/// Pi's own configuration: setup writes one `litellm-<route>` provider per
/// LiteLLM route into the home's `models.json`, with the canonical models the
/// live catalogue says that route's key reaches, so the file is the one
/// non-interactive source of what a Pi client on that route can run. The
/// client id names the route (`pi-vibe` is `litellm-vibe`); a file that is
/// missing, is not JSON or has no such provider leaves the client out with the
/// reason, rather than offering a model a run would be refused on.
fn pi_listing(runtime: &Runtime, client: &str) -> ClientListing {
    let provider = crate::run::pi_provider(client);
    let path = match pi_models_path(&runtime.env) {
        Ok(path) => path,
        Err(error) => return ClientListing::Reason(format!("listing failed: {}", error.message)),
    };
    match runtime.files.read_text(&path) {
        Ok(text) => parse_pi_listing(&text, &provider, &path),
        Err(error) => ClientListing::Reason(format!(
            "listing failed: {path} cannot be read ({}): re-run agent-kit setup, which writes the {provider} provider",
            error.message
        )),
    }
}

/// The models a Pi `models.json` gives one provider, in the order it lists them.
pub fn parse_pi_listing(text: &str, provider: &str, path: &str) -> ClientListing {
    let Ok(parsed) = json::parse(text) else {
        return ClientListing::Reason(format!("listing failed: {path} is not JSON"));
    };
    let providers = parsed.as_object().and_then(|object| object.get("providers")).and_then(Value::as_object);
    let models = providers.and_then(|providers| providers.get(provider)).and_then(Value::as_object).and_then(|entry| entry.get("models"));
    let Some(Value::Arr(models)) = models else {
        // Setup writes a provider only for a route that has a usable model (the local route has none until a local model
        // is probed), so where it wrote the others, this one is absent on purpose and running setup again changes nothing.
        let written = providers.is_some_and(|providers| providers.entries().iter().any(|(name, _)| *name != provider && name.starts_with("litellm-")));
        let route = provider.strip_prefix("litellm-").unwrap_or(provider);
        return ClientListing::Reason(if written {
            format!("listing failed: {path} has no {provider} provider: setup writes one only for a route with a usable model, and the {route} route has none")
        } else {
            format!("listing failed: {path} has no {provider} provider with models: re-run agent-kit setup, which writes it")
        });
    };
    ClientListing::Models(
        models
            .iter()
            .filter_map(|model| model.as_object().and_then(|model| model.get("id")).and_then(Value::as_str))
            .filter(|id| !id.is_empty())
            .map(|id| ClientModel { model: id.to_string(), names: Vec::new(), provider: None })
            .collect(),
    )
}

/// Whether the Pi provider's entry for a model prices every field at zero: a `cost` object with at least one
/// field, each a number equal to 0. A missing, partial or non-numeric `cost` is not free.
pub fn pi_entry_is_free(text: &str, provider: &str, model: &str) -> bool {
    let Ok(parsed) = json::parse(text) else {
        return false;
    };
    let models = parsed.as_object().and_then(|object| object.get("providers")).and_then(Value::as_object).and_then(|providers| providers.get(provider)).and_then(Value::as_object).and_then(|entry| entry.get("models"));
    let Some(Value::Arr(models)) = models else {
        return false;
    };
    let entry = models.iter().filter_map(Value::as_object).find(|entry| entry.get("id").and_then(Value::as_str) == Some(model));
    let Some(cost) = entry.and_then(|entry| entry.get("cost")).and_then(Value::as_object) else {
        return false;
    };
    let fields: Vec<&Value> = cost.entries().into_iter().map(|(_, value)| value).collect();
    !fields.is_empty() && fields.iter().all(|value| matches!(value, Value::Num(number) if *number == 0.0))
}

/// Whether a Pi client's model is free on this machine, read from the home's `models.json`; `false` for a client
/// that is not Pi's or a file that cannot be read.
pub fn pi_model_is_free(runtime: &Runtime, client: &str, model: &str) -> bool {
    if !crate::run::is_pi_client(client) {
        return false;
    }
    let Ok(path) = pi_models_path(&runtime.env) else {
        return false;
    };
    runtime.files.read_text(&path).is_ok_and(|text| pi_entry_is_free(&text, &crate::run::pi_provider(client), model))
}

/// The listing with every free model marked and, when it is on a subsidised client, moved to the `free` class; any
/// other class is kept. Free is `litellm_catalogue::is_free`: the catalogue's flag, else Pi's zero price.
pub fn mark_free(runtime: &Runtime, config: &GatewayConfig, catalogue: Option<&LiteLlmCatalogue>, listing: ModelListing) -> ModelListing {
    let models = listing
        .models
        .into_iter()
        .map(|model| {
            let free = crate::litellm_catalogue::is_free(runtime, config, catalogue, &model.client, &model.model);
            let cost_class = if free && model.cost_class == CostClass::Subsidised { CostClass::Free } else { model.cost_class };
            ListedModel { free, cost_class, ..model }
        })
        .collect();
    ModelListing { models, ..listing }
}

/// Why a model of the OpenRouter client that is not free is never offered.
pub const OPENROUTER_BILLED: &str = "this client runs models priced at zero only (OpenRouter is allowed for no other), and the catalogue does not mark this one free";

/// The listing with every model of a free-only client (`pi-openrouter`, `pi-free`) that is not free moved to `skipped`. Call it after `mark_free`.
pub fn hold_openrouter_to_free(listing: ModelListing) -> ModelListing {
    let (kept, billed): (Vec<ListedModel>, Vec<ListedModel>) =
        listing.models.into_iter().partition(|model| !crate::run::FREE_ONLY_CLIENTS.contains(&model.client.as_str()) || model.free);
    let mut skipped = listing.skipped;
    skipped.extend(billed.into_iter().map(|model| SkippedModel { client: model.client, model: model.model, reason: OPENROUTER_BILLED.to_string() }));
    skipped.sort_by(|left, right| left.client.cmp(&right.client).then_with(|| left.model.cmp(&right.model)));
    ModelListing { models: kept, skipped, ..listing }
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
    if !(0.0..PROBE_CACHE_TTL_MS).contains(&age) {
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
