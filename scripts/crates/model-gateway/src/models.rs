//! Listing the models a machine can use now. This file's types are shared by
//! `policy`, `sources` and the `models` command; the functions are the port of
//! `domain/models.ts`.

use crate::config::{ClientConfig, CostClass, GatewayConfig, ModelEntry, ModelSource, Tier};
use crate::json::{self, Value};
use crate::toml::read_toml;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// One model the gateway can list now, with where its list came from.
#[derive(Clone, Debug, PartialEq)]
pub struct ListedModel {
    pub client: String,
    pub model: String,
    pub family: String,
    pub cost_class: CostClass,
    /// The highest tier the catalogue says it handles; `None` when the catalogue does not know it.
    pub tier: Option<Tier>,
    pub notes: String,
    pub source: ModelSource,
    /// Set when the LiteLLM catalogue marks the model free on its route.
    pub free: bool,
}

/// A client the gateway knows but cannot use on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingClient {
    pub client: String,
    pub reason: String,
}

/// A model a client listed that the gateway cannot offer, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedModel {
    pub client: String,
    pub model: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelListing {
    pub models: Vec<ListedModel>,
    pub missing: Vec<MissingClient>,
    pub skipped: Vec<SkippedModel>,
}

/// One model a client listed, and the provider it runs on where the client names one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientModel {
    /// The id the client accepts.
    pub model: String,
    /// Other names the client's own listing gives the model.
    pub names: Vec<String>,
    pub provider: Option<String>,
}

/// What a client's own listing gave: the models it offers, or why it could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientListing {
    Models(Vec<ClientModel>),
    Reason(String),
}

/// The key the override's `disabled` list names a model by.
pub fn model_key(client: &str, model: &str) -> String {
    format!("{client}:{model}")
}

/// The model family each prefix names, in the order the gateway reads them.
const FAMILY_PREFIXES: [(&str, &str); 12] = [
    ("gemini", "google"),
    ("claude", "anthropic"),
    ("gpt", "openai"),
    ("deepseek", "deepseek"),
    ("glm", "zhipu"),
    ("zai", "zhipu"),
    ("kimi", "moonshot"),
    ("minimax", "minimax"),
    ("grok", "xai"),
    ("qwen", "alibaba"),
    ("mistral", "mistral"),
    ("codestral", "mistral"),
];

/// The model family a model id names, from its prefix; `None` when the gateway
/// cannot tell, which leaves the model out: the family allowlist decides where a
/// delegation may go, so a model of no known family has no place to go.
pub fn family_of(model: &str) -> Option<&'static str> {
    FAMILY_PREFIXES.iter().find(|(prefix, _)| model.starts_with(prefix)).map(|(_, family)| *family)
}

/// The family of a model id as a client writes it, which may name its provider
/// first (`anthropic/claude-sonnet-4.6`): the family is the model part's, never
/// the provider's.
pub fn family_of_model_id(model: &str) -> Option<&'static str> {
    family_of(model_part(model))
}

/// The model ids an `agy models` listing names, in the order it printed them: a
/// `Fetching available models...` line, then one `<id>\t<display name>` per
/// model. A line that names no model is skipped.
pub fn parse_antigravity_models(stdout: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in stdout.split('\n') {
        let mut fields = line.split('\t');
        let id = fields.next().unwrap_or_default();
        if fields.next().is_none() {
            continue;
        }
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            ids.push(trimmed.to_string());
        }
    }
    ids
}

/// The models an `opencode models` listing names, one `<provider>/<model>` per line.
pub fn parse_opencode_models(stdout: &str) -> Vec<ClientModel> {
    let mut models = Vec::new();
    for line in stdout.split('\n') {
        let id = line.trim();
        let Some(slash) = id.find('/') else {
            continue;
        };
        if slash == 0 || slash == id.len() - 1 {
            continue;
        }
        models.push(ClientModel { model: id.to_string(), names: Vec::new(), provider: Some(id[..slash].to_string()) });
    }
    models
}

