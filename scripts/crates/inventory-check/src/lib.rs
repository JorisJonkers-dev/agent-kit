//! Checks `docs/gold-band-port-inventory.md` against the repository.
//!
//! Every table row has six cells: `Name | Path | What | Target | Lanes | Notes`.
//! Rules, each reported as one problem line:
//! * every directory under `skills/workstation/` and `skills/repo/` has a row
//!   in the matching `### skills/<group>` section, and every row of those
//!   sections names a directory that exists (rows for skills not built yet
//!   live in `## Planned`, which is not matched against the disk);
//! * every hook declared in `manifest.yaml`, every role file (the critique
//!   roles and `agents/*.md`) and every MCP server in
//!   `registry/estate-tooling.yaml` has a row in `## Hooks`, `## Roles` and
//!   `## MCP servers`;
//! * a name has one row per section;
//! * every row has exactly one target from [`TARGETS`]: none, or two, fails;
//! * every row has at least one lane, each from [`LANES`];
//! * a hook, and any row that stays outside, carries no Gold Band lane;
//! * `## Waves` groups the rows that port: each `### Wave N` table lists rows
//!   as `<section>/<name>` with their target, every row with a Gold Band lane
//!   sits in exactly one wave, and no other row sits in any.
//!
//! Usage: `inventory-check <REPO_ROOT>`. Exit 0 clean, 1 problems, 2 bad usage.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::Path;

pub const INVENTORY: &str = "docs/gold-band-port-inventory.md";
pub const MANIFEST: &str = "manifest.yaml";
pub const ROLE_DIRS: [&str; 2] = ["skills/workstation/critique/roles", "agents"];
pub const REGISTRY: &str = "registry/estate-tooling.yaml";
pub const GROUPS: [&str; 2] = ["workstation", "repo"];
pub const TARGETS: [&str; 5] = [
    "workflow",
    "role",
    "skill set",
    "MCP server",
    "stays outside",
];
pub const OUTSIDE: &str = "stays outside";
pub const LANES: [&str; 3] = ["direct", "gold-band", "gold-band-personal"];
const HOOKS: &str = "Hooks";
const CELLS: usize = 6;
const WAVES: &str = "Waves";
const WAVE_CELLS: usize = 3;

/// One table row: the cells between the pipes, trimmed.
fn cells(line: &str) -> Option<Vec<&str>> {
    let line = line.trim();
    if !line.starts_with('|') || !line.ends_with('|') {
        return None;
    }
    Some(line[1..line.len() - 1].split('|').map(str::trim).collect())
}

/// A table's data rows (header and separator dropped) with their section.
fn rows(text: &str) -> Vec<(String, Vec<&str>)> {
    let mut section = String::new();
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("### ") {
            section = title.split_whitespace().next().unwrap_or("").to_string();
        } else if let Some(title) = line.strip_prefix("## ") {
            section = title.trim().to_string();
        } else if section.starts_with("Wave") {
            continue;
        } else if let Some(c) = cells(line) {
            let head = c[0].eq_ignore_ascii_case("name");
            let rule = !c[0].is_empty() && c[0].chars().all(|ch| ch == '-');
            if !head && !rule {
                out.push((section.clone(), c));
            }
        }
    }
    out
}

/// The rows of the `## Waves` tables, each with its wave heading.
fn wave_rows(text: &str) -> Vec<(String, Vec<&str>)> {
    let (mut in_waves, mut wave) = (false, String::new());
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            in_waves = title.trim() == WAVES;
        } else if let (true, Some(title)) = (in_waves, line.strip_prefix("### ")) {
            wave = title.trim().to_string();
        } else if let (true, Some(c)) = (in_waves, cells(line)) {
            let head = c[0].eq_ignore_ascii_case("row");
            let rule = !c[0].is_empty() && c[0].chars().all(|ch| ch == '-');
            if !head && !rule {
                out.push((wave.clone(), c));
            }
        }
    }
    out
}

