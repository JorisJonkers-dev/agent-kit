//! Checks `docs/gold-band-port-inventory.md` against the repository
//! (#351, #369).
//!
//! Rules, each reported as one problem line:
//! * every directory under `skills/workstation/` and `skills/repo/` has a row
//!   in the matching `### skills/<group>` section of the inventory;
//! * every row in those sections names a directory that exists;
//! * every hook declared in `manifest.yaml`, every critique role file and every
//!   MCP server in `registry/estate-tooling.yaml` has a row in the `## Hooks`,
//!   `## Roles` and `## MCP servers` sections;
//! * every row of every table carries exactly one target kind from [`KINDS`]
//!   and a reach from [`REACHES`] (`n/a` only for "stays outside");
//! * every row of the `## Hooks` section stays outside;
//! * every row's last cell, `Target`, is [`NO_TARGET`] (nothing is ported yet)
//!   or names something that exists for the row's kind.
//!
//! Usage: `inventory-check <REPO_ROOT>`. Exit 0 clean, 1 problems, 2 bad usage.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

pub const INVENTORY: &str = "docs/gold-band-port-inventory.md";
pub const MANIFEST: &str = "manifest.yaml";
pub const ROLES_DIR: &str = "skills/workstation/critique/roles";
pub const REGISTRY: &str = "registry/estate-tooling.yaml";
pub const GROUPS: [&str; 2] = ["workstation", "repo"];
pub const KINDS: [&str; 5] = [
    "workflow",
    "role",
    "skill set",
    "MCP server",
    "stays outside",
];
pub const REACHES: [&str; 7] = [
    "read",
    "read-web",
    "research",
    "write-scoped",
    "write",
    "full",
    "run-one-command",
];
/// The `Target` cell of a row whose entry is not ported yet.
pub const NO_TARGET: &str = "-";
/// The section whose every row must stay outside Gold Band.
const HOOKS: &str = "Hooks";

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
        } else if let Some(c) = cells(line) {
            let head = c[0].eq_ignore_ascii_case("name");
            let rule = c[0].chars().all(|ch| ch == '-' || ch == ' ');
            if !head && !rule {
                out.push((section.clone(), c));
            }
        }
    }
    out
}

/// Names of the sub-directories of `dir`, sorted.
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

/// Whether the registry declares `name` as an agent id or a skill-set name.
///
/// The Gold Band section of the registry is not built yet, so the rule for now
/// is a text match: some line, after its list dash, reads exactly `id: <name>`
/// or `name: <name>`.
fn declares(registry: &str, name: &str) -> bool {
    let id = format!("id: {name}");
    let named = format!("name: {name}");
    registry.lines().any(|line| {
        let line = line.trim();
        let line = line.strip_prefix("- ").unwrap_or(line);
        line == id.as_str() || line == named.as_str()
    })
}

/// Whether `target` names something that exists for `kind`.
fn target_exists(root: &Path, registry: &str, kind: &str, target: &str) -> bool {
    match kind {
        "workflow" => root.join(target).exists(),
        "MCP server" => mcp_names(registry).contains(target),
        "role" | "skill set" => declares(registry, target),
        _ => false,
    }
}

/// One inventory row, as the checks see it.
struct Row<'a> {
    section: &'a str,
    name: &'a str,
    kind: &'a str,
    reach: &'a str,
    target: &'a str,
}

/// How many of [`KINDS`] the kind cell names, so `workflow, role` counts two.
fn kinds_named(kind: &str) -> usize {
    let mut named = 0;
    for known in KINDS {
        if kind.contains(known) {
            named += 1;
        }
    }
    named
}

/// Whether the row's `Target` cell names something that does not exist.
fn target_missing(root: &Path, registry: &str, row: &Row) -> bool {
    if row.target == NO_TARGET || !KINDS.contains(&row.kind) {
        return false;
    }
    !target_exists(root, registry, row.kind, row.target)
}

