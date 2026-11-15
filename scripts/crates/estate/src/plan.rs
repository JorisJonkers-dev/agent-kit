//! `estate plan`: plans a pause, rollback or resume of one Project in the
//! JorisJonkers-dev estate.
//!
//! Prints the plan and the exact `gh workflow run` command for the owner to
//! run. It never runs a workflow and never writes anything: its only `gh` call
//! is a GET of the Project's pin file (`gh api`).
//!
//! Exit status: 0 plan printed; 1 refused (reason on stderr, no command
//! printed); 2 bad usage.

use crate::args::{self, Spec};
use crate::proc::{run_capture, which, RunError};
use crate::text::shell_quote;
use crate::Ctx;
use std::collections::HashMap;
use std::time::Duration;

const REPO: &str = "JorisJonkers-dev/estate";
const GH_TIMEOUT: Duration = Duration::from_secs(120);
const PIN_KEY: &str = "event.toolkit.fluxcd.io/version:";
const ANNOTATION_PREFIX: &str = "estate.jorisjonkers.dev/";

const SPEC: Spec = Spec {
    cmd: "plan",
    usage: "usage: estate plan --project P --action pause|rollback|resume [--target V] [--reason R] [--pin V] [--proof-names V | --no-proof]",
    values: &["--project", "--action", "--target", "--reason", "--pin", "--proof-names"],
    required: &["--project", "--action"],
    flags: &["--no-proof"],
    min_positional: 0,
    max_positional: 0,
};

fn is_project(s: &str) -> bool {
    let ok = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let (Some(first), Some(last)) = (s.chars().next(), s.chars().last()) else {
        return false;
    };
    ok(first) && ok(last) && s.chars().all(|c| ok(c) || c == '-')
}

/// `X.Y.Z` with no leading zeros.
fn is_version(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (*p == "0" || !p.starts_with('0'))
        })
}

/// Numeric order without overflow: a longer number is larger, equal lengths compare as text.
fn version_key(version: &str) -> Vec<(usize, &str)> {
    version.split('.').map(|p| (p.len(), p)).collect()
}

fn has_control(s: &str) -> bool {
    s.chars()
        .any(|c| c <= '\x1f' || matches!(c, '\x7f' | '\u{85}' | '\u{2028}' | '\u{2029}'))
}

/// GETs `projects/<project>/source.yaml` from the estate repository, read-only.
fn read_pin_file(project: &str, ctx: &mut Ctx) -> Result<String, String> {
    let Some(gh) = which("gh", &ctx.path) else {
        return Err("gh is not on PATH, so the pin cannot be read. Stop; do not fill the pin in from memory. Install gh or pass --pin from a reading you made yourself.".into());
    };
    let command = vec![
        "api".to_string(),
        format!("repos/{REPO}/contents/projects/{project}/source.yaml"),
        "-H".to_string(),
        "Accept: application/vnd.github.raw".to_string(),
    ];
    let limit = ctx.timeout.unwrap_or(GH_TIMEOUT);
    let result = match run_capture(&gh, &command, limit) {
        Ok(result) => result,
        Err(RunError::Timeout) => {
            return Err(format!(
                "gh did not answer within {} seconds reading projects/{project}/source.yaml. Stop and retry.",
                limit.as_secs_f64()
            ))
        }
        Err(RunError::Spawn(error)) => return Err(format!("could not run gh: {error}. Stop.")),
    };
    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        let detail = if stderr.trim().is_empty() {
            "no detail"
        } else {
            stderr.trim()
        };
        let code = result
            .status
            .code()
            .map_or("none".to_string(), |c| c.to_string());
        return Err(format!(
            "could not read projects/{project}/source.yaml from {REPO} (gh exit {code}: {detail}). Check `gh auth status` and that the repository is reachable. Stop."
        ));
    }
    Ok(String::from_utf8_lossy(&result.stdout).into_owned())
}

/// The pin annotation on a line: `event.toolkit.fluxcd.io/version: "X" # note`.
fn pin_of(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix(PIN_KEY)?;
    let rest = rest.trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix(['"', '\'']).unwrap_or(rest);
    let end = rest
        .find(|c: char| c == '"' || c == '\'' || c == '#' || c.is_whitespace())
        .unwrap_or(rest.len());
    let (pin, tail) = rest.split_at(end);
    let tail = tail
        .strip_prefix(['"', '\''])
        .unwrap_or(tail)
        .trim_start_matches([' ', '\t']);
    (!pin.is_empty() && (tail.is_empty() || tail.starts_with('#'))).then_some(pin)
}

