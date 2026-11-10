//! ticket-sync: plan, and with `--apply` carry out, the changes that bring
//! the GitHub issues in line with docs/specs/gold-band/tickets.yaml.
//!
//! The default is a dry run: it reads the live state, prints the full plan
//! and writes nothing. Closing superseded issues needs `--apply` and
//! `--approve-close` together; a dry run only lists them.

pub mod body;
pub mod client;
pub mod model;
pub mod plan;

use client::{Client, GhClient, ProcessRunner};
use model::{Model, Result};
use std::ffi::OsString;
use std::io::Write;

const USAGE: &str = "usage: ticket-sync [--repo OWNER/REPO] [--tickets FILE] [--spec FILE] [--apply [--approve-close]]\n\
\n\
Dry run by default: prints the plan, changes nothing.\n\
--apply            carry the plan out (creates, edits, sub-issue and blocked-by links)\n\
--approve-close    with --apply, also close the superseded issues\n";

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub repo: String,
    pub tickets: String,
    pub spec: String,
    pub apply: bool,
    pub approve_close: bool,
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Options>> {
    let mut o = Options {
        repo: "JorisJonkers-dev/agent-kit".into(),
        tickets: "docs/specs/gold-band/tickets.yaml".into(),
        spec: "docs/specs/gold-band/spec.md".into(),
        apply: false,
        approve_close: false,
    };
    let mut it = args.into_iter().map(|a| a.to_string_lossy().into_owned());
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--repo" => o.repo = value("--repo")?,
            "--tickets" => o.tickets = value("--tickets")?,
            "--spec" => o.spec = value("--spec")?,
            "--apply" => o.apply = true,
            "--approve-close" => o.approve_close = true,
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if o.approve_close && !o.apply {
        return Err("--approve-close only means something with --apply".into());
    }
    Ok(Some(o))
}

/// Record created issue numbers in the tickets file with line edits, so the
/// rest of the file (order, quoting) is left exactly as it was.
pub fn record_numbers(text: &str, created: &[(String, u64)]) -> Result<String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (key, n) in created {
        let ticket = format!("- key: {key}");
        let branch = format!("  {key}:");
        if let Some(i) = lines.iter().position(|l| *l == ticket) {
            lines.insert(i + 1, format!("  issue: {n}"));
        } else if let Some(i) = lines.iter().position(|l| *l == branch) {
            lines.insert(i + 2, format!("    issue: {n}"));
        } else {
            return Err(format!("cannot record #{n} for {key}: not found in the tickets file"));
        }
    }
    Ok(lines.join("\n") + "\n")
}

fn write_back(o: &Options, text: &str, created: &[(String, u64)]) -> Result<()> {
    if created.is_empty() {
        return Ok(());
    }
    std::fs::write(&o.tickets, record_numbers(text, created)?).map_err(|e| format!("{}: {e}", o.tickets))
}

/// Sync with an injected client; `run` builds the real one.
pub fn sync(o: &Options, client: &mut dyn Client, out: &mut dyn Write) -> Result<()> {
    let text = std::fs::read_to_string(&o.tickets).map_err(|e| format!("{}: {e}", o.tickets))?;
    let spec_text = std::fs::read_to_string(&o.spec).map_err(|e| format!("{}: {e}", o.spec))?;
    let mut model = Model::parse(&text)?;
    let stories = model::stories(&spec_text);
    let p = plan::plan(&model, &stories, client)?;
    let say = |out: &mut dyn Write, s: &str| out.write_all(s.as_bytes()).map_err(|e| e.to_string());
    say(out, &p.render(&model, o.apply && o.approve_close))?;
    if !o.apply {
        return say(out, "dry run: nothing was written\n");
    }
    let mut done = plan::Applied::default();
    let result = plan::apply(&p, &mut model, &stories, client, o.approve_close, &mut done);
    // Record what was created even when a later step failed, or a retry would
    // create the same issues again.
    let written = write_back(o, &text, &done.created);
    result?;
    written?;
    say(
        out,
        &format!(
            "applied: {} actions, {} created, {} closed, {} closes held back\n",
            p.actions.len(),
            done.created.len(),
            done.closed.len(),
            done.skipped_closes
        ),
    )
}

/// The command: returns the exit status. `gh` is the program that talks to
/// GitHub (the tests point it at a script).
pub fn run(args: impl IntoIterator<Item = OsString>, gh: &str, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let result = parse_args(args).and_then(|o| match o {
        None => out.write_all(USAGE.as_bytes()).map_err(|e| e.to_string()),
        Some(o) => {
            let mut client = GhClient::new(&o.repo, Box::new(ProcessRunner::new(gh)));
            sync(&o, &mut client, out)
        }
    });
    match result {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "ticket-sync: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests;
