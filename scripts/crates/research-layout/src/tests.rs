//! Unit tests: they execute every line of the crate except `src/main.rs`.
//! Fixtures live under `std::env::temp_dir()`; `fetch` runs a stub `curl`
//! (a shell script) and `verify` runs the real `git` over a fixture
//! repository.

use super::*;

#[path = "../../../test-support/fake_exe.rs"]
mod fake_exe;
use crate::fetch::{adopt, fetch, read_plan, Failure, Limits, LOG};
use crate::html::page_text;
use crate::layout::{note_quotes, MARKER};
use crate::verify::{changed_notes, verify};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const P1: &str = r#"{"id":"p1","title":"Host updates","theme":"hosts/updates","tier":"A","files":["hosts/updates/p1.html"],"year":2019,"summary":"Updates to hosts."}"#;
const P2: &str = r#"{"id":"p2","title":"Privacy papers","theme":"privacy","tier":"B","files":["privacy/p2.html"],"year":2020,"abstract":"PDF pages."}"#;

fn run_args(args: &[&str]) -> i32 {
    run(args.iter().map(OsString::from))
}

fn fixture(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("research-layout-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A papers directory that breaks no rule: two themes, two entries, one
/// note citing one entry, one verified quote.
fn passing(tag: &str) -> PathBuf {
    let root = fixture(tag);
    fs::write(root.join("catalog.json"), format!("[{P1},{P2}]")).unwrap();
    fs::write(root.join("README.md"), "# Papers\n").unwrap();
    fs::write(root.join("themes.md"), "# Themes\n").unwrap();
    fs::create_dir_all(root.join("notes")).unwrap();
    fs::write(root.join("notes/README.md"), "# Notes\n").unwrap();
    fs::write(root.join("notes/plain.md"), "a note without front matter\n").unwrap();
    let note = "---\nmin_tier: B\ncites: [p1]\n---\n\nA study note about hosts.\n\n\
                > The host\n> updated.\n\n-- hosts/updates/p1.html\n\n\
                > Orphan quote, cleared by prose.\n\nProse.\n\n-- privacy/p2.html\n";
    fs::write(root.join("notes/good-note.md"), note).unwrap();
    fs::create_dir_all(root.join("evaluations")).unwrap();
    fs::write(root.join("evaluations/README.md"), "# Evaluations\n").unwrap();
    fs::create_dir_all(root.join("hosts/updates/p1-images")).unwrap();
    fs::write(
        root.join("hosts/updates/p1.html"),
        "<html>\nThe host\n  updated. Other words.\n</html>\n",
    )
    .unwrap();
    fs::write(root.join("hosts/updates/README.md"), "# Hosts\n").unwrap();
    fs::write(root.join("hosts/updates/p1-images/fig.png"), "png").unwrap();
    fs::create_dir_all(root.join("privacy")).unwrap();
    fs::write(
        root.join("privacy/p2.html"),
        "<html>A privacy paper is one that protects users.\n</html>",
    )
    .unwrap();
    fs::write(root.join("privacy/README.md"), "# Privacy\n").unwrap();
    write_indexes(&root).unwrap();
    root
}

#[test]
fn clean_fixture_passes() {
    let root = passing("clean");
    assert!(check(&root).is_empty());
    assert_eq!(
        run_args(&["check", root.to_str().unwrap()]),
        0,
        "clean fixture must exit zero"
    );
}

#[test]
fn unlisted_theme_file_fails() {
    let root = passing("unlisted");
    fs::write(root.join("hosts/updates/rogue.pdf"), "x").unwrap();
    assert_eq!(check(&root), vec!["hosts/updates/rogue.pdf: unlisted-file"]);
    assert_eq!(run_args(&["check", root.to_str().unwrap()]), 1);
}

#[test]
fn missing_entry_file_fails() {
    let root = passing("missing-file");
    fs::remove_file(root.join("hosts/updates/p1.html")).unwrap();
    assert_eq!(
        check(&root),
        vec![
            "hosts/updates/p1.html: file-not-found",
            "hosts/updates/p1.html: quote-not-found",
        ]
    );
}

#[test]
fn theme_without_readme_fails() {
    let root = passing("no-readme");
    fs::remove_file(root.join("privacy/README.md")).unwrap();
    assert_eq!(check(&root), vec!["privacy: missing-theme-readme"]);
}

#[test]
fn missing_theme_folder_reports_readme_and_stale_index() {
    let root = passing("no-theme");
    fs::remove_dir_all(root.join("privacy")).unwrap();
    assert_eq!(
        check(&root),
        vec![
            "privacy/p2.html: file-not-found",
            "privacy: missing-theme-readme",
            "privacy/INDEX.md: stale-index",
        ]
    );
}

#[test]
fn stale_indexes_fail() {
    let root = passing("stale");
    fs::write(root.join("INDEX.md"), "old\n").unwrap();
    fs::remove_file(root.join("hosts/updates/INDEX.md")).unwrap();
    assert_eq!(
        check(&root),
        vec![
            "INDEX.md: stale-index",
            "hosts/updates/INDEX.md: stale-index",
        ]
    );
    assert_eq!(run_args(&["check", root.to_str().unwrap()]), 1);
}

#[test]
fn top_level_junk_fails() {
    let root = passing("junk");
    fs::create_dir_all(root.join("junk")).unwrap();
    fs::write(root.join("stray.txt"), "x").unwrap();
    assert_eq!(
        check(&root),
        vec!["junk: not-allowed", "stray.txt: not-allowed"]
    );
}

#[test]
fn entry_with_missing_field_fails() {
    let root = passing("missing-field");
    let bad = r#"{"id":"p3","theme":"hosts/updates","tier":"A","files":[]}"#;
    fs::write(root.join("catalog.json"), format!("[{P1},{P2},{bad}]")).unwrap();
    assert_eq!(check(&root), vec!["p3: missing-field: title"]);
}

#[test]
fn entry_without_id_reports_the_catalog() {
    let root = passing("no-id");
    fs::write(root.join("catalog.json"), format!("[{P1},{P2},{{}}]")).unwrap();
    assert_eq!(check(&root), vec!["catalog.json: missing-field: id"]);
}

#[test]
fn entry_with_bad_field_type_fails() {
    let root = passing("bad-field");
    let bad = r#"{"title":3}"#;
    fs::write(root.join("catalog.json"), format!("[{P1},{P2},{bad}]")).unwrap();
    let problems = check(&root);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("catalog.json: bad-catalog-entry: "));
}

#[test]
fn entry_with_bad_tier_fails() {
    let root = passing("bad-tier");
    let bad = r#"{"id":"p3","title":"Third","theme":"hosts/updates","tier":"E","files":[]}"#;
    fs::write(root.join("catalog.json"), format!("[{P1},{P2},{bad}]")).unwrap();
    assert_eq!(check(&root), vec!["p3: bad-tier"]);
}

#[test]
fn note_citing_unknown_id_fails() {
    let root = passing("unknown-cite");
    fs::write(
        root.join("notes/good-note.md"),
        "---\nmin_tier: B\ncites: [ghost]\n---\n\nText.\n",
    )
    .unwrap();
    assert_eq!(check(&root), vec!["notes/good-note.md: unknown-cite"]);
}

#[test]
fn note_citing_below_min_tier_fails() {
    let root = passing("tier-below-min");
    fs::write(
        root.join("notes/good-note.md"),
        "---\nmin_tier: A\ncites: [p1, p2]\n---\n\nText.\n",
    )
    .unwrap();
    assert_eq!(check(&root), vec!["notes/good-note.md: tier-below-min"]);
}

#[test]
fn unknown_min_tier_defaults_to_b() {
    let root = passing("min-tier");
    fs::write(
        root.join("notes/good-note.md"),
        "---\nmin_tier: Q\ncites: [p2]\n---\n\nText.\n",
    )
    .unwrap();
    assert!(check(&root).is_empty());
}

#[test]
fn note_without_front_matter_takes_the_defaults() {
    let root = passing("no-front");
    fs::write(
        root.join("notes/plain.md"),
        "> A quote with no page.\n\nText.\n-- nothing\n",
    )
    .unwrap();
    // No front matter: min_tier defaults to B and cites to none, so the
    // note breaks no rule while the cleared orphan quote goes unchecked.
    assert!(check(&root).is_empty());
}

#[test]
fn unclosed_front_matter_swallows_the_body() {
    let root = passing("unclosed");
    fs::write(
        root.join("notes/good-note.md"),
        "---\ncites: [ghost]\nno closing marker\n",
    )
    .unwrap();
    assert_eq!(check(&root), vec!["notes/good-note.md: unknown-cite"]);
}

#[test]
fn note_quote_text_missing_fails() {
    let root = passing("quote-missing");
    fs::write(
        root.join("notes/good-note.md"),
        "> Not in the page.\n\n-- hosts/updates/p1.html\n",
    )
    .unwrap();
    assert_eq!(check(&root), vec!["hosts/updates/p1.html: quote-not-found"]);
}

#[test]
fn note_quote_page_missing_fails() {
    let root = passing("quote-gone");
    fs::write(
        root.join("notes/good-note.md"),
        "> The host updated.\n\n-- privacy/gone.html\n",
    )
    .unwrap();
    assert_eq!(check(&root), vec!["privacy/gone.html: quote-not-found"]);
}

#[test]
fn entry_quote_found_passes() {
    let root = passing("entry-quote-pass");
    let p1 = r#"{"id":"p1","title":"Host updates","theme":"hosts/updates","tier":"A","files":["hosts/updates/p1.html"],"year":2019,"quotes":[{"text":"The   host updated.","page":"hosts/updates/p1.html"}]}"#;
    fs::write(root.join("catalog.json"), format!("[{p1},{P2}]")).unwrap();
    // The new catalog changes the generated indexes; regenerate them so
    // only the entries' own rules are under test.
    write_indexes(&root).unwrap();
    assert!(check(&root).is_empty());
}

#[test]
fn entry_quote_not_found_fails() {
    let root = passing("entry-quote-fail");
    let p1 = r#"{"id":"p1","title":"Host updates","theme":"hosts/updates","tier":"A","files":["hosts/updates/p1.html"],"year":2019,"quotes":[{"text":"not there","page":"hosts/updates/p1.html"}]}"#;
    fs::write(root.join("catalog.json"), format!("[{p1},{P2}]")).unwrap();
    // The new catalog changes the generated indexes; regenerate them so
    // only the quote rule is under test.
    write_indexes(&root).unwrap();
    assert_eq!(check(&root), vec!["hosts/updates/p1.html: quote-not-found"]);
}

#[test]
fn unreadable_catalog_fails() {
    let root = fixture("no-catalog");
    let problems = check(&root);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("catalog.json: cannot read:"));
    assert_eq!(run_args(&["check", root.to_str().unwrap()]), 1);
}