/// The model Hermes' own configuration names as its default, from
/// `hermes config get model --json`: an object with `default` and `provider`, or
/// the model id alone. Hermes has no non-interactive model listing — its
/// `hermes model` is an interactive picker — so its configuration is the only
/// reliable source, and it names one model rather than a list.
pub fn hermes_default_model(stdout: &str) -> Option<ClientModel> {
    let parsed = json::parse(stdout).ok()?;
    let object = match &parsed {
        Value::Str(text) => {
            return if text.is_empty() {
                None
            } else {
                Some(ClientModel { model: text.clone(), names: Vec::new(), provider: None })
            }
        }
        Value::Obj(object) => object,
        _ => return None,
    };
    let model = match object.get("default") {
        Some(Value::Str(text)) if !text.is_empty() => text.clone(),
        _ => return None,
    };
    let provider = match object.get("provider") {
        Some(Value::Str(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    };
    Some(ClientModel { model, names: Vec::new(), provider })
}

/// A model id Vibe may name, held to one shape so the listing never acts on text
/// that cannot be one: a letter or digit first, then letters, digits, dots,
/// underscores and hyphens.
pub fn is_vibe_model_id(model: &str) -> bool {
    let mut chars = model.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {
            chars.all(|char| char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-'))
        }
        _ => false,
    }
}

/// The text of an answer without its terminal escapes, so the words, not their
/// colour, are read.
pub fn strip_ansi(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '\u{1b}' {
            if let Some(end) = ansi_end(&chars, at) {
                at = end;
                continue;
            }
        }
        out.push(chars[at]);
        at += 1;
    }
    out
}

/// The index just past the escape sequence that starts at `at`, when one does:
/// `ESC [ params intermediates final`, or `ESC ] ... BEL` / `ESC ] ... ESC \`.
fn ansi_end(chars: &[char], at: usize) -> Option<usize> {
    match chars.get(at + 1) {
        Some('[') => {
            let mut index = at + 2;
            while matches!(chars.get(index), Some(char) if char.is_ascii_digit() || matches!(char, ';' | '?')) {
                index += 1;
            }
            while matches!(chars.get(index), Some(char) if (' '..='/').contains(char)) {
                index += 1;
            }
            match chars.get(index) {
                Some(char) if ('@'..='~').contains(char) => Some(index + 1),
                _ => None,
            }
        }
        Some(']') => {
            let start = at + 2;
            // A title runs to the bell that ends it, else to the last `ESC \` it carries.
            match chars[start..].iter().position(|char| *char == '\u{7}') {
                Some(offset) => Some(start + offset + 1),
                None => {
                    let mut end = None;
                    let mut index = start;
                    while index + 1 < chars.len() {
                        if chars[index] == '\u{1b}' && chars[index + 1] == '\\' {
                            end = Some(index + 2);
                        }
                        index += 1;
                    }
                    end
                }
            }
        }
        _ => None,
    }
}

/// The models Vibe itself accepts, from its answer to a model id it does not
/// know: its refusal ends with `Available: <id>, <id>, ...`, every line it
/// printed is looked at, so the connection warnings it prints first are
/// tolerated, and the ids after `Available:` are kept in the order they stand.
/// The answer is stripped of its terminal escapes first, and the list is read to
/// the end of its line and no further than the sentence that may follow it —
/// the first full stop a whitespace follows — then split on commas, trimmed, and
/// one full stop at the end of an id is dropped, because the refusal may end
/// with a sentence. Every id left must be one id of the only shape Vibe names
/// them in: one that is not — or a refusal that names nothing at all — has said
/// nothing the gateway can safely read, so it reads none and the caller fails
/// closed rather than offering a model a run would be refused on, or text Vibe
/// never named as a model.
pub fn parse_vibe_available(output: &str) -> Option<Vec<String>> {
    let plain = strip_ansi(output);
    let at = plain.rfind("Available:")?;
    let after = plain[at + "Available:".len()..].split('\n').next().unwrap_or_default();
    let ids: Vec<String> = before_sentence(after)
        .split(',')
        .map(|id| {
            let trimmed = id.trim();
            trimmed.strip_suffix('.').unwrap_or(trimmed).to_string()
        })
        .collect();
    if ids.iter().any(|id| !is_vibe_model_id(id)) {
        return None;
    }
    Some(ids)
}

/// The text before the first full stop a whitespace follows.
fn before_sentence(text: &str) -> &str {
    let mut chars = text.char_indices().peekable();
    while let Some((index, char)) = chars.next() {
        if char == '.' && chars.peek().is_some_and(|(_, next)| next.is_whitespace()) {
            return &text[..index];
        }
    }
    text
}

/// The models a Vibe `config.toml` declares, in the order it declares them: the
/// alias is what `vibe --model` accepts, and the model's own name is kept beside
/// it when it differs, because the catalogue may know the model by either. Setup
/// writes the models as an array of tables (`[[models]]`), one entry per alias; a
/// hand-written configuration may key them by table instead (`[models.<alias>]`),
/// whose key stands in when the entry names no alias. An entry that is not a
/// table declares no model, and an array entry that names no alias declares none
/// either. Fails on a file that is not TOML, so a caller that acts on its words
/// never acts on half a file.
pub fn vibe_models(toml: &str) -> Result<Vec<ClientModel>, String> {
    let document = read_toml(toml)?;
    let models = document.as_object().and_then(|object| object.get("models"));
    let mut declared = Vec::new();
    match models {
        Some(Value::Arr(items)) => {
            for value in items {
                declared.extend(declared_model(value, None));
            }
        }
        Some(Value::Obj(table)) => {
            for (key, value) in table.entries() {
                declared.extend(declared_model(value, Some(key)));
            }
        }
        _ => {}
    }
    Ok(declared)
}

/// One `models` entry as the model it declares, when it declares one.
fn declared_model(value: &Value, key: Option<&str>) -> Option<ClientModel> {
    let table = value.as_object()?;
    let alias = match table.get("alias") {
        Some(Value::Str(alias)) if !alias.is_empty() => alias.clone(),
        _ => key?.to_string(),
    };
    let name = match table.get("name") {
        Some(Value::Str(name)) if !name.is_empty() && *name != alias => Some(name.clone()),
        _ => None,
    };
    Some(ClientModel { model: alias, names: name.into_iter().collect(), provider: None })
}

/// Every model the gateway can list now, sorted by client then model id.
///
/// A client whose program is not on PATH is left out and named in `missing`,
/// whatever its model source. A declared client lists its catalogue entries —
/// and one whose own configuration declares models (Hermes) or answers for
/// itself (Antigravity, OpenCode, Vibe) lists what `listings` holds for it,
/// joined to the catalogue. A listing that failed is named in `missing` with its
/// reason and lists NOTHING, Vibe included: the only measure of what a client
/// accepts is its own answer, and offering a catalogue entry the client never
/// named would send a run to a model the client refuses. A client the gateway
/// has no listing for lists nothing and is not reported missing. A catalogue
/// entry of a client with built-in models — Vibe — that its own answer did not
/// name is not offered either: it is reported in `skipped`, with the reason. A
/// client model the gateway remembers its own answer said cannot run
/// (`unavailable`, from the ledger) is not offered either: it is moved to
/// `skipped`, with the reason its answer gave.
pub fn list_models(
    config: &GatewayConfig,
    installed: &BTreeSet<String>,
    listings: &BTreeMap<String, ClientListing>,
    unavailable: &[SkippedModel],
) -> ModelListing {
    let mut models: Vec<ListedModel> = Vec::new();
    let mut missing: Vec<MissingClient> = Vec::new();
    let mut skipped: Vec<SkippedModel> = Vec::new();
    for (id, client) in sorted_clients(config) {
        if !installed.contains(id) {
            missing.push(MissingClient { client: id.clone(), reason: format!("not installed: {} is not on PATH", client.binary) });
            continue;
        }
        if client.model_source == ModelSource::Declared {
            let entries = catalogue_entries(config, id, client, &BTreeSet::new());
            models.extend(entries.into_iter().map(|entry| listed_entry(id, client, entry)));
            continue;
        }
        let Some(listing) = listings.get(id) else {
            continue;
        };
        let listed = match listing {
            ClientListing::Reason(reason) => {
                missing.push(MissingClient { client: id.clone(), reason: reason.clone() });
                continue;
            }
            ClientListing::Models(listed) => listed_models(config, id, client, listed),
        };
        models.extend(listed.models);
        skipped.extend(listed.skipped);
        if BUILT_IN_MODEL_CLIENTS.contains(&id.as_str()) {
            let entries = catalogue_entries(config, id, client, &listed.matched);
            skipped.extend(entries.into_iter().map(|entry| SkippedModel {
                client: id.clone(),
                model: entry.model.clone(),
                reason: NOT_ACCEPTED_BY_CLIENT.to_string(),
            }));
        }
    }
    let mut refused: BTreeMap<String, String> = BTreeMap::new();
    for refusal in unavailable {
        // An answer the ledger recorded under a name the client no longer lists
        // still names the same catalogue entry, whose other ids and names are the
        // same client model, so the refusal hides them all.
        for key in keys_of(config, &refusal.client, &refusal.model) {
            refused.insert(key, refusal.reason.clone());
        }
    }
    let refused_key = |model: &ListedModel| model_key(&model.client, &model.model);
    models.sort_by(|left, right| compare(&left.client, &right.client).then_with(|| compare(&left.model, &right.model)));
    missing.sort_by(|left, right| compare(&left.client, &right.client));
    skipped.sort_by(|left, right| compare(&left.client, &right.client).then_with(|| compare(&left.model, &right.model)));
    let mut all_skipped = skipped;
    all_skipped.extend(models.iter().filter(|model| refused.contains_key(&refused_key(model))).map(|model| SkippedModel {
        client: model.client.clone(),
        model: model.model.clone(),
        reason: refused.get(&refused_key(model)).cloned().unwrap_or_default(),
    }));
    all_skipped.sort_by(|left, right| compare(&left.client, &right.client).then_with(|| compare(&left.model, &right.model)));
    ModelListing {
        models: models.into_iter().filter(|model| !refused.contains_key(&refused_key(model))).collect(),
        missing,
        skipped: all_skipped,
    }
}

/// The clients that offer built-in models of their own, which only the client
/// itself can be asked about: Vibe's answer to a model id it does not know names
/// every alias it accepts, and `config.toml`'s declarations are not a measure of
/// that — the aliases Vibe accepts come from its built-in catalogue too. A
/// matching catalogue entry supplies family, tier and notes; an entry the
/// client's own answer did not name is reported in `skipped`, because a run on
/// it would be refused before it starts. Hermes has no built-ins; its catalogue
/// entries only describe the model its configuration names.
const BUILT_IN_MODEL_CLIENTS: [&str; 1] = ["vibe"];

/// Why a built-in catalogue entry the client's own answer did not name is never offered.
pub const NOT_ACCEPTED_BY_CLIENT: &str = "the client does not accept it, so a run naming it would be refused before it starts: its own answer names the models it accepts and this is not one of them, so the entry only records the model's prices; remove it from defaults.json or change it to the id the client accepts";

/// Why a listed model is not offered: the catalogue gives it no family and tier,
/// and its id names no known family.
pub const FAMILY_UNKNOWN: &str = "no catalogue entry with a family and a tier, and the id names no known family: add one to defaults.json or the machine override";

/// The catalogue entries a client offers, minus everything but them: an entry in
/// `except` is already listed from the client's own source, which wins, so it is
/// not offered twice; one of a family the client may not reach is left out, and
/// so is one the machine disabled under its id or any of its names.
fn catalogue_entries<'a>(
    config: &'a GatewayConfig,
    id: &str,
    client: &ClientConfig,
    except: &BTreeSet<usize>,
) -> Vec<&'a ModelEntry> {
    config
        .models
        .iter()
        .enumerate()
        .filter(|(index, entry)| {
            entry.client == id
                && !except.contains(index)
                && client.families.as_ref().map_or(true, |families| families.contains(&entry.family))
                && !entry.all_names().iter().any(|key| config.disabled.contains(&model_key(id, key)))
        })
        .map(|(_, entry)| entry)
        .collect()
}

