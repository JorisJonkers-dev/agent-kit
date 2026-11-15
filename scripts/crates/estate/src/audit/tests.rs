use super::*;
use crate::testkit::{run_in, run_plain, Dir};

const LATEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const STALE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PIPELINE: &str = "jobs:\n  done:\n    name: Pipeline Complete\n    runs-on: ubuntu-latest\n";
const PRESET_JSON: &str = r#"{"extends": ["github>JorisJonkers-dev/renovate-config"]}"#;

fn pin(reference: &str, name: &str) -> String {
    format!(
        "      - uses: JorisJonkers-dev/github-workflows/.github/workflows/{name}@{reference}\n"
    )
}

fn clean(dir: &Dir) {
    dir.write(
        ".github/workflows/ci.yml",
        &format!(
            "{PIPELINE}{}      # uses: x/github-workflows@main\n",
            pin(LATEST, "go.yml")
        ),
    );
    dir.write("renovate.json", PRESET_JSON);
}

fn bad() -> Dir {
    let dir = Dir::new();
    let body = [
        pin("main", "a.yml"),
        pin("v0.45.3", "b.yml"),
        pin(STALE, "c.yml"),
        pin(LATEST, "d.yml"),
    ]
    .concat();
    dir.write(
        ".github/workflows/ci.yml",
        &format!("jobs:\n  x:\n    steps:\n{body}"),
    );
    dir.write(".github/workflows/notes.txt", &pin("main", "go.yml"));
    dir.write("platform/fleet.yaml", "x: 1\n");
    dir.write(
        "deploy/app.yml",
        "token: packages-proxy-token x\nschema: deploy-config-schema\n",
    );
    dir.write("Taskfile.yml", "image: ExtraToast/app\n");
    dir.write("node_modules/pkg/fleet.yml", "ignored\n");
    dir.write(".github/ignored/vendor/x.yml", "ExtraToast/\n");
    dir
}

fn audit_out(dir: &Dir, extra: &[&str]) -> Out {
    let root = dir.str();
    let mut args = vec!["audit", root.as_str()];
    args.extend_from_slice(extra);
    run_plain(&args)
}

use crate::testkit::Out;

fn lines(out: &Out) -> Vec<String> {
    out.out.lines().map(String::from).collect()
}

#[test]
fn a_clean_repository_prints_no_gaps() {
    let dir = Dir::new();
    clean(&dir);
    let out = audit_out(&dir, &["--latest-sha", LATEST]);
    assert_eq!(
        (out.code, out.out.as_str()),
        (0, "no gaps\n"),
        "{}",
        out.err
    );
}

#[test]
fn each_gap_is_listed_with_its_file_in_the_documented_line_shape() {
    let dir = bad();
    let out = audit_out(&dir, &["--latest-sha", LATEST]);
    assert_eq!(out.code, 0, "{}", out.err);
    let lines = lines(&out);
    let ci = ".github/workflows/ci.yml";
    assert!(lines.contains(&format!(
        "{ci}: line 4: github-workflows .github/workflows/a.yml@main pinned to main (source: github-workflows README.md)"
    )));
    assert!(lines
        .iter()
        .any(|l| l.starts_with(&format!("{ci}: line 5:"))
            && l.contains("v0.45.3 not pinned by 40-hex SHA")));
    assert!(lines.contains(&format!(
        "{ci}: line 6: github-workflows .github/workflows/c.yml@{STALE} is not the latest release SHA {LATEST} (source: github-workflows latest release)"
    )));
    assert!(!lines
        .iter()
        .any(|l| l.contains("d.yml") || l.contains("notes.txt")));
    assert!(lines.iter().any(|l| l
        .starts_with(".github/workflows: no job named Pipeline Complete")
        && l.contains("ADR-0018")));
    assert!(lines
        .iter()
        .any(|l| l.starts_with("renovate.json: no renovate.json at the root")));
    assert!(lines
        .iter()
        .any(|l| l.starts_with("platform/: top-level platform/ directory")));
    assert!(lines.iter().any(
        |l| l.starts_with("platform/fleet.yaml: fleet.yaml is flagged as retired (unverified")
    ));
    assert!(
        lines
            .iter()
            .any(|l| l
                .starts_with("deploy/app.yml: line 1: retired reference 'packages-proxy-token'"))
    );
    assert!(
        lines
            .iter()
            .any(|l| l
                .starts_with("deploy/app.yml: line 2: retired reference 'deploy-config-schema'"))
    );
    assert!(lines
        .iter()
        .any(|l| l.starts_with("Taskfile.yml: line 1: retired reference 'ExtraToast/'")));
    assert!(!lines
        .iter()
        .any(|l| l.contains("node_modules") || l.contains("vendor")));
    assert!(lines
        .iter()
        .all(|l| l.ends_with(')') && l.contains(" (source: ")));
}

