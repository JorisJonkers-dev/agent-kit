//! code-nav: a read-only code-navigation MCP server over stdio.
//!
//! The server searches the live working tree (no index, so no stale result)
//! under an allowlist of canonical roots. Four modules, each a small seam:
//!
//! * [`boundary`] decides which paths may be read: a root outside the
//!   allowlist, a symlink that leaves it and a secret file are refused.
//! * [`search`] finds and ranks matching lines and caps the result.
//! * [`outline`] lists the symbols a file declares and expands one of them.
//! * [`server`] speaks MCP (newline-delimited JSON-RPC) and exposes `search`,
//!   `outline` and `expand`.
//!
//! Usage: `code-nav --root DIR [--root DIR ...] [--limit N]`. The roots are
//! the allowlist; the caller (setup, per instance) passes only roots that fit
//! the instance's repository rule. `--limit` sets the default result cap. A
//! root of `~` or `~/...` is expanded against `HOME`, because a registry
//! config passes the argument literally, with no shell to expand it.

pub mod boundary;
pub mod outline;
pub mod search;
pub mod server;

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// What the command line configures. A root's leading `~` is already expanded.
#[derive(Debug, PartialEq)]
pub struct Config {
    pub roots: Vec<PathBuf>,
    pub limit: usize,
}

const USAGE: &str = "usage: code-nav --root DIR [--root DIR ...] [--limit N]";

/// Parse the command line. `Err` is the message to print.
pub fn parse_args(args: &[String]) -> Result<Config, String> {
    parse_args_with_home(args, std::env::var("HOME").ok().as_deref())
}

/// Parse the command line, expanding a root's leading `~` against `home`.
pub fn parse_args_with_home(args: &[String], home: Option<&str>) -> Result<Config, String> {
    let mut roots = Vec::new();
    let mut limit = search::DEFAULT_LIMIT;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--root" => {
                let dir = it.next().ok_or("--root needs a directory")?;
                roots.push(PathBuf::from(expand_home(dir, home)?));
            }
            "--limit" => {
                let n: usize = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--limit needs a whole number")?;
                if n == 0 || n > search::MAX_LIMIT {
                    return Err(format!("--limit must be between 1 and {}", search::MAX_LIMIT));
                }
                limit = n;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if roots.is_empty() {
        return Err("at least one --root is required".to_string());
    }
    Ok(Config { roots, limit })
}

/// Expand a leading `~` or `~/` against `home`; any other value is unchanged.
fn expand_home(dir: &str, home: Option<&str>) -> Result<String, String> {
    let rest = match dir.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => rest,
        _ => return Ok(dir.to_string()),
    };
    match home {
        Some(home) if !home.is_empty() => Ok(format!("{home}{rest}")),
        _ => Err("--root ~ needs HOME to be set".to_string()),
    }
}

/// Run the server on the given streams; the exit code.
pub fn cli(args: &[String], input: &mut dyn BufRead, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let config = match parse_args(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(err, "code-nav: {e}\n{USAGE}");
            return 2;
        }
    };
    let boundary = match boundary::Boundary::new(&config.roots) {
        Ok(b) => b,
        Err(e) => {
            let _ = writeln!(err, "code-nav: {e}");
            return 2;
        }
    };
    match server::serve(&boundary, config.limit, input, out) {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "code-nav: {e}");
            1
        }
    }
}

/// The process entry: stdio streams.
pub fn run(args: impl IntoIterator<Item = OsString>) -> i32 {
    let args: Vec<String> = args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect();
    cli(&args, &mut std::io::stdin().lock(), &mut std::io::stdout().lock(), &mut std::io::stderr())
}

#[cfg(test)]
mod tests;
