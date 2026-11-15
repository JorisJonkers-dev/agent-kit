//! `estate intent`: scaffolds and validates a deploy-kit 1.x Project Intent.
//!
//! ```text
//! estate intent scaffold --project NAME --owner OWNER --host HOST --audience AUDIENCE \
//!     --image IMAGE --process PROCESS --port N --out DIR
//! estate intent validate PATH [EXTRA_PATH ...]
//! ```
//!
//! `scaffold` writes `DIR/deploy/<project>.project.yml` in the shape of the
//! minimal example, plus `DIR/deploy/env/<process>/base.env`. Lines marked
//! CHECK are the author's choices; read the live spec before trusting them.
//!
//! `validate` runs `deploy-kit validate PATH [EXTRA_PATH ...]` and passes its
//! output through unchanged. Exit status mirrors deploy-kit: 0 accepted, 1
//! refused, 2 called wrongly (including deploy-kit not installed).

use crate::args::{self, Spec};
use crate::proc::{run_capture, which, RunError};
use crate::Ctx;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

const AUDIENCES: [&str; 4] = ["anonymous", "authenticated", "internal", "lan"];
const VALIDATE_TIMEOUT: Duration = Duration::from_secs(300);

const SCAFFOLD: Spec = Spec {
    cmd: "intent scaffold",
    usage: "usage: estate intent scaffold --project NAME --owner OWNER --host HOST --audience AUDIENCE --image IMAGE --process PROCESS --port N --out DIR",
    values: &["--project", "--owner", "--host", "--audience", "--image", "--process", "--out", "--port"],
    required: &["--project", "--owner", "--host", "--audience", "--image", "--process", "--out", "--port"],
    flags: &[],
    min_positional: 0,
    max_positional: 0,
};

const VALIDATE: Spec = Spec {
    cmd: "intent validate",
    usage: "usage: estate intent validate PATH [EXTRA_PATH ...]",
    values: &[],
    required: &[],
    flags: &[],
    min_positional: 1,
    max_positional: usize::MAX,
};

const INTENT_USAGE: &str = "usage: estate intent {scaffold,validate} ...";

fn lower_alnum(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

/// `^[a-z]([-a-z0-9]{0,61}[a-z0-9])?$`
fn dns_label(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let (Some(first), Some(last)) = (chars.first(), chars.last()) else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.len() <= 63
        && lower_alnum(*last)
        && chars.iter().all(|&c| lower_alnum(c) || c == '-')
}

/// `[a-z0-9]([-a-z0-9]{0,61}[a-z0-9])?`, one DNS name part.
fn host_part(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let (Some(first), Some(last)) = (chars.first(), chars.last()) else {
        return false;
    };
    lower_alnum(*first)
        && chars.len() <= 63
        && lower_alnum(*last)
        && chars.iter().all(|&c| lower_alnum(c) || c == '-')
}

/// A lowercase DNS name of 1 to 253 characters.
fn host(s: &str) -> bool {
    s.chars().count() <= 253 && s.split('.').all(host_part)
}

/// `^[A-Za-z0-9][-A-Za-z0-9_.]{0,62}$`
fn owner(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars().count() <= 63
        && chars.all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

/// `^[a-z0-9]([a-z0-9._/-]{0,198}[a-z0-9])?$`
fn image(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    let (Some(first), Some(last)) = (chars.first(), chars.last()) else {
        return false;
    };
    lower_alnum(*first)
        && chars.len() <= 200
        && lower_alnum(*last)
        && chars.iter().all(|&c| lower_alnum(c) || "._/-".contains(c))
}

/// Creates `path` exclusively, so an existing file is never overwritten.
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(text.as_bytes())
}

struct Values<'a> {
    project: &'a str,
    owner: &'a str,
    host: &'a str,
    audience: &'a str,
    image: &'a str,
    process: &'a str,
    port: i64,
}

fn project_text(v: &Values) -> String {
    let Values {
        project,
        owner,
        host,
        audience,
        image,
        process,
        port,
    } = v;
    format!(
        r"# Project Intent for {project}. See spec/v1/10-project-intent.md.
# Lines marked CHECK are defaults this skill chose; confirm each against the
# live spec before validating.

apiVersion: intent.jorisjonkers.dev/v1
kind: Project
schemaVersion: 1.0.0

project: {project}
owner: {owner}

applications:
  - id: {project}
    exposure:
      - name: public
        host: {host}
        audience: {audience}
        routes:
          - {{ path: /, match: prefix, process: {process}, surface: http }}

    processes:
      - name: {process}
        lifecycle: application
        image: {image}
        runtime: node    # CHECK

        provides:
          http: {port}

        placement:
          memory: 256Mi  # CHECK
          cpu: 50m       # CHECK

        probes:
          readiness: {{ path: /healthz/ready, port: {port} }}  # CHECK
          liveness: {{ path: /healthz/live, port: {port} }}    # CHECK

        startupBudget: 20s   # CHECK
        cutover: continuous  # CHECK
"
    )
}

/// The first value that fails its rule, as the message to print.
fn refusal(v: &Values) -> Option<String> {
    for (label, value) in [("--project", v.project), ("--process", v.process)] {
        if !dns_label(value) {
            return Some(format!(
                "{label} must be a DNS-1123 label, got {}",
                repr(value)
            ));
        }
    }
    type Rule<'a> = (&'a str, &'a str, fn(&str) -> bool, &'a str);
    let rules: [Rule; 3] = [
        ("--host", v.host, host, "a lowercase DNS name"),
        (
            "--owner",
            v.owner,
            owner,
            "letters, digits, '-', '_' or '.'",
        ),
        (
            "--image",
            v.image,
            image,
            "an image alias of lowercase letters, digits, '.', '_', '/' or '-'",
        ),
    ];
    for (label, value, rule, what) in rules {
        if !rule(value) {
            return Some(format!("{label} must be {what}, got {}", repr(value)));
        }
    }
    if !AUDIENCES.contains(&v.audience) {
        return Some(format!(
            "--audience must be one of {}",
            AUDIENCES.join(", ")
        ));
    }
    if !(1..=65535).contains(&v.port) {
        return Some("--port must be 1-65535".into());
    }
    None
}

