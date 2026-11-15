use super::*;
use std::fs;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

const DEBT: &str = "text\nrust-migration-debt: #810 fixing a crash\n";

#[test]
fn watched_paths() {
    for p in [
        "a.py", "x/b.pyi", "c.ts", "d.tsx", "e.mts", "f.cts", "g.js", "h.mjs", "i.cjs",
    ] {
        assert!(is_watched(p), "{p}");
    }
    for p in [
        "a.rs",
        "Makefile",
        "x/.py",
        "node_modules/a.js",
        "a/target/b.py",
        "a.py.txt",
        "scripts/.venv/x.py",
    ] {
        assert!(!is_watched(p), "{p}");
    }
}

#[test]
fn allowlist_parsing() {
    let a = parse_allowlist("# c\n\n  ./a.py \nb.ts\n");
    assert_eq!(a, set(&["a.py", "b.ts"]));
}

#[test]
fn debt_line_forms() {
    assert!(has_debt_line(DEBT));
    assert!(has_debt_line("  rust-migration-debt:   #5 why"));
    for bad in [
        "",
        "rust-migration-debt: #810",
        "rust-migration-debt: #810   ",
        "rust-migration-debt: 810 reason",
        "rust-migration-debt: #x reason",
        "note rust-migration-debt: #1 reason",
        "rust-migration-debt #1 reason",
    ] {
        assert!(!has_debt_line(bad), "{bad:?}");
    }
}

#[test]
fn change_parsing() {
    let raw =
        "A\0new.py\0M\0m.py\0D\0gone.py\0R100\0old.py\0moved.py\0C90\0src.py\0copy.py\0T\0t.py\0";
    assert_eq!(
        parse_changes(raw),
        vec![
            Change::Added("new.py".into()),
            Change::Modified("m.py".into()),
            Change::Deleted("gone.py".into()),
            Change::Renamed {
                from: "old.py".into(),
                to: "moved.py".into()
            },
            Change::Added("copy.py".into()),
            Change::Modified("t.py".into()),
        ]
    );
    assert_eq!(parse_changes(""), vec![]);
    assert_eq!(parse_changes("A\0"), vec![Change::Added(String::new())]);
    assert_eq!(
        parse_changes("R\0a\0"),
        vec![Change::Renamed {
            from: "a".into(),
            to: String::new()
        }]
    );
}

#[test]
fn new_file_fails() {
    let v = evaluate(
        &[Change::Added("n.py".into())],
        &set(&[]),
        Some(&set(&[])),
        DEBT,
    );
    assert_eq!(v.len(), 1);
    assert!(v[0].starts_with("n.py: new file"));
    // Not Python or TypeScript: fine.
    assert!(evaluate(&[Change::Added("n.rs".into())], &set(&[]), None, "").is_empty());
}

#[test]
fn allowlisted_edit_needs_debt_line() {
    let list = set(&["a.py"]);
    let edit = [Change::Modified("a.py".into())];
    let v = evaluate(&edit, &list, Some(&list), "no note");
    assert_eq!(v.len(), 1);
    assert!(v[0].contains("allowlisted file changed"));
    assert!(evaluate(&edit, &list, Some(&list), DEBT).is_empty());
}

#[test]
fn unlisted_edit_fails_even_with_debt() {
    let v = evaluate(
        &[Change::Modified("b.ts".into())],
        &set(&["a.py"]),
        None,
        DEBT,
    );
    assert_eq!(v.len(), 1);
    assert!(v[0].contains("outside the allowlist"));
    assert!(evaluate(&[Change::Modified("b.rs".into())], &set(&[]), None, "").is_empty());
}

#[test]
fn shrink_and_delete_pass() {
    let base = set(&["a.py", "b.py"]);
    let head = set(&["a.py"]);
    let changes = [Change::Deleted("b.py".into())];
    assert!(evaluate(&changes, &head, Some(&base), "").is_empty());
}

#[test]
fn growing_the_allowlist_needs_debt_line() {
    let base = set(&["a.py"]);
    let head = set(&["a.py", "z.py"]);
    let v = evaluate(&[], &head, Some(&base), "");
    assert_eq!(v.len(), 1);
    assert!(v[0].starts_with("z.py: added to the allowlist"));
    assert!(evaluate(&[], &head, Some(&base), DEBT).is_empty());
    // No allowlist at the base yet: nothing to compare against.
    assert!(evaluate(&[], &head, None, "").is_empty());
}