#[test]
fn catalog_not_json_or_not_array_fails() {
    let root = fixture("bad-json");
    fs::write(root.join("catalog.json"), "{not json").unwrap();
    let problems = check(&root);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("catalog.json: not valid JSON:"));
    fs::write(root.join("catalog.json"), "{\"id\":\"p1\"}").unwrap();
    assert_eq!(check(&root), vec!["catalog.json: not an array"]);
}

#[test]
fn index_generates_deterministic_output_twice() {
    let root = passing("index-idempotent");
    let top_before = fs::read_to_string(root.join("INDEX.md")).unwrap();
    let theme_before = fs::read_to_string(root.join("hosts/updates/INDEX.md")).unwrap();
    let want_top = format!(
        "{MARKER}\n# Papers\n\n## Themes\n\n\
         - [hosts/updates](hosts/updates) (1 entry)\n\
         - [privacy](privacy) (1 entry)\n\n\
         ## Entries\n\n\
         - **Host updates** (2019, tier A) — [hosts/updates](hosts/updates)\n\
         - **Privacy papers** (2020, tier B) — [privacy](privacy)\n"
    );
    assert_eq!(top_before, want_top);
    assert_eq!(
        theme_before,
        format!(
            "{MARKER}\n# hosts/updates\n\n\
             - **Host updates** (2019, tier A): Updates to hosts.\n"
        )
    );
    fs::write(root.join("INDEX.md"), "clobbered\n").unwrap();
    assert_eq!(run_args(&["index", root.to_str().unwrap()]), 0);
    assert_eq!(fs::read_to_string(root.join("INDEX.md")).unwrap(), want_top);
    assert_eq!(
        fs::read_to_string(root.join("privacy/INDEX.md")).unwrap(),
        format!("{MARKER}\n# privacy\n\n- **Privacy papers** (2020, tier B)\n")
    );
    assert_eq!(run_args(&["index", root.to_str().unwrap()]), 0);
    assert_eq!(
        fs::read_to_string(root.join("INDEX.md")).unwrap(),
        want_top,
        "a second run must change nothing"
    );
    assert_eq!(
        fs::read_to_string(root.join("hosts/updates/INDEX.md")).unwrap(),
        theme_before,
        "a second run must change nothing"
    );
}

