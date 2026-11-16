//! Config-and-merge library (issue #809, part of the Rust migration #802).
//!
//! Atomic-enough settings writes, marked-block edits and MCP config merges, the
//! Rust port of the `*-merge-mcp`, `package-proxy-merge`, `instruction-block`
//! and `codex-add-url-mcp` scripts. The `agent-kit-merge` binary is the command
//! line over it; the later converge port (#810) reuses the functions.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub mod block;
pub mod cli;
pub mod converge;
pub mod gold_band;
pub mod gold_band_roles;
pub mod json;
pub mod mcp_json;
pub mod mcp_toml;
pub mod mcp_yaml;
pub mod proxy;
pub mod text;

#[cfg(test)]
mod tests;

/// Why an operation stopped: a message for stderr.
#[derive(Debug, PartialEq)]
pub struct Failure(pub String);

impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Failure(error.to_string())
    }
}

/// Where a command reads and writes besides files.
pub struct Io<'a> {
    pub stdin: &'a mut dyn Read,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

/// Looks up an environment variable; a closure so tests need no process state.
pub type Env<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The home directory, from `HOME`.
pub fn home(env: Env) -> PathBuf {
    PathBuf::from(env("HOME").unwrap_or_default())
}

/// `path` with `suffix` added to its file name (`config.json` -> `config.json.bak`).
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// A copy that keeps permissions and modification time, as `shutil.copy2` does.
pub fn copy_keeping(from: &Path, to: &Path) -> Result<(), Failure> {
    fs::copy(from, to)?;
    let modified = fs::metadata(from)?.modified()?;
    fs::OpenOptions::new().write(true).open(to)?.set_modified(modified)?;
    Ok(())
}

/// The first backup is the owner's own file from before agent-kit; keep it.
pub fn backup_once(config: &Path) -> Result<(), Failure> {
    let backup = with_suffix(config, ".bak");
    if config.is_file() && !backup.exists() {
        copy_keeping(config, &backup)?;
    }
    Ok(())
}

/// Creates the parent directory of `path`.
pub fn make_parent(path: &Path) -> Result<(), Failure> {
    fs::create_dir_all(path.parent().unwrap_or(Path::new("")))?;
    Ok(())
}

/// Ends a merge that read its result back: a problem goes to stderr and is status 1, otherwise
/// the success line goes to stdout and is status 0.
pub fn finish(problem: Option<String>, success: String, io: &mut Io) -> Result<i32, Failure> {
    let failed = problem.is_some();
    match problem {
        Some(message) => writeln!(io.err, "{message}")?,
        None => writeln!(io.out, "{success}")?,
    }
    Ok(i32::from(failed))
}

/// Python's list repr for a list of strings: `['a', 'b']`.
pub fn repr_list(names: &[String]) -> String {
    let items: Vec<String> = names.iter().map(|name| text::repr(name)).collect();
    format!("[{}]", items.join(", "))
}
