//! Decides whether a repository may send content to critics, and reads this
//! machine's family allowlist.
//!
//! A `nedap` repository sends nothing, whatever its visibility. A repository
//! GitHub says is public is not held to the allowlist. Any other repository,
//! including one whose visibility cannot be read, sends only what its entry
//! allows, and nothing without one.

use crate::allowlist::{parse_allowlist, Allowlist, Entry, ALLOWLIST_FAMILIES};
use crate::error::CritiqueError;
use crate::locations::allowlist_path;
use crate::runtime::Runtime;
use model_gateway::git_remote::{identify_repository, RemoteLocation, RepositoryIdentity, DENIED_OWNER};
use model_gateway::visibility::repository_visibility;
use model_gateway::json::{self, Value};

/// A repository that may be critiqued.
pub struct Resolved {
    pub root: String,
    pub common_dir: String,
    /// The allowlist key: `<host>/<owner>/<repo>`.
    pub key: String,
}

/// What resolving a repository came to: the repository, or the refusal its remotes earn.
pub enum Resolution {
    Resolved(Resolved),
    Refused(Value),
}

/// A refusal: a normal result, which the command line prints and exits 0 on.
pub fn refusal(reason: &str, message: &str) -> Value {
    json::object(vec![("message", Value::str(message)), ("reason", Value::str(reason)), ("status", Value::str("refused"))])
}

/// The repository's allowlist key and git directories, or the refusal its remotes earn.
pub fn resolve(runtime: &Runtime, cwd: &str) -> Result<Resolution, CritiqueError> {
    let remotes = runtime.git.remotes(cwd).map_err(CritiqueError::unexpected)?;
    match identify_repository(&remotes) {
        RepositoryIdentity::DeniedOwner { remote, .. } => {
            Ok(Resolution::Refused(refusal("nedap", &format!("remote {remote} belongs to the {DENIED_OWNER} organisation; nothing is sent to any critic"))))
        }
        RepositoryIdentity::NoRemote => Ok(Resolution::Refused(refusal("no-remote", "the repository has no remote, so it counts as not opted in"))),
        RepositoryIdentity::Identified { key, .. } => {
            let common_dir = runtime.git.common_dir(cwd).map_err(CritiqueError::unexpected)?;
            let root = runtime.git.root(cwd).map_err(CritiqueError::unexpected)?;
            Ok(Resolution::Resolved(Resolved { root, common_dir, key }))
        }
    }
}

/// This machine's family allowlist; a missing file is an empty one.
pub fn read_allowlist(runtime: &Runtime) -> Result<Allowlist, CritiqueError> {
    let path = allowlist_path(&runtime.env)?;
    // Node treats any failure to look as "not there"; the write that follows then says what is wrong.
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Ok(Allowlist::default());
    }
    let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
    parse_allowlist(&text).map_err(|error| error.into_error("invalid-allowlist", &path))
}

/// What a repository may send: the entry it is held to, and whether that is because GitHub says it is public.
pub struct Permission {
    pub entry: Entry,
    /// True only when GitHub says the repository is public; a private one, and one whose visibility cannot be read,
    /// are not.
    pub public: bool,
}

/// The entry a resolved repository is held to: every family, with code and history, when GitHub says it is public;
/// else its allowlist entry; `None` when it has none.
pub fn permission(runtime: &Runtime, repository: &Resolved) -> Result<Option<Permission>, CritiqueError> {
    if location_of(&repository.key).is_some_and(|location| repository_visibility(&runtime.gateway, &location).is_public()) {
        return Ok(Some(Permission { entry: public_entry(), public: true }));
    }
    Ok(read_allowlist(runtime)?.get(&repository.key).map(|entry| Permission { entry: entry.clone(), public: false }))
}

/// What a public repository is held to, whatever its allowlist entry says: every family, with code and history.
pub fn public_entry() -> Entry {
    Entry { code: true, families: ALLOWLIST_FAMILIES.iter().map(|family| family.to_string()).collect(), history: true, updated_at: String::new() }
}

/// An allowlist key, `<host>/<owner>/<repo>`, read back as the location it was made from.
fn location_of(key: &str) -> Option<RemoteLocation> {
    let (host, rest) = key.split_once('/')?;
    let (owner, repo) = rest.rsplit_once('/')?;
    Some(RemoteLocation { host: host.to_string(), owner: owner.to_string(), repo: repo.to_string() })
}
