//! The data policy: what a delegation may reach and carry.
//!
//! The scope is what a delegation would carry — a repository's content, or only
//! what the caller declares — and the decision is per client model: whether a
//! delegation may reach it at all, and whether it may carry code and history.

use crate::allowlist::{load_allowlist, AllowlistEntry};
use crate::config::{CostClass, GatewayConfig};
use crate::error::GatewayError;
use crate::git_remote::{identify_repository, RepositoryIdentity};
use crate::json::{object, Value};
use crate::models::{ListedModel, MissingClient, ModelListing, SkippedModel};
use crate::ports::Runtime;

/// What a delegation to one client model may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendPermission {
    pub code: bool,
    pub history: bool,
}

/// One listed model with what the policy lets a delegation to it carry.
#[derive(Clone, Debug, PartialEq)]
pub struct PermittedModel {
    pub listed: ListedModel,
    pub inline_only: bool,
    pub may_send: SendPermission,
}

/// What the `models` command lists: the permitted models, the clients that are missing and the models skipped.
#[derive(Clone, Debug, PartialEq)]
pub struct PolicyListing {
    pub models: Vec<PermittedModel>,
    pub missing: Vec<MissingClient>,
    pub skipped: Vec<SkippedModel>,
}

/// What the data policy says about one client model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelPermission {
    pub client: String,
    pub model: String,
    /// Whether a delegation may reach this client model at all.
    pub permitted: bool,
    /// True when the delegation carries only what the caller declares, never a repository.
    pub inline_only: bool,
    pub may_send: SendPermission,
}

/// The data policy's decision for every client model, and one line saying why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyDecision {
    pub reason: String,
    pub permissions: Vec<ModelPermission>,
}

/// What a delegation would carry: a repository's content, or only what the
/// caller declares. A repository with no remote, and one owned by `nedap`, are
/// decided by their own rule rather than by an allowlist entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataScope {
    Outside,
    NoRemote,
    Nedap,
    Repository { key: String, entry: Option<AllowlistEntry> },
}

/// One client model the policy decides about: the client that serves it, its
/// id, and the family it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicySubject {
    pub client: String,
    pub model: String,
    pub family: String,
}

/// The two clients a `nedap` repository may reach, whatever an allowlist entry
/// says. It is the client that counts, not the family: a Claude-family model
/// served by another client is not Claude Code.
const NEDAP_CLIENTS: [&str; 2] = ["claude", "codex"];

/// The cost classes a delegation outside any repository may reach.
const INLINE_COST_CLASSES: [CostClass; 3] = [CostClass::Free, CostClass::Subsidised, CostClass::MeteredPersonal];

const NOTHING: SendPermission = SendPermission { code: false, history: false };
const EVERYTHING: SendPermission = SendPermission { code: true, history: true };

/// The key the override's `disabled` list names a model by.
fn model_key(client: &str, model: &str) -> String {
    format!("{client}:{model}")
}

/// The data policy for one scope: per client model, whether a delegation may
/// reach it and what it may carry. A model the owner disabled is left out.
pub fn decide_policy(config: &GatewayConfig, scope: &DataScope, subjects: &[PolicySubject]) -> PolicyDecision {
    let permissions = subjects
        .iter()
        .filter(|subject| !config.disabled.contains(&model_key(&subject.client, &subject.model)))
        .map(|subject| permission(config, subject, scope))
        .collect();
    PolicyDecision { reason: reason_for(scope), permissions }
}

fn permission(config: &GatewayConfig, subject: &PolicySubject, scope: &DataScope) -> ModelPermission {
    let base = |inline_only: bool, may_send: SendPermission, permitted: bool| ModelPermission {
        client: subject.client.clone(),
        model: subject.model.clone(),
        permitted,
        inline_only,
        may_send,
    };
    match scope {
        DataScope::Outside => {
            let permitted = config.clients.get(&subject.client).is_some_and(|client| INLINE_COST_CLASSES.contains(&client.cost_class));
            base(true, NOTHING, permitted)
        }
        DataScope::NoRemote => base(false, NOTHING, false),
        DataScope::Nedap => base(false, EVERYTHING, NEDAP_CLIENTS.contains(&subject.client.as_str())),
        DataScope::Repository { entry, .. } => {
            let may_send = match entry {
                None => NOTHING,
                Some(entry) => SendPermission { code: entry.code, history: entry.history },
            };
            // A client on a public-only route reaches a repository only when the entry says the repository is public,
            // and only the families that entry allows; a private or unrecorded repository never reaches it.
            let public_only = config.clients.get(&subject.client).is_some_and(|client| client.public_only);
            let allows_family = entry.as_ref().is_some_and(|entry| entry.families.contains(&subject.family));
            let public_repository = entry.as_ref().is_some_and(|entry| entry.public);
            let permitted = allows_family && (!public_only || public_repository);
            base(false, may_send, permitted)
        }
    }
}