/// Problems between the wave tables and the inventory rows.
fn wave_problems(text: &str, table: &[(String, Vec<&str>)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut placed: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (wave, c) in wave_rows(text) {
        if c.len() != WAVE_CELLS {
            out.push(format!(
                "{WAVES}/{wave}: row {:?} has {} cells, not {WAVE_CELLS}",
                c[0],
                c.len()
            ));
            continue;
        }
        placed
            .entry(c[0].to_string())
            .or_default()
            .push(wave.clone());
        let found = table
            .iter()
            .find(|(s, r)| format!("{s}/{}", r[0]) == c[0] && r.len() == CELLS);
        let Some((_, row)) = found else {
            out.push(format!("{WAVES}/{wave}: {} is not an inventory row", c[0]));
            continue;
        };
        if !has_gold_lane(row[4]) || row[3] == OUTSIDE {
            out.push(format!(
                "{WAVES}/{wave}: {} has no Gold Band lane to port",
                c[0]
            ));
        }
        if c[1] != row[3] {
            out.push(format!(
                "{WAVES}/{wave}: {} target {:?} is not the inventory's {:?}",
                c[0], c[1], row[3]
            ));
        }
    }
    for (name, waves) in placed.iter().filter(|(_, w)| w.len() > 1) {
        out.push(format!(
            "{WAVES}/{name}: row is listed {} times",
            waves.len()
        ));
    }
    for (section, c) in table.iter().filter(|(_, c)| c.len() == CELLS) {
        if has_gold_lane(c[4])
            && c[3] != OUTSIDE
            && !placed.contains_key(&format!("{section}/{}", c[0]))
        {
            out.push(format!("{section}/{}: Gold Band row is in no wave", c[0]));
        }
    }
    out
}

/// Whether a lanes cell names a Gold Band lane.
fn has_gold_lane(lanes: &str) -> bool {
    lanes.split(',').any(|l| l.trim().starts_with("gold-band"))
}

/// Names of the sub-directories of `dir`.
fn dirs(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.path().is_dir() {
                out.insert(e.file_name().to_string_lossy().into_owned());
            }
        }
    }
    out
}

/// Stems of the `.md` files directly in `dir`.
fn md_stems(dir: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "md") {
                if let Some(stem) = path.file_stem() {
                    out.insert(stem.to_string_lossy().into_owned());
                }
            }
        }
    }
    out
}

/// The lines of the top-level YAML block `key:`, up to the next top-level line.
fn block<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    let head = format!("{key}:");
    let mut lines = text.lines().skip_while(|l| l.trim_end() != head);
    lines.next();
    lines
        .take_while(|l| {
            l.is_empty() || l.starts_with(' ') || l.starts_with('#') || l.starts_with('-')
        })
        .collect()
}

/// Names of the hooks under `hooks:` (two-space keys) in the manifest.
fn hook_names(manifest: &str) -> BTreeSet<String> {
    block(manifest, "hooks")
        .iter()
        .filter_map(|l| l.strip_prefix("  ")?.strip_suffix(':'))
        .filter(|n| !n.is_empty() && !n.contains(' '))
        .map(str::to_string)
        .collect()
}

/// Names of the servers under `mcp_servers:` (`- name: X` items) in the registry.
fn mcp_names(registry: &str) -> BTreeSet<String> {
    block(registry, "mcp_servers")
        .iter()
        .filter_map(|l| l.trim_start().strip_prefix("- name:"))
        .map(|n| n.trim().to_string())
        .collect()
}

/// How many of [`TARGETS`] the cell names, so `workflow, role` counts two.
fn targets_named(cell: &str) -> usize {
    TARGETS.iter().filter(|t| cell.contains(**t)).count()
}