/// An `estate.jorisjonkers.dev/<name>: <value>` annotation on a line.
fn annotation_of(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim_start().strip_prefix(ANNOTATION_PREFIX)?;
    let colon = rest.find(':')?;
    let name = &rest[..colon];
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
        return None;
    }
    let value = rest[colon + 1..].trim_start_matches([' ', '\t']);
    let value = value.strip_prefix(['"', '\'']).unwrap_or(value);
    let value = value.trim_end_matches([' ', '\t']);
    Some((name, value.strip_suffix(['"', '\'']).unwrap_or(value)))
}

/// The pin version and the annotations of a `source.yaml` text.
fn parse_pin_file<'a>(
    text: &'a str,
    project: &str,
) -> Result<(&'a str, HashMap<&'a str, &'a str>), String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let pins: Vec<&str> = lines
        .iter()
        .filter_map(|l| pin_of(l.trim_end_matches('\r')))
        .collect();
    if pins.len() != 1 {
        return Err(format!(
            "expected exactly one event.toolkit.fluxcd.io/version annotation in projects/{project}/source.yaml, found {}.",
            pins.len()
        ));
    }
    let pin = pins[0];
    if !is_version(pin) {
        return Err(format!(
            "the pin '{pin}' in projects/{project}/source.yaml is not a release version X.Y.Z."
        ));
    }
    let annotations = lines
        .iter()
        .filter_map(|l| annotation_of(l.trim_end_matches('\r')))
        .collect();
    Ok((pin, annotations))
}

fn gh_run_command(workflow: &str, pairs: &[(&str, &str)]) -> String {
    let mut parts: Vec<String> = [
        "gh", "workflow", "run", workflow, "--repo", REPO, "--ref", "main",
    ]
    .iter()
    .map(|p| p.to_string())
    .collect();
    for (key, value) in pairs {
        parts.push("-f".into());
        parts.push(format!("{key}={value}"));
    }
    parts
        .iter()
        .map(|p| shell_quote(p))
        .collect::<Vec<_>>()
        .join(" ")
}

fn check_reason(reason: Option<&str>) -> Result<&str, String> {
    match reason {
        Some(r) if !r.trim().is_empty() => {
            if has_control(r) {
                Err("--reason must be one line with no control characters.".into())
            } else {
                Ok(r)
            }
        }
        _ => Err(
            "this action needs --reason (non-blank); the workflow records it on the pin file."
                .into(),
        ),
    }
}

struct Request<'a> {
    project: &'a str,
    action: &'a str,
    target: Option<&'a str>,
    reason: Option<&'a str>,
    pin: Option<&'a str>,
    proof_names: Option<&'a str>,
    no_proof: bool,
}

