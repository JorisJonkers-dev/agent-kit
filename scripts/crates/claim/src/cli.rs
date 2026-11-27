//! The setup canary crate: the Rust workspace's proof of path.
//!
//! The standards repo's ADR-0014 puts all of agent-kit's Rust in one workspace
//! under `scripts/`. This member is deliberately trivial and std-only, with no
//! dependency of any kind.
//! `--version` proves the build end to end; the command also validates a path.
//! New Rust joins this workspace as a new member, never as its own Cargo
//! project.
//!
//! The command line for claims: argument errors earn exit 2 with usage on
//! stderr, runtime errors earn exit 1 with `claim: <message>` on stderr.

use std::io::Write;

use crate::gh::{is_slug, Gh};
use crate::lease::{format_time, Clock, FixedClock};
use crate::store::{Git, Outcome, Store};

pub(crate) const USAGE: &str = "usage: claim acquire <repo> <key> --issue <n> [--lease-seconds S] [--device D] [--run R] [--owner O] [--label L]\n       claim renew <repo> <key> [--lease-seconds S] [--device D] [--run R]\n       claim release <repo> <key> --issue <n> [--label L] [--comment TEXT]\n       claim list <repo>\n";

const ACQUIRE_FLAGS: &[&str] = &["issue", "lease-seconds", "device", "run", "owner", "label"];
const RENEW_FLAGS: &[&str] = &["lease-seconds", "device", "run"];
const RELEASE_FLAGS: &[&str] = &["issue", "label", "comment"];

#[derive(Default)]
struct Flags {
    issue: Option<String>,
    lease_seconds: Option<String>,
    device: Option<String>,
    run: Option<String>,
    owner: Option<String>,
    label: Option<String>,
    comment: Option<String>,
}

/// Answers one command line whose first argument is the command itself.
pub fn run_with(
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    git: &dyn Git,
    gh: &dyn Gh,
    clock: &dyn Clock,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (command, rest) = match args.split_first() {
        Some(parts) => parts,
        None => return bad_usage(err),
    };
    match command.as_str() {
        "acquire" => acquire(rest, env, git, gh, clock, out, err),
        "renew" => renew(rest, env, git, clock, out, err),
        "release" => release(rest, git, gh, clock, out, err),
        "list" => list(rest, git, clock, out, err),
        _ => bad_usage(err),
    }
}

fn bad_usage(err: &mut dyn Write) -> i32 {
    let _ = err.write_all(USAGE.as_bytes());
    2
}

fn runtime_error(err: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(err, "claim: {message}");
    1
}

fn url_for(repo: &str) -> String {
    if is_slug(repo) {
        format!("https://github.com/{repo}.git")
    } else {
        repo.to_string()
    }
}

fn call_gh(gh: &dyn Gh, args: &[&str], err: &mut dyn Write) -> Option<i32> {
    match gh.gh(args) {
        Ok(()) => None,
        Err(message) => Some(runtime_error(err, &message)),
    }
}

fn parse_flags(rest: &[String], allowed: &[&str]) -> Result<(Vec<String>, Flags), ()> {
    let mut positional = Vec::new();
    let mut flags = Flags::default();
    let mut i = 0;
    while i < rest.len() {
        let arg = &rest[i];
        if let Some(name) = arg.strip_prefix("--") {
            let value = rest
                .get(i + 1)
                .filter(|value| !value.starts_with("--"))
                .ok_or(())?;
            match name {
                "issue" if allowed.contains(&"issue") => flags.issue = Some(value.clone()),
                "lease-seconds" if allowed.contains(&"lease-seconds") => {
                    flags.lease_seconds = Some(value.clone());
                }
                "device" if allowed.contains(&"device") => flags.device = Some(value.clone()),
                "run" if allowed.contains(&"run") => flags.run = Some(value.clone()),
                "owner" if allowed.contains(&"owner") => flags.owner = Some(value.clone()),
                "label" if allowed.contains(&"label") => flags.label = Some(value.clone()),
                "comment" if allowed.contains(&"comment") => flags.comment = Some(value.clone()),
                _ => return Err(()),
            }
            i += 2;
        } else {
            positional.push(arg.clone());
            i += 1;
        }
    }
    Ok((positional, flags))
}

fn two_positionals(positional: Vec<String>) -> Result<(String, String), ()> {
    let mut values = positional.into_iter();
    let repo = values.next().ok_or(())?;
    let key = values.next().ok_or(())?;
    if values.next().is_some() {
        return Err(());
    }
    Ok((repo, key))
}

fn one_positional(positional: Vec<String>) -> Result<String, ()> {
    let mut values = positional.into_iter();
    let repo = values.next().ok_or(())?;
    if values.next().is_some() {
        return Err(());
    }
    Ok(repo)
}

fn parse_number(value: &str) -> Option<u64> {
    value.parse::<u64>().ok()
}