#[test]
fn empty_catalog_generates_the_exact_top_index() {
    let root = fixture("empty-index");
    fs::write(root.join("catalog.json"), "[]").unwrap();
    assert_eq!(write_indexes(&root), Ok(1));
    let want =
        "<!-- generated by research-layout index; do not edit -->\n# Papers\n\nNo entries yet.\n";
    assert_eq!(fs::read_to_string(root.join("INDEX.md")).unwrap(), want);
    assert_eq!(write_indexes(&root), Ok(1));
    assert_eq!(fs::read_to_string(root.join("INDEX.md")).unwrap(), want);
}

#[test]
fn index_sorts_entries_by_year_then_title() {
    let root = fixture("index-sort");
    let a = r#"{"id":"a","title":"A paper","theme":"t","tier":"C","files":[],"year":2021}"#;
    let b = r#"{"id":"b","title":"B paper","theme":"t","tier":"A","files":[]}"#;
    let c = r#"{"id":"c","title":"C paper","theme":"t","tier":"D","files":[],"year":2019}"#;
    fs::write(root.join("catalog.json"), format!("[{a},{b},{c}]")).unwrap();
    assert_eq!(write_indexes(&root), Ok(2));
    assert_eq!(
        fs::read_to_string(root.join("t/INDEX.md")).unwrap(),
        format!(
            "{MARKER}\n# t\n\n\
             - **B paper** (tier A)\n\
             - **C paper** (2019, tier D)\n\
             - **A paper** (2021, tier C)\n"
        )
    );
    let top = fs::read_to_string(root.join("INDEX.md")).unwrap();
    assert!(top.contains("- [t](t) (3 entries)\n"));
}

#[test]
fn index_refuses_a_broken_catalog() {
    let root = passing("index-broken");
    let bad = r#"{"id":"p3","theme":"hosts/updates","tier":"A","files":[]}"#;
    fs::write(root.join("catalog.json"), format!("[{P1},{bad}]")).unwrap();
    assert_eq!(
        write_indexes(&root),
        Err(vec!["p3: missing-field: title".to_string()])
    );
    assert_eq!(run_args(&["index", root.to_str().unwrap()]), 1);
}