#[test]
fn rename_handling() {
    let list = set(&["new/a.py"]);
    let to_listed = [Change::Renamed {
        from: "a.py".into(),
        to: "new/a.py".into(),
    }];
    assert!(evaluate(&to_listed, &list, Some(&list), "")[0]
        .contains("renamed onto an allowlisted path"));
    assert!(evaluate(&to_listed, &list, Some(&list), DEBT).is_empty());
    let to_unlisted = [Change::Renamed {
        from: "a.py".into(),
        to: "other.py".into(),
    }];
    assert!(
        evaluate(&to_unlisted, &list, Some(&list), DEBT)[0].contains("renamed to a path outside")
    );
    // Rewritten in Rust: the Python file is gone, which always passes.
    let to_rust = [Change::Renamed {
        from: "a.py".into(),
        to: "a.rs".into(),
    }];
    assert!(evaluate(&to_rust, &list, Some(&list), "").is_empty());
}

// ---- git-backed tests ----

fn sh(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// A repository whose `base` branch has `a.py`, `b.py`, `keep.rs` and an
/// allowlist of the two scripts; the checkout is left on branch `pr`.
fn repo(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rmg-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    sh(&dir, &["init", "-q", "-b", "base"]);
    write(&dir, "a.py", "print(1)\nprint(2)\nprint(3)\nprint(4)\n");
    write(&dir, "b.py", "x = 1\ny = 2\nz = 3\nw = 4\nv = 5\n");
    write(&dir, "keep.rs", "fn main() {}\n");
    write(&dir, DEFAULT_ALLOWLIST, "# group\na.py\nb.py\n");
    sh(&dir, &["add", "."]);
    sh(&dir, &["commit", "-q", "-m", "base"]);
    sh(&dir, &["checkout", "-q", "-b", "pr"]);
    dir
}

fn commit(dir: &Path) {
    sh(dir, &["add", "-A"]);
    sh(dir, &["commit", "-q", "-m", "pr"]);
}

fn local(dir: &Path, body: &str) -> (i32, String) {
    let bf = dir.join("..").join(format!(
        "body-{}",
        dir.file_name().unwrap().to_string_lossy()
    ));
    fs::write(&bf, body).unwrap();
    let code = run([
        "--repo-dir".into(),
        dir.as_os_str().to_owned(),
        "--base".into(),
        "base".into(),
        "--body-file".into(),
        bf.into_os_string(),
    ]);
    (code, String::new())
}

#[test]
fn git_new_file_fails() {
    let d = repo("new");
    write(&d, "tool.py", "print('x')\n");
    commit(&d);
    assert_eq!(local(&d, DEBT).0, 1);
    let v = check(&d, DEFAULT_ALLOWLIST, "base", "").unwrap();
    assert_eq!(v.len(), 1);
}

#[test]
fn git_rust_only_change_passes() {
    let d = repo("rust");
    write(&d, "keep.rs", "fn main() { }\n");
    commit(&d);
    assert_eq!(local(&d, "").0, 0);
}

#[test]
fn git_allowlisted_edit() {
    let d = repo("edit");
    write(&d, "a.py", "print(1)\nprint(2)\nprint(3)\nprint(5)\n");
    commit(&d);
    assert_eq!(local(&d, "nothing").0, 1);
    assert_eq!(local(&d, DEBT).0, 0);
}

#[test]
fn git_delete_and_shrink_pass() {
    let d = repo("shrink");
    fs::remove_file(d.join("b.py")).unwrap();
    write(&d, DEFAULT_ALLOWLIST, "# group\na.py\n");
    commit(&d);
    assert_eq!(local(&d, "").0, 0);
}

#[test]
fn git_allowlist_growth_fails() {
    let d = repo("grow");
    write(&d, "c.py", "print('c')\n");
    write(&d, DEFAULT_ALLOWLIST, "# group\na.py\nb.py\nc.py\n");
    commit(&d);
    assert_eq!(local(&d, "").0, 1);
    assert_eq!(local(&d, DEBT).0, 0);
}

#[test]
fn git_rename_is_detected() {
    let d = repo("rename");
    sh(&d, &["mv", "a.py", "moved.py"]);
    commit(&d);
    let changes = diff_changes(&d, "base").unwrap();
    assert_eq!(
        changes,
        vec![Change::Renamed {
            from: "a.py".into(),
            to: "moved.py".into()
        }]
    );
    assert_eq!(local(&d, DEBT).0, 1);
    write(&d, DEFAULT_ALLOWLIST, "# group\nmoved.py\nb.py\n");
    commit(&d);
    assert_eq!(local(&d, "").0, 1);
    assert_eq!(local(&d, DEBT).0, 0);
}

#[test]
fn git_base_without_allowlist_compares_nothing() {
    let d = repo("fresh");
    sh(&d, &["checkout", "-q", "base"]);
    sh(&d, &["rm", "-q", DEFAULT_ALLOWLIST]);
    sh(&d, &["commit", "-q", "-m", "drop list"]);
    sh(&d, &["checkout", "-q", "-B", "pr"]);
    write(&d, DEFAULT_ALLOWLIST, "a.py\nb.py\nnew.py\n");
    commit(&d);
    assert_eq!(
        check(&d, DEFAULT_ALLOWLIST, "base", "").unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn git_errors_exit_two() {
    let d = repo("errors");
    assert!(check(&d, "missing.txt", "base", "")
        .unwrap_err()
        .contains("cannot read allowlist"));
    assert!(check(&d, DEFAULT_ALLOWLIST, "no-such-ref", "")
        .unwrap_err()
        .contains("git diff"));
    let not_a_repo = std::env::temp_dir().join(format!("rmg-nogit-{}", std::process::id()));
    fs::create_dir_all(&not_a_repo).unwrap();
    write(&not_a_repo, DEFAULT_ALLOWLIST, "");
    assert!(check(&not_a_repo, DEFAULT_ALLOWLIST, "base", "").is_err());
}

#[test]
fn git_cannot_run_git() {
    let err = run_git("/nonexistent-rmg-git", Path::new("."), &["status"]).unwrap_err();
    assert!(err.starts_with("cannot run git"));
    assert!(git(Path::new("/nonexistent-rmg-dir"), &["status"]).is_err());
}

// ---- command line and event payload ----

fn event(name: &str, json: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("rmg-event-{name}-{}.json", std::process::id()));
    fs::write(&p, json).unwrap();
    p
}

fn os(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

#[test]
fn event_mode_reads_base_and_body() {
    let d = repo("event");
    write(&d, "a.py", "print(1)\nprint(2)\nprint(3)\nprint(5)\n");
    commit(&d);
    let base = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(&d)
            .args(["rev-parse", "base"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let base = base.trim();
    let dir = d.to_string_lossy().into_owned();
    let without = event(
        "nobody",
        &format!(r#"{{"pull_request":{{"base":{{"sha":"{base}"}},"body":null}}}}"#),
    );
    let with = event(
        "body",
        &format!(
            r#"{{"pull_request":{{"base":{{"sha":"{base}"}},"body":"x\nrust-migration-debt: #810 reason"}}}}"#
        ),
    );
    let ep = |p: &PathBuf| os(&["--repo-dir", &dir, "--event-path", &p.to_string_lossy()]);
    assert_eq!(run(ep(&without)), 1);
    assert_eq!(run(ep(&with)), 0);
}

#[test]
fn event_mode_edge_cases() {
    let push = event("push", r#"{"ref":"refs/heads/main"}"#);
    assert_eq!(run(os(&["--event-path", &push.to_string_lossy()])), 0);
    let nobase = event("nobase", r#"{"pull_request":{}}"#);
    assert_eq!(run(os(&["--event-path", &nobase.to_string_lossy()])), 2);
    let junk = event("junk", "not json");
    assert_eq!(run(os(&["--event-path", &junk.to_string_lossy()])), 2);
    assert_eq!(run(os(&["--event-path", "/nonexistent-rmg-event"])), 2);
    assert_eq!(
        run(os(&["--base", "x", "--body-file", "/nonexistent-rmg-body"])),
        2
    );
}

#[test]
fn usage_errors_exit_two() {
    assert_eq!(run(os(&[])), 2);
    assert_eq!(run(os(&["--base", "x"])), 2);
    assert_eq!(run(os(&["--event-path"])), 2);
    assert_eq!(run(os(&["--bogus", "1"])), 2);
    assert_eq!(
        run(os(&[
            "--event-path",
            "e",
            "--base",
            "x",
            "--body-file",
            "b"
        ])),
        2
    );
    let o = parse_args(os(&[
        "--allowlist",
        "l.txt",
        "--base",
        "x",
        "--body-file",
        "b",
    ]))
    .unwrap();
    assert_eq!(o.allowlist, "l.txt");
}
