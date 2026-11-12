//! The workflows this machine has started, in one JSON file under the state
//! directory. A change reads the file, applies itself and writes the result
//! under an exclusive lock beside it, so two gateway processes starting or
//! ending workflows at once never lose each other's change. The write goes to a
//! temporary file that is renamed into place, so a reader never sees half a
//! file.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::lock::FileLock;
use crate::ports::Runtime;
use crate::text::{dirname, join};
use crate::workflows::{self, Change, WorkflowRecord, WORKFLOWS_VERSION};
use std::collections::HashMap;

/// The gateway's own state, outside every repository:
/// `$XDG_STATE_HOME/agent-kit/model-gateway`, or
/// `~/.local/state/agent-kit/model-gateway` when XDG_STATE_HOME is unset or not an absolute path.
pub fn state_dir(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    if let Some(xdg) = env.get("XDG_STATE_HOME") {
        if xdg.starts_with('/') {
            return Ok(join(&[xdg, "agent-kit", "model-gateway"]));
        }
    }
    let Some(home) = env.get("HOME") else {
        return Err(GatewayError::new("unexpected", "required environment variable is missing: HOME"));
    };
    Ok(join(&[home, ".local", "state", "agent-kit", "model-gateway"]))
}

/// The workflows this machine has started, and the moments each one started and ended.
pub fn workflows_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&state_dir(env)?, "workflows.json"]))
}

/// The workflows file, read and written through the runtime's files and clock.
pub struct Workflows<'a> {
    pub runtime: &'a Runtime,
}

impl Workflows<'_> {
    /// The workflows the file holds; a file that is not there yet holds none.
    pub fn read(&self) -> Result<Vec<WorkflowRecord>, GatewayError> {
        self.load()
    }

    /// Applies `change` to the workflows under the lock and writes the result.
    pub fn update(&self, change: impl FnOnce(&[WorkflowRecord]) -> Result<Change, GatewayError>) -> Result<WorkflowRecord, GatewayError> {
        let path = workflows_path(&self.runtime.env)?;
        self.runtime.files.mkdir_all(dirname(&path)).map_err(GatewayError::from_fs)?;
        let lock = FileLock::new("workflows", self.runtime.lock);
        let files = self.runtime.files.as_ref();
        let clock = self.runtime.clock.as_ref();
        lock.run(&format!("{path}.lock"), files, clock, || {
            let changed = change(&self.load()?)?;
            let mut object = Object::new();
            object.set("version", Value::Num(WORKFLOWS_VERSION));
            object.set("workflows", Value::Arr(changed.workflows.iter().map(WorkflowRecord::to_json).collect()));
            let text = format!("{}\n", json::pretty(&Value::Obj(object)));
            self.runtime.files.write_atomic(&path, &text).map_err(GatewayError::from_fs)?;
            Ok(changed.workflow)
        })
    }

    /// The workflows the file holds; a file that is not there yet holds none.
    fn load(&self) -> Result<Vec<WorkflowRecord>, GatewayError> {
        let path = workflows_path(&self.runtime.env)?;
        let text = match self.runtime.files.read_text(&path) {
            Ok(text) => text,
            Err(error) if error.code == "ENOENT" => return Ok(Vec::new()),
            Err(error) => {
                return Err(GatewayError::new("invalid-config", format!("{path}: cannot read the workflows: {}", error.message)));
            }
        };
        workflows::parse_file(&text).map_err(|message| GatewayError::new("invalid-config", format!("{path}: {message}")))
    }
}
