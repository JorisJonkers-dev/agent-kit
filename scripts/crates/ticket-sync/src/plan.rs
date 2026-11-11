//! Compare the graph with the live issues and list what would change; then
//! carry the list out. Planning only reads. Applying runs the actions in a
//! fixed order: creates, edits, unlinks, unblocks, links, blocks. Closes are listed
//! last but only carried out by the approved close action.

use crate::body::{body, labels, normalize, title};
use crate::client::{Client, Link};
use crate::model::{Model, Result, SPEC_KEY};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create { key: String },
    Retitle { key: String, number: u64, from: String, to: String },
    Rebody { key: String, number: u64 },
    Unlink { parent: String, child: Link, child_key: String },
    Link { parent: String, child: String },
    Unblock { key: String, blocker: Link, blocker_key: String },
    Block { key: String, blocker: String },
    /// Close a superseded issue with a pointer to its replacement.
    Close { number: u64, by: String },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub actions: Vec<Action>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn render(&self, model: &Model) -> String {
        let n = |k: &str| model.number(k).map_or_else(|| "(new)".to_string(), |n| format!("#{n}"));
        let mut out = String::new();
        for a in &self.actions {
            out.push_str(&match a {
                Action::Create { key } => format!("create    {key}: {}\n", title(model, key)),
                Action::Retitle { key, number, from, to } => format!("retitle   {key} #{number}: {from:?} -> {to:?}\n"),
                Action::Rebody { key, number } => format!("rebody    {key} #{number}\n"),
                Action::Unlink { parent, child, child_key } => format!("unlink    {child_key} #{} from {parent} {}\n", child.number, n(parent)),
                Action::Link { parent, child } => format!("link      {child} {} under {parent} {}\n", n(child), n(parent)),
                Action::Unblock { key, blocker, blocker_key } => format!("unblock   {key} {} no longer blocked by {blocker_key} #{}\n", n(key), blocker.number),
                Action::Block { key, blocker } => format!("block     {key} {} blocked by {blocker} {}\n", n(key), n(blocker)),
                Action::Close { number, by } => format!("close     #{number} superseded by {by} {}  (needs an approved plan: --approve-plan, then --close)\n", n(by)),
            });
        }
        let count = |f: fn(&Action) -> bool| self.actions.iter().filter(|a| f(a)).count();
        out.push_str(&format!(
            "plan: {} create, {} retitle, {} rebody, {} unlink, {} link, {} unblock, {} block, {} close\n",
            count(|a| matches!(a, Action::Create { .. })),
            count(|a| matches!(a, Action::Retitle { .. })),
            count(|a| matches!(a, Action::Rebody { .. })),
            count(|a| matches!(a, Action::Unlink { .. })),
            count(|a| matches!(a, Action::Link { .. })),
            count(|a| matches!(a, Action::Unblock { .. })),
            count(|a| matches!(a, Action::Block { .. })),
            count(|a| matches!(a, Action::Close { .. })),
        ));
        if self.is_empty() {
            out.push_str("nothing to do\n");
        }
        out
    }
}

