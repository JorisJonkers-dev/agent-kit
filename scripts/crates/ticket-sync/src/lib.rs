//! ticket-sync: plan, and with `--apply` carry out, the changes that bring
//! the GitHub issues in line with docs/specs/gold-band/tickets.yaml.
//!
//! The default is a dry run: it reads the live state, prints the full plan
//! and writes nothing. `--apply` never closes. Closing the superseded and
//! delivered issues is its own action, `--close`, and it closes nothing unless
//! `--approve-plan` recorded an approval of the same close plan first.

pub mod body;
pub mod client;
pub mod close;
pub mod model;
pub mod plan;

use client::{Client, GhClient, ProcessRunner};
use model::{Model, Result};
use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

const USAGE: &str = "usage: ticket-sync [--repo OWNER/REPO] [--tickets FILE] [--spec FILE] [--approval FILE] [--apply | --approve-plan | --close]\n\
\n\
Dry run by default: prints the plan, changes nothing.\n\
--apply            carry the plan out (creates, edits, sub-issue and blocked-by links); never closes\n\
--approve-plan     record the owner's approval of the close plan as printed (writes the approval file only)\n\
--close            close the issues in the close plan, each with its mapping comment; refuses (exit 1,\n\
                   closing nothing) unless the recorded approval is for the current close plan\n\
--approval FILE    where the approval is kept (default: $XDG_STATE_HOME/agent-kit/ticket-sync-close-approval)\n";

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub repo: String,
    pub tickets: String,
    pub spec: String,
    pub approval: String,
    pub mode: Mode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    DryRun,
    Apply,
    ApprovePlan,
    Close,
}

pub fn approval_path(xdg_state_home: Option<String>, home: Option<String>) -> String {
    let state = xdg_state_home.filter(|s| !s.is_empty()).unwrap_or_else(|| format!("{}/.local/state", home.unwrap_or_default()));
    format!("{state}/agent-kit/ticket-sync-close-approval")
}

pub fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Option<Options>> {
    let mut o = Options {
        repo: "JorisJonkers-dev/agent-kit".into(),
        tickets: "docs/specs/gold-band/tickets.yaml".into(),
        spec: "docs/specs/gold-band/spec.md".into(),
        approval: approval_path(std::env::var("XDG_STATE_HOME").ok(), std::env::var("HOME").ok()),
        mode: Mode::DryRun,
    };
    let mut it = args.into_iter().map(|a| a.to_string_lossy().into_owned());
    while let Some(a) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--repo" => o.repo = value("--repo")?,
            "--tickets" => o.tickets = value("--tickets")?,
            "--spec" => o.spec = value("--spec")?,
            "--approval" => o.approval = value("--approval")?,
            "--apply" | "--approve-plan" | "--close" => {
                if o.mode != Mode::DryRun {
                    return Err("--apply, --approve-plan and --close are mutually exclusive".into());
                }
                o.mode = match a.as_str() {
                    "--apply" => Mode::Apply,
                    "--approve-plan" => Mode::ApprovePlan,
                    _ => Mode::Close,
                };
            }
            "-h" | "--help" => return Ok(None),
            other => return Err(format!("unknown argument {other}")),
        }
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
    match o.mode {
        Mode::DryRun => {
            say(out, &p.render(&model))?;
            let closings = close::closings(&p, &model, &model::dispositions(&spec_text))?;
            say(out, &close::render(&o.repo, &closings))?;
            say(out, "dry run: nothing was written\n")
        }
        Mode::ApprovePlan => {
            let closings = close::closings(&p, &model, &model::dispositions(&spec_text))?;
            say(out, &close::render(&o.repo, &closings))?;
            close::approve(Path::new(&o.approval), &o.repo, &closings)?;
            say(out, &format!("approval recorded in {}\n", o.approval))
        }
        Mode::Close => {
            let closings = close::closings(&p, &model, &model::dispositions(&spec_text))?;
            let mut closed = Vec::new();
            let result = close::close(Path::new(&o.approval), &o.repo, &closings, client, &mut closed);
            for n in &closed {
                say(out, &format!("closed #{n}\n"))?;
            }
            result?;
            say(out, &format!("closed {} issues\n", closed.len()))
        }
        Mode::Apply => {
            say(out, &p.render(&model))?;
            let mut done = plan::Applied::default();
            let result = plan::apply(&p, &mut model, &stories, client, &mut done);
            // Record what was created even when a later step failed, or a retry would
            // create the same issues again.
            let written = write_back(o, &text, &done.created);
            result?;
            written?;
            say(out, &format!("applied: {} actions, {} created, {} closes left for --close\n", p.actions.len(), done.created.len(), done.skipped_closes))
        }
    }
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