/// Every problem one row earns, in a stable order.
fn row_problems(section: &str, c: &[&str]) -> Vec<String> {
    let name = c[0];
    if c.len() != CELLS {
        return vec![format!(
            "{section}/{name}: row has {} cells, not {CELLS}",
            c.len()
        )];
    }
    let (target, lanes) = (c[3], c[4]);
    let mut out = Vec::new();
    match targets_named(target) {
        0 => out.push(format!("{section}/{name}: row has no target")),
        1 if !TARGETS.contains(&target) => out.push(format!(
            "{section}/{name}: target {target:?} is not exactly one of {TARGETS:?}"
        )),
        1 => {}
        _ => out.push(format!("{section}/{name}: row has two targets")),
    }
    let named: Vec<&str> = lanes
        .split(',')
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if named.is_empty() {
        out.push(format!("{section}/{name}: row has no lanes"));
    }
    for lane in &named {
        if !LANES.contains(lane) {
            out.push(format!(
                "{section}/{name}: lane {lane:?} is not one of {LANES:?}"
            ));
        }
    }
    let gold = has_gold_lane(lanes);
    if (section == HOOKS || target == OUTSIDE) && gold {
        out.push(format!(
            "{section}/{name}: a row that stays outside carries a Gold Band lane"
        ));
    }
    if section == HOOKS && target != OUTSIDE {
        out.push(format!("{section}/{name}: a hook must stay outside"));
    }
    out
}

/// Missing and extra names of `section`'s rows against `declared`.
fn compare(
    table: &[(String, Vec<&str>)],
    section: &str,
    declared: &BTreeSet<String>,
    source: &str,
    extra_is_problem: bool,
) -> Vec<String> {
    let listed: BTreeSet<&str> = table
        .iter()
        .filter(|(s, _)| s == section)
        .map(|(_, c)| c[0])
        .collect();
    let mut out = Vec::new();
    for name in declared.iter().filter(|n| !listed.contains(n.as_str())) {
        out.push(format!(
            "{section}/{name}: declared in {source} but has no inventory row"
        ));
    }
    if extra_is_problem {
        for name in listed.iter().filter(|n| !declared.contains(**n)) {
            out.push(format!(
                "{section}/{name}: inventory row has no directory in {source}"
            ));
        }
    }
    out
}

/// Every problem found for `root`, in a stable order.
pub fn check(root: &Path) -> Vec<String> {
    let text = match fs::read_to_string(root.join(INVENTORY)) {
        Ok(t) => t,
        Err(e) => return vec![format!("cannot read {INVENTORY}: {e}")],
    };
    let table = rows(&text);
    let mut problems = Vec::new();
    for group in GROUPS {
        let on_disk = dirs(&root.join("skills").join(group));
        let dir = format!("skills/{group}");
        problems.extend(compare(&table, &dir, &on_disk, &dir, true));
    }
    let read = |file: &str| fs::read_to_string(root.join(file)).unwrap_or_default();
    let mut roles = BTreeSet::new();
    for dir in ROLE_DIRS {
        roles.extend(md_stems(&root.join(dir)));
    }
    let declared = [
        (HOOKS, hook_names(&read(MANIFEST)), MANIFEST),
        ("Roles", roles, "the role directories"),
        ("MCP servers", mcp_names(&read(REGISTRY)), REGISTRY),
    ];
    for (section, names, source) in declared {
        if names.is_empty() {
            problems.push(format!("{section}: nothing found in {source}"));
        }
        problems.extend(compare(&table, section, &names, source, false));
    }
    let mut seen = BTreeSet::new();
    for (section, c) in &table {
        if !seen.insert((section.as_str(), c[0])) {
            problems.push(format!("{section}/{}: name has two rows", c[0]));
        }
        problems.extend(row_problems(section, c));
    }
    problems.extend(wave_problems(&text, &table));
    problems
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let (Some(root), None) = (args.next(), args.next()) else {
        eprintln!("usage: inventory-check <REPO_ROOT>");
        return 2;
    };
    let problems = check(Path::new(&root));
    for p in &problems {
        eprintln!("inventory-check: {p}");
    }
    if problems.is_empty() {
        println!("inventory-check: ok");
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests;
