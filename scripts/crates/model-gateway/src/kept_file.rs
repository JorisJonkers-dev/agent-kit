//! The writing worktrees the gateway keeps until `worktrees clean` removes them,
//! in one JSON file under the state directory. A change reads the file, applies
//! itself and writes the result under an exclusive lock beside it. Entries are
//! kept as the file holds them, so a field this port does not read survives.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::lock::FileLock;
use crate::ports::Runtime;
use crate::state_file::state_dir;
use crate::text::{dirname, join};

/// The version the kept worktrees file is written as.
pub const KEPT_VERSION: f64 = 1.0;

/// The kept worktrees file.
pub fn kept_path(env: &std::collections::HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&state_dir(env)?, "kept-worktrees.json"]))
}

/// The kept worktrees file, read and written through the runtime's files and clock.
pub struct KeptStore<'a> {
    pub runtime: &'a Runtime,
}

impl KeptStore<'_> {
    /// Every kept worktree, read under the lock: the file is not written.
    pub fn list(&self) -> Result<Vec<Value>, GatewayError> {
        self.update(&|kept| kept.to_vec(), false)
    }

    /// Forgets every kept worktree whose run id is in `run_ids` (`None` is an entry that has none).
    pub fn remove(&self, run_ids: &[Option<Value>]) -> Result<(), GatewayError> {
        let wanted = |entry: &Value| {
            let held = entry.as_object().and_then(|object| object.get("run_id"));
            run_ids.iter().any(|run| run.as_ref() == held)
        };
        self.update(&|kept| kept.iter().filter(|entry| !wanted(entry)).cloned().collect(), true).map(|_| ())
    }

    /// Reads the list under the lock, and writes back what `change` makes of it unless the change is only a look.
    fn update(&self, change: &dyn Fn(&[Value]) -> Vec<Value>, write: bool) -> Result<Vec<Value>, GatewayError> {
        let path = kept_path(&self.runtime.env)?;
        self.runtime.files.mkdir_all(dirname(&path)).map_err(GatewayError::from_fs)?;
        let lock = FileLock::new("kept-worktrees", self.runtime.lock);
        let files = self.runtime.files.as_ref();
        let clock = self.runtime.clock.as_ref();
        lock.run(&format!("{path}.lock"), files, clock, || {
            let changed = change(&self.load(&path)?);
            if write {
                let mut object = Object::new();
                object.set("version", Value::Num(KEPT_VERSION));
                object.set("worktrees", Value::Arr(changed.clone()));
                let text = format!("{}\n", json::pretty(&Value::Obj(object)));
                self.runtime.files.write_atomic(&path, &text).map_err(GatewayError::from_fs)?;
            }
            Ok(changed)
        })
    }

    /// The worktrees the file holds; a file that is not there yet holds none.
    fn load(&self, path: &str) -> Result<Vec<Value>, GatewayError> {
        let text = match self.runtime.files.read_text(path) {
            Ok(text) => text,
            Err(error) if error.code == "ENOENT" => return Ok(Vec::new()),
            Err(error) => return Err(GatewayError::from_fs(error)),
        };
        let parsed = json::parse(&text).map_err(|message| GatewayError::new("invalid-config", format!("{path}: not JSON: {message}")))?;
        match parsed.as_object().and_then(|object| object.get("worktrees")) {
            Some(Value::Arr(worktrees)) => Ok(worktrees.clone()),
            _ => Err(GatewayError::new(
                "invalid-config",
                format!("{path}: must be {{\"version\": {}, \"worktrees\": [...]}}", json::number_text(KEPT_VERSION)),
            )),
        }
    }
}
