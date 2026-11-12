//! The machine's grants file, read and written under a lock, never inside a repository.
//!
//! The grants the owner gave live in one JSON file in the machine configuration
//! (`$XDG_CONFIG_HOME/agent-kit/model-gateway-grants.json`, else
//! `~/.config/agent-kit/...`). A change reads the file, applies itself and
//! writes the result under an exclusive lock beside it, so two gateway
//! processes never lose each other's grant, and the write goes to a temporary
//! file that is renamed into place.
//!
//! A grant is never written inside a repository, so every read and every write
//! first asks git whether the file's directory lies in one, and refuses when it
//! does. The directory need not exist yet: the nearest one that does is the one
//! git is asked about. When git cannot answer (it is missing, refuses the
//! directory or does not reply) the file is refused as well: only git saying
//! "not a repository" lets a grant be read or written.

use crate::error::GatewayError;
use crate::grants::{self, Grant};
use crate::locations;
use crate::lock::FileLock;
use crate::ports::Runtime;
use crate::text::dirname;

/// What a change to the grants asks for: the grants to write, and the one it reports.
pub struct GrantChange {
    pub grants: Vec<Grant>,
    pub grant: Grant,
}

/// The grants file, read and written through the runtime's files, git and clock.
pub struct GrantsStore<'a> {
    pub runtime: &'a Runtime,
}

impl GrantsStore<'_> {
    /// The grants the file holds; a file that is not there yet holds none.
    pub fn read(&self) -> Result<Vec<Grant>, GatewayError> {
        self.require_outside_repository()?;
        self.load()
    }

    /// Applies `change` to the grants under the lock and writes the result.
    pub fn update(&self, change: impl FnOnce(&[Grant]) -> Result<GrantChange, GatewayError>) -> Result<Grant, GatewayError> {
        self.require_outside_repository()?;
        let path = locations::grants_path(&self.runtime.env)?;
        self.runtime.files.mkdir_all(dirname(&path)).map_err(GatewayError::from_fs)?;
        let lock = FileLock::new("grants", self.runtime.lock);
        let files = self.runtime.files.as_ref();
        let clock = self.runtime.clock.as_ref();
        lock.run(&format!("{path}.lock"), files, clock, || {
            let changed = change(&self.load()?)?;
            self.runtime.files.write_atomic(&path, &grants::grant_file_text(&changed.grants)).map_err(GatewayError::from_fs)?;
            Ok(changed.grant)
        })
    }

    /// Refuses a grants file that lies inside a git repository, naming the repository.
    fn require_outside_repository(&self) -> Result<(), GatewayError> {
        let path = locations::grants_path(&self.runtime.env)?;
        let directory = self.nearest_existing(dirname(&path));
        match self.runtime.git.root(&directory) {
            // git could not answer, which is not the same as git saying there is no repository: refuse.
            Err(message) => Err(GatewayError::new(
                "invalid-config",
                format!("{path}: cannot tell whether {directory} is inside a git repository, so no grant is read or written: {message}"),
            )),
            Ok(Some(root)) => Err(GatewayError::new(
                "grants-in-repository",
                format!("{path} is inside the git repository {root}: a grant lives in the machine configuration, never in a repository"),
            )),
            Ok(None) => Ok(()),
        }
    }

    /// The nearest directory at or above `path` that exists, which is the one git can be asked about.
    fn nearest_existing(&self, path: &str) -> String {
        let mut current = path.to_string();
        loop {
            if matches!(self.runtime.files.exists(&current), Ok(true)) {
                return current;
            }
            let parent = dirname(&current).to_string();
            if parent == current {
                return current;
            }
            current = parent;
        }
    }

    /// The grants the file holds; a file that is not there yet holds none.
    fn load(&self) -> Result<Vec<Grant>, GatewayError> {
        let path = locations::grants_path(&self.runtime.env)?;
        let text = match self.runtime.files.read_text(&path) {
            Ok(text) => text,
            Err(error) if error.code == "ENOENT" => return Ok(Vec::new()),
            Err(error) => {
                return Err(GatewayError::new("invalid-config", format!("{path}: cannot read the grants: {}", error.message)));
            }
        };
        grants::parse_grants_file(&text).map_err(|message| GatewayError::new("invalid-config", format!("{path}: {message}")))
    }
}
