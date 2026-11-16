//! projects-layout-guard: the PreToolUse hook of docs/adr/0009-projects-layout-guard.md.
//!
//! Denies `git clone`, `gh repo clone` and `git worktree add` when the target
//! breaks the workstation layout in the projects-layout skill:
//!
//! * clones:    `~/Projects/<owner>/<repo>`
//! * worktrees: `~/Projects/<owner>/worktrees/<repo>/<name>`
//!
//! It also denies Claude Code's `EnterWorktree` with `name`, which creates
//! `.claude/worktrees/`.
//!
//! It follows ADR-0002's rules for every hook: it denies by printing the JSON
//! `deny` and exiting 0, reads local state only (no network), and fails open: an
//! unparsable command, an unknown repository or any error of its own allows the
//! call. It does nothing when `~/Projects` does not exist or
//! `PROJECTS_LAYOUT_OFF=1` is set.

mod paths;
mod shell;
mod url;

use paths::{basename, dirname, join, norm, relpath};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const RULES: &str = "See the projects-layout skill.";
const GIT_TIMEOUT: Duration = Duration::from_secs(2);

/// What the guard reads from its machine: `~`, `~/Projects`, the git program
/// and how long one git call may take.
pub struct Ctx {
    pub home: String,
    pub projects: String,
    pub git: String,
    pub timeout: Duration,
}

/// An explicit decision: the one line the agent reads.
struct Deny(String);

fn block(msg: String) -> Result<(), Deny> {
    Err(Deny(format!("{msg} {RULES}")))
}

fn segments_of(command: &str) -> Vec<Vec<String>> {
    shell::segments(command)
}

fn strip_env(mut argv: &[String]) -> &[String] {
    while argv.first().is_some_and(|a| is_assignment(a)) {
        argv = &argv[1..];
    }
    while argv
        .first()
        .is_some_and(|a| ["command", "exec", "env", "nice", "time"].contains(&a.as_str()))
    {
        argv = &argv[1..];
    }
    argv
}

/// `^[A-Za-z_][A-Za-z0-9_]*=`
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Consume git's global options (`-C`, `-c`, ...). Returns the rest and the working directory.
fn git_global_opts<'a>(argv: &'a [String], cwd: &str, ctx: &Ctx) -> (&'a [String], String) {
    let mut cwd = cwd.to_string();
    let mut i = 1;
    while i < argv.len() {
        let a = argv[i].as_str();
        if a == "-C" && i + 1 < argv.len() {
            cwd = norm(&argv[i + 1], &cwd, &ctx.home);
            i += 2;
        } else if ["-c", "--git-dir", "--work-tree", "--namespace"].contains(&a)
            && i + 1 < argv.len()
        {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    (&argv[i..], cwd)
}

/// The positional arguments, skipping the values of `opts_with_value`.
fn positional<'a>(args: &'a [String], opts_with_value: &[&str]) -> Vec<&'a str> {
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            pos.extend(args[i + 1..].iter().map(String::as_str));
            break;
        }
        if a.starts_with('-') {
            let name = a.split('=').next().unwrap_or(a);
            if opts_with_value.contains(&name) && !a.contains('=') {
                i += 1;
            }
        } else {
            pos.push(a);
        }
        i += 1;
    }
    pos
}

fn check_clone_target(owner: &str, repo: &str, target: &str, ctx: &Ctx) -> Result<(), Deny> {
    let want = join(&join(&ctx.projects, owner), repo);
    if target != want {
        return block(format!("clone {owner}/{repo} into {want}, not {target}. If {want} already exists, use that clone."));
    }
    Ok(())
}

fn clone_target(pos: &[&str], repo: &str, cwd: &str, ctx: &Ctx) -> String {
    match pos.get(1) {
        Some(p) => norm(p, cwd, &ctx.home),
        None => join(cwd, repo),
    }
}

const GIT_CLONE_VALUE_OPTS: [&str; 20] = [
    "-o",
    "--origin",
    "-b",
    "--branch",
    "-u",
    "--upload-pack",
    "--reference",
    "--reference-if-able",
    "--separate-git-dir",
    "--depth",
    "--shallow-since",
    "--shallow-exclude",
    "-c",
    "--config",
    "-j",
    "--jobs",
    "--template",
    "--filter",
    "--server-option",
    "--bundle-uri",
];