#[test]
fn index_reports_a_write_failure() {
    let root = fixture("index-write-fail");
    fs::write(root.join("catalog.json"), format!("[{P1}]")).unwrap();
    fs::create_dir_all(root.join("INDEX.md")).unwrap();
    match write_indexes(&root) {
        Err(problems) => {
            assert_eq!(problems.len(), 1);
            assert!(problems[0].starts_with("INDEX.md: cannot write:"));
        }
        Ok(_) => panic!("writing an INDEX.md directory must fail"),
    }
}

#[test]
fn index_reports_a_theme_dir_failure() {
    let root = fixture("index-dir-fail");
    fs::write(root.join("catalog.json"), format!("[{P1}]")).unwrap();
    fs::write(root.join("hosts"), "a file where a folder belongs").unwrap();
    match write_indexes(&root) {
        Err(problems) => {
            assert_eq!(problems.len(), 1);
            assert!(problems[0].starts_with("hosts/updates/INDEX.md: cannot create:"));
        }
        Ok(_) => panic!("creating a theme dir under a file must fail"),
    }
    assert_eq!(run_args(&["index", root.to_str().unwrap()]), 1);
}

// ---------------------------------------------------------------------
// Note quotes, page text.

#[test]
fn note_quotes_pairs_quotes_with_their_page() {
    let body = vec![
        "> one",
        "> two",
        "",
        "-- a.txt",
        "-- b.txt",
        "> dropped",
        "prose",
        "-- c.txt",
    ];
    assert_eq!(
        note_quotes(&body),
        vec![("one two".to_string(), "a.txt".to_string())]
    );
}

#[test]
fn page_text_reads_html() {
    let html = "<html><head><style>p { x: 1 }</style><script>var a = '<p>';</script>\
                <SCRIPT>never</SCRIPT></head><body><!-- hidden --><h1>Title</h1>\
                <p>Fish &amp; chips &lt;3 &gt; &quot;q&quot; &apos;a&#39; &nbsp;x&#x41;&#66;</p>\
                <p>a&bogus; b&#xZZ; c&#99999999; d&; e&#; f&amp</p><span>in</span>line<br>\
                é done</body></html>";
    assert_eq!(
        page_text(html),
        "Title\nFish & chips <3 > \"q\" 'a' xAB\na&bogus; b&#xZZ; c&#99999999; d&; e&#; f&amp\nin line\né done\n"
    );
}

#[test]
fn page_text_handles_unclosed_markup() {
    assert_eq!(page_text("before<!-- never closed"), "before\n");
    assert_eq!(page_text("a<script>never closed"), "a\n");
    assert_eq!(page_text("a<style>x</style"), "a\n");
    assert_eq!(page_text("a<b"), "a\n");
}

#[test]
fn page_text_keeps_plain_text_and_drops_empty_lines() {
    assert_eq!(page_text("  one   two \n\n\t three\n"), "one two\nthree\n");
    assert_eq!(page_text(" \n "), "");
}

// ---------------------------------------------------------------------
// fetch.

/// A stub curl: writes a page for the URL after `--` into `--output`, fails
/// for a URL holding `fail`, writes nothing but markup for `empty`, and
/// writes a large page for `big`.
fn stub_curl(dir: &Path) -> String {
    let path = dir.join("curl");
    let script = r#"#!/bin/sh
out=""; url=""
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2;;
    --) url="$2"; shift 2;;
    *) shift;;
  esac
done
case "$url" in
  *fail*) echo "curl: (22) The requested URL returned error: 404" >&2; exit 22;;
  *empty*) printf '<html></html>' > "$out";;
  *big*) head -c 4096 /dev/zero | tr '\0' 'a' > "$out";;
  *) printf '<html><body><p>The page at %s says:</p><p>Retries  back off\nexponentially.</p></body></html>' "$url" > "$out";;
esac
"#;
    fake_exe::write_script(&path, script);
    path.to_string_lossy().into_owned()
}

fn limits(curl: &str) -> Limits {
    Limits {
        max_sources: 4,
        max_bytes: 1000,
        timeout_secs: 5,
        curl: curl.to_string(),
    }
}

fn write_plan(dir: &Path, sources: &str) -> PathBuf {
    let plan = dir.join("plan.json");
    fs::write(
        &plan,
        format!("{{\"question\":\"q\",\"sources\":{sources}}}"),
    )
    .unwrap();
    plan
}

fn log(out: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(out.join(LOG)).unwrap()).unwrap()
}

#[test]
fn fetch_stores_each_page_as_text_and_logs_it() {
    let root = fixture("fetch-ok");
    let curl = stub_curl(&root);
    let plan = write_plan(
        &root,
        r#"[{"id":"good","url":"https://example.org/a"},{"id":"gone","url":"https://example.org/fail"},
            {"id":"blank","url":"https://example.org/empty"},{"id":"huge","url":"https://example.org/big"}]"#,
    );
    let out = root.join("pages");
    assert_eq!(fetch(&plan, &out, &limits(&curl)), Ok(1));
    let text = fs::read_to_string(out.join("good.txt")).unwrap();
    assert_eq!(
        text,
        "The page at https://example.org/a says:\nRetries back off\nexponentially.\n"
    );
    let log = log(&out);
    assert_eq!(log["max_sources"], 4);
    let sources = log["sources"].as_array().unwrap();
    assert_eq!(sources[0]["file"], "good.txt");
    assert_eq!(
        sources[0]["sha256"],
        gold_band_report::sha256_hex(text.as_bytes())
    );
    assert_eq!(sources[0]["bytes"], text.len());
    assert_eq!(
        sources[1]["error"],
        "curl exit 22: curl: (22) The requested URL returned error: 404"
    );
    assert_eq!(sources[2]["error"], "no-text");
    assert_eq!(sources[3]["error"], "too-large: over 1000 bytes");
    assert!(!out.join("good.raw").exists());
}

