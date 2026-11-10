//! The published issue title, body and labels for a branch or a ticket.
//! The templates reproduce the ones the graph was first published with.

use crate::model::{Model, Result};
use std::collections::BTreeMap;

/// Collapse a story list into ranges: `[1,2,3,5]` becomes `1-3, 5`.
pub fn ranges(stories: &[u64]) -> String {
    let mut v = stories.to_vec();
    v.sort_unstable();
    v.dedup();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v[j + 1] == v[j] + 1 {
            j += 1;
        }
        out.push(if i == j { v[i].to_string() } else { format!("{}-{}", v[i], v[j]) });
        i = j + 1;
    }
    out.join(", ")
}

fn num(model: &Model, key: &str) -> String {
    model.number(key).map_or_else(|| "?".to_string(), |n| n.to_string())
}

pub fn title(model: &Model, key: &str) -> String {
    match model.branches.get(key) {
        Some(b) => format!("{} (spec #{})", b.title, model.spec),
        None => model.ticket(key).map(|t| t.title.clone()).unwrap_or_default(),
    }
}

pub fn labels(model: &Model, key: &str) -> Vec<String> {
    let mut l = if model.branches.contains_key(key) { vec!["area: agents".to_string()] } else { model.labels.clone() };
    l.push("type: feature".to_string());
    l
}

pub fn body(model: &Model, key: &str, stories: &BTreeMap<u64, String>) -> Result<String> {
    let spec = model.spec;
    if let Some(b) = model.branches.get(key) {
        let mut lines = Vec::new();
        for s in model.stories_of_branch(key) {
            lines.push(stories.get(&s).cloned().ok_or_else(|| format!("{key}: story {s} is not in the spec"))?);
        }
        return Ok(format!(
            "## Parent\n\n#{spec}\n\n## What this branch covers\n\n{}\n\nIts tickets are the sub-issues of this issue; their blocking edges are set as native dependencies. Blockers in other branches are tickets of the same spec #{spec}.\n\n## User stories\n\nThe stories of spec #{spec} that this branch's tickets deliver, numbered as in the spec.\n\n{}\n",
            b.what,
            lines.join("\n")
        ));
    }
    let t = model.ticket(key).ok_or_else(|| format!("unknown key {key}"))?;
    let br = &model.branches[&t.branch];
    let ac: Vec<String> = t.ac.iter().map(|a| format!("- [ ] {a}")).collect();
    let user_stories = if t.stories.is_empty() {
        "none (supporting ticket)".to_string()
    } else {
        format!("{} (spec #{spec})", ranges(&t.stories))
    };
    let mut out = format!(
        "## Parent\n\n#{} ({}), spec #{spec}\n\n## What to build\n\n{}\n\n## Acceptance criteria\n\n{}\n\n## Scope\n\n- Ticket key: `{}`\n- Lanes: {}\n- Hosts: {}\n- User stories: {user_stories}\n",
        num(model, &t.branch),
        br.title,
        t.what,
        ac.join("\n"),
        t.key,
        t.lanes.join(", "),
        t.hosts.join(", ")
    );
    if !t.supersedes.is_empty() {
        let list: Vec<String> = t.supersedes.iter().map(|n| format!("- #{n}")).collect();
        out.push_str(&format!(
            "\n## Replaces\n\n{}\n\nThese are closed with a pointer to this ticket by the approved sync (B0-06).\n",
            list.join("\n")
        ));
    }
    Ok(out)
}

/// Whitespace the host may normalise: line endings and trailing blanks.
pub fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n").trim_end().to_string()
}