fn build(req: &Request, ctx: &mut Ctx) -> Result<String, String> {
    let project = req.project;
    if !is_project(project) {
        return Err(format!(
            "'{project}' is not a Project's name (lowercase letters, digits and hyphens)."
        ));
    }
    let given = |v: Option<&str>| v.is_some_and(|s| !s.is_empty());
    if req.action != "rollback" && (given(req.target) || given(req.proof_names) || req.no_proof) {
        return Err(
            "--target, --proof-names and --no-proof only apply to --action rollback.".into(),
        );
    }
    let reason = if req.action == "resume" {
        ""
    } else {
        check_reason(req.reason)?
    };

    let text;
    let (pin, annotations, pin_source, pin_note) = match req.pin {
        Some(pin) => {
            if !is_version(pin) {
                return Err(format!("--pin '{pin}' is not a release version X.Y.Z."));
            }
            (
                pin,
                HashMap::new(),
                "given with --pin (file not read)".to_string(),
                Some("pin given by hand: the pin file was not read, so a recorded rollback is not checked"),
            )
        }
        None => {
            text = read_pin_file(project, ctx)?;
            let (pin, annotations) = parse_pin_file(&text, project)?;
            (
                pin,
                annotations,
                format!("projects/{project}/source.yaml, event.toolkit.fluxcd.io/version"),
                None,
            )
        }
    };

    let mut lines = vec![
        "estate rollback plan (this tool runs nothing; the owner runs the command)".to_string(),
        format!("project:  {project}"),
        format!("action:   {}", req.action),
        format!("pin:      {pin} ({pin_source})"),
    ];
    lines.extend(pin_note.map(String::from));
    let note =
        |key: &str, otherwise: &'static str| annotations.get(key).copied().unwrap_or(otherwise);
    if let Some(by) = annotations.get("paused-by") {
        lines.push(format!(
            "state:    paused by {by} at {}: {}",
            note("paused-at", "?"),
            note("paused-reason", "")
        ));
    }
    if let Some(version) = annotations.get("rollback-version") {
        lines.push(format!("state:    rolled back to {version}"));
    }

    let resume = gh_run_command("resume.yml", &[("project", project)]);
    match req.action {
        "pause" => {
            let command = gh_run_command("pause.yml", &[("project", project), ("reason", reason)]);
            lines.extend([
                String::new(),
                format!("The pin stays at {pin} until a resume. Composition still checks"),
                "the newest fragment and reports what it finds.".into(),
                String::new(),
                "Step 1, run by the owner:".into(),
                format!("  {command}"),
            ]);
        }
        "resume" => {
            if annotations.contains_key("rollback-fragment") {
                lines.push("WARNING: this Project is rolled back. Resuming moves the pin to its NEWEST fragment, so the rolled-back release returns unless the fix has landed. Only resume once it has.".into());
            }
            lines.extend([
                String::new(),
                "Step 1, run by the owner (starts a composition that moves the pin):".into(),
                format!("  {resume}"),
            ]);
        }
        _ => {
            let target = req
                .target
                .filter(|t| !t.is_empty())
                .ok_or("--action rollback needs --target VERSION (the release to go back to).")?;
            if !is_version(target) {
                return Err(format!(
                    "--target '{target}' is not a release version X.Y.Z."
                ));
            }
            if annotations.contains_key("rollback-fragment") {
                return Err(format!(
                    "{project} is already rolled back to {}; resume it first (rollback.yml refuses this too).",
                    note("rollback-version", "?")
                ));
            }
            if version_key(target) >= version_key(pin) {
                return Err(format!(
                    "target {target} is not earlier than the pin {pin}; a rollback goes back, never forward."
                ));
            }
            if req.proof_names.is_some() == req.no_proof {
                return Err(format!(
                    "Migration Proof not stated: pass --proof-names <release the proof names> or --no-proof when the pinned release carries none{}",
                    if req.no_proof { " (not both)" } else { "" }
                ));
            }
            let proof_line = match req.proof_names {
                Some(names) => {
                    if !is_version(names) {
                        return Err(format!("--proof-names '{names}' is not a release version X.Y.Z."));
                    }
                    if target != names {
                        return Err(format!(
                            "target {target} is not the release the pinned release {pin}'s Migration Proof names ({names}); only that one runs against the current schema."
                        ));
                    }
                    format!("Migration Proof: target {target} is the release --proof-names gives: ok")
                }
                None => format!(
                    "Migration Proof: --no-proof says release {pin} carries none; rollback.yml still checks the real lock."
                ),
            };
            let command = gh_run_command(
                "rollback.yml",
                &[
                    ("project", project),
                    ("version", target),
                    ("reason", reason),
                ],
            );
            lines.extend([
                format!("target:   {target} (earlier than the pin {pin}: ok)"),
                proof_line,
                String::new(),
                "Step 1, run by the owner (rollback.yml refuses an unproven target before anything runs):".into(),
                format!("  {command}"),
                String::new(),
                "rollback.yml then records the pause and the target in its own commit, starts".into(),
                "composition, and leaves the Project paused. Its live steps are in rollback.yml.".into(),
                String::new(),
                "Later, only once the fix has landed (resume moves the pin to the newest fragment):".into(),
                format!("  {resume}"),
            ]);
        }
    }
    Ok(lines.join("\n"))
}

pub fn run(argv: &[String], ctx: &mut Ctx) -> i32 {
    let parsed = match args::parse(&SPEC, argv, ctx) {
        Ok(p) => p,
        Err(status) => return status,
    };
    let action = parsed.value("--action").unwrap_or_default();
    if !["pause", "rollback", "resume"].contains(&action) {
        ctx.warn(SPEC.usage);
        ctx.warn(&format!(
            "estate plan: error: argument --action: invalid choice: '{action}' (choose from 'pause', 'rollback', 'resume')"
        ));
        return 2;
    }
    let request = Request {
        project: parsed.value("--project").unwrap_or_default(),
        action,
        target: parsed.value("--target"),
        reason: parsed.value("--reason"),
        pin: parsed.value("--pin"),
        proof_names: parsed.value("--proof-names"),
        no_proof: parsed.flag("--no-proof"),
    };
    match build(&request, ctx) {
        Ok(plan) => {
            ctx.say(&plan);
            0
        }
        Err(reason) => {
            ctx.warn(&format!("REFUSED: {reason}"));
            1
        }
    }
}

#[cfg(test)]
mod tests;