/// One catalogue entry as the gateway lists it, marked `declared`.
fn listed_entry(id: &str, client: &ClientConfig, entry: &ModelEntry) -> ListedModel {
    ListedModel {
        client: id.to_string(),
        model: entry.model.clone(),
        family: entry.family.clone(),
        cost_class: client.cost_class,
        tier: Some(entry.tier),
        notes: entry.notes.clone(),
        source: ModelSource::Declared,
        free: false,
    }
}

/// What a client's own listing gave, joined to the catalogue.
struct Listed {
    models: Vec<ListedModel>,
    skipped: Vec<SkippedModel>,
    /// The catalogue entries a listed model joined to, whether or not it was then offered.
    matched: BTreeSet<usize>,
}

/// The models a client listed, joined to the catalogue on the listed id or any
/// name the listing gives it (the catalogue knows a model by its id or by its
/// `names`): a model the catalogue knows takes its family, tier and notes from
/// there, and one it does not is still listed, with its family worked out from
/// its id or names and no tier. A model is listed by the id the client accepts,
/// never by a name. A model a listing names twice is listed once. A model of a
/// family the client may not reach is left out; one whose family the gateway
/// cannot work out is reported in `skipped`, so it never vanishes silently.
/// OpenCode models of a provider the client's `providers` allowlist (else
/// `opencode-go`) does not name, and its pay-per-token ones, are dropped without
/// a trace: no run could reach them.
fn listed_models(config: &GatewayConfig, id: &str, client: &ClientConfig, listed: &[ClientModel]) -> Listed {
    let mut models = Vec::new();
    let mut skipped = Vec::new();
    let mut matched = BTreeSet::new();
    let mut seen: Vec<&str> = Vec::new();
    for client_model in listed {
        let model = client_model.model.as_str();
        if seen.contains(&model) {
            continue;
        }
        seen.push(model);
        let runs_on = if id == "opencode" {
            client_model.provider.clone().or_else(|| open_code_provider(model))
        } else {
            client_model.provider.clone()
        };
        if id == "opencode" && !is_provider_allowed(runs_on.as_deref(), client.providers.as_deref()) {
            continue;
        }
        let known: Vec<&str> = std::iter::once(model).chain(client_model.names.iter().map(String::as_str)).collect();
        let entry = config
            .models
            .iter()
            .enumerate()
            .find(|(_, candidate)| candidate.client == id && candidate.all_names().iter().any(|name| known.contains(name)));
        let family = match entry {
            Some((_, entry)) => Some(entry.family.clone()),
            None => known.iter().filter_map(|name| family_of(model_part(name))).next().map(str::to_string),
        };
        let barred = match (&family, &client.families) {
            (Some(family), Some(families)) => !families.contains(family),
            _ => false,
        };
        // A model of a barred family leaves its catalogue entry to the family
        // restriction, which drops it too.
        if let Some((index, _)) = entry {
            if !barred {
                matched.insert(index);
            }
        }
        let mut keys: Vec<&str> = vec![model];
        if let Some((_, entry)) = entry {
            keys.extend(entry.all_names());
        }
        if keys.iter().any(|key| config.disabled.contains(&model_key(id, key))) {
            continue;
        }
        let Some(family) = family else {
            skipped.push(SkippedModel { client: id.to_string(), model: model.to_string(), reason: FAMILY_UNKNOWN.to_string() });
            continue;
        };
        if barred {
            continue;
        }
        models.push(ListedModel {
            client: id.to_string(),
            model: model.to_string(),
            family,
            cost_class: cost_class_for(client, runs_on.as_deref()),
            tier: entry.map(|(_, entry)| entry.tier),
            notes: entry.map(|(_, entry)| entry.notes.clone()).unwrap_or_default(),
            source: client.model_source,
            free: false,
        });
    }
    Listed { models, skipped, matched }
}