#[test]
fn fetch_refuses_a_plan_that_breaks_the_rules_before_fetching() {
    let root = fixture("fetch-refused");
    let curl = stub_curl(&root);
    let plan = write_plan(
        &root,
        r#"[{"id":"a","url":"http://example.org"},{"id":"a","url":"https://127.0.0.1/x"},
            {"id":"Bad","url":"https://localhost:8080/"},{"url":"https://u@example.org/"},
            {"id":"c","url":"https://printer.local/"},{"id":"d","url":"https://[::1]/"},
            {"id":"e","url":"https://intranet/"}]"#,
    );
    let out = root.join("pages");
    let Err(Failure::Refused(problems)) = fetch(&plan, &out, &limits(&curl)) else {
        panic!("the plan must be refused");
    };
    assert_eq!(
        problems,
        vec![
            "plan: too-many-sources: 7 > 4",
            "a: not-https",
            "a: duplicate-id",
            "a: not-public",
            "Bad: bad-id",
            "Bad: not-public",
            "sources[3]: bad-id",
            "sources[3]: not-public",
            "c: not-public",
            "d: not-public",
            "e: not-public",
        ]
    );
    assert!(!out.exists(), "nothing is fetched for a refused plan");
}

#[test]
fn fetch_ids_are_bounded_file_stems() {
    let root = fixture("fetch-ids");
    let long = "a".repeat(65);
    let plan = write_plan(
        &root,
        &format!(
            r#"[{{"id":"{long}","url":"https://example.org/"}},{{"id":"-x","url":"https://example.org/"}},{{"id":"9-ok","url":"https://example.org/"}},{{"id":"a.b","url":"https://example.org/"}}]"#
        ),
    );
    let Err(Failure::Refused(problems)) = read_plan(&plan, 8) else {
        panic!("bad ids must be refused");
    };
    assert_eq!(
        problems,
        vec![
            format!("{long}: bad-id"),
            "-x: bad-id".to_string(),
            "a.b: bad-id".to_string()
        ]
    );
}

