//! Everything a command needs from its surroundings; tests replace any of it.

use crate::git::{Git, SystemGit};
use model_gateway::adapters::clock::SystemClock;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::ports::{Clock, Files};
use std::collections::HashMap;
use std::io::Read;

pub struct Runtime {
    pub clock: Box<dyn Clock>,
    pub files: Box<dyn Files>,
    pub env: HashMap<String, String>,
    pub git: Box<dyn Git>,
    /// What a command reads when it is given neither `--input` nor `--json`.
    pub stdin: Box<dyn Fn() -> String + Send + Sync>,
}

/// The real runtime: the system's clock and files, the environment, the working directory and standard input.
pub fn default_runtime() -> Runtime {
    let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    let env: HashMap<String, String> =
        std::env::vars_os().map(|(key, value)| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned())).collect();
    Runtime {
        clock: Box::new(SystemClock),
        files: Box::new(SystemFiles { cwd }),
        git: Box::new(SystemGit { env: env.clone() }),
        env,
        stdin: Box::new(|| {
            let mut bytes = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        }),
    }
}