/// Python's `repr` of a string, enough for the values shown in messages.
fn repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

fn scaffold(argv: &[String], ctx: &mut Ctx) -> i32 {
    let parsed = match args::parse(&SCAFFOLD, argv, ctx) {
        Ok(p) => p,
        Err(status) => return status,
    };
    let get = |name: &str| parsed.value(name).unwrap_or_default();
    let Ok(port) = get("--port").trim().parse::<i64>() else {
        ctx.warn(SCAFFOLD.usage);
        ctx.warn(&format!(
            "estate intent scaffold: error: argument --port: invalid int value: {}",
            repr(get("--port"))
        ));
        return 2;
    };
    let values = Values {
        project: get("--project"),
        owner: get("--owner"),
        host: get("--host"),
        audience: get("--audience"),
        image: get("--image"),
        process: get("--process"),
        port,
    };
    if let Some(message) = refusal(&values) {
        ctx.warn(&format!("estate intent: {message}"));
        return 2;
    }
    let out = Path::new(get("--out"));
    let project_file = out
        .join("deploy")
        .join(format!("{}.project.yml", values.project));
    let env_file = out
        .join("deploy")
        .join("env")
        .join(values.process)
        .join("base.env");
    let existing: Vec<_> = [&project_file, &env_file]
        .into_iter()
        .filter(|p| p.exists())
        .collect();
    if !existing.is_empty() {
        for p in existing {
            ctx.warn(&format!(
                "estate intent: refusing to overwrite {}",
                p.display()
            ));
        }
        return 2;
    }
    let env_text = format!(
        "# Environment for the {} Process. See spec/v1/10-project-intent.md.\n",
        values.process
    );
    let written = (|| {
        for (file, text) in [
            (&project_file, project_text(&values)),
            (&env_file, env_text),
        ] {
            file.parent().map_or(Ok(()), std::fs::create_dir_all)?;
            write_new(file, &text)?;
        }
        Ok::<(), std::io::Error>(())
    })();
    if let Err(error) = written {
        ctx.warn(&format!(
            "estate intent: cannot write the scaffold: {error}"
        ));
        return 2;
    }
    ctx.say(&format!("wrote {}", project_file.display()));
    ctx.say(&format!("wrote {}", env_file.display()));
    0
}

fn validate(argv: &[String], ctx: &mut Ctx) -> i32 {
    let parsed = match args::parse(&VALIDATE, argv, ctx) {
        Ok(p) => p,
        Err(status) => return status,
    };
    for path in &parsed.positional {
        if !Path::new(path).exists() {
            ctx.warn(&format!("estate intent: no such path: {path}"));
            return 2;
        }
    }
    let Some(tool) = which("deploy-kit", &ctx.path) else {
        ctx.warn(
            "estate intent: deploy-kit is not installed on PATH; install the publish version (and the repository's pin, when different) and rerun. Nothing was validated.",
        );
        return 2;
    };
    let mut command = vec!["validate".to_string()];
    command.extend(parsed.positional.iter().cloned());
    let limit = ctx.timeout.unwrap_or(VALIDATE_TIMEOUT);
    // deploy-kit's own output is passed through as it was printed.
    let result = match run_capture(&tool, &command, limit) {
        Ok(result) => result,
        Err(RunError::Timeout) => {
            ctx.warn(&format!(
                "estate intent: deploy-kit did not finish within {}s. Nothing was validated.",
                limit.as_secs_f64()
            ));
            return 2;
        }
        Err(RunError::Spawn(error)) => {
            ctx.warn(&format!(
                "estate intent: cannot run deploy-kit: {error}. Nothing was validated."
            ));
            return 2;
        }
    };
    let _ = ctx.out.write_all(&result.stdout);
    let _ = ctx.err.write_all(&result.stderr);
    let code = exit_code(&result.status);
    ctx.say(&match code {
        0 => "deploy-kit validate: accepted (exit 0)".to_string(),
        1 => {
            "deploy-kit validate: REFUSED (exit 1). Report the output above to the owner verbatim."
                .to_string()
        }
        2 => "deploy-kit validate: called wrongly (exit 2). Check the paths and arguments."
            .to_string(),
        n => format!("deploy-kit validate: unexpected exit {n}. Report it to the owner verbatim."),
    });
    if (0..=2).contains(&code) {
        code
    } else {
        2
    }
}

/// The exit code, or the negated signal number for a killed process (as Python reports it).
fn exit_code(status: &std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .or_else(|| status.signal().map(|s| -s))
        .unwrap_or(-1)
}

pub fn run(argv: &[String], ctx: &mut Ctx) -> i32 {
    let rest = argv.get(1..).unwrap_or(&[]);
    match argv.first().map(String::as_str) {
        Some("scaffold") => scaffold(rest, ctx),
        Some("validate") => validate(rest, ctx),
        Some("--help" | "-h") => {
            ctx.say(INTENT_USAGE);
            0
        }
        _ => {
            ctx.warn(INTENT_USAGE);
            2
        }
    }
}

#[cfg(test)]
mod tests;