/// The `<client>:<model>` keys that name one client model: the id the ledger or a
/// run named, plus every id and name of the catalogue entry it belongs to, which
/// are the same model under other names. A model no catalogue entry names is
/// itself alone.
fn keys_of(config: &GatewayConfig, client: &str, model: &str) -> Vec<String> {
    let entry = config.models.iter().find(|candidate| candidate.client == client && candidate.all_names().contains(&model));
    let mut keys: Vec<&str> = vec![model];
    if let Some(entry) = entry {
        keys.extend(entry.all_names());
    }
    let mut unique: Vec<&str> = Vec::new();
    for key in keys {
        if !key.is_empty() && !unique.contains(&key) {
            unique.push(key);
        }
    }
    unique.into_iter().map(|key| model_key(client, key)).collect()
}

/// The model part of a `<provider>/<model>` id, which is what names its family.
pub fn model_part(model: &str) -> &str {
    &model[model.rfind('/').map_or(0, |slash| slash + 1)..]
}

/// The cost class of the provider a model runs on, else the client's own.
pub fn cost_class_for(client: &ClientConfig, provider: Option<&str>) -> CostClass {
    let Some(provider) = provider else {
        return client.cost_class;
    };
    client.provider_cost_classes.as_ref().and_then(|classes| classes.get(provider)).copied().unwrap_or(client.cost_class)
}

