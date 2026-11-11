use super::*;

const HEAD: &str =
    "| Name | Path | What | Target | Lanes | Notes |\n| --- | --- | --- | --- | --- | --- |\n";
const GOOD: &str = "\
## Skills

### skills/workstation (2)

| Name | Path | What | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| s | p | w | workflow | direct, gold-band | |
| critique | p | w | workflow | direct | |

### skills/repo (0)

| Name | Path | What | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |

## Hooks

| Name | Path | What | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| h | p | w | stays outside | direct | |

## Roles

| Name | Path | What | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| r | p | w | role | direct, gold-band-personal | |
| a | p | w | stays outside | direct | |

## MCP servers

| Name | Path | What | Target | Lanes | Notes |
| --- | --- | --- | --- | --- | --- |
| m | p | w | MCP server | direct | |

## Waves

### Wave 2: first

| Row | Target | Notes |
| --- | --- | --- |
| skills/workstation/s | workflow | |

### Wave 3: second

| Row | Target | Notes |
| --- | --- | --- |
| Roles/r | role | |
";

fn fixture(tag: &str, inventory: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("inventory-check-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("skills/workstation/s")).unwrap();
    fs::create_dir_all(root.join("skills/repo")).unwrap();
    fs::create_dir_all(root.join("skills/workstation/critique/roles")).unwrap();
    fs::create_dir_all(root.join("agents")).unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("registry")).unwrap();
    fs::write(root.join(INVENTORY), inventory).unwrap();
    fs::write(
        root.join(MANIFEST),
        "hooks:\n  h:\n    adr: x\nnext:\n  other:\n",
    )
    .unwrap();
    fs::write(root.join("skills/workstation/critique/roles/r.md"), "r").unwrap();
    fs::write(root.join("skills/workstation/critique/roles/n.txt"), "n").unwrap();
    fs::write(root.join("agents/a.md"), "a").unwrap();
    fs::write(
        root.join(REGISTRY),
        "mcp_servers:\n  - name: m\n    transport: http\nclis:\n  - name: no\n",
    )
    .unwrap();
    root
}

fn problems_of(tag: &str, inventory: &str) -> Vec<String> {
    let root = fixture(tag, inventory);
    let p = check(&root);
    let _ = fs::remove_dir_all(&root);
    p
}

fn has(problems: &[String], needle: &str) -> bool {
    problems.iter().any(|p| p.contains(needle))
}

#[test]
fn clean_inventory_passes() {
    assert_eq!(problems_of("clean", GOOD), Vec::<String>::new());
}

#[test]
fn missing_role_and_skill_directories_count_as_empty() {
    let root = fixture("nodirs", GOOD);
    fs::remove_dir_all(root.join("skills/workstation/critique/roles")).unwrap();
    fs::remove_dir_all(root.join("skills/repo")).unwrap();
    let p = check(&root);
    let _ = fs::remove_dir_all(&root);
    assert!(!has(&p, "cannot read"));
}

#[test]
fn missing_inventory_is_reported() {
    let root = fixture("noinv", GOOD);
    fs::remove_file(root.join(INVENTORY)).unwrap();
    let p = check(&root);
    let _ = fs::remove_dir_all(&root);
    assert!(has(&p, "cannot read"));
}

#[test]
fn on_disk_skill_without_row_fails_and_row_without_skill_fails() {
    let root = fixture("skill", GOOD);
    fs::create_dir_all(root.join("skills/repo/extra")).unwrap();
    fs::remove_dir_all(root.join("skills/workstation/s")).unwrap();
    let p = check(&root);
    let _ = fs::remove_dir_all(&root);
    assert!(has(&p, "skills/repo/extra: declared in skills/repo"));
    assert!(has(
        &p,
        "skills/workstation/s: inventory row has no directory"
    ));
}

#[test]
fn undeclared_hook_role_and_server_fail() {
    let root = fixture("undeclared", GOOD);
    fs::write(
        root.join(MANIFEST),
        "hooks:\n  h:\n    adr: x\n  h2:\n    adr: y\n",
    )
    .unwrap();
    fs::write(root.join("agents/b.md"), "b").unwrap();
    fs::write(
        root.join(REGISTRY),
        "mcp_servers:\n  - name: m\n  - name: m2\n",
    )
    .unwrap();
    let p = check(&root);
    let _ = fs::remove_dir_all(&root);
    assert!(has(&p, "Hooks/h2: declared in manifest.yaml"));
    assert!(has(&p, "Roles/b: declared in the role directories"));
    assert!(has(&p, "MCP servers/m2: declared in registry"));
}

#[test]
fn empty_sources_fail() {
    let root = fixture("empty", GOOD);
    fs::write(root.join(MANIFEST), "").unwrap();
    fs::remove_file(root.join(REGISTRY)).unwrap();
    let p = check(&root);
    let _ = fs::remove_dir_all(&root);
    assert!(has(&p, "Hooks: nothing found"));
    assert!(has(&p, "MCP servers: nothing found"));
}

#[test]
fn two_targets_fail() {
    let doc = GOOD.replace("| s | p | w | workflow |", "| s | p | w | workflow, role |");
    assert!(has(
        &problems_of("two", &doc),
        "skills/workstation/s: row has two targets"
    ));
}

