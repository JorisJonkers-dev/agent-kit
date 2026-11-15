//! rust-migration-guard (#804, parent #802): agent-kit is going 100 percent
//! Rust, so a pull request may not add or change Python or TypeScript except
//! as recorded migration debt.
//!
//! ```text
//! rust-migration-guard [--repo-dir DIR] [--allowlist PATH]
//!     (--event-path FILE | --base REF --body-file FILE)
//! ```
//!
//! * `--event-path FILE` is CI mode: FILE is the GitHub event payload
//!   (`GITHUB_EVENT_PATH`). The base is `pull_request.base.sha` and the body
//!   is `pull_request.body`. A payload without `pull_request` (a push to
//!   main) has nothing to compare and passes.
//! * `--base REF --body-file FILE` is local mode, for tests and for running
//!   the check by hand.
//! * `--allowlist PATH` is relative to the repository root and defaults to
//!   [`DEFAULT_ALLOWLIST`].
//!
//! Rules, over `git diff --name-status -M BASE...HEAD`, for every file whose
//! extension is in [`EXTENSIONS`] and which is not vendored or built output
//! ([`EXCLUDED_DIRS`]):
//!
//! * added, or renamed to a new path, and not on the allowlist: fail;
//! * modified and not on the allowlist: fail;
//! * modified, or renamed onto an allowlisted path: fail unless the pull
//!   request body has a debt line;
//! * an allowlist entry that the base allowlist did not have: fail unless
//!   the body has a debt line;
//! * deleted files, and allowlist entries removed: always pass.
//!
//! A debt line is `rust-migration-debt: #<child ticket> <reason>` on a line
//! of its own, with a non-empty reason.
//!
//! Exit 0 passes, 1 means violations (each on its own stderr line), 2 is bad
//! usage or a repository the check cannot read.

#[cfg(test)]
mod tests;

use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the allowlist lives, relative to the repository root.
pub const DEFAULT_ALLOWLIST: &str = "ci/rust-migration-allowlist.txt";

/// The file extensions the guard watches.
pub const EXTENSIONS: [&str; 9] = ["py", "pyi", "ts", "tsx", "mts", "cts", "js", "mjs", "cjs"];

/// Directory names whose contents are vendored or built output, not source.
/// No tracked file in the repository sits under one today; the list keeps a
/// local `node_modules` or a `target` directory from being mistaken for new
/// source. Fixture and golden data are deliberately not excluded: a script
/// hidden in a fixture directory is still a script.
pub const EXCLUDED_DIRS: [&str; 6] = [
    "node_modules",
    "target",
    ".venv",
    "venv",
    "__pycache__",
    "dist-packages",
];

const USAGE: &str = "usage: rust-migration-guard [--repo-dir DIR] [--allowlist PATH] (--event-path FILE | --base REF --body-file FILE)\n";

/// One path change of the pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Added(String),
    Modified(String),
    Deleted(String),
    Renamed { from: String, to: String },
}

/// True for a path the guard watches: a watched extension, outside the
/// excluded directories.
pub fn is_watched(path: &str) -> bool {
    let mut parts = path.split('/').collect::<Vec<_>>();
    let file = parts.pop().unwrap_or("");
    if parts.iter().any(|p| EXCLUDED_DIRS.contains(p)) {
        return false;
    }
    match file.rsplit_once('.') {
        Some((stem, ext)) => !stem.is_empty() && EXTENSIONS.contains(&ext),
        None => false,
    }
}

/// The allowlist's paths: one per line, `#` comments and blank lines ignored.
pub fn parse_allowlist(text: &str) -> BTreeSet<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.trim_start_matches("./").to_string())
        .collect()
}

/// True when the body has a `rust-migration-debt: #<digits> <reason>` line.
pub fn has_debt_line(body: &str) -> bool {
    body.lines().any(|line| {
        let Some(rest) = line.trim().strip_prefix("rust-migration-debt:") else {
            return false;
        };
        let Some(rest) = rest.trim_start().strip_prefix('#') else {
            return false;
        };
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        digits > 0 && !rest[digits..].trim().is_empty()
    })
}

/// Parses `git diff --name-status -M -z` output into changes. Copies count as
/// additions; type changes and unmerged entries as modifications.
pub fn parse_changes(raw: &str) -> Vec<Change> {
    let mut tokens = raw.split('\0').filter(|t| !t.is_empty());
    let mut out = Vec::new();
    while let Some(status) = tokens.next() {
        let first = status.chars().next().unwrap_or('M');
        let a = tokens.next().unwrap_or("").to_string();
        out.push(match first {
            'A' => Change::Added(a),
            'D' => Change::Deleted(a),
            'R' | 'C' => {
                let b = tokens.next().unwrap_or("").to_string();
                if first == 'R' {
                    Change::Renamed { from: a, to: b }
                } else {
                    Change::Added(b)
                }
            }
            _ => Change::Modified(a),
        });
    }
    out
}

