//! Checks `docs/gold-band-port-inventory.md` against the repository (#351).
//!
//! Rules, each reported as one problem line:
//! * every directory under `skills/workstation/` and `skills/repo/` has a row
//!   in the matching `### skills/<group>` section of the inventory;
//! * every row in those sections names a directory that exists;
//! * every hook declared in `manifest.yaml`, every critique role file and every
//!   MCP server in `registry/estate-tooling.yaml` has a row in the `## Hooks`,
//!   `## Roles` and `## MCP servers` sections;
//! * every row of every table carries exactly one target kind from
//!   [`KINDS`] and a reach from [`REACHES`] (`n/a` only for "stays outside").
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
    let declared = [
        ("Hooks", hook_names(&read(MANIFEST)), MANIFEST),
        ("Roles", md_stems(&root.join(ROLES_DIR)), ROLES_DIR),
        ("MCP servers", mcp_names(&read(REGISTRY)), REGISTRY),
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
        let kind = c[3];
        let reach = c[4];
        if !KINDS.contains(&kind) {
            problems.push(format!(
                "{section}/{name}: target kind {kind:?} is not one of {KINDS:?}"
            ));
        }
        let outside = kind == "stays outside";
        let ok = if outside {
            reach == "n/a"
        } else {
            REACHES.contains(&reach)
        };
        if !ok {
            problems.push(format!(
                "{section}/{name}: reach {reach:?} does not fit kind {kind:?}"
            ));
        }
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

    fn fixture(tag: &str, skills: &[(&str, &str)], inventory: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("inventory-check-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (group, name) in skills {
            fs::create_dir_all(root.join("skills").join(group).join(name)).unwrap();
        }
        fs::create_dir_all(root.join("skills/workstation")).unwrap();
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
            "mcp_servers:\n  - name: m\n    transport: http\n  - name: m2\nclis:\n  - name: no\n",
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
}