#[test]
fn fetch_reports_unreadable_plans_and_outputs() {
    let root = fixture("fetch-usage");
    let curl = stub_curl(&root);
    let missing = root.join("missing.json");
    let Err(Failure::Usage(problem)) = fetch(&missing, &root, &limits(&curl)) else {
        panic!("a missing plan is a usage failure");
    };
    assert!(problem.ends_with("cannot read: No such file or directory (os error 2)"));
    fs::write(root.join("bad.json"), "{").unwrap();
    let Err(Failure::Usage(problem)) = fetch(&root.join("bad.json"), &root, &limits(&curl)) else {
        panic!("a broken plan is a usage failure");
    };
    assert!(problem.contains("not valid JSON"));
    fs::write(root.join("bare.json"), "{}").unwrap();
    assert_eq!(
        read_plan(&root.join("bare.json"), 1).err(),
        Some(Failure::Usage(format!(
            "{}: no sources array",
            root.join("bare.json").display()
        )))
    );
    let plan = write_plan(&root, r#"[{"id":"a","url":"https://example.org/a"}]"#);
    fs::write(root.join("file"), "x").unwrap();
    let Err(Failure::Usage(problem)) = fetch(&plan, &root.join("file"), &limits(&curl)) else {
        panic!("an output path under a file is a usage failure");
    };
    assert!(problem.contains("cannot create"));
    let out = root.join("log-dir");
    fs::create_dir_all(out.join(LOG)).unwrap();
    assert_eq!(
        fetch(&plan, &out, &limits(&curl)),
        Err(Failure::Usage(
            "fetch-log.json: cannot write: Is a directory (os error 21)".to_string()
        ))
    );
}

#[test]
fn fetch_records_a_curl_that_cannot_start_or_a_page_it_cannot_write() {
    let root = fixture("fetch-broken");
    let curl = stub_curl(&root);
    let plan = write_plan(&root, r#"[{"id":"a","url":"https://example.org/a"}]"#);
    let out = root.join("pages");
    assert_eq!(
        fetch(&plan, &out, &limits("/nonexistent/curl")),
        Err(Failure::Refused(vec!["plan: no-page-fetched".to_string()]))
    );
    assert_eq!(
        log(&out)["sources"][0]["error"],
        "cannot start /nonexistent/curl: No such file or directory (os error 2)"
    );
    fs::create_dir_all(out.join("a.txt")).unwrap();
    assert_eq!(
        fetch(&plan, &out, &limits(&curl)),
        Err(Failure::Refused(vec!["plan: no-page-fetched".to_string()]))
    );
    assert_eq!(
        log(&out)["sources"][0]["error"],
        "cannot write a.txt: Is a directory (os error 21)"
    );
}

#[test]
fn fetch_command_line() {
    let root = fixture("fetch-cli");
    let curl = stub_curl(&root);
    let plan = write_plan(&root, r#"[{"id":"a","url":"https://example.org/a"}]"#);
    let out = root.join("pages");
    let (p, o) = (plan.to_str().unwrap(), out.to_str().unwrap());
    let full = [
        "fetch",
        "--plan",
        p,
        "--out",
        o,
        "--max-sources",
        "2",
        "--max-bytes",
        "9000",
        "--timeout-secs",
        "3",
        "--curl",
        &curl,
    ];
    assert_eq!(run_args(&full), 0);
    assert!(out.join("a.txt").exists());
    assert_eq!(
        run_args(&["fetch", "--plan", p, "--out", o, "--curl", "/nonexistent"]),
        1
    );
    assert_eq!(
        run_args(&["fetch", "--plan", "/nonexistent", "--out", o]),
        2
    );
    assert_eq!(run_args(&["fetch", "--plan", p]), 2);
    assert_eq!(run_args(&["fetch", "--plan", p, "--plan", p]), 2);
    assert_eq!(run_args(&["fetch", "--plan"]), 2);
    assert_eq!(run_args(&["fetch", "--bogus", "x"]), 2);
    for (flag, value) in [
        ("--max-sources", "0"),
        ("--max-sources", "x"),
        ("--max-bytes", "0"),
        ("--timeout-secs", "601"),
    ] {
        assert_eq!(
            run_args(&["fetch", "--plan", p, "--out", o, flag, value]),
            2
        );
    }
}

// ---------------------------------------------------------------------
// verify.

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.org"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
}

const PAGE_URL: &str = "https://example.org/retries";

/// A papers repository on its base commit (the passing fixture), the fetch
/// shim's output for one page, and the run's merge: the page copied into a
/// theme as a catalog entry with its url, and one note quoting it.
fn run_fixture(tag: &str) -> (PathBuf, PathBuf) {
    let repo = fixture(&format!("{tag}-repo"));
    let papers = repo.join("papers");
    fs::rename(passing(tag), &papers).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    let fetched = repo.join("fetched");
    fs::create_dir_all(&fetched).unwrap();
    let page = "Retries back off exponentially.\nJitter spreads them out.\n";
    fs::write(fetched.join("retries.txt"), page).unwrap();
    let sha = gold_band_report::sha256_hex(page.as_bytes());
    fs::write(
        fetched.join(LOG),
        format!(
            r#"{{"sources":[{{"id":"retries","url":"{PAGE_URL}","file":"retries.txt","sha256":"{sha}"}},{{"id":"gone","url":"https://example.org/gone","error":"curl exit 22"}}]}}"#
        ),
    )
    .unwrap();
    let entry = format!(
        r#"{{"id":"retries","title":"Retries","theme":"privacy","tier":"C","files":["privacy/retries.txt"],"url":"{PAGE_URL}"}}"#
    );
    fs::write(papers.join("catalog.json"), format!("[{P1},{P2},{entry}]")).unwrap();
    fs::write(papers.join("privacy/retries.txt"), page).unwrap();
    fs::write(
        papers.join("notes/retries.md"),
        "---\nmin_tier: C\ncites: [retries]\n---\n\n> Retries back off\n> exponentially.\n\n-- privacy/retries.txt\n",
    )
    .unwrap();
    write_indexes(&papers).unwrap();
    (papers, fetched)
}

fn verify_main(papers: &Path, fetched: &Path) -> Vec<String> {
    verify("git", papers, fetched, "main").unwrap()
}

#[test]
fn a_run_whose_quotes_are_on_their_fetched_pages_passes() {
    let (papers, fetched) = run_fixture("verify-ok");
    assert_eq!(verify_main(&papers, &fetched), Vec::<String>::new());
    let (p, f) = (papers.to_str().unwrap(), fetched.to_str().unwrap());
    assert_eq!(
        run_args(&["verify", "--papers", p, "--fetched", f, "--base", "main"]),
        0
    );
}

#[test]
fn a_quote_that_is_not_on_its_fetched_page_fails_verification() {
    let (papers, fetched) = run_fixture("verify-quote");
    fs::write(
        papers.join("notes/retries.md"),
        "> Retries never back off.\n\n-- privacy/retries.txt\n",
    )
    .unwrap();
    assert_eq!(
        verify_main(&papers, &fetched),
        vec!["privacy/retries.txt: quote-not-found"]
    );
    let (p, f) = (papers.to_str().unwrap(), fetched.to_str().unwrap());
    assert_eq!(
        run_args(&["verify", "--papers", p, "--fetched", f, "--base", "main"]),
        1
    );
}

#[test]
fn a_page_edited_after_the_fetch_fails_verification() {
    let (papers, fetched) = run_fixture("verify-edited");
    let forged = "Retries back off exponentially.\nRetries never fail.\n";
    fs::write(papers.join("privacy/retries.txt"), forged).unwrap();
    fs::write(
        papers.join("notes/retries.md"),
        "> Retries never fail.\n\n-- privacy/retries.txt\n",
    )
    .unwrap();
    assert_eq!(
        verify_main(&papers, &fetched),
        vec![
            "retries: page-not-as-fetched",
            "notes/retries.md: quote-page-not-fetched: privacy/retries.txt",
        ]
    );
}

#[test]
fn a_run_note_must_quote_fetched_pages_with_real_quotes() {
    let (papers, fetched) = run_fixture("verify-notes");
    fs::write(
        papers.join("notes/retries.md"),
        "> Retries\n\n-- privacy/retries.txt\n\n> The host updated.\n\n-- hosts/updates/p1.html\n\n> x\n\n-- ../escape.txt\n",
    )
    .unwrap();
    fs::write(papers.join("notes/bare.md"), "No quote at all.\n").unwrap();
    assert_eq!(
        verify_main(&papers, &fetched),
        vec![
            "../escape.txt: path-escapes",
            "notes/bare.md: no-quote",
            "notes/retries.md: quote-too-short",
            "notes/retries.md: quote-page-not-fetched: hosts/updates/p1.html",
            "notes/retries.md: quote-too-short",
            "notes/retries.md: quote-page-not-fetched: ../escape.txt",
        ]
    );
}

#[test]
fn a_run_that_adds_no_note_fails_and_old_notes_are_not_its_own() {
    let (papers, fetched) = run_fixture("verify-none");
    fs::remove_file(papers.join("notes/retries.md")).unwrap();
    assert_eq!(verify_main(&papers, &fetched), vec!["notes: no-run-note"]);
    // A changed README and a nested file under notes/ are not run notes.
    fs::write(papers.join("notes/README.md"), "# Notes, changed\n").unwrap();
    fs::create_dir_all(papers.join("notes/deep")).unwrap();
    fs::write(papers.join("notes/deep/x.md"), "x\n").unwrap();
    assert_eq!(
        changed_notes("git", &papers, "main").unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn an_entry_without_a_fetched_url_is_left_to_check() {
    let (papers, fetched) = run_fixture("verify-other-url");
    let catalog = fs::read_to_string(papers.join("catalog.json"))
        .unwrap()
        .replace(PAGE_URL, "https://example.org/elsewhere");
    fs::write(papers.join("catalog.json"), catalog).unwrap();
    assert_eq!(verify_main(&papers, &fetched), Vec::<String>::new());
    fs::write(papers.join("catalog.json"), "{").unwrap();
    let problems = verify_main(&papers, &fetched);
    assert!(
        problems[0].starts_with("catalog.json: not valid JSON"),
        "{problems:?}"
    );
}

#[test]
fn verify_reports_unreadable_inputs() {
    let (papers, fetched) = run_fixture("verify-inputs");
    let (p, f) = (papers.to_str().unwrap(), fetched.to_str().unwrap());
    assert_eq!(
        verify("/nonexistent/git", &papers, &fetched, "main"),
        Err("cannot start /nonexistent/git: No such file or directory (os error 2)".to_string())
    );
    let err = verify("git", &papers, &fetched, "no-such-ref").unwrap_err();
    assert!(err.starts_with("git diff: "), "{err}");
    assert_eq!(
        run_with_git(
            ["verify", "--papers", p, "--fetched", f].map(OsString::from),
            "/nonexistent/git"
        ),
        2
    );
    fs::write(fetched.join(LOG), "{}").unwrap();
    assert!(verify("git", &papers, &fetched, "main")
        .unwrap_err()
        .ends_with("no sources array"));
    fs::write(fetched.join(LOG), "{").unwrap();
    assert!(verify("git", &papers, &fetched, "main")
        .unwrap_err()
        .contains("not valid JSON"));
    fs::remove_file(fetched.join(LOG)).unwrap();
    assert!(verify("git", &papers, &fetched, "main")
        .unwrap_err()
        .contains("cannot read"));
    assert_eq!(run_args(&["verify", "--papers", p]), 2);
    assert_eq!(run_args(&["verify", "--bogus", p]), 2);
}

// ---------------------------------------------------------------------
// The command line.

#[test]
fn bad_command_lines_print_the_usage() {
    assert_eq!(run_args(&[]), 2);
    assert_eq!(run_args(&["check"]), 2);
    assert_eq!(run_args(&["index", "a", "b"]), 2);
    assert_eq!(run_args(&["mount", "a", "b"]), 2);
}

#[test]
fn an_entry_file_that_leaves_the_papers_directory_fails() {
    let root = passing("entry-escape");
    let bad =
        r#"{"id":"p3","title":"Third","theme":"privacy","tier":"A","files":["../outside.txt"]}"#;
    fs::write(root.join("catalog.json"), format!("[{P1},{P2},{bad}]")).unwrap();
    write_indexes(&root).unwrap();
    assert_eq!(check(&root), vec!["../outside.txt: path-escapes"]);
}

#[test]
fn index_refuses_an_unreadable_catalog() {
    let root = fixture("index-no-catalog");
    let problems = write_indexes(&root).unwrap_err();
    assert!(
        problems[0].starts_with("catalog.json: cannot read:"),
        "{problems:?}"
    );
}

// ---------------------------------------------------------------------
// adopt.

/// A fetch output with two stored pages and one failed source, and a plan
/// naming `theme`.
fn adopt_fixture(tag: &str, theme: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = fixture(tag);
    let fetched = root.join("pages");
    fs::create_dir_all(&fetched).unwrap();
    fs::write(fetched.join("a.txt"), "Page a.\n").unwrap();
    fs::write(fetched.join("b.txt"), "Page b.\n").unwrap();
    fs::write(
        fetched.join(LOG),
        r#"{"sources":[{"id":"a","file":"a.txt"},{"id":"b","file":"b.txt"},{"id":"c","error":"curl exit 22"}]}"#,
    )
    .unwrap();
    let plan = root.join("plan.json");
    fs::write(&plan, format!(r#"{{"theme":"{theme}"}}"#)).unwrap();
    (plan, fetched, root.join("papers"))
}

#[test]
fn adopt_copies_every_stored_page_into_the_theme_byte_for_byte() {
    let (plan, fetched, papers) = adopt_fixture("adopt-ok", "distributed-systems/resilience");
    assert_eq!(adopt(&plan, &fetched, &papers), Ok(2));
    let dir = papers.join("distributed-systems/resilience");
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "Page a.\n");
    assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "Page b.\n");
    // A second run finds the same bytes and changes nothing.
    assert_eq!(adopt(&plan, &fetched, &papers), Ok(2));
    fs::write(dir.join("b.txt"), "Edited.\n").unwrap();
    assert_eq!(
        adopt(&plan, &fetched, &papers),
        Err(Failure::Refused(vec![
            "distributed-systems/resilience/b.txt: exists".to_string()
        ]))
    );
    let (p, f, d) = (
        plan.to_str().unwrap(),
        fetched.to_str().unwrap(),
        papers.to_str().unwrap(),
    );
    assert_eq!(
        run_args(&["adopt", "--plan", p, "--fetched", f, "--papers", d]),
        1
    );
    fs::write(dir.join("b.txt"), "Page b.\n").unwrap();
    assert_eq!(
        run_args(&["adopt", "--plan", p, "--fetched", f, "--papers", d]),
        0
    );
    assert_eq!(run_args(&["adopt", "--plan", p, "--fetched", f]), 2);
    assert_eq!(run_args(&["adopt", "--plan"]), 2);
}

#[test]
fn adopt_refuses_a_bad_theme_or_log_and_reports_missing_pages() {
    for theme in ["../out", "a/b/c", "", "Upper"] {
        let (plan, fetched, papers) = adopt_fixture("adopt-theme", theme);
        assert_eq!(
            adopt(&plan, &fetched, &papers),
            Err(Failure::Refused(vec![format!("plan: bad-theme: {theme}")]))
        );
    }
    let (plan, fetched, papers) = adopt_fixture("adopt-log", "t");
    fs::write(
        fetched.join(LOG),
        r#"{"sources":[{"id":"a","file":"../a.txt"},{"id":"gone","file":"gone.txt"}]}"#,
    )
    .unwrap();
    assert_eq!(
        adopt(&plan, &fetched, &papers),
        Err(Failure::Refused(vec![
            "a: bad-log-entry".to_string(),
            "gone.txt: missing-page".to_string()
        ]))
    );
    fs::write(fetched.join(LOG), "{}").unwrap();
    assert_eq!(adopt(&plan, &fetched, &papers), Ok(0));
    fs::write(fetched.join(LOG), "{").unwrap();
    assert!(matches!(
        adopt(&plan, &fetched, &papers),
        Err(Failure::Usage(_))
    ));
    let p = plan.to_str().unwrap();
    let (f, d) = (fetched.to_str().unwrap(), papers.to_str().unwrap());
    assert_eq!(
        run_args(&["adopt", "--plan", p, "--fetched", f, "--papers", d]),
        2
    );
}

#[test]
fn adopt_reports_unreadable_plans_and_unwritable_targets() {
    let (plan, fetched, papers) = adopt_fixture("adopt-io", "t");
    let missing = plan.with_file_name("missing.json");
    assert!(matches!(
        adopt(&missing, &fetched, &papers),
        Err(Failure::Usage(_))
    ));
    fs::write(&plan, "{").unwrap();
    assert!(matches!(
        adopt(&plan, &fetched, &papers),
        Err(Failure::Usage(_))
    ));
    fs::write(&plan, r#"{"theme":"t"}"#).unwrap();
    fs::write(&papers, "a file where the papers directory belongs").unwrap();
    assert!(matches!(
        adopt(&plan, &fetched, &papers),
        Err(Failure::Usage(_))
    ));
    fs::remove_file(&papers).unwrap();
    fs::create_dir_all(papers.join("t/a.txt/inner")).unwrap();
    fs::remove_dir_all(papers.join("t/a.txt/inner")).unwrap();
    // `a.txt` is a directory: reading it fails, and so does writing it.
    let Err(Failure::Refused(problems)) = adopt(&plan, &fetched, &papers) else {
        panic!("a directory in the page's place cannot be written");
    };
    assert_eq!(problems.len(), 1);
    assert!(
        problems[0].starts_with("t/a.txt: cannot write:"),
        "{problems:?}"
    );
}
