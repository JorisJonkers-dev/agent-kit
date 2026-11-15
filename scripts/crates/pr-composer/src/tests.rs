//! Unit tests: they execute every line of the crate except `src/main.rs`.
//! The golden files under `tests/fixtures/expected` for `check-body` are what
//! the Python helper printed for the same fixtures; the `mine-prs` ones were
//! compared with the Python profile (they differ only where Python's set
//! iteration order broke a tie).

use super::*;
use crate::json::J;
use crate::regex::{Match, Regex, I, M, NONE, S};
use crate::text::{float_repr, os_error, round, splitlines};
use std::fs;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn fx(name: &str) -> String {
    fixtures().join(name).to_string_lossy().into_owned()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pr-composer-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn exec(args: &[&str]) -> (i32, String, String) {
    let mut io = Io::default();
    let status = run_io(args.iter().map(OsString::from), &mut io);
    (status, io.out, io.err)
}

fn golden(name: &str) -> String {
    fs::read_to_string(fixtures().join("expected").join(name)).unwrap()
}

// ---- the dispatcher -------------------------------------------------------

#[test]
fn dispatcher_routes_and_refuses() {
    let (status, out, _) = exec(&["--help"]);
    assert_eq!(
        (status, out.starts_with("usage: pr-composer check-body")),
        (0, true)
    );
    assert_eq!(exec(&["-h"]).0, 0);
    let (status, out, err) = exec(&[]);
    assert_eq!(
        (status, out.as_str(), err.starts_with("usage: ")),
        (2, "", true)
    );
    assert_eq!(exec(&["nope"]).0, 2);
    assert_eq!(exec(&["check-body", "--help"]).0, 0);
    assert_eq!(exec(&["mine-prs", "-h"]).0, 0);
}

#[test]
fn run_writes_to_the_real_streams() {
    assert_eq!(run(["--help"].iter().map(OsString::from)), 0);
    assert_eq!(run(["nope"].iter().map(OsString::from)), 2);
}

// ---- check-body -----------------------------------------------------------

fn check(body: &str, extra: &[&str]) -> (i32, String, String) {
    let mut args = vec!["check-body", "-t", "", "-b", ""];
    let (t, b) = (fx("template.md"), fx(body));
    args[2] = &t;
    args[4] = &b;
    args.extend_from_slice(extra);
    exec(&args)
}

#[test]
fn check_body_matches_the_python_output() {
    for (body, extra, name, status) in [
        (
            "body_good.md",
            vec!["--skipped", "Screenshots"],
            "good_skipped.txt",
            0,
        ),
        ("body_good.md", vec![], "good_missing.txt", 1),
        ("body_bad.md", vec![], "bad.txt", 1),
        ("body_warn.md", vec![], "warn.txt", 0),
        (
            "body_order.md",
            vec!["--skipped", "Screenshots, Changelog"],
            "order.txt",
            1,
        ),
    ] {
        let (code, out, err) = check(body, &extra);
        assert_eq!(
            (code, out, err),
            (status, golden(name), String::new()),
            "{body} {extra:?}"
        );
    }
}

#[test]
fn check_body_takes_long_and_equals_forms() {
    let (t, b) = (fx("template.md"), fx("body_good.md"));
    let t_eq = format!("--template={t}");
    let (code, out, _) = exec(&["check-body", &t_eq, "--body", &b, "--skipped=Screenshots"]);
    assert_eq!((code, out), (0, golden("good_skipped.txt")));
}

#[test]
fn check_body_reports_a_bad_command_line() {
    let (code, _, err) = exec(&["check-body"]);
    assert_eq!(code, 2);
    assert!(err.contains("pr-composer check-body: error: the following arguments are required: -t/--template, -b/--body"), "{err}");
    let (_, _, err) = exec(&["check-body", "-t"]);
    assert!(
        err.contains("argument -t/--template: expected one argument"),
        "{err}"
    );
    let (_, _, err) = exec(&["check-body", "-t", "-b", "x"]);
    assert!(err.contains("expected one argument"), "{err}");
    let (t, b) = (fx("template.md"), fx("body_good.md"));
    let (code, _, err) = exec(&["check-body", "-t", &t, "-b", &b, "stray", "--other"]);
    assert_eq!(code, 2);
    assert!(
        err.contains("unrecognized arguments: stray --other"),
        "{err}"
    );
    // A lone dash is a value, not an option.
    let (code, _, err) = exec(&["check-body", "-t", "-", "-b", &b]);
    assert_eq!(code, 2);
    assert!(err.starts_with("cannot read input: "), "{err}");
}

#[test]
fn check_body_cannot_read_its_input() {
    let b = fx("body_good.md");
    let (code, out, err) = exec(&["check-body", "-t", "/no/such/template.md", "-b", &b]);
    assert_eq!((code, out.as_str()), (2, ""));
    assert_eq!(
        err,
        "cannot read input: [Errno 2] No such file or directory: '/no/such/template.md'\n"
    );
    let dir = scratch("bad-utf8");
    let bad = dir.join("body.md");
    fs::write(&bad, [0xff, 0xfe]).unwrap();
    let t = fx("template.md");
    let (code, _, err) = exec(&["check-body", "-t", &t, "-b", &bad.to_string_lossy()]);
    assert_eq!(code, 2);
    assert!(
        err.contains("cannot read input: stream did not contain valid UTF-8"),
        "{err}"
    );
}

#[test]
fn check_body_reads_universal_newlines_and_duplicate_headings() {
    let dir = scratch("newlines");
    let template = dir.join("t.md");
    let body = dir.join("b.md");
    fs::write(&template, "## A\r\n- [ ] one\r\n## a\r\n").unwrap();
    fs::write(&body, "## A\r- [ ] one\r## A\r").unwrap();
    let (code, out, _) = exec(&[
        "check-body",
        "-t",
        &template.to_string_lossy(),
        "-b",
        &body.to_string_lossy(),
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with(
        "template sections: 2 · body sections: 2 · template checkboxes: 1 · body checkboxes: 1\n"
    ));
}

#[test]
fn check_body_without_a_template_checklist_never_truncates() {
    let dir = scratch("nochecks");
    let (t, b) = (dir.join("t.md"), dir.join("b.md"));
    fs::write(&t, "## A\n").unwrap();
    fs::write(&b, "## A\n- [ ] extra\n").unwrap();
    let (code, out, _) = exec(&[
        "check-body",
        "-t",
        &t.to_string_lossy(),
        "-b",
        &b.to_string_lossy(),
    ]);
    assert_eq!((code, out.contains("TRUNCATED")), (0, false));
}

#[test]
fn norm_compares_headings_loosely() {
    assert_eq!(check_body::norm("**Test_Plan:** 🚀  Now"), "testplan now");
    assert_eq!(check_body::norm(""), "");
}

// ---- text helpers ---------------------------------------------------------

#[test]
fn splitlines_breaks_like_python() {
    assert_eq!(
        splitlines("a\nb\r\nc\rd\x0be\u{2028}f"),
        ["a", "b", "c", "d", "e", "f"]
    );
    assert_eq!(splitlines("a\n\nb\n"), ["a", "", "b"]);
    assert!(splitlines("").is_empty());
}

#[test]
fn errors_and_floats_print_like_python() {
    let missing = fs::read_to_string("/no/such/file").unwrap_err();
    assert_eq!(
        os_error(&missing, "/no/such/file"),
        "[Errno 2] No such file or directory: '/no/such/file'"
    );
    let custom = std::io::Error::other("boom");
    assert_eq!(os_error(&custom, "p"), "boom: 'p'");
    assert_eq!(
        (float_repr(50.0), float_repr(0.5), float_repr(0.0)),
        ("50.0".to_string(), "0.5".to_string(), "0.0".to_string())
    );
    assert_eq!(
        (round(1.25, 1), round(2.675, 2), round(83.333, 1)),
        (1.2, 2.67, 83.3)
    );
}

// ---- json -----------------------------------------------------------------

#[test]
fn json_dump_matches_python_indent_two() {
    let value = J::obj(vec![
        ("empty_list", J::Arr(vec![])),
        ("empty_obj", J::obj(vec![])),
        ("null", J::Null),
        ("int", J::Int(-3)),
        ("float", J::Float(2.0)),
        ("text", J::s("q\"b\\n\n\r\t\u{8}\u{c}\u{1}é🚀\u{7f}")),
        ("list", J::Arr(vec![J::Int(1), J::s("x")])),
        (
            "nested",
            J::obj(vec![("k", J::Arr(vec![J::obj(vec![("a", J::Int(1))])]))]),
        ),
    ]);
    let expected = "{\n  \"empty_list\": [],\n  \"empty_obj\": {},\n  \"null\": null,\n  \"int\": -3,\n  \"float\": 2.0,\n  \"text\": \"q\\\"b\\\\n\\n\\r\\t\\b\\f\\u0001\\u00e9\\ud83d\\ude80\\u007f\",\n  \"list\": [\n    1,\n    \"x\"\n  ],\n  \"nested\": {\n    \"k\": [\n      {\n        \"a\": 1\n      }\n    ]\n  }\n}";
    assert_eq!(value.dump(), expected);
}

#[test]
fn json_accessors_are_forgiving() {
    let v = J::obj(vec![
        ("a", J::Int(2)),
        ("b", J::Float(1.5)),
        ("c", J::Null),
        ("d", J::s("t")),
    ]);
    assert_eq!(
        (
            v.get("a").number(),
            v.get("b").number(),
            v.get("d").number(),
            v.get("zz").number()
        ),
        (2.0, 1.5, 0.0, 0.0)
    );
    assert_eq!(
        (
            J::Int(1).get("a").show(),
            v.get("c").show(),
            v.get("d").show()
        ),
        ("None".to_string(), "None".to_string(), "t".to_string())
    );
    assert_eq!(
        (
            v.get("a").show(),
            v.get("b").show(),
            J::Arr(vec![J::Int(1)]).show().replace('\n', "")
        ),
        ("2".to_string(), "1.5".to_string(), "[  1]".to_string())
    );
    assert!(J::Null.entries().is_empty() && v.entries().len() == 4);
    assert!(J::Null.is_empty_list() && !J::Arr(vec![J::Null]).is_empty_list());
}

// ---- the regex engine -----------------------------------------------------

fn cs(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn found(pattern: &str, flags: crate::regex::Flags, text: &str) -> Option<String> {
    let t = cs(text);
    Regex::new(pattern, flags)
        .search(&t, 0)
        .map(|m| t[m.start..m.end].iter().collect())
}

#[test]
fn regex_literals_classes_and_escapes() {
    assert_eq!(found(r"a.c", NONE, "xabcx").as_deref(), Some("abc"));
    assert_eq!(found(r"a.c", NONE, "a\nc"), None);
    assert_eq!(found(r"a.c", S, "a\nc").as_deref(), Some("a\nc"));
    assert_eq!(found(r"\d+\.\d", NONE, "v 12.5!").as_deref(), Some("12.5"));
    assert_eq!(found(r"\D\S\W", NONE, "1a-_ ").as_deref(), Some("-_ "));
    assert_eq!(found(r"\s\w", NONE, "a é").as_deref(), Some(" é"));
    assert_eq!(
        found(r"[a-c0-2_\]-]+", NONE, "xx_b]-1z").as_deref(),
        Some("_b]-1")
    );
    assert_eq!(found(r"[^a-c\n]+", NONE, "abxy\nz").as_deref(), Some("xy"));
    assert_eq!(
        found(r"[\w.-]+", NONE, "!!a.b-c!").as_deref(),
        Some("a.b-c")
    );
    assert_eq!(found(r"[\S\d]+", NONE, " x1 ").as_deref(), Some("x1"));
    assert_eq!(found(r"\(x\)", NONE, "(x)").as_deref(), Some("(x)"));
    assert_eq!(found(r"[A-Z]+", I, "abC").as_deref(), Some("abC"));
    assert_eq!(found(r"[xX]y", NONE, "Xy").as_deref(), Some("Xy"));
    assert_eq!(found(r"K\w", I, "kk").as_deref(), Some("kk"));
}

#[test]
fn regex_anchors_and_boundaries() {
    assert_eq!(found(r"^b", NONE, "a\nb"), None);
    assert_eq!(found(r"^b", M, "a\nb").as_deref(), Some("b"));
    assert_eq!(found(r"a$", NONE, "a\n").as_deref(), Some("a"));
    assert_eq!(found(r"a$", NONE, "a\nb"), None);
    assert_eq!(found(r"a$", M, "a\nb").as_deref(), Some("a"));
    assert_eq!(
        found(r"\bfix\b", NONE, "prefix fix").as_deref(),
        Some("fix")
    );
    assert_eq!(found(r"\bfix\b", NONE, "prefix"), None);
    assert_eq!(
        found(r"(?<![\w/])#\d", NONE, "a#1 b/#2 #3").as_deref(),
        Some("#3")
    );
    assert_eq!(found(r"(?<![\w/])#\d", NONE, "#4").as_deref(), Some("#4"));
}

#[test]
fn regex_groups_alternation_and_repeats() {
    let t = cs("[AB-12] fix");
    let re = Regex::new(r"^\[?([A-Z][A-Z0-9]{1,9})[- ](\d+)\]?[:\s]", NONE);
    let m: Match = re.match_at(&t, 0).unwrap();
    assert_eq!(
        (
            m.group(&t, 1).as_deref(),
            m.group(&t, 2).as_deref(),
            m.span(3),
            m.group(&t, 9)
        ),
        (Some("AB"), Some("12"), None, None)
    );
    assert!(re.match_at(&cs("[A-12] x"), 0).is_none());
    assert!(re.match_at(&cs("ABCDEFGHIJK-1 "), 0).is_none());
    // Lazy and greedy, with backtracking through a group.
    assert_eq!(
        found(r"<!--.*?-->", S, "<!-- a --> b -->").as_deref(),
        Some("<!-- a -->")
    );
    assert_eq!(
        found(r"<!--.*-->", S, "<!-- a --> b -->").as_deref(),
        Some("<!-- a --> b -->")
    );
    assert_eq!(
        found(r"(?:ab|a)(?:b)?c", NONE, "abc").as_deref(),
        Some("abc")
    );
    assert_eq!(
        found(r"(?:s|erences)?x", NONE, "erencesx").as_deref(),
        Some("erencesx")
    );
    assert_eq!(found(r"(?:ab){2}", NONE, "ababab").as_deref(), Some("abab"));
    assert_eq!(
        found(r"(?:ab){2,}?c", NONE, "ababc").as_deref(),
        Some("ababc")
    );
    assert_eq!(found(r"(?:ab)+?c", NONE, "ababc").as_deref(), Some("ababc"));
    assert_eq!(found(r"(?:ab){3}", NONE, "abab"), None);
    assert_eq!(found(r"x{2,3}", NONE, "xxxxx").as_deref(), Some("xxx"));
    assert_eq!(found(r"x{2}", NONE, "x"), None);
    // A group that matches nothing cannot loop forever.
    assert_eq!(found(r"(?:a*)*b", NONE, "aab").as_deref(), Some("aab"));
    // A capture set in a failed branch is undone.
    let t = cs("ab");
    let m = Regex::new(r"(?:(a)c|ab)", NONE).match_at(&t, 0).unwrap();
    assert_eq!(m.span(1), None);
}

#[test]
fn regex_find_all_is_non_overlapping() {
    let t = cs("a1b22c");
    let all = Regex::new(r"\d+", NONE).find_all(&t);
    assert_eq!(
        all.iter().map(|m| (m.start, m.end)).collect::<Vec<_>>(),
        [(1, 2), (3, 5)]
    );
    assert!(Regex::new(r"z", NONE).find_all(&t).is_empty());
    // An empty match moves on by one rather than looping.
    assert_eq!(Regex::new(r"x*", NONE).find_all(&cs("ab")).len(), 3);
}

// ---- mine-prs -------------------------------------------------------------

fn mine(dump: &str, extra: &[&str]) -> (i32, String, String) {
    let mut args = vec!["mine-prs", dump];
    args.extend_from_slice(extra);
    exec(&args)
}

fn file(dir: &std::path::Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).unwrap()
}

#[test]
fn mine_prs_matches_the_reference_profile() {
    for name in ["merged_prs", "merged_small", "empty"] {
        let dir = scratch(name);
        let out = dir.join("profile.json");
        let (code, stdout, err) = mine(
            &fx(&format!("{name}.json")),
            &["--out", &out.to_string_lossy(), "--md"],
        );
        assert_eq!((code, err.as_str()), (0, ""));
        assert!(
            stdout.starts_with(&format!(
                "profile -> {}\nsummary -> {}\n",
                out.display(),
                dir.join("profile.md").display()
            )),
            "{stdout}"
        );
        assert_eq!(
            file(&dir, "profile.json"),
            golden(&format!("{name}.json")),
            "{name}"
        );
        assert_eq!(
            file(&dir, "profile.md"),
            golden(&format!("{name}.md")),
            "{name}"
        );
    }
}

#[test]
fn mine_prs_prints_the_sample_size_and_a_warning() {
    let dir = scratch("small");
    let out = dir.join("p.json");
    let (_, stdout, _) = mine(&fx("merged_small.json"), &["--out", &out.to_string_lossy()]);
    assert_eq!(
        stdout,
        format!("profile -> {}\nsample_size=3\nWARNING: only 3 merged PRs — treat every rate here as anecdote, lean on the template and conventional defaults instead\n", out.display())
    );
    assert!(!dir.join("p.md").exists());
}

#[test]
fn mine_prs_unwraps_data_and_prs_objects() {
    for (name, size) in [("merged_wrapped.json", 2), ("merged_wrapped_prs.json", 1)] {
        let dir = scratch(name);
        let (code, stdout, _) = mine(&fx(name), &["--out", &dir.join("p.json").to_string_lossy()]);
        assert_eq!(code, 0);
        assert!(
            stdout.contains(&format!("sample_size={size}\n")),
            "{stdout}"
        );
    }
    // An object with neither key is an empty dump.
    let dir = scratch("object");
    let (code, stdout, _) = mine(
        &fx("object.json"),
        &["--out", &dir.join("p.json").to_string_lossy()],
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("sample_size=0\nWARNING: no merged PRs"),
        "{stdout}"
    );
}

#[test]
fn mine_prs_names_its_output_after_the_dump() {
    let dir = scratch("default-out");
    let dump = dir.join("prs.v1.json");
    fs::copy(fx("merged_small.json"), &dump).unwrap();
    let (code, stdout, _) = mine(&dump.to_string_lossy(), &["--md"]);
    assert_eq!(code, 0);
    assert!(
        stdout.starts_with(&format!(
            "profile -> {}/prs.v1.style.json\nsummary -> {}/prs.v1.style.md\n",
            dir.display(),
            dir.display()
        )),
        "{stdout}"
    );
    assert!(dir.join("prs.v1.style.json").exists() && dir.join("prs.v1.style.md").exists());
    // No extension, and a hidden file.
    let bare = dir.join("bare");
    fs::copy(fx("merged_small.json"), &bare).unwrap();
    assert!(mine(&bare.to_string_lossy(), &[])
        .1
        .starts_with(&format!("profile -> {}/bare.style.json\n", dir.display())));
    let hidden = dir.join(".hidden");
    fs::copy(fx("merged_small.json"), &hidden).unwrap();
    assert!(mine(&hidden.to_string_lossy(), &[]).1.starts_with(&format!(
        "profile -> {}/.hidden.style.json\n",
        dir.display()
    )));
    let out = dir.join("noext");
    assert!(mine(
        &fx("merged_small.json"),
        &["--out", &out.to_string_lossy(), "--md"]
    )
    .1
    .contains(&format!("summary -> {}.md", out.display())));
}

#[test]
fn mine_prs_refuses_what_it_cannot_read_or_write() {
    let (code, out, err) = mine("/no/such/dump.json", &[]);
    assert_eq!((code, out.as_str()), (1, ""));
    assert_eq!(err, "could not read /no/such/dump.json: [Errno 2] No such file or directory: '/no/such/dump.json'\n");
    let (code, _, err) = mine(&fx("bad.json"), &[]);
    assert_eq!(code, 1);
    assert!(
        err.starts_with(&format!("could not read {}: ", fx("bad.json"))),
        "{err}"
    );
    let (code, _, err) = mine(&fx("scalar.json"), &[]);
    assert_eq!((code, err.as_str()), (1, "expected a JSON array of PRs\n"));

    let dir = scratch("unwritable");
    let missing = dir.join("no-dir").join("p.json");
    let (code, _, err) = mine(&fx("empty.json"), &["--out", &missing.to_string_lossy()]);
    assert_eq!(code, 1);
    assert!(
        err.starts_with(&format!("could not write {}: [Errno 2]", missing.display())),
        "{err}"
    );
    // The summary path is a directory.
    fs::create_dir(dir.join("p.md")).unwrap();
    let (code, _, err) = mine(
        &fx("empty.json"),
        &["--out", &dir.join("p.json").to_string_lossy(), "--md"],
    );
    assert_eq!(code, 1);
    assert!(
        err.contains("could not write") && err.contains("p.md"),
        "{err}"
    );
}

#[test]
fn mine_prs_reports_a_bad_command_line() {
    let (code, _, err) = exec(&["mine-prs"]);
    assert_eq!(code, 2);
    assert!(
        err.contains("pr-composer mine-prs: error: the following arguments are required: dump"),
        "{err}"
    );
    let (code, _, err) = exec(&["mine-prs", "a.json", "b.json"]);
    assert_eq!(code, 2);
    assert!(err.contains("unrecognized arguments: b.json"), "{err}");
}

fn analyse_text(json: &str) -> J {
    let prs: Vec<serde_json::Value> = serde_json::from_str(json).unwrap();
    mine_prs::analyse(&prs)
}

#[test]
fn analyse_handles_sparse_and_odd_records() {
    let mut prs = vec![
        r#"{"number":1}"#.to_string(),
        "7".to_string(),
        "null".to_string(),
    ];
    // Names, slugs and requested reviewers; blank and non-string fields.
    prs.push(r#"{"number":2,"title":5,"body":null,"labels":[{"name":"a/b"},{"name":""},3,{"name":":x"},{"name":"v : 1"}],
        "reviews":[{"author":{"name":"n1"},"state":"approved"},{"author":"str"},{"author":{"slug":"s1"}},{"author":{"login":""}}],
        "reviewRequests":[{"requestedReviewer":{"login":"r1"}},{"slug":"s2"},{},5],
        "assignees":[{"login":"as"},{}],"files":[{"path":""},{"path":"/"},{"path":"solo"},9],
        "additions":1.9,"deletions":null,"changedFiles":"x","baseRefName":""}"#.to_string());
    let prof = analyse_text(&format!("[{}]", prs.join(",")));
    assert_eq!(prof.get("sample_size").show(), "4");
    let people = prof.get("people");
    assert_eq!(
        people.get("top_approvers").list()[0].get("value").show(),
        "n1"
    );
    assert_eq!(people.get("top_requested").list().len(), 2);
    assert_eq!(people.get("top_assignees").list().len(), 1);
    let paths: Vec<String> = match people.get("reviewer_by_path") {
        J::Obj(items) => items.iter().map(|(k, _)| k.clone()).collect(),
        _ => vec![],
    };
    assert_eq!(paths, ["/", "solo"]);
    assert_eq!(
        prof.get("meta").get("base_branches").list()[0]
            .get("value")
            .show(),
        "?"
    );
}

#[test]
fn analyse_orders_ties_by_first_seen_and_ranks_families() {
    let mut prs = Vec::new();
    for i in 0..30 {
        let labels = if i % 2 == 0 {
            r#"[{"name":"type: a"},{"name":"size/s"}]"#
        } else {
            r#"[{"name":"type: b"},{"name":"area/x"}]"#
        };
        let body = if i % 3 == 0 {
            "## Summary\n\nTBD\n\n## Notes\n\nlong enough words here to count over the limit of eight"
        } else {
            "## Summary\n\nTBD"
        };
        prs.push(format!(r#"{{"number":{i},"title":"feat(s{}): t","body":{},"labels":{labels},"additions":{i},"deletions":0,"changedFiles":{}}}"#, i % 2, serde_json::Value::String(body.into()), i % 4));
    }
    let prof = analyse_text(&format!("[{}]", prs.join(",")));
    let meta = prof.get("meta");
    assert_eq!(meta.get("required_label_families").list().len(), 1);
    assert_eq!(
        meta.get("required_label_families").list()[0]
            .get("family")
            .show(),
        "type"
    );
    assert_eq!(meta.get("label_families").list().len(), 3);
    let hollow = prof.get("body").get("routinely_hollow_sections").list();
    assert_eq!(hollow[0].get("section").show(), "summary");
    assert_eq!(prof.get("title").get("common_scopes").list().len(), 2);
    assert!(!prof
        .get("body")
        .get("modal_section_order")
        .list()
        .is_empty());
    assert_eq!(prof.get("title").get("p90_length").show(), "11");
    assert_eq!(prof.get("meta").get("median_files_changed").show(), "1");
    assert!(prof.get("warning") == &J::Null);
}

#[test]
fn render_md_covers_every_optional_section() {
    // No people, no labels, no checklists: the short form.
    let prof = analyse_text(r#"[{"number":1,"title":"x","body":"hi"}]"#);
    let md = mine_prs::render_md(&prof);
    assert!(
        md.contains("preferred keyword: **none**") && md.contains("- labelled PRs: 0.0%\n"),
        "{md}"
    );
    assert!(
        !md.contains("Sections by frequency")
            && !md.contains("Typical order")
            && !md.contains("Reviewers by area")
            && !md.contains("frequent reviewers")
    );
    assert!(md.contains("> **only 1 merged PRs"), "{md}");
    // A hollow section under 40 percent still prints the heading line.
    let busy = (0..10)
        .map(|i| {
            format!(
                r###"{{"number":{i},"title":"fix: y","body":"## Sec\n\n{}"}}"###,
                if i < 2 {
                    "few"
                } else {
                    "plenty of words here, more than eight of them in this section"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let md = mine_prs::render_md(&analyse_text(&format!("[{busy}]")));
    assert!(
        md.contains("Sections usually left hollow") && !md.contains("hollow of"),
        "{md}"
    );
}

#[test]
fn mine_prs_dump_shapes_by_truthiness() {
    let dir = scratch("shapes");
    let out = dir.join("p.json").to_string_lossy().into_owned();
    for (json, code, text) in [
        (
            r#"{"data": false, "prs": [{"number":1}]}"#,
            0,
            "sample_size=1\n",
        ),
        (r#"{"data": 0, "prs": {}}"#, 0, "sample_size=0\n"),
        (r#"{"data": null, "prs": ""}"#, 0, "sample_size=0\n"),
        (r#"{"data": 2.5}"#, 1, ""),
        (r#"{"data": true}"#, 1, ""),
        (r#"{"data": {"a": 1}}"#, 1, ""),
    ] {
        let dump = dir.join("d.json");
        fs::write(&dump, json).unwrap();
        let (status, stdout, _) = mine(&dump.to_string_lossy(), &["--out", &out]);
        assert_eq!(status, code, "{json}");
        assert!(stdout.contains(text), "{json}: {stdout}");
    }
}

#[test]
fn analyse_deduplicates_repeated_headings_and_reviewers() {
    let prof = analyse_text(
        r###"[{"number":1,"title":"x","body":"## A\n\ntext\n\n## A\n\ntext\n\n## ***\n\nmore","reviews":[{"author":{"login":"u"},"state":"APPROVED"}],"reviewRequests":[{"login":"u"}]}]"###,
    );
    let freq = prof.get("body").get("section_frequency").list();
    assert_eq!(
        (
            freq.len(),
            freq[0].get("section").show(),
            freq[0].get("rate").show()
        ),
        (1, "a".to_string(), "100.0".to_string())
    );
    assert_eq!(
        prof.get("people").get("top_reviewers").list()[0]
            .get("count")
            .show(),
        "1"
    );
}