/// One line saying why the policy decided what it did, and what a client on a public-only route may do with it: such a
/// client is refused a repository that is not recorded as public, and permitted it when it is.
fn reason_for(scope: &DataScope) -> String {
    match scope {
        DataScope::Outside => "no repository: only inline content, to subsidised and metered-personal clients".to_string(),
        DataScope::NoRemote => "the repository has no remote, so nothing is sent".to_string(),
        DataScope::Nedap => "a nedap repository: only Codex and Claude Code, with code and history".to_string(),
        DataScope::Repository { key, entry } => match entry {
            None => format!("{key} has no allowlist entry, so nothing is sent"),
            Some(entry) if entry.families.is_empty() => format!("{key}: the entry allows no family"),
            Some(entry) if entry.public => format!(
                "{key}: the entry allows {}, and records the repository as public, so a public-only route may read it",
                entry.families.join(", ")
            ),
            Some(entry) => format!("{key}: the entry allows {}, and does not record the repository as public", entry.families.join(", ")),
        },
    }
}

/// The listed models the policy permits, each with what it may carry; the
/// missing and skipped lists stay as they were. A listed model the policy does
/// not permit is skipped with the policy's reason, so a client on a public-only
/// route says why it is left out of a repository that is not recorded as public.
pub fn permitted_models(listing: ModelListing, decision: &PolicyDecision) -> PolicyListing {
    let mut skipped = listing.skipped;
    let mut models = Vec::new();
    for model in listing.models {
        let permission = decision
            .permissions
            .iter()
            .rev()
            .find(|permission| permission.client == model.client && permission.model == model.model);
        match permission {
            Some(permission) if permission.permitted => {
                models.push(PermittedModel { listed: model, inline_only: permission.inline_only, may_send: permission.may_send });
            }
            Some(_) => skipped.push(SkippedModel { client: model.client, model: model.model, reason: decision.reason.clone() }),
            None => {}
        }
    }
    PolicyListing { models, missing: listing.missing, skipped }
}

/// What a delegation from `repo` would carry: a repository's content, or only
/// what the caller declares. No `repo`, and a path in no repository, both mean
/// the caller declares its content itself; a path git cannot read is an error,
/// never a permissive scope.
pub fn resolve_scope(runtime: &Runtime, repo: Option<&str>) -> Result<DataScope, GatewayError> {
    let Some(repo) = repo else {
        return Ok(DataScope::Outside);
    };
    let Some(root) = git_or_fail(repo, runtime.git.root(repo))? else {
        return Ok(DataScope::Outside);
    };
    match identify_repository(&git_or_fail(repo, runtime.git.remotes(&root))?) {
        RepositoryIdentity::DeniedOwner { .. } => Ok(DataScope::Nedap),
        RepositoryIdentity::NoRemote => Ok(DataScope::NoRemote),
        RepositoryIdentity::Identified { key, .. } => {
            let allowlist = load_allowlist(runtime)?;
            Ok(DataScope::Repository { entry: allowlist.repositories.get(&key).cloned(), key })
        }
    }
}

/// A git that cannot read the path is an error naming the path and git's message.
fn git_or_fail<T>(repo: &str, result: Result<T, String>) -> Result<T, GatewayError> {
    result.map_err(|message| GatewayError::new("repository-unreadable", format!("{repo}: {message}")))
}

/// One permitted model as the `models` command prints it.
pub fn permitted_model_json(model: &PermittedModel) -> Value {
    let free = if model.listed.free { vec![("free", Value::Bool(true))] } else { Vec::new() };
    object([

        ("client", Value::str(&model.listed.client)),
        ("cost_class", Value::str(model.listed.cost_class.as_str())),
        ("family", Value::str(&model.listed.family)),
        ("model", Value::str(&model.listed.model)),
        ("notes", Value::str(&model.listed.notes)),
        ("source", Value::str(model.listed.source.as_str())),
        ("tier", model.listed.tier.map_or(Value::Null, |tier| Value::str(tier.as_str()))),
    ]
    .into_iter()
    .chain(free)
    .chain([
        ("inline_only", Value::Bool(model.inline_only)),
        ("may_send", object(vec![("code", Value::Bool(model.may_send.code)), ("history", Value::Bool(model.may_send.history))])),
    ])
    .collect())
}

/// The `models` command's result: the policy line, the status, and the listing.
pub fn models_result_json(policy_reason: &str, listing: &PolicyListing, profiles: &Value, warning: Option<&str>) -> Value {
    let missing = listing
        .missing
        .iter()
        .map(|missing| object(vec![("client", Value::str(&missing.client)), ("reason", Value::str(&missing.reason))]))
        .collect();
    let skipped = listing
        .skipped
        .iter()
        .map(|skipped| {
            object(vec![
                ("client", Value::str(&skipped.client)),
                ("model", Value::str(&skipped.model)),
                ("reason", Value::str(&skipped.reason)),
            ])
        })
        .collect();
    object(
        [
        ("policy", Value::str(policy_reason)),
        ("status", Value::str("ok")),
        ("missing", Value::Arr(missing)),
        ("models", Value::Arr(listing.models.iter().map(permitted_model_json).collect())),
        ("skipped", Value::Arr(skipped)),
        ("profiles", profiles.clone()),
    ]
    .into_iter()
    .chain(warning.map(|warning| ("warnings", Value::Arr(vec![Value::str(warning)]))))
    .collect())
}