fn check_git_clone(args: &[String], cwd: &str, ctx: &Ctx) -> Result<(), Deny> {
    let pos = positional(args, &GIT_CLONE_VALUE_OPTS);
    let Some((owner, repo)) = pos.first().and_then(|p| url::github(p)) else {
        return Ok(());
    };
    let target = clone_target(&pos, &repo, cwd, ctx);
    check_clone_target(&owner, &repo, &target, ctx)
}

fn check_gh_clone(args: &[String], cwd: &str, ctx: &Ctx) -> Result<(), Deny> {
    let args = args
        .iter()
        .position(|a| a == "--")
        .map_or(args, |i| &args[..i]);
    let pos = positional(args, &["-u", "--upstream-remote-name"]);
    // A bare repo name means the signed-in user's repo: finding that user takes
    // the network, so the guard lets it through.
    let Some((owner, repo)) = pos
        .first()
        .and_then(|s| url::github(s).or_else(|| url::short_spec(s)))
    else {
        return Ok(());
    };
    let repo = repo.strip_suffix(".git").unwrap_or(&repo).to_string();
    let target = clone_target(&pos, &repo, cwd, ctx);
    check_clone_target(&owner, &repo, &target, ctx)
}

/// stdout of one git call in `cwd`, or `None` when git cannot run or is too slow.
fn git_stdout(ctx: &Ctx, cwd: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(&ctx.git)
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    let bytes = rx.recv_timeout(ctx.timeout).ok();
    if bytes.is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    bytes.map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

/// The (owner, repo) of the repository that contains `cwd`, from its main worktree path.
fn repo_identity(cwd: &str, ctx: &Ctx) -> Option<(String, String)> {
    let common = git_stdout(
        ctx,
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if common.is_empty() {
        return None;
    }
    let main = if basename(&common) == ".git" {
        dirname(&common)
    } else {
        common
    };
    let here = process_cwd();
    let rel = relpath(
        &norm(&main, &here, &ctx.home),
        &norm(&ctx.projects, &here, &ctx.home),
    );
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.len() == 2 && !rel.starts_with("..") {
        return Some((parts[0].to_string(), parts[1].to_string()));
    }
    // Repo outside the layout: fall back to the origin URL.
    let url = git_stdout(ctx, cwd, &["remote", "get-url", "origin"])?;
    url::github(&url)
}

fn check_worktree_add(args: &[String], cwd: &str, ctx: &Ctx) -> Result<(), Deny> {
    let pos = positional(args, &["-b", "-B", "--reason", "--orphan"]);
    let Some(first) = pos.first() else {
        return Ok(());
    };
    let Some((owner, repo)) = repo_identity(cwd, ctx) else {
        return Ok(());
    };
    let target = norm(first, cwd, &ctx.home);
    let base = join(&join(&join(&ctx.projects, &owner), "worktrees"), &repo);
    if dirname(&target) != base {
        let name = match basename(&target) {
            "" => "<name>",
            n => n,
        };
        return block(format!("worktrees of {owner}/{repo} go in {base}/<name> (for example {base}/{name}), not {target}."));
    }
    Ok(())
}

fn check_bash(command: &str, cwd: &str, ctx: &Ctx) -> Result<(), Deny> {
    let mut cwd = cwd.to_string();
    for segment in segments_of(command) {
        let argv = strip_env(&segment);
        let Some(program) = argv.first() else {
            continue;
        };
        if program == "cd" {
            match argv.get(1) {
                Some(dir) if dir != "-" => cwd = norm(dir, &cwd, &ctx.home),
                Some(_) => {}
                None => cwd = ctx.home.clone(),
            }
            continue;
        }
        let program = basename(program);
        if ["bash", "sh", "zsh"].contains(&program)
            && argv.len() > 2
            && ["-c", "-lc", "-ic"].contains(&argv[1].as_str())
        {
            check_bash(&argv[2], &cwd, ctx)?;
        } else if program == "git" {
            let (rest, gcwd) = git_global_opts(argv, &cwd, ctx);
            if rest.first().is_some_and(|a| a == "clone") {
                check_git_clone(&rest[1..], &gcwd, ctx)?;
            } else if rest.len() >= 2 && rest[0] == "worktree" && rest[1] == "add" {
                check_worktree_add(&rest[2..], &gcwd, ctx)?;
            }
        } else if program == "gh" && argv.len() >= 3 && argv[1] == "repo" && argv[2] == "clone" {
            check_gh_clone(&argv[3..], &cwd, ctx)?;
        }
    }
    Ok(())
}

/// The hook's own working directory.
fn process_cwd() -> String {
    std::env::current_dir()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Python truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// `Ok(Err(deny))` is a decision; `Err(())` is an error of the guard's own, which allows the call.
fn decide(data: &Value, ctx: &Ctx) -> Result<Result<(), Deny>, ()> {
    let data = data.as_object().ok_or(())?;
    let tool = data.get("tool_name").and_then(Value::as_str).unwrap_or("");
    let empty = Value::Object(Default::default());
    let tin = match data.get("tool_input") {
        Some(v) if truthy(v) => v,
        _ => &empty,
    };
    let tin = tin.as_object().ok_or(())?;
    if tool == "EnterWorktree" {
        if !tin.get("path").is_some_and(truthy) {
            return Ok(block(
                "EnterWorktree with `name` creates .claude/worktrees/ inside the repo. Run `git worktree add \
                 ~/Projects/<owner>/worktrees/<repo>/<name> -b <branch>` from the main repository, then call \
                 EnterWorktree with `path`."
                    .to_string(),
            ));
        }
        return Ok(Ok(()));
    }
    let command = match tin.get("command") {
        Some(Value::Array(items)) => {
            let words: Option<Vec<String>> =
                items.iter().map(|w| w.as_str().map(shell::quote)).collect();
            Some(words.ok_or(())?.join(" "))
        }
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    let Some(command) = command else {
        return Ok(Ok(()));
    };
    let cwd = match data.get("cwd") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(v) if truthy(v) => return Err(()),
        _ => process_cwd(),
    };
    Ok(check_bash(&command, &cwd, ctx))
}

/// The hook's whole behaviour: the JSON line to print for `input`, if any.
pub fn run(input: &str, ctx: &Ctx) -> Option<String> {
    let data: Value = serde_json::from_str(input).ok()?;
    match decide(&data, ctx) {
        Ok(Err(Deny(reason))) => Some(
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format!("projects-layout-guard: {reason}"),
            }})
            .to_string(),
        ),
        // Allowed, or fail open: a bug in the guard never blocks work.
        _ => None,
    }
}