fn acquire(
    rest: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    git: &dyn Git,
    gh: &dyn Gh,
    clock: &dyn Clock,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positional, flags) = match parse_flags(rest, ACQUIRE_FLAGS) {
        Ok(parsed) => parsed,
        Err(()) => return bad_usage(err),
    };
    let (repo, key) = match two_positionals(positional) {
        Ok(positionals) => positionals,
        Err(()) => return bad_usage(err),
    };
    let issue_text = match flags.issue {
        Some(value) => value,
        None => return bad_usage(err),
    };
    let issue = match parse_number(&issue_text) {
        Some(number) => number,
        None => return bad_usage(err),
    };
    let lease_seconds = match flags.lease_seconds {
        Some(value) => match parse_number(&value) {
            Some(number) => number,
            None => return bad_usage(err),
        },
        None => 3600,
    };
    let device = match flags.device {
        Some(value) => value,
        None => env("HOSTNAME").unwrap_or_else(|| "unknown".to_string()),
    };
    let now = clock.now();
    let fixed_clock = FixedClock(now);
    let run = match flags.run {
        Some(value) => value,
        None => match env("CLAIM_RUN") {
            Some(value) => value,
            None => now.to_string(),
        },
    };
    let owner = flags.owner.unwrap_or_else(|| "ExtraToast".to_string());
    let label = flags
        .label
        .unwrap_or_else(|| "status: in-progress".to_string());
    let slug = is_slug(&repo);
    let store = Store {
        git,
        clock: &fixed_clock,
        url: url_for(&repo),
    };
    match store.acquire(&key, &device, &run, issue, lease_seconds) {
        Ok(Outcome::Won(sha)) => {
            let _ = writeln!(out, "{sha}");
            if slug {
                let until = format_time(now + lease_seconds);
                let body = format!("Claimed by {device}, run {run}, lease until {until}");
                if let Some(code) = call_gh(
                    gh,
                    &[
                        "issue",
                        "edit",
                        &issue_text,
                        "--repo",
                        &repo,
                        "--add-assignee",
                        &owner,
                        "--add-label",
                        &label,
                    ],
                    err,
                ) {
                    return code;
                }
                if let Some(code) = call_gh(
                    gh,
                    &[
                        "issue",
                        "comment",
                        &issue_text,
                        "--repo",
                        &repo,
                        "--body",
                        &body,
                    ],
                    err,
                ) {
                    return code;
                }
            }
            0
        }
        Ok(Outcome::Lost(lease)) => {
            let _ = writeln!(
                err,
                "held by {} run {} until {}",
                lease.device,
                lease.run,
                format_time(lease.expires)
            );
            1
        }
        Err(message) => runtime_error(err, &message),
    }
}

fn renew(
    rest: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    git: &dyn Git,
    clock: &dyn Clock,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positional, flags) = match parse_flags(rest, RENEW_FLAGS) {
        Ok(parsed) => parsed,
        Err(()) => return bad_usage(err),
    };
    let (repo, key) = match two_positionals(positional) {
        Ok(positionals) => positionals,
        Err(()) => return bad_usage(err),
    };
    let lease_seconds = match flags.lease_seconds {
        Some(value) => match parse_number(&value) {
            Some(number) => number,
            None => return bad_usage(err),
        },
        None => 3600,
    };
    let device = match flags.device {
        Some(value) => value,
        None => env("HOSTNAME").unwrap_or_else(|| "unknown".to_string()),
    };
    let run = match flags.run {
        Some(value) => value,
        None => match env("CLAIM_RUN") {
            Some(value) => value,
            None => clock.now().to_string(),
        },
    };
    let store = Store {
        git,
        clock,
        url: url_for(&repo),
    };
    match store.renew(&key, &device, &run, lease_seconds) {
        Ok(sha) => {
            let _ = writeln!(out, "{sha}");
            0
        }
        Err(message) => runtime_error(err, &message),
    }
}

fn release(
    rest: &[String],
    git: &dyn Git,
    gh: &dyn Gh,
    clock: &dyn Clock,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positional, flags) = match parse_flags(rest, RELEASE_FLAGS) {
        Ok(parsed) => parsed,
        Err(()) => return bad_usage(err),
    };
    let (repo, key) = match two_positionals(positional) {
        Ok(positionals) => positionals,
        Err(()) => return bad_usage(err),
    };
    let issue_text = match flags.issue {
        Some(value) => value,
        None => return bad_usage(err),
    };
    if parse_number(&issue_text).is_none() {
        return bad_usage(err);
    }
    let label = flags
        .label
        .unwrap_or_else(|| "status: in-progress".to_string());
    let store = Store {
        git,
        clock,
        url: url_for(&repo),
    };
    match store.release(&key) {
        Ok(_) => {
            let _ = out;
            if is_slug(&repo) {
                if let Some(code) = call_gh(
                    gh,
                    &[
                        "issue",
                        "edit",
                        &issue_text,
                        "--repo",
                        &repo,
                        "--remove-label",
                        &label,
                    ],
                    err,
                ) {
                    return code;
                }
                if let Some(comment) = flags.comment {
                    if let Some(code) = call_gh(
                        gh,
                        &[
                            "issue",
                            "comment",
                            &issue_text,
                            "--repo",
                            &repo,
                            "--body",
                            &comment,
                        ],
                        err,
                    ) {
                        return code;
                    }
                }
            }
            0
        }
        Err(message) => runtime_error(err, &message),
    }
}

fn list(
    rest: &[String],
    git: &dyn Git,
    clock: &dyn Clock,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (positional, _) = match parse_flags(rest, &[]) {
        Ok(parsed) => parsed,
        Err(()) => return bad_usage(err),
    };
    let repo = match one_positional(positional) {
        Ok(repo) => repo,
        Err(()) => return bad_usage(err),
    };
    let store = Store {
        git,
        clock,
        url: url_for(&repo),
    };
    match store.list() {
        Ok(claims) => {
            let now = clock.now();
            for (key, lease) in claims {
                let state = if lease.expired(now) {
                    "expired"
                } else {
                    "active"
                };
                let _ = writeln!(
                    out,
                    "{} device={} run={} issue={} expires={} {}",
                    key,
                    lease.device,
                    lease.run,
                    lease.issue,
                    format_time(lease.expires),
                    state
                );
            }
            0
        }
        Err(message) => runtime_error(err, &message),
    }
}