/// Every problem one row earns, in a stable order.
fn row_problems(root: &Path, registry: &str, row: &Row) -> Vec<String> {
    let (section, name, kind) = (row.section, row.name, row.kind);
    let mut out = Vec::new();
    if kind.is_empty() {
        out.push(format!("{section}/{name}: row has no target kind"));
    } else if kinds_named(kind) > 1 {
        out.push(format!("{section}/{name}: row has two target kinds"));
    } else if !KINDS.contains(&kind) {
        out.push(format!(
            "{section}/{name}: target kind {kind:?} is not one of {KINDS:?}"
        ));
    }
    if section == HOOKS && kind != "stays outside" {
        out.push(format!("{section}/{name}: a hook must stay outside"));
    }
    let reach = row.reach;
    let target = row.target;
    let reach_ok = if kind == "stays outside" {
        reach == "n/a"
    } else {
        REACHES.contains(&reach)
    };
    if !reach_ok {
        out.push(format!(
            "{section}/{name}: reach {reach:?} does not fit kind {kind:?}"
        ));
    }
    if target_missing(root, registry, row) {
        out.push(format!("{section}/{name}: target {target} does not exist"));
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
        let section = format!("skills/{group}");
        let listed: BTreeSet<String> = table
            .iter()
            .filter(|(s, _)| *s == section)
            .map(|(_, c)| c[0].to_string())
            .collect();
        let on_disk = dirs(&root.join("skills").join(group));
        for name in on_disk.difference(&listed) {
            problems.push(format!(
                "{section}/{name}: skill directory has no inventory row"
            ));
        }
        for name in listed.difference(&on_disk) {
            problems.push(format!(
                "{section}/{name}: inventory row has no skill directory"
            ));
        }
    }
    let read = |file: &str| fs::read_to_string(root.join(file)).unwrap_or_default();
    let manifest = read(MANIFEST);
    let registry = read(REGISTRY);
    let declared = [
        (HOOKS, hook_names(&manifest), MANIFEST),
        ("Roles", md_stems(&root.join(ROLES_DIR)), ROLES_DIR),
        ("MCP servers", mcp_names(&registry), REGISTRY),
    ];
    for (section, names, source) in declared {
        if names.is_empty() {
            problems.push(format!("{section}: nothing found in {source}"));
        }
        let listed: BTreeSet<&str> = table
            .iter()
            .filter(|(s, _)| s == section)
            .map(|(_, c)| c[0])
            .collect();
        for name in names.iter().filter(|n| !listed.contains(n.as_str())) {
            problems.push(format!(
                "{section}/{name}: declared in {source} but has no inventory row"
            ));
        }
    }
    for (section, c) in &table {
        let name = c[0];
        if c.len() < 5 {
            problems.push(format!("{section}/{name}: row has fewer than 5 cells"));
            continue;
        }
        let row = Row {
            section: section.as_str(),
            name,
            kind: c[3],
            reach: c[4],
            target: c.get(6).copied().unwrap_or(NO_TARGET),
        };
        problems.extend(row_problems(root, &registry, &row));
    }
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
mod tests {
    use super::*;

    const HEAD: &str = "| Name | Path | What | Target kind | Reach | Dep |\n| --- | --- | --- | --- | --- | --- |\n";
    const HEAD7: &str = "| Name | Path | What | Target kind | Reach | Dep | Target |\n| --- | --- | --- | --- | --- | --- | --- |\n";
    const HOOKS7: &str = "| h | p | w | stays outside | n/a | no | - |\n";
    const ROLES7: &str = "| r | p | w | role | read | no | - |\n";
    const SERVERS7: &str =
        "| m | p | w | MCP server | read | no | - |\n| m2 | p | w | MCP server | write | no | - |\n";

    fn fixture(tag: &str, skills: &[(&str, &str)], inventory: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("inventory-check-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (group, name) in skills {
            fs::create_dir_all(root.join("skills").join(group).join(name)).unwrap();
        }
        fs::create_dir_all(root.join("skills/workstation")).unwrap();
        // Both fixture documents list a `critique` row, so its directory exists.
        fs::create_dir_all(root.join("skills/workstation/critique")).unwrap();
        fs::create_dir_all(root.join("skills/repo")).unwrap();
        fs::create_dir_all(root.join("docs")).unwrap();
        fs::write(root.join(INVENTORY), inventory).unwrap();
        fs::write(
            root.join(MANIFEST),
            "hooks:\n  h:\n    adr: x\n    event: PreToolUse\nnext:\n  other:\n",
        )
        .unwrap();
        fs::create_dir_all(root.join(ROLES_DIR)).unwrap();
        fs::create_dir_all(root.join("registry")).unwrap();
        fs::write(root.join(ROLES_DIR).join("r.md"), "role").unwrap();
        fs::write(root.join(ROLES_DIR).join("notes.txt"), "not a role").unwrap();
        fs::write(
            root.join(REGISTRY),
            "mcp_servers:\n  - name: m\n    transport: http\n  - name: m2\nclis:\n  - name: no\nagents:\n  - id: agent-x\n  - name: skill-y\n",
        )
        .unwrap();
        root
    }

    const SECTIONS: &str = "\n## Hooks\n\n{H}| h | p | w | stays outside | n/a | no |\n\n## Roles\n\n{H}| r | p | w | role | read | no |\n\n## MCP servers\n\n{H}| m | p | w | MCP server | read | no |\n| m2 | p | w | MCP server | write | no |\n";

    fn sections(extra: &str) -> String {
        format!("{}{extra}", SECTIONS.replace("{H}", HEAD))
    }

    fn doc(ws: &str, repo: &str) -> String {
        format!("## Skill directories\n\n### skills/workstation (1)\n\n{HEAD}| critique | p | w | workflow | run-one-command | no |\n{ws}\n### skills/repo (1)\n\n{HEAD}{repo}{}", sections(""))
    }

    /// A complete inventory with a `Target` column, its five sections holding
    /// the given rows.
    fn inventory(ws: &str, repo: &str, hooks: &str, roles: &str, servers: &str) -> String {
        format!(
            "## Skill directories\n\n### skills/workstation (1)\n\n{HEAD7}| critique | p | w | workflow | run-one-command | no | - |\n{ws}\n### skills/repo (1)\n\n{HEAD7}{repo}\n## Hooks\n\n{HEAD7}{hooks}\n## Roles\n\n{HEAD7}{roles}\n## MCP servers\n\n{HEAD7}{servers}"
        )
    }

    fn doc7(ws: &str, repo: &str) -> String {
        inventory(ws, repo, HOOKS7, ROLES7, SERVERS7)
    }

    #[test]
    fn clean_inventory_passes() {
        let d = doc(
            "| a | p | w | workflow | read | no |\n",
            "| b | p | w | stays outside | n/a | no |\n",
        );
        let root = fixture("clean", &[("workstation", "a"), ("repo", "b")], &d);
        assert!(check(&root).is_empty());
        assert_eq!(run(vec![root.into_os_string()]), 0);
    }

    #[test]
    fn directory_without_row_fails() {
        let d = doc("| a | p | w | workflow | read | no |\n", "");
        let root = fixture("norow", &[("workstation", "a"), ("repo", "b")], &d);
        let p = check(&root);
        assert_eq!(
            p,
            vec!["skills/repo/b: skill directory has no inventory row"]
        );
        assert_eq!(run(vec![root.into_os_string()]), 1);
    }

    #[test]
    fn row_without_directory_fails() {
        let d = doc(
            "| a | p | w | workflow | read | no |\n| ghost | p | w | role | read | no |\n",
            "",
        );
        let root = fixture("nodir", &[("workstation", "a")], &d);
        let p = check(&root);
        assert_eq!(
            p,
            vec!["skills/workstation/ghost: inventory row has no skill directory"]
        );
    }

    #[test]
    fn bad_kind_bad_reach_and_short_row_fail() {
        let d = doc(
            "| a | p | w | magic | read | no |\n| b | p | w | workflow | huge | no |\n| c | p | w | stays outside | read | no |\n| d | p |\n",
            "",
        );
        let root = fixture(
            "bad",
            &[
                ("workstation", "a"),
                ("workstation", "b"),
                ("workstation", "c"),
                ("workstation", "d"),
            ],
            &d,
        );
        let p = check(&root);
        assert_eq!(p.len(), 4);
        assert!(p[0].contains("target kind"));
        assert!(p.iter().any(|l| l.contains("reach \"huge\"")));
        assert!(p
            .iter()
            .any(|l| l.contains("reach \"read\" does not fit kind \"stays outside\"")));
        assert!(p.iter().any(|l| l.contains("fewer than 5 cells")));
    }

    #[test]
    fn non_skill_tables_are_checked_for_kind_and_reach() {
        let d = format!("{}{}", doc("", ""), "| bad | p | w | role | n/a | no |\n");
        let root = fixture("badrow", &[], &d);
        let p = check(&root);
        assert_eq!(
            p,
            vec!["MCP servers/bad: reach \"n/a\" does not fit kind \"role\""]
        );
    }

    #[test]
    fn hook_role_and_server_without_row_fail() {
        let d = format!(
            "## Skill directories\n\n### skills/workstation (1)\n\n{HEAD}| critique | p | w | workflow | run-one-command | no |\n\n### skills/repo (0)\n\n{HEAD}\n## Hooks\n\n{HEAD}\n## Roles\n\n{HEAD}\n## MCP servers\n\n{HEAD}| m | p | w | MCP server | read | no |\n"
        );
        let root = fixture("undeclared", &[], &d);
        let p = check(&root);
        assert_eq!(
            p,
            vec![
                "Hooks/h: declared in manifest.yaml but has no inventory row",
                "Roles/r: declared in skills/workstation/critique/roles but has no inventory row",
                "MCP servers/m2: declared in registry/estate-tooling.yaml but has no inventory row",
            ]
        );
    }

    #[test]
    fn empty_sources_fail_instead_of_passing() {
        let root = fixture("nosource", &[], &doc("", ""));
        fs::write(root.join(MANIFEST), "other: 1\n").unwrap();
        fs::remove_file(root.join(REGISTRY)).unwrap();
        fs::remove_file(root.join(ROLES_DIR).join("r.md")).unwrap();
        let p = check(&root);
        assert!(p.contains(&"Hooks: nothing found in manifest.yaml".to_string()));
        assert!(
            p.contains(&"Roles: nothing found in skills/workstation/critique/roles".to_string())
        );
        assert!(
            p.contains(&"MCP servers: nothing found in registry/estate-tooling.yaml".to_string())
        );
    }

    #[test]
    fn missing_inventory_and_bad_usage() {
        let root =
            std::env::temp_dir().join(format!("inventory-check-none-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert!(check(&root)[0].starts_with("cannot read"));
        assert_eq!(run(std::iter::empty()), 2);
        assert_eq!(run(vec!["a".into(), "b".into()]), 2);
        assert!(dirs(&root.join("absent")).is_empty());
    }

    #[test]
    fn non_row_lines_are_ignored() {
        assert!(cells("plain text").is_none());
        assert!(rows("no tables here").is_empty());
    }

    #[test]
    fn row_with_two_kinds_fails() {
        let d = doc7(
            concat!(
                "| a | p | w | workflow, role | read | no | - |\n",
                "| b | p | w | workflow / role | read | no | - |\n",
            ),
            "",
        );
        let root = fixture(
            "twokinds",
            &[("workstation", "a"), ("workstation", "b")],
            &d,
        );
        assert_eq!(
            check(&root),
            vec![
                "skills/workstation/a: row has two target kinds",
                "skills/workstation/b: row has two target kinds",
            ]
        );
    }

    #[test]
    fn row_without_kind_fails() {
        let d = doc7("| a | p | w |  | read | no | - |\n", "");
        let root = fixture("nokind", &[("workstation", "a")], &d);
        assert_eq!(
            check(&root),
            vec!["skills/workstation/a: row has no target kind"]
        );
    }

    #[test]
    fn hook_that_does_not_stay_outside_fails() {
        let d = inventory(
            "",
            "",
            "| h | p | w | workflow | read | no | - |\n",
            ROLES7,
            SERVERS7,
        );
        let root = fixture("hookkind", &[], &d);
        assert_eq!(check(&root), vec!["Hooks/h: a hook must stay outside"]);
    }

    #[test]
    fn workflow_target_must_exist() {
        let d = doc7(
            concat!(
                "| a | p | w | workflow | read | no | manifest.yaml |\n",
                "| b | p | w | workflow | read | no | gone.yaml |\n",
            ),
            "",
        );
        let root = fixture(
            "workflowtarget",
            &[("workstation", "a"), ("workstation", "b")],
            &d,
        );
        assert_eq!(
            check(&root),
            vec!["skills/workstation/b: target gone.yaml does not exist"]
        );
    }

    #[test]
    fn mcp_server_target_must_be_declared() {
        let d = doc7(
            concat!(
                "| a | p | w | MCP server | read | no | m |\n",
                "| b | p | w | MCP server | read | no | zzz |\n",
            ),
            "",
        );
        let root = fixture(
            "mcptarget",
            &[("workstation", "a"), ("workstation", "b")],
            &d,
        );
        assert_eq!(
            check(&root),
            vec!["skills/workstation/b: target zzz does not exist"]
        );
    }

    #[test]
    fn role_and_skill_set_targets_must_be_declared() {
        let d = doc7(
            concat!(
                "| a | p | w | role | read | no | agent-x |\n",
                "| b | p | w | skill set | read | no | skill-y |\n",
                "| c | p | w | role | read | no | ghost |\n",
                "| d | p | w | skill set | read | no | ghost |\n",
            ),
            "",
        );
        let root = fixture(
            "roletarget",
            &[
                ("workstation", "a"),
                ("workstation", "b"),
                ("workstation", "c"),
                ("workstation", "d"),
            ],
            &d,
        );
        assert_eq!(
            check(&root),
            vec![
                "skills/workstation/c: target ghost does not exist",
                "skills/workstation/d: target ghost does not exist",
            ]
        );
    }

    #[test]
    fn stays_outside_target_must_be_a_dash() {
        let d = doc7("| a | p | w | stays outside | n/a | no | out |\n", "");
        let root = fixture("outsidetarget", &[("workstation", "a")], &d);
        assert_eq!(
            check(&root),
            vec!["skills/workstation/a: target out does not exist"]
        );
    }

    #[test]
    fn unknown_kind_skips_the_target_rule() {
        let d = doc7("| a | p | w | magic | read | no | ghost |\n", "");
        let root = fixture("magictarget", &[("workstation", "a")], &d);
        let p = check(&root);
        assert_eq!(p.len(), 1);
        assert!(p[0].contains("target kind \"magic\""));
    }

    #[test]
    fn six_and_seven_cell_rows_both_pass() {
        let d6 = doc(
            "| a | p | w | workflow | read | no |\n",
            "| b | p | w | stays outside | n/a | no |\n",
        );
        let root = fixture("six", &[("workstation", "a"), ("repo", "b")], &d6);
        assert!(check(&root).is_empty());
        let d7 = doc7(
            "| a | p | w | workflow | read | no | - |\n",
            "| b | p | w | stays outside | n/a | no | - |\n",
        );
        let root = fixture("seven", &[("workstation", "a"), ("repo", "b")], &d7);
        assert!(check(&root).is_empty());
    }
}