/// What `--help` prints: the hook is run by the agent, so a person sees this only when checking that it starts.
pub const USAGE: &str = "usage: projects-layout-guard < PAYLOAD
A PreToolUse hook: reads the agent's tool call as JSON on stdin and prints a deny
when it clones or adds a worktree outside ~/Projects/<owner>/<repo> and
~/Projects/<owner>/worktrees/<repo>/<name>. It always exits 0.";

/// The command line: `--help` or `-h` prints [`USAGE`]; anything else runs the hook.
pub fn run_cli(
    args: &[String],
    stdin: impl Read,
    mut stdout: impl Write,
    env: impl Fn(&str) -> Option<String>,
) -> i32 {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        let _ = writeln!(stdout, "{USAGE}");
        return 0;
    }
    run_main(stdin, stdout, env)
}

/// The executable: reads the payload from `stdin`, prints a deny to `stdout`,
/// and always returns 0. `env` reads one environment variable.
pub fn run_main(
    mut stdin: impl Read,
    mut stdout: impl Write,
    env: impl Fn(&str) -> Option<String>,
) -> i32 {
    let Some(home) = env("HOME") else { return 0 };
    let home = match home.trim_end_matches('/') {
        "" => "/".to_string(),
        trimmed => trimmed.to_string(),
    };
    let ctx = Ctx {
        projects: join(&home, "Projects"),
        home,
        git: "git".to_string(),
        timeout: GIT_TIMEOUT,
    };
    if env("PROJECTS_LAYOUT_OFF").as_deref() == Some("1")
        || !std::path::Path::new(&ctx.projects).is_dir()
    {
        return 0;
    }
    let mut input = String::new();
    if stdin.read_to_string(&mut input).is_err() {
        return 0;
    }
    if let Some(line) = run(&input, &ctx) {
        let _ = writeln!(stdout, "{line}");
    }
    0
}

#[cfg(test)]
mod tests;