/// Every client the gateway knows, in client order.
pub fn sorted_clients(config: &GatewayConfig) -> Vec<(&String, &ClientConfig)> {
    config.clients.iter().collect()
}

fn compare(left: &str, right: &str) -> Ordering {
    left.cmp(right)
}

/// The providers an OpenCode run may reach when the machine names none.
pub const DEFAULT_OPENCODE_PROVIDERS: [&str; 1] = ["opencode-go"];

/// OpenCode's own pay-per-token providers (`opencode/<model>` is its Zen). The
/// owner's rule is that a run never reaches one, so no allowlist can open it.
const PAY_PER_TOKEN_PROVIDERS: [&str; 1] = ["opencode"];

/// The provider of an OpenCode model named `<provider>/<model>`; `None` when it names none.
pub fn open_code_provider(model: &str) -> Option<String> {
    let slash = model.find('/')?;
    if slash == 0 || slash == model.len() - 1 {
        return None;
    }
    Some(model[..slash].to_string())
}

/// Whether a run may reach the provider: the allowlist (else `opencode-go` alone)
/// names it and it is not pay-per-token.
pub fn is_provider_allowed(provider: Option<&str>, allowed: Option<&[String]>) -> bool {
    let Some(provider) = provider else {
        return false;
    };
    if PAY_PER_TOKEN_PROVIDERS.contains(&provider) {
        return false;
    }
    match allowed {
        Some(allowed) => allowed.iter().any(|name| name == provider),
        None => DEFAULT_OPENCODE_PROVIDERS.contains(&provider),
    }
}
