//! Everything a command needs from its surroundings; tests replace any of it.

use crate::dirs::{Dirs, SystemDirs};
use crate::gh::{Gh, SystemGh};
use crate::git::{Git, SystemGit};
use crate::search_client::SearchCommand;
use model_gateway::adapters::clock::SystemClock;
use model_gateway::adapters::fs::SystemFiles;
use model_gateway::ports::{Clock, Files};
use std::collections::HashMap;
use std::io::Read;

pub struct Runtime {
    pub clock: Box<dyn Clock>,
    pub files: Box<dyn Files>,
    pub dirs: Box<dyn Dirs>,
    pub env: HashMap<String, String>,
    /// The folder the runner's shipped files (routing.json, prompts) sit in; the embedded copies are what is read, this is the path messages name.
    pub assets_root: String,
    pub git: Box<dyn Git>,
    /// What the runner reads from GitHub.
    pub gh: Box<dyn Gh>,
    /// What the model gateway's own code runs through: its ledger, its plan limits and the client programs a critic launches (same clock, files and environment as above).
    pub gateway: model_gateway::ports::Runtime,
    /// What a command reads when it is given neither `--input` nor `--json`.
    pub stdin: Box<dyn Fn() -> String + Send + Sync>,
    /// How the runner starts its own search server for a Vibe critic; without it every Vibe critic reads alone.
    pub search: Option<SearchCommand>,
}

/// Where the shipped assets (prompts, roles, lenses, schemas, `routing.json` and the node runner) are:
/// `AGENT_KIT_CRITIQUE_ASSETS` when it names a directory, as the skill's launcher sets it to the skill directory, else
/// the directory the binary is in.
pub fn assets_root(env: &HashMap<String, String>, exe: Option<std::path::PathBuf>) -> String {
    match env.get("AGENT_KIT_CRITIQUE_ASSETS").filter(|dir| !dir.is_empty()) {
        Some(dir) => dir.clone(),
        None => exe.as_deref().and_then(std::path::Path::parent).map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default(),
    }
}

/// The real runtime: the system's clock and files, the environment, the working directory and standard input.
pub fn default_runtime() -> Runtime {
    let cwd = std::env::current_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    let env: HashMap<String, String> =
        std::env::vars_os().map(|(key, value)| (key.to_string_lossy().into_owned(), value.to_string_lossy().into_owned())).collect();
    Runtime {
        gateway: model_gateway::default_runtime(),
        clock: Box::new(SystemClock),
        files: Box::new(SystemFiles { cwd }),
        dirs: Box::new(SystemDirs),
        git: Box::new(SystemGit { env: env.clone() }),
        gh: Box::new(SystemGh { env: env.clone() }),
        assets_root: assets_root(&env, std::env::current_exe().ok()),
        search: std::env::current_exe().ok().map(|exe| SearchCommand { command: exe.to_string_lossy().into_owned(), args: Vec::new() }),
        env,
        stdin: Box::new(|| {
            let mut bytes = Vec::new();
            let _ = std::io::stdin().read_to_end(&mut bytes);
            String::from_utf8_lossy(&bytes).into_owned()
        }),
    }
}