#[test]
fn pins_are_not_compared_to_a_release_without_latest_sha() {
    let out = audit_out(&bad(), &[]);
    assert!(!out.out.contains("is not the latest release SHA"));
    assert!(out.out.contains("pinned to main"));
}

#[test]
fn json_output_is_a_list_of_file_gap_source() {
    let out = audit_out(&bad(), &["--json"]);
    assert_eq!(out.code, 0);
    let rows: serde_json::Value = serde_json::from_str(&out.out).unwrap();
    let rows = rows.as_array().unwrap();
    assert!(!rows.is_empty());
    assert!(out.out.starts_with("[\n  {\n    \"file\": "));
    for row in rows {
        let mut keys: Vec<&String> = row.as_object().unwrap().keys().collect();
        keys.sort();
        assert_eq!(keys, ["file", "gap", "source"]);
    }
    let sources = |file: &str| -> Vec<String> {
        rows.iter()
            .filter(|r| r["file"] == file)
            .map(|r| r["source"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        sources("platform/"),
        ["workspace CLAUDE.md (Flux reads deploy/production)"]
    );
    assert_eq!(sources("platform/fleet.yaml"), ["none (unverified)"]);
}

#[test]
fn json_output_for_a_clean_repository_is_an_empty_list() {
    let dir = Dir::new();
    clean(&dir);
    assert_eq!(audit_out(&dir, &["--json"]).out, "[]\n");
}

#[test]
fn json_escapes_non_ascii_like_python() {
    let dir = Dir::new();
    clean(&dir);
    dir.write("\u{e9}\u{1f600}/fleet.yaml", "x\n");
    dir.write("quo\"te/fleet.yml", "x\n");
    let out = audit_out(&dir, &["--json"]);
    assert!(
        out.out.contains("\\u00e9\\ud83d\\ude00/fleet.yaml"),
        "{}",
        out.out
    );
    assert!(out.out.contains("quo\\\"te/fleet.yml"));
}

#[test]
fn a_missing_directory_exits_2() {
    let dir = Dir::new();
    let missing = dir.join("nope");
    let out = run_plain(&["audit", missing.to_str().unwrap()]);
    assert_eq!(out.code, 2);
    assert!(out.err.contains("not a directory"));
    assert_eq!(out.out, "");
}

#[test]
fn a_malformed_latest_sha_exits_2() {
    let dir = Dir::new();
    for sha in ["abc", &"A".repeat(40), &"g".repeat(40), &"a".repeat(41)] {
        let out = audit_out(&dir, &["--latest-sha", sha]);
        assert_eq!(out.code, 2);
        assert!(out
            .err
            .contains("--latest-sha must be 40 lowercase hex characters"));
    }
}

#[test]
fn a_tilde_directory_is_expanded_against_home() {
    let dir = Dir::new();
    clean(&dir);
    let out = run_in(&["audit", "~"], "", "/nonexistent", None, Some(&dir.str()));
    assert_eq!(out.out, "no gaps\n");
    let out = run_in(
        &["audit", "~/sub"],
        "",
        "/nonexistent",
        None,
        Some(&dir.str()),
    );
    assert_eq!(out.code, 2);
}

#[test]
fn usage_mistakes_and_help() {
    assert_eq!(run_plain(&["audit"]).code, 2);
    assert_eq!(run_plain(&["audit", "a", "b"]).code, 2);
    let out = run_plain(&["audit", "--bogus", "a"]);
    assert_eq!(out.code, 2);
    assert!(out
        .err
        .contains("estate audit: error: unrecognized arguments: --bogus"));
    let out = run_plain(&["audit", "--latest-sha"]);
    assert!(out
        .err
        .contains("argument --latest-sha: expected one argument"));
    let out = run_plain(&["audit", "--help"]);
    assert_eq!(out.code, 0);
    assert!(out.out.starts_with("usage: estate audit"));
}

#[test]
fn two_pipeline_complete_jobs_are_a_gap() {
    let dir = Dir::new();
    clean(&dir);
    dir.write(
        ".github/workflows/other.yaml",
        &PIPELINE.replace("Pipeline Complete", "'Pipeline Complete'"),
    );
    assert!(audit_out(&dir, &[])
        .out
        .contains(".github/workflows: 2 jobs named Pipeline Complete; expected one"));
}

#[test]
fn a_commented_pipeline_complete_does_not_count() {
    let dir = Dir::new();
    dir.write(".github/workflows/ci.yml", "# name: Pipeline Complete\n");
    dir.write("renovate.json", PRESET_JSON);
    assert!(audit_out(&dir, &[])
        .out
        .contains("no job named Pipeline Complete"));
}

#[test]
fn renovate_in_dot_github_is_found() {
    let dir = Dir::new();
    clean(&dir);
    std::fs::rename(dir.join("renovate.json"), dir.join(".github/renovate.json")).unwrap();
    assert_eq!(audit_out(&dir, &[]).out, "no gaps\n");
}

#[test]
fn renovate_json_that_is_not_json_is_a_gap() {
    let dir = Dir::new();
    clean(&dir);
    dir.write("renovate.json", "{nope");
    let out = audit_out(&dir, &[]).out;
    assert!(out.contains("renovate.json: could not confirm the preset"));
    assert!(!out.contains("not valid JSON"));
}

#[test]
fn renovate_without_the_preset_is_a_gap() {
    for text in [r#"{"extends": ["config:base"]}"#, "[]", "{}"] {
        let dir = Dir::new();
        clean(&dir);
        dir.write("renovate.json", text);
        assert!(audit_out(&dir, &[]).out.contains(
            "renovate.json: does not extend \"github>JorisJonkers-dev/renovate-config\""
        ));
    }
}

#[test]
fn a_repository_without_workflows_is_missing_the_pipeline_job() {
    let dir = Dir::new();
    assert!(audit_out(&dir, &[])
        .out
        .contains(".github/workflows: no job named Pipeline Complete"));
}

#[test]
fn pipeline_complete_counts_only_job_names_under_jobs() {
    let dir = Dir::new();
    clean(&dir);
    dir.write(
        ".github/workflows/other.yml",
        "name: Pipeline Complete\njobs:\n  build:\n    name: Build\n    steps:\n      - uses: some/action@v1\n        with:\n          name: Pipeline Complete\n  done:\n    runs-on: x\n    name: Pipeline Complete # the check\non: push\nenv:\n  name: Pipeline Complete\n",
    );
    assert!(audit_out(&dir, &[])
        .out
        .contains("2 jobs named Pipeline Complete; expected one"));
}

#[test]
fn a_top_level_or_input_name_is_not_a_pipeline_job() {
    let dir = Dir::new();
    dir.write(
        ".github/workflows/ci.yml",
        "name: Pipeline Complete\njobs:\n  a:\n    steps:\n      - with:\n          name: Pipeline Complete\n",
    );
    dir.write("renovate.json", PRESET_JSON);
    assert!(audit_out(&dir, &[])
        .out
        .contains(".github/workflows: no job named Pipeline Complete"));
}

#[test]
fn the_pipeline_name_must_be_exactly_the_check() {
    assert!(is_pipeline_name("name: Pipeline Complete"));
    assert!(is_pipeline_name("name:   \"Pipeline Complete\""));
    assert!(!is_pipeline_name("title: Pipeline Complete"));
    assert!(!is_pipeline_name("name: Pipeline Completed"));
    assert!(!is_pipeline_name("name: Pipeline Complete x"));
}

#[test]
fn a_pin_mentioned_only_in_a_trailing_comment_is_ignored() {
    let dir = Dir::new();
    clean(&dir);
    dir.write(
        ".github/workflows/more.yml",
        &format!(
            "jobs:\n  a:\n    steps:\n      - run: echo hi # uses JorisJonkers-dev/github-workflows/x.yml@main\n{} # v1.2.3\n",
            pin(LATEST, "go.yml").trim_end_matches('\n')
        ),
    );
    assert_eq!(audit_out(&dir, &["--latest-sha", LATEST]).out, "no gaps\n");
}

#[test]
fn pins_in_composite_actions_and_tmpl_templates_are_checked() {
    let dir = Dir::new();
    clean(&dir);
    dir.write(".github/actions/x/action.yml", &pin("main", "go.yml"));
    dir.write(
        "templates/go/.github/workflows/ci.yml.tmpl",
        &pin("v1", "go.yml"),
    );
    dir.write("templates/go/readme.md", &pin("main", "go.yml"));
    dir.write(".github/actions/node_modules/y.yml", &pin("main", "go.yml"));
    let out = audit_out(&dir, &[]);
    let lines = lines(&out);
    assert!(lines
        .iter()
        .any(|l| l.starts_with(".github/actions/x/action.yml: line 1:")
            && l.contains("pinned to main")));
    assert!(lines.iter().any(|l| l
        .starts_with("templates/go/.github/workflows/ci.yml.tmpl: line 1:")
        && l.contains("not pinned")));
    assert!(!lines
        .iter()
        .any(|l| l.contains("readme.md") || l.contains("node_modules")));
}

#[test]
fn find_pins_reads_every_pin_on_a_line_and_skips_malformed_ones() {
    let line = "a JorisJonkers-dev/github-workflows/x.yml@main b JorisJonkers-dev/github-workflows/y.yml@v1";
    assert_eq!(
        find_pins(line),
        [
            ("x.yml".to_string(), "main".to_string()),
            ("y.yml".to_string(), "v1".to_string())
        ]
    );
    for line in [
        "JorisJonkers-dev/github-workflows/@main",
        "JorisJonkers-dev/github-workflows/x.yml",
        "JorisJonkers-dev/github-workflows/x.yml @main",
        "JorisJonkers-dev/github-workflows/x.yml@",
        "JorisJonkers-dev/github-workflows/x.yml@ main",
        "JorisJonkers-dev/github-workflows/",
    ] {
        assert!(find_pins(line).is_empty(), "{line}");
    }
    assert_eq!(
        find_pins("JorisJonkers-dev/github-workflows/x@a@b'")[0].1,
        "a@b"
    );
}

#[test]
fn comments_start_at_a_hash_after_whitespace_or_at_the_line_start() {
    assert_eq!(strip_comment("# all"), "");
    assert_eq!(strip_comment("a: b # c"), "a: b");
    assert_eq!(strip_comment("a: b#c"), "a: b#c");
    assert_eq!(strip_comment("a: \u{e9} #c"), "a: \u{e9}");
    assert_eq!(strip_comment("plain"), "plain");
}

#[test]
fn packages_proxy_token_is_found_without_a_colon_in_any_text_file() {
    let dir = Dir::new();
    clean(&dir);
    dir.write("docs/notes.md", "uses packages-proxy-token here\n");
    dir.write_bytes("src/blob.bin", b"packages-proxy-token\0binary");
    dir.write(
        "src/huge.txt",
        &format!("packages-proxy-token\n{}", "x".repeat(1_100_000)),
    );
    dir.write(
        "docs/ExtraToast.md",
        "ExtraToast/ outside the scoped paths\n",
    );
    std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("src/dangling")).unwrap();
    std::os::unix::fs::symlink(dir.join("docs"), dir.join("src/linked-dir")).unwrap();
    let out = audit_out(&dir, &[]).out;
    assert!(out.contains("docs/notes.md: line 1: retired reference 'packages-proxy-token'"));
    for absent in [
        "blob.bin",
        "huge.txt",
        "dangling",
        "linked-dir",
        "ExtraToast.md",
    ] {
        assert!(!out.contains(absent), "{absent}");
    }
}

#[test]
fn renovate_json_with_comments_is_read() {
    let dir = Dir::new();
    clean(&dir);
    dir.write(
        "renovate.json",
        "// header\n{\n  /* the preset */\n  \"extends\": [\"github>JorisJonkers-dev/renovate-config\"], // x\n  \"note\": \"see https://example.com/a//b\"\n}\n",
    );
    assert_eq!(audit_out(&dir, &[]).out, "no gaps\n");
}

#[test]
fn json_comment_stripping_keeps_strings_and_survives_odd_input() {
    assert_eq!(
        strip_json_comments(r#"{"a": "x\"//y"} // c"#),
        r#"{"a": "x\"//y"} "#
    );
    assert_eq!(strip_json_comments("a /* b */ c"), "a  c");
    assert_eq!(strip_json_comments("a // b\nc"), "a \nc");
    assert_eq!(strip_json_comments("a /* open"), "a /* open");
    assert_eq!(strip_json_comments("a / b"), "a / b");
    assert_eq!(strip_json_comments("\"open // x"), "\"open ");
    assert_eq!(strip_json_comments("\"end\\"), "\"end\\");
    assert_eq!(strip_json_comments("/*"), "/*");
}

#[test]
fn renovate_extends_is_normalised_and_compared_by_entry() {
    let cases = [
        (r#""github>JorisJonkers-dev/renovate-config""#, true),
        (
            r##"["config:base", "github>JorisJonkers-dev/renovate-config#v1.2.3"]"##,
            true,
        ),
        (
            r#"["github>JorisJonkers-dev/renovate-config-other"]"#,
            false,
        ),
        (r#"["x github>JorisJonkers-dev/renovate-config"]"#, false),
        (r#""github>JorisJonkers-dev/renovate-config-extra""#, false),
        (r#"{"github>JorisJonkers-dev/renovate-config": 1}"#, false),
        ("[5, null]", false),
    ];
    for (extends, ok) in cases {
        let dir = Dir::new();
        clean(&dir);
        dir.write("renovate.json", &format!("{{\"extends\": {extends}}}"));
        assert_eq!(audit_out(&dir, &[]).out == "no gaps\n", ok, "{extends}");
    }
}

#[test]
fn only_regular_yaml_files_in_the_workflows_directory_count() {
    let dir = Dir::new();
    clean(&dir);
    dir.write(".github/workflows/sub.yml/inner.txt", "x");
    dir.write(".github/workflows/.yml", &pin("main", "a.yml"));
    assert_eq!(audit_out(&dir, &["--latest-sha", LATEST]).out, "no gaps\n");
}