/// Plan against the live state. `stories` are the spec's user stories.
pub fn plan(model: &Model, stories: &BTreeMap<u64, String>, client: &mut dyn Client) -> Result<Plan> {
    let keys = model.keys();
    // Live ids of every graph issue that exists, and the reverse lookup.
    let mut by_number: BTreeMap<u64, String> = BTreeMap::new();
    let mut p = Plan::default();
    let mut live = BTreeMap::new();
    for key in std::iter::once(SPEC_KEY.to_string()).chain(keys.iter().cloned()) {
        if let Some(number) = model.number(&key) {
            let issue = client.issue(number)?;
            by_number.insert(number, key.clone());
            live.insert(key, issue);
        }
    }

    let mut edits = Vec::new();
    for key in &keys {
        match live.get(key) {
            None => p.actions.push(Action::Create { key: key.clone() }),
            Some(issue) => {
                let want = title(model, key);
                if issue.title != want {
                    edits.push(Action::Retitle { key: key.clone(), number: issue.number, from: issue.title.clone(), to: want });
                }
                if normalize(&issue.body) != normalize(&body(model, key, stories)?) {
                    edits.push(Action::Rebody { key: key.clone(), number: issue.number });
                }
            }
        }
    }
    p.actions.extend(edits);

    // Sub-issue links: spec -> branch -> ticket. Only children that are
    // themselves graph issues are ever unlinked.
    let (mut unlinks, mut links) = (Vec::new(), Vec::new());
    for parent in std::iter::once(SPEC_KEY.to_string()).chain(model.branches.keys().cloned()) {
        let actual: Vec<Link> = match model.number(&parent) {
            Some(n) => client.sub_issues(n)?,
            None => Vec::new(),
        };
        let want: Vec<&String> = keys.iter().filter(|k| model.parent(k) == parent).collect();
        for l in &actual {
            if let Some(k) = by_number.get(&l.number) {
                if !want.contains(&k) {
                    unlinks.push(Action::Unlink { parent: parent.clone(), child: *l, child_key: k.clone() });
                }
            }
        }
        for k in want {
            if !actual.iter().any(|l| model.number(k) == Some(l.number)) {
                links.push(Action::Link { parent: parent.clone(), child: k.clone() });
            }
        }
    }

    // Native blocked-by links between tickets.
    let (mut unblocks, mut blocks) = (Vec::new(), Vec::new());
    for t in &model.tickets {
        let actual: Vec<Link> = match t.issue {
            Some(n) => client.blocked_by(n)?,
            None => Vec::new(),
        };
        for l in &actual {
            if let Some(k) = by_number.get(&l.number) {
                if !t.blocked_by.contains(k) {
                    unblocks.push(Action::Unblock { key: t.key.clone(), blocker: *l, blocker_key: k.clone() });
                }
            }
        }
        for b in &t.blocked_by {
            if !actual.iter().any(|l| model.number(b) == Some(l.number)) {
                blocks.push(Action::Block { key: t.key.clone(), blocker: b.clone() });
            }
        }
    }
    p.actions.extend(unlinks);
    p.actions.extend(unblocks);
    p.actions.extend(links);
    p.actions.extend(blocks);

    for t in &model.tickets {
        for &n in &t.supersedes {
            if client.issue(n)?.open {
                p.actions.push(Action::Close { number: n, by: t.key.clone() });
            }
        }
    }
    Ok(p)
}

/// What an apply did: the issues it created, and the closes it left for the
/// approved close action.
#[derive(Debug, Default)]
pub struct Applied {
    pub created: Vec<(String, u64)>,
    pub skipped_closes: usize,
}

/// Carry out a plan. `done` is filled as it goes, so the issues created before
/// a failure are still known to the caller. `model` gains the numbers of created issues as it goes,
/// so later bodies and links can name them. Closes are never run here: they
/// belong to the approved close action (`close.rs`).
pub fn apply(
    plan: &Plan,
    model: &mut Model,
    stories: &BTreeMap<u64, String>,
    client: &mut dyn Client,
    done: &mut Applied,
) -> Result<()> {
    let mut ids: BTreeMap<String, u64> = BTreeMap::new();
    ids.insert(SPEC_KEY.to_string(), client.issue(model.spec)?.id);
    for key in model.keys() {
        if let Some(n) = model.number(&key) {
            ids.insert(key, client.issue(n)?.id);
        }
    }
    for action in &plan.actions {
        match action {
            Action::Create { key } => {
                let issue = client.create(&title(model, key), &body(model, key, stories)?, &labels(model, key))?;
                model.set_number(key, issue.number);
                ids.insert(key.clone(), issue.id);
                done.created.push((key.clone(), issue.number));
            }
            Action::Retitle { number, to, .. } => client.edit(*number, Some(to), None)?,
            Action::Rebody { key, number } => client.edit(*number, None, Some(&body(model, key, stories)?))?,
            Action::Unlink { parent, child, .. } => client.remove_sub_issue(model.number(parent).unwrap_or(0), child.id)?,
            Action::Link { parent, child } => client.add_sub_issue(model.number(parent).unwrap_or(0), ids[child])?,
            Action::Unblock { key, blocker, .. } => client.remove_blocked_by(model.number(key).unwrap_or(0), blocker.id)?,
            Action::Block { key, blocker } => client.add_blocked_by(model.number(key).unwrap_or(0), ids[blocker])?,
            Action::Close { .. } => done.skipped_closes += 1,
        }
    }
    Ok(())
}
