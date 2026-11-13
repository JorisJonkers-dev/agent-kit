//! Decides whether a repository may send content to critics, and reads this
//! machine's family allowlist.

use crate::allowlist::{parse_allowlist, Allowlist, Entry};
use crate::error::CritiqueError;
use crate::locations::allowlist_path;
use crate::runtime::Runtime;
use model_gateway::git_remote::{identify_repository, RepositoryIdentity, DENIED_OWNER};
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

/// What resolving a repository for a roster came to: the repository, and whether it is a `nedap` one that has the
/// work-only roster, or the refusal its remotes earn.
pub enum RosterResolution {
    Resolved(Resolved, bool),
    Refused(Value),
}

/// Like `resolve`, but a `nedap` repository resolves, flagged as work, to the work-only roster rather than being
/// refused; only prepare asks. It is keyed by its own location, so its tracker reads.
pub fn resolve_for_roster(runtime: &Runtime, cwd: &str) -> Result<RosterResolution, CritiqueError> {
    let remotes = runtime.git.remotes(cwd).map_err(CritiqueError::unexpected)?;
    let (key, work) = match identify_repository(&remotes) {
        RepositoryIdentity::DeniedOwner { location, .. } => (format!("{}/{}/{}", location.host, location.owner, location.repo), true),
        RepositoryIdentity::NoRemote => {
            return Ok(RosterResolution::Refused(refusal("no-remote", "the repository has no remote, so it counts as not opted in")));
        }
        RepositoryIdentity::Identified { key, .. } => (key, false),
    };
    let common_dir = runtime.git.common_dir(cwd).map_err(CritiqueError::unexpected)?;
    let root = runtime.git.root(cwd).map_err(CritiqueError::unexpected)?;
    Ok(RosterResolution::Resolved(Resolved { root, common_dir, key }, work))
}

/// What a `nedap` repository's entry is, whatever its allowlist says: the work families, with code and history.
pub fn work_entry() -> Entry {
    Entry { code: true, families: vec!["anthropic".to_string(), "openai".to_string()], history: true, updated_at: String::new() }
}