#[test]
fn no_target_and_unknown_target_fail() {
    let none = GOOD.replace("| s | p | w | workflow |", "| s | p | w |  |");
    assert!(has(&problems_of("none", &none), "row has no target"));
    let odd = GOOD.replace("| s | p | w | workflow |", "| s | p | w | the workflow |");
    assert!(has(&problems_of("odd", &odd), "not exactly one of"));
}

#[test]
fn row_with_no_lanes_fails() {
    let doc = GOOD.replace("| direct, gold-band |", "|  |");
    assert!(has(
        &problems_of("lanes", &doc),
        "skills/workstation/s: row has no lanes"
    ));
}

#[test]
fn unknown_lane_fails() {
    let doc = GOOD.replace("direct, gold-band |", "direct, nowhere |");
    assert!(has(&problems_of("lane", &doc), "lane \"nowhere\""));
}

#[test]
fn outside_and_hook_rows_carry_no_gold_band_lane() {
    let hook = GOOD.replace(
        "| h | p | w | stays outside | direct |",
        "| h | p | w | stays outside | direct, gold-band |",
    );
    assert!(has(
        &problems_of("hookgold", &hook),
        "Hooks/h: a row that stays outside carries"
    ));
    let notout = GOOD.replace(
        "| h | p | w | stays outside | direct |",
        "| h | p | w | role | direct |",
    );
    assert!(has(
        &problems_of("hookkind", &notout),
        "a hook must stay outside"
    ));
}

#[test]
fn short_row_and_duplicate_row_fail() {
    let short = GOOD.replace(
        "| m | p | w | MCP server | direct | |",
        "| m | p | w | MCP server | direct |",
    );
    assert!(has(
        &problems_of("short", &short),
        "MCP servers/m: row has 5 cells, not 6"
    ));
    let dup = format!("{GOOD}\n## Planned\n\n{HEAD}| x | p | w | skill set | direct | |\n| x | p | w | skill set | direct | |\n");
    assert!(has(
        &problems_of("dup", &dup),
        "Planned/x: name has two rows"
    ));
}

#[test]
fn blank_name_row_is_still_checked() {
    let doc = format!("{GOOD}\n## Planned\n\n{HEAD}|  | p | w |  |  | |\n");
    let p = problems_of("blank", &doc);
    assert!(has(&p, "Planned/: row has no target"));
    assert!(has(&p, "Planned/: row has no lanes"));
}

#[test]
fn planned_rows_need_no_directory() {
    let doc = format!("{GOOD}\n## Planned\n\n{HEAD}| estate-x | - | w | skill set | direct | |\n");
    assert_eq!(problems_of("planned", &doc), Vec::<String>::new());
}

#[test]
fn run_reports_usage_problems_and_success() {
    let args = |v: &[&std::path::Path]| {
        v.iter()
            .map(|p| p.as_os_str().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(run(Vec::<OsString>::new()), 2);
    let root = fixture("run", GOOD);
    assert_eq!(run(args(&[&root])), 0);
    assert_eq!(run(args(&[&root, &root])), 2);
    fs::write(root.join(INVENTORY), "").unwrap();
    assert_eq!(run(args(&[&root])), 1);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn gold_band_row_in_no_wave_fails() {
    let doc = GOOD.replace("| Roles/r | role | |\n", "");
    assert!(has(
        &problems_of("nowave", &doc),
        "Roles/r: Gold Band row is in no wave"
    ));
}

#[test]
fn row_in_two_waves_fails() {
    let doc = GOOD.replace(
        "| Roles/r | role | |\n",
        "| Roles/r | role | |\n| skills/workstation/s | workflow | |\n",
    );
    assert!(has(
        &problems_of("twowaves", &doc),
        "Waves/skills/workstation/s: row is listed 2 times"
    ));
}

#[test]
fn wave_row_must_exist_port_and_agree_on_target() {
    let ghost = GOOD.replace(
        "| Roles/r | role | |",
        "| Roles/ghost | role | |\n| Roles/r | role | |",
    );
    assert!(has(
        &problems_of("ghost", &ghost),
        "Roles/ghost is not an inventory row"
    ));
    let direct = GOOD.replace(
        "| Roles/r | role | |",
        "| Roles/r | role | |\n| skills/workstation/critique | workflow | |",
    );
    assert!(has(
        &problems_of("direct", &direct),
        "skills/workstation/critique has no Gold Band lane to port"
    ));
    let outside = GOOD.replace(
        "| Roles/r | role | |",
        "| Roles/r | role | |\n| Roles/a | stays outside | |",
    );
    assert!(has(
        &problems_of("outside", &outside),
        "Roles/a has no Gold Band lane to port"
    ));
    let target = GOOD.replace("| Roles/r | role | |", "| Roles/r | workflow | |");
    assert!(has(
        &problems_of("target", &target),
        "target \"workflow\" is not the inventory's \"role\""
    ));
}

#[test]
fn short_wave_row_fails_and_waves_are_not_inventory_rows() {
    let doc = GOOD.replace(
        "| Roles/r | role | |",
        "| Roles/r | role |\n| Roles/r | role | |",
    );
    let p = problems_of("shortwave", &doc);
    assert!(has(&p, "has 2 cells, not 3"));
    assert!(!has(&p, "row has 3 cells"));
}