/// Every violation of the rules, in a stable order. `base_allowlist` is
/// `None` when the base has no allowlist yet (the pull request that
/// introduces it), which leaves nothing to compare entries against.
pub fn evaluate(
    changes: &[Change],
    allowlist: &BTreeSet<String>,
    base_allowlist: Option<&BTreeSet<String>>,
    body: &str,
) -> Vec<String> {
    let debt = has_debt_line(body);
    let need_debt =
        "needs a line `rust-migration-debt: #<child ticket> <reason>` in the pull request body";
    let mut out = Vec::new();
    for change in changes {
        match change {
            Change::Added(p) if is_watched(p) && !allowlist.contains(p) => {
                out.push(format!(
                    "{p}: new file outside the allowlist (the repository is going 100% Rust, #802)"
                ));
            }
            Change::Renamed { to, .. } if is_watched(to) => {
                if !allowlist.contains(to) {
                    out.push(format!("{to}: renamed to a path outside the allowlist"));
                } else if !debt {
                    out.push(format!(
                        "{to}: renamed onto an allowlisted path; {need_debt}"
                    ));
                }
            }
            Change::Modified(p) if is_watched(p) => {
                if !allowlist.contains(p) {
                    out.push(format!("{p}: changed file outside the allowlist"));
                } else if !debt {
                    out.push(format!("{p}: allowlisted file changed; {need_debt}"));
                }
            }
            _ => {}
        }
    }
    if let Some(base) = base_allowlist {
        if !debt {
            for added in allowlist.difference(base) {
                out.push(format!(
                    "{added}: added to the allowlist, which only shrinks; {need_debt}"
                ));
            }
        }
    }
    out
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    run_git("git", dir, args)
}

/// `git` is a parameter so a test can name a program that does not exist.
fn run_git(program: &str, dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The changes of `base...HEAD` in the repository at `dir`.
pub fn diff_changes(dir: &Path, base: &str) -> Result<Vec<Change>, String> {
    let range = format!("{base}...HEAD");
    git(dir, &["diff", "--name-status", "-M", "-z", &range]).map(|raw| parse_changes(&raw))
}

/// Runs the check on a repository. Returns the violations.
pub fn check(
    dir: &Path,
    allowlist_path: &str,
    base: &str,
    body: &str,
) -> Result<Vec<String>, String> {
    let head_text = std::fs::read_to_string(dir.join(allowlist_path))
        .map_err(|e| format!("cannot read allowlist {allowlist_path}: {e}"))?;
    let base_text = git(dir, &["show", &format!("{base}:{allowlist_path}")]).ok();
    let changes = diff_changes(dir, base)?;
    Ok(evaluate(
        &changes,
        &parse_allowlist(&head_text),
        base_text.as_deref().map(parse_allowlist).as_ref(),
        body,
    ))
}

struct Options {
    dir: PathBuf,
    allowlist: String,
    event_path: Option<PathBuf>,
    base: Option<String>,
    body_file: Option<PathBuf>,
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Options, String> {
    let mut o = Options {
        dir: PathBuf::from("."),
        allowlist: DEFAULT_ALLOWLIST.to_string(),
        event_path: None,
        base: None,
        body_file: None,
    };
    let mut it = args.into_iter();
    while let Some(flag) = it.next() {
        let flag = flag.to_string_lossy().into_owned();
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--repo-dir" => o.dir = PathBuf::from(value),
            "--allowlist" => o.allowlist = value.to_string_lossy().into_owned(),
            "--event-path" => o.event_path = Some(PathBuf::from(value)),
            "--base" => o.base = Some(value.to_string_lossy().into_owned()),
            "--body-file" => o.body_file = Some(PathBuf::from(value)),
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    let local = o.base.is_some() && o.body_file.is_some();
    if o.event_path.is_some() == local {
        return Err("give --event-path, or --base with --body-file".to_string());
    }
    Ok(o)
}

/// The base and body of a pull-request payload; `None` for any other event.
fn from_event(path: &Path) -> Result<Option<(String, String)>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read event payload {}: {e}", path.display()))?;
    let v: Value =
        serde_json::from_str(&text).map_err(|e| format!("event payload is not JSON: {e}"))?;
    let Some(pr) = v.get("pull_request") else {
        return Ok(None);
    };
    let base = pr
        .pointer("/base/sha")
        .and_then(Value::as_str)
        .ok_or("event payload has no pull_request.base.sha")?;
    let body = pr.get("body").and_then(Value::as_str).unwrap_or("");
    Ok(Some((base.to_string(), body.to_string())))
}

fn resolve(o: &Options) -> Result<Option<(String, String)>, String> {
    if let Some(path) = &o.event_path {
        return from_event(path);
    }
    let body = std::fs::read_to_string(o.body_file.as_deref().unwrap_or(Path::new("")))
        .map_err(|e| format!("cannot read body file: {e}"))?;
    Ok(Some((o.base.clone().unwrap_or_default(), body)))
}

/// The command line. Returns the exit code.
pub fn run(args: impl IntoIterator<Item = OsString>) -> i32 {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(e) => {
            eprint!("rust-migration-guard: {e}\n{USAGE}");
            return 2;
        }
    };
    let outcome = resolve(&opts).and_then(|r| match r {
        None => Ok(None),
        Some((base, body)) => check(&opts.dir, &opts.allowlist, &base, &body).map(Some),
    });
    match outcome {
        Err(e) => {
            eprintln!("rust-migration-guard: {e}");
            2
        }
        Ok(None) => {
            println!("rust-migration-guard: not a pull request, nothing to compare");
            0
        }
        Ok(Some(v)) if v.is_empty() => {
            println!("rust-migration-guard: no new or changed Python or TypeScript outside the allowlist");
            0
        }
        Ok(Some(v)) => {
            for line in &v {
                eprintln!("rust-migration-guard: {line}");
            }
            eprintln!(
                "rust-migration-guard: {} violation(s); see docs/RUST.md, \"Rust migration guard\"",
                v.len()
            );
            1
        }
    }
}
