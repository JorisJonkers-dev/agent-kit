//! The approved close. Closing issues is the one step of the sync that cannot
//! be undone quietly, so it is a separate action behind a recorded approval:
//! `--approve-plan` writes the fingerprint of the close plan the owner has
//! read, and `--close` closes only when the plan it computes now has that same
//! fingerprint. No approval, or a plan that changed since, closes nothing.

use crate::client::Client;
use crate::model::{Model, Result};
use crate::plan::{Action, Plan};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One issue the plan closes, with the comment that maps it to its ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Closing {
    pub number: u64,
    pub comment: String,
    /// Delivered work closes as completed; superseded work as not planned.
    pub completed: bool,
}

/// Dispositions that mean the work was delivered, with evidence.
fn is_delivered(why: &str) -> bool {
    ["done (", "done:", "covered by "].iter().any(|p| why.starts_with(p)) || why == "done"
}

/// The closings of a plan, in plan order. Every comment links the ticket that
/// replaces the issue; a delivered issue's comment also carries its evidence.
pub fn closings(plan: &Plan, model: &Model, dispositions: &BTreeMap<u64, String>) -> Result<Vec<Closing>> {
    let graph: BTreeSet<u64> = std::iter::once(model.spec).chain(model.keys().iter().filter_map(|k| model.number(k))).collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for action in &plan.actions {
        let Action::Close { number, by } = action else { continue };
        if graph.contains(number) {
            return Err(format!("#{number} is part of the ticket graph and cannot be closed as superseded"));
        }
        let ticket = model.number(by).ok_or_else(|| format!("#{number}: its ticket {by} is not published yet"))?;
        if !seen.insert(*number) {
            continue;
        }
        let why = dispositions.get(number).map(String::as_str).unwrap_or("");
        let completed = is_delivered(why);
        let comment = if completed {
            format!("Delivered: {why}. Tracked by #{ticket} ({by}) of spec #{}.", model.spec)
        } else {
            let note = if why.is_empty() || why.ends_with("superseded by this spec") { String::new() } else { format!(" ({why})") };
            format!("Superseded by #{ticket} ({by}) of spec #{}{note}.", model.spec)
        };
        out.push(Closing { number: *number, comment, completed });
    }
    Ok(out)
}

/// A stable fingerprint of what would be closed and said (FNV-1a, 64 bit).
/// It detects a plan that changed after it was read; it is not a secret.
/// It covers the repository too, and every field is length-prefixed so that
/// two different plans cannot produce the same byte stream.
pub fn fingerprint(repo: &str, closings: &[Closing]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    let mut feed = |s: String| {
        for byte in s.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
        }
    };
    feed(format!("{}:{repo};", repo.len()));
    for c in closings {
        feed(format!("{};{};{}:{};", c.number, c.completed, c.comment.len(), c.comment));
    }
    format!("{hash:016x}")
}

pub fn render(repo: &str, closings: &[Closing]) -> String {
    let mut out = String::new();
    for c in closings {
        let how = if c.completed { "completed" } else { "not planned" };
        out.push_str(&format!("close     #{} ({how}): {}\n", c.number, c.comment));
    }
    out.push_str(&format!("close plan for {repo}: {} issues, fingerprint {}\n", closings.len(), fingerprint(repo, closings)));
    out
}

#[derive(Serialize, Deserialize)]
struct Approval {
    repo: String,
    fingerprint: String,
    /// For the reader of the file; the fingerprint is what is enforced.
    issues: Vec<u64>,
}

/// Record the owner's approval of exactly this close plan for this repository.
pub fn approve(path: &Path, repo: &str, closings: &[Closing]) -> Result<()> {
    if closings.is_empty() {
        return Err("the close plan is empty: there is nothing to approve".into());
    }
    let approval = Approval { repo: repo.to_string(), fingerprint: fingerprint(repo, closings), issues: closings.iter().map(|c| c.number).collect() };
    let dir = path.parent().unwrap_or(Path::new(""));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let text = serde_json::to_string_pretty(&approval).expect("an approval serialises") + "\n";
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Close what the plan lists, but only under an approval of this same plan in
/// this same repository. `done` collects the numbers closed. Nothing is
/// touched when the check fails. A fully carried-out close uses the approval
/// up: a later plan, such as a reopened issue, needs a new one.
pub fn close(path: &Path, repo: &str, closings: &[Closing], client: &mut dyn Client, done: &mut Vec<u64>) -> Result<()> {
    let text = std::fs::read_to_string(path).map_err(|_| format!("no approval recorded at {}: run --approve-plan first", path.display()))?;
    let approval: Approval = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let now = fingerprint(repo, closings);
    if approval.repo != repo || approval.fingerprint != now {
        return Err(format!(
            "the recorded approval ({} for {}) is not for the current close plan ({now} for {repo}); read the plan and approve it again",
            approval.fingerprint, approval.repo
        ));
    }
    for c in closings {
        client.close(c.number, &c.comment, c.completed)?;
        done.push(c.number);
    }
    std::fs::remove_file(path).map_err(|e| format!("closed {} issues, but the approval at {} could not be used up: {e}; remove it before another close", done.len(), path.display()))
}
