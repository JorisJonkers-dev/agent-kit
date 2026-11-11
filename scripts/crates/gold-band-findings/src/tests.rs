#![allow(clippy::vec_init_then_push)]
use super::*;
use std::path::PathBuf;

// ---- fixtures ---------------------------------------------------------

/// The diff under review: the writer changed the total from 1 to 2.
const DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 fn main() {
-    let total = 1;
+    let total = 2;
 }
";

/// The ticket: it asks for the total to stay at 2.
const TICKET: &str = "\
# 450: the finding check

The writer must keep the total at 2 and log it.
";

/// A quote that is in the diff.
const IN_DIFF: &str = "let total = 2;";
/// A quote that is in the ticket only.
const IN_TICKET: &str = "writer must keep the total at 2";
/// A quote that is in neither the diff nor the ticket.
const NOWHERE: &str = "a fabricated line that is nowhere";

/// A finding that validates, quoting `quote`.
fn finding(quote: &str) -> Value {
    json!({
        "id": "F1",
        "severity": "major",
        "category": "bug",
        "check": "quote-check",
        "location": "src/lib.rs:1",
        "claim": "the total is wrong",
        "evidence": [{ "kind": "quote", "value": quote, "file": "src/lib.rs" }],
    })
}

fn with(mut finding: Value, key: &str, value: Value) -> Value {
    finding[key] = value;
    finding
}

fn without(mut finding: Value, key: &str) -> Value {
    finding.as_object_mut().unwrap().remove(key);
    finding
}

fn evidence(kind: &str, value: &str) -> Value {
    json!({ "kind": kind, "value": value })
}

/// Judges against the normalised inputs, as `check` does.
fn judged(finding: &Value) -> Result<(), Hint> {
    judge(finding, &normalise(DIFF), &normalise(TICKET))
}

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gb-findings-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(dir: &Path, name: &str, text: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, text).unwrap();
    path.to_str().unwrap().to_string()
}

fn strs(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

fn call(args: &[String]) -> (i32, String, String) {
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let code = run_with(args, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

// ---- normalise --------------------------------------------------------

#[test]
fn normalise_collapses_whitespace_runs_and_trims_the_ends() {
    assert_eq!(normalise("  a\n\n b\t\tc  "), "a b c");
    assert_eq!(normalise(""), "");
    assert_eq!(normalise("one"), "one");
}

// ---- validate_finding -------------------------------------------------

#[test]
fn a_valid_finding_validates() {
    assert_eq!(validate_finding(&finding(IN_DIFF)), Ok(()));
    // A string suggestion and evidence of every kind are allowed.
    let full = with(finding(IN_DIFF), "suggestion", json!("keep the total at 2"));
    let full = with(
        full,
        "evidence",
        json!([
            evidence("input", "cargo test --locked"),
            evidence("failing-test", "the_total_is_two"),
            evidence("trace", "run-9"),
            evidence("quote", IN_DIFF),
        ]),
    );
    assert_eq!(validate_finding(&full), Ok(()));
}

#[test]
fn every_schema_violation_is_a_schema_hint() {
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "not an object",
            json!(["a finding"]),
            "a finding is not an object",
        ),
        (
            "unknown key",
            with(finding(IN_DIFF), "note", json!("x")),
            "unknown key `note`",
        ),
        (
            "missing key",
            without(finding(IN_DIFF), "claim"),
            "missing key `claim`",
        ),
        (
            "empty id",
            with(finding(IN_DIFF), "id", json!("")),
            "`id` is not a non-empty string",
        ),
        (
            "non-string id",
            with(finding(IN_DIFF), "id", json!(7)),
            "`id` is not a non-empty string",
        ),
        (
            "empty location",
            with(finding(IN_DIFF), "location", json!("")),
            "`location` is not a non-empty string",
        ),
        (
            "empty claim",
            with(finding(IN_DIFF), "claim", json!("")),
            "`claim` is not a non-empty string",
        ),
        (
            "bad severity",
            with(finding(IN_DIFF), "severity", json!("blocker")),
            "`severity` is not one of critical, major, minor",
        ),
        (
            "bad category",
            with(finding(IN_DIFF), "category", json!("vibes")),
            "`category` is not one of unasked-question, contradiction, omission, \
             invention, untestable, failure-mode, security, structure, bug, \
             integration, drift, stale, undocumented-decision",
        ),
        (
            "empty check id",
            with(finding(IN_DIFF), "check", json!("")),
            "`check` is not a non-empty string",
        ),
        (
            "uppercase check id",
            with(finding(IN_DIFF), "check", json!("Bad_Id")),
            "`check` is not a kebab-case check id",
        ),
        (
            "double dash check id",
            with(finding(IN_DIFF), "check", json!("a--b")),
            "`check` is not a kebab-case check id",
        ),
        (
            "leading dash check id",
            with(finding(IN_DIFF), "check", json!("-a")),
            "`check` is not a kebab-case check id",
        ),
        (
            "non-string suggestion",
            with(finding(IN_DIFF), "suggestion", json!(5)),
            "`suggestion` is not a string",
        ),
        (
            "evidence not an array",
            with(finding(IN_DIFF), "evidence", json!({})),
            "`evidence` is not an array",
        ),
        (
            "evidence item not an object",
            with(finding(IN_DIFF), "evidence", json!(["x"])),
            "an evidence item is not an object",
        ),
        (
            "unknown evidence key",
            with(
                finding(IN_DIFF),
                "evidence",
                json!([{ "kind": "quote", "value": IN_DIFF, "note": "x" }]),
            ),
            "unknown evidence key `note`",
        ),
        (
            "bad evidence kind",
            with(
                finding(IN_DIFF),
                "evidence",
                json!([{ "kind": "vibes", "value": IN_DIFF }]),
            ),
            "evidence `kind` is not one of quote, input, failing-test, trace",
        ),
        (
            "missing evidence kind",
            with(finding(IN_DIFF), "evidence", json!([{ "value": IN_DIFF }])),
            "evidence `kind` is not one of quote, input, failing-test, trace",
        ),
        (
            "empty evidence value",
            with(
                finding(IN_DIFF),
                "evidence",
                json!([{ "kind": "quote", "value": "" }]),
            ),
            "evidence `value` is not a non-empty string",
        ),
        (
            "non-string evidence value",
            with(
                finding(IN_DIFF),
                "evidence",
                json!([{ "kind": "quote", "value": 5 }]),
            ),
            "evidence `value` is not a non-empty string",
        ),
        (
            "non-string evidence file",
            with(
                finding(IN_DIFF),
                "evidence",
                json!([{ "kind": "quote", "value": IN_DIFF, "file": 5 }]),
            ),
            "evidence `file` is not a non-empty string",
        ),
        (
            "empty evidence file",
            with(
                finding(IN_DIFF),
                "evidence",
                json!([{ "kind": "quote", "value": IN_DIFF, "file": "" }]),
            ),
            "evidence `file` is not a non-empty string",
        ),
    ];
    for (name, bad, why) in cases {
        let hint = judged(&bad).unwrap_err();
        assert_eq!(hint, Hint::Schema(why.to_string()), "{name}");
        assert_eq!(hint.code(), "schema", "{name}");
        assert_eq!(hint.detail(), why, "{name}");
    }
}

// ---- judge ------------------------------------------------------------

#[test]
fn a_quote_in_the_diff_is_kept() {
    assert_eq!(judged(&finding(IN_DIFF)), Ok(()));
}

#[test]
fn whitespace_differences_between_quote_and_diff_still_match() {
    // The diff has the line on one row; the quote spreads it over three with
    // a run of spaces.
    let quote = "let total\n\n      = 2;";
    assert_eq!(judged(&finding(quote)), Ok(()));
    // A run of spaces and a tab in the quote are one space in the diff.
    assert_eq!(judged(&finding("let   total\t= 2;")), Ok(()));
}

#[test]
fn a_quote_found_only_in_the_ticket_is_kept() {
    assert_eq!(judged(&finding(IN_TICKET)), Ok(()));
}

#[test]
fn a_quote_in_neither_is_quote_not_found() {
    let hint = judged(&finding(NOWHERE)).unwrap_err();
    assert_eq!(hint, Hint::QuoteNotFound);
    assert_eq!(hint.code(), "quote-not-found");
    assert_eq!(
        hint.detail(),
        "a quote found in neither the diff nor the ticket"
    );
}

#[test]
fn evidence_without_a_quote_is_no_quote() {
    for kind in ["input", "failing-test", "trace"] {
        let f = with(
            finding(IN_DIFF),
            "evidence",
            json!([evidence(kind, "cargo test --locked")]),
        );
        let hint = judged(&f).unwrap_err();
        assert_eq!(hint, Hint::NoQuote, "{kind}");
        assert_eq!(hint.code(), "no-quote", "{kind}");
        assert_eq!(hint.detail(), "no quote evidence to check", "{kind}");
    }
    // An empty evidence array has nothing to check either.
    let empty = with(finding(IN_DIFF), "evidence", json!([]));
    assert_eq!(judged(&empty), Err(Hint::NoQuote));
}

#[test]
fn a_quote_shorter_than_the_minimum_is_quote_too_short() {
    let hint = judged(&finding("let x")).unwrap_err();
    assert_eq!(hint, Hint::QuoteTooShort);
    assert_eq!(hint.code(), "quote-too-short");
    assert_eq!(
        hint.detail(),
        format!("a quote shorter than {MIN_QUOTE_CHARS} characters")
    );
    // Exactly the minimum is long enough to be checked, and then not found.
    assert_eq!(judged(&finding("abcdefgh")), Err(Hint::QuoteNotFound));
}

#[test]
fn one_good_quote_and_one_fabricated_quote_is_a_hint() {
    let f = with(
        finding(IN_DIFF),
        "evidence",
        json!([evidence("quote", IN_DIFF), evidence("quote", NOWHERE)]),
    );
    assert_eq!(judged(&f), Err(Hint::QuoteNotFound));
}

#[test]
fn a_good_quote_beside_other_evidence_is_kept() {
    let f = with(
        finding(IN_DIFF),
        "evidence",
        json!([
            evidence("input", "cargo test --locked"),
            evidence("quote", IN_DIFF),
        ]),
    );
    assert_eq!(judged(&f), Ok(()));
}

// ---- check ------------------------------------------------------------

#[test]
fn check_sorts_every_source_and_normalises_the_inputs() {
    let documents = vec![
        vec![
            finding(IN_DIFF),
            finding(NOWHERE),
            json!({ "severity": "major" }),
        ],
        vec![finding(IN_TICKET)],
    ];
    // The raw diff and ticket carry the whitespace the quotes do not.
    let checked = check(&documents, DIFF, TICKET);
    assert_eq!(checked.kept.len(), 2);
    assert_eq!(checked.kept[0]["source"], json!(0));
    assert_eq!(checked.kept[0]["finding"], finding(IN_DIFF));
    assert_eq!(checked.kept[1]["source"], json!(1));
    assert_eq!(checked.kept[1]["finding"], finding(IN_TICKET));
    assert_eq!(checked.hints.len(), 2);
    assert_eq!(checked.hints[0]["source"], json!(0));
    assert_eq!(checked.hints[0]["id"], json!("F1"));
    assert_eq!(checked.hints[0]["reason"], json!("quote-not-found"));
    assert_eq!(
        checked.hints[0]["detail"],
        json!("a quote found in neither the diff nor the ticket")
    );
    assert_eq!(checked.hints[0]["finding"], finding(NOWHERE));
    // A finding without an id still gets a hint, with a null id.
    assert_eq!(checked.hints[1]["id"], Value::Null);
    assert_eq!(checked.hints[1]["reason"], json!("schema"));
    assert_eq!(checked.hints[1]["detail"], json!("missing key `id`"));
}

#[test]
fn check_of_no_documents_keeps_and_hints_nothing() {
    let checked = check(&[], DIFF, TICKET);
    assert_eq!(checked.kept, Vec::<Value>::new());
    assert_eq!(checked.hints, Vec::<Value>::new());
}

// ---- report -----------------------------------------------------------

#[test]
fn the_report_counts_kept_findings_as_actionable() {
    let checked = check(&[vec![finding(IN_DIFF), finding(NOWHERE)]], DIFF, TICKET);
    let report = report("run-9", 3, &checked);
    assert_eq!(report["run_id"], json!("run-9"));
    assert_eq!(report["attempt"], json!(3));
    assert_eq!(report["actionable"], json!(1));
    assert_eq!(report["hint_count"], json!(1));
    assert_eq!(report["kept"].as_array().unwrap().len(), 1);
    assert_eq!(report["hints"].as_array().unwrap().len(), 1);
}

#[test]
fn a_run_with_hints_only_is_not_actionable() {
    let checked = check(&[vec![finding(NOWHERE)]], DIFF, TICKET);
    let report = report("run-9", 1, &checked);
    assert_eq!(report["actionable"], json!(0));
    assert_eq!(report["hint_count"], json!(1));
    assert_eq!(report["kept"], json!([]));
}

// ---- parse_document ---------------------------------------------------

#[test]
fn a_document_that_is_not_json_is_rejected() {
    let why = parse_document("nope").unwrap_err();
    assert!(why.starts_with("not JSON: "), "{why}");
}

#[test]
fn a_document_that_is_not_an_object_is_rejected() {
    assert_eq!(
        parse_document("[]").unwrap_err(),
        "the answer is not an object"
    );
}

#[test]
fn an_unknown_document_key_is_rejected() {
    let text = json!({ "findings": [], "no_findings_reason": "nothing", "extra": 1 }).to_string();
    assert_eq!(parse_document(&text).unwrap_err(), "unknown key `extra`");
}

#[test]
fn findings_that_are_not_an_array_are_rejected() {
    assert_eq!(
        parse_document(r#"{"findings":{}}"#).unwrap_err(),
        "`findings` is not an array"
    );
    assert_eq!(
        parse_document("{}").unwrap_err(),
        "`findings` is not an array"
    );
}

#[test]
fn no_findings_needs_a_non_empty_reason() {
    let why = "no findings and no `no_findings_reason`";
    assert_eq!(parse_document(r#"{"findings":[]}"#).unwrap_err(), why);
    assert_eq!(
        parse_document(r#"{"findings":[],"no_findings_reason":""}"#).unwrap_err(),
        why
    );
    assert_eq!(
        parse_document(r#"{"findings":[],"no_findings_reason":5}"#).unwrap_err(),
        why
    );
    assert_eq!(
        parse_document(r#"{"findings":[],"no_findings_reason":"nothing to report"}"#).unwrap(),
        Vec::<Value>::new()
    );
}

#[test]
fn findings_with_a_reason_that_is_not_a_string_are_rejected() {
    let text = json!({ "findings": [finding(IN_DIFF)], "no_findings_reason": 5 }).to_string();
    assert_eq!(
        parse_document(&text).unwrap_err(),
        "`no_findings_reason` is not a string"
    );
}

#[test]
fn findings_with_a_string_reason_or_none_are_accepted() {
    let bare = json!({ "findings": [finding(IN_DIFF)] }).to_string();
    assert_eq!(parse_document(&bare).unwrap(), vec![finding(IN_DIFF)]);
    let reasoned =
        json!({ "findings": [finding(IN_DIFF)], "no_findings_reason": "still a string" })
            .to_string();
    assert_eq!(parse_document(&reasoned).unwrap(), vec![finding(IN_DIFF)]);
}

// ---- read_bounded -----------------------------------------------------

#[test]
fn a_missing_input_cannot_be_read() {
    let d = dir("missing");
    let path = d.join("absent");
    let why = read_bounded(path.to_str().unwrap()).unwrap_err();
    assert!(
        why.starts_with("cannot read ") && why.contains("absent"),
        "{why}"
    );
}

#[test]
fn an_input_that_cannot_be_read_is_refused() {
    let d = dir("unreadable");
    // A directory opens, and then cannot be read.
    let why = read_bounded(d.to_str().unwrap()).unwrap_err();
    assert!(
        why.starts_with("cannot read ") && why.contains("directory"),
        "{why}"
    );
}

#[test]
fn an_input_over_the_cap_is_refused() {
    let d = dir("over");
    let path = d.join("big");
    let file = File::create(&path).unwrap();
    file.set_len(MAX_INPUT_BYTES + 1).unwrap();
    assert_eq!(
        read_bounded(path.to_str().unwrap()).unwrap_err(),
        format!("{} is over {MAX_INPUT_BYTES} bytes", path.display())
    );
}

#[test]
fn an_input_exactly_at_the_cap_is_read() {
    let d = dir("atcap");
    let path = d.join("exact");
    let file = File::create(&path).unwrap();
    file.set_len(MAX_INPUT_BYTES).unwrap();
    let text = read_bounded(path.to_str().unwrap()).unwrap();
    assert_eq!(text.len() as u64, MAX_INPUT_BYTES);
}

#[test]
fn an_input_that_is_not_utf8_is_refused() {
    let d = dir("utf8");
    let path = d.join("bad");
    fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
    assert_eq!(
        read_bounded(path.to_str().unwrap()).unwrap_err(),
        format!("{} is not UTF-8", path.display())
    );
}

// ---- write_report -----------------------------------------------------

#[test]
fn the_report_is_written_through_a_temp_file() {
    let d = dir("write");
    write_report(&d, "hello\n").unwrap();
    assert_eq!(fs::read_to_string(d.join(REPORT_FILE)).unwrap(), "hello\n");
    let names: Vec<_> = fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [OsString::from(REPORT_FILE)], "a temp file was left");
}

#[test]
fn a_report_directory_that_is_a_file_cannot_be_written() {
    let d = dir("notdir");
    let blocker = d.join("file");
    fs::write(&blocker, "x").unwrap();
    assert!(write_report(&blocker, "x").is_err());
}

#[test]
fn a_temp_file_that_cannot_be_written_is_an_error() {
    let d = dir("notmp");
    // A directory where the temp file belongs cannot be written as a file.
    let tmp = d.join(format!(".{REPORT_FILE}.{}.tmp", std::process::id()));
    fs::create_dir_all(&tmp).unwrap();
    assert!(write_report(&d, "x").is_err());
    assert!(!d.join(REPORT_FILE).exists());
}

#[test]
fn a_report_that_cannot_be_renamed_into_place_leaves_no_temp_file() {
    let d = dir("norename");
    // A directory where the report file belongs cannot be replaced by a file.
    fs::create_dir_all(d.join(REPORT_FILE)).unwrap();
    assert!(write_report(&d, "x").is_err());
    let names: Vec<_> = fs::read_dir(&d)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [OsString::from(REPORT_FILE)], "a temp file was left");
}

// ---- run_with ---------------------------------------------------------

/// A directory holding the diff and the ticket.
struct Case {
    dir: PathBuf,
    diff: String,
    ticket: String,
}

fn case(tag: &str) -> Case {
    let dir = dir(tag);
    let diff = write(&dir, "diff.txt", DIFF);
    let ticket = write(&dir, "ticket.txt", TICKET);
    Case { dir, diff, ticket }
}

fn full(c: &Case, findings: &[&str]) -> Vec<String> {
    let mut args = strs(&[
        "check",
        "--run-id",
        "run-9",
        "--attempt",
        "3",
        "--diff",
        &c.diff,
        "--ticket",
        &c.ticket,
        "--report-dir",
        c.dir.to_str().unwrap(),
    ]);
    args.extend(findings.iter().map(|s| s.to_string()));
    args
}

fn drop_flag(args: &mut Vec<String>, flag: &str) {
    let at = args.iter().position(|a| a == flag).unwrap();
    args.drain(at..at + 2);
}

fn set_value(args: &mut [String], flag: &str, value: &str) {
    let at = args.iter().position(|a| a == flag).unwrap();
    args[at + 1] = value.to_string();
}

#[test]
fn a_check_writes_the_report_and_prints_the_same_json() {
    let c = case("ok");
    let findings = write(
        &c.dir,
        "critic-0.json",
        &json!({ "findings": [finding(IN_DIFF)] }).to_string(),
    );
    let (code, out, err) = call(&full(&c, &[&findings]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(err, "");
    let written = fs::read_to_string(c.dir.join(REPORT_FILE)).unwrap();
    assert_eq!(out, written);
    assert!(out.ends_with("}\n"), "{out}");
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["run_id"], json!("run-9"));
    assert_eq!(report["attempt"], json!(3));
    assert_eq!(report["actionable"], json!(1));
    assert_eq!(report["hint_count"], json!(0));
    assert_eq!(report["kept"][0]["source"], json!(0));
    assert_eq!(report["kept"][0]["finding"], finding(IN_DIFF));
    assert_eq!(report["hints"], json!([]));
}

#[test]
fn each_findings_file_is_its_own_source() {
    let c = case("sources");
    let first = write(
        &c.dir,
        "critic-0.json",
        &json!({ "findings": [finding(IN_DIFF)] }).to_string(),
    );
    let second = write(
        &c.dir,
        "critic-1.json",
        &json!({ "findings": [finding(IN_TICKET), finding(NOWHERE)] }).to_string(),
    );
    let (code, out, err) = call(&full(&c, &[&first, &second]));
    assert_eq!(code, 0, "{err}");
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["actionable"], json!(2));
    assert_eq!(report["hint_count"], json!(1));
    assert_eq!(report["kept"][0]["source"], json!(0));
    assert_eq!(report["kept"][1]["source"], json!(1));
    assert_eq!(report["hints"][0]["source"], json!(1));
}

#[test]
fn a_run_with_hints_only_reports_nothing_actionable() {
    let c = case("hints");
    let findings = write(
        &c.dir,
        "critic-0.json",
        &json!({ "findings": [finding(NOWHERE)] }).to_string(),
    );
    let (code, out, err) = call(&full(&c, &[&findings]));
    assert_eq!(code, 0, "{err}");
    let report: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(report["actionable"], json!(0));
    assert_eq!(report["hint_count"], json!(1));
    assert_eq!(report["kept"], json!([]));
    assert_eq!(report["hints"][0]["reason"], json!("quote-not-found"));
}

#[test]
fn a_critic_answer_that_is_not_json_is_exit_1_and_writes_no_report() {
    let c = case("malformed");
    let findings = write(&c.dir, "critic-0.json", "not json");
    let (code, out, err) = call(&full(&c, &[&findings]));
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(
        err.starts_with(&format!("{NAME}: malformed: {findings}: not JSON: ")),
        "{err}"
    );
    assert!(!c.dir.join(REPORT_FILE).exists());
}

#[test]
fn a_critic_answer_that_breaks_the_document_schema_is_exit_1() {
    let c = case("baddoc");
    let findings = write(&c.dir, "critic-0.json", r#"{"findings":[]}"#);
    let (code, out, err) = call(&full(&c, &[&findings]));
    assert_eq!(code, 1);
    assert_eq!(out, "");
    assert!(
        err.contains("malformed: ") && err.contains("no findings and no `no_findings_reason`"),
        "{err}"
    );
    assert!(!c.dir.join(REPORT_FILE).exists());
}

#[test]
fn bad_usage_is_exit_2_and_writes_no_report() {
    let c = case("usage");
    let findings = write(
        &c.dir,
        "critic-0.json",
        &json!({ "findings": [], "no_findings_reason": "nothing to report" }).to_string(),
    );
    let absent = c.dir.join("absent").to_str().unwrap().to_string();
    let blocker = c.dir.join("blocker");
    fs::write(&blocker, "x").unwrap();
    let blocker = blocker.to_str().unwrap().to_string();
    let report_dir = c.dir.to_str().unwrap().to_string();

    #[allow(clippy::vec_init_then_push)]
    let mut cases: Vec<(Vec<String>, String)> = Vec::new();
    cases.push((strs(&[]), "the one subcommand is `check`".into()));
    cases.push((strs(&["verify"]), "the one subcommand is `check`".into()));
    cases.push((
        strs(&["check", "--bogus", "x"]),
        "unknown flag --bogus".into(),
    ));
    cases.push((
        strs(&["check", "--run-id"]),
        "--run-id needs a value".into(),
    ));
    cases.push((
        strs(&["check", "--run-id", "a", "--run-id", "b"]),
        "--run-id given twice".into(),
    ));

    let mut missing = full(&c, &[&findings]);
    drop_flag(&mut missing, "--run-id");
    cases.push((missing, "--run-id is required".into()));

    let mut no_attempt = full(&c, &[&findings]);
    drop_flag(&mut no_attempt, "--attempt");
    cases.push((no_attempt, "--attempt is required".into()));

    let mut no_diff_flag = full(&c, &[&findings]);
    drop_flag(&mut no_diff_flag, "--diff");
    cases.push((no_diff_flag, "--diff is required".into()));

    let mut no_ticket_flag = full(&c, &[&findings]);
    drop_flag(&mut no_ticket_flag, "--ticket");
    cases.push((no_ticket_flag, "--ticket is required".into()));

    let mut no_report_dir = full(&c, &[&findings]);
    drop_flag(&mut no_report_dir, "--report-dir");
    cases.push((no_report_dir, "--report-dir is required".into()));

    let mut zero = full(&c, &[&findings]);
    set_value(&mut zero, "--attempt", "0");
    cases.push((zero, "--attempt is not a positive integer".into()));

    let mut words = full(&c, &[&findings]);
    set_value(&mut words, "--attempt", "three");
    cases.push((words, "--attempt is not a positive integer".into()));

    cases.push((
        full(&c, &[]),
        "at least one findings file is required".into(),
    ));
    cases.push((
        full(&c, &[&findings, &findings]),
        "a findings file is named twice".into(),
    ));

    let mut empty_id = full(&c, &[&findings]);
    set_value(&mut empty_id, "--run-id", "");
    cases.push((empty_id, "--run-id is empty".into()));

    let mut no_diff = full(&c, &[&findings]);
    set_value(&mut no_diff, "--diff", &absent);
    cases.push((no_diff, "cannot read ".into()));

    let mut no_ticket = full(&c, &[&findings]);
    set_value(&mut no_ticket, "--ticket", &absent);
    cases.push((no_ticket, "cannot read ".into()));

    cases.push((full(&c, &[&absent]), "cannot read ".into()));

    let mut not_a_dir = full(&c, &[&findings]);
    set_value(&mut not_a_dir, "--report-dir", &blocker);
    cases.push((not_a_dir, "cannot write the report: ".into()));

    // A directory where the report file belongs: the rename cannot land.
    let nested = c.dir.join("nested");
    fs::create_dir_all(nested.join(REPORT_FILE)).unwrap();
    let mut no_rename = full(&c, &[&findings]);
    set_value(&mut no_rename, "--report-dir", nested.to_str().unwrap());
    cases.push((no_rename, "cannot write the report: ".into()));

    for (args, why) in cases {
        let (code, out, err) = call(&args);
        assert_eq!(code, 2, "{args:?}");
        assert_eq!(out, "", "{args:?}");
        assert!(err.contains("usage:"), "{args:?}: {err}");
        assert!(err.contains(&why), "{args:?}: {err}");
        assert!(!c.dir.join(REPORT_FILE).exists(), "{args:?}");
    }
    assert!(!Path::new(&report_dir).join(REPORT_FILE).exists());
}

#[test]
fn a_report_directory_that_is_created_when_missing() {
    let c = case("mkdir");
    let findings = write(
        &c.dir,
        "critic-0.json",
        &json!({ "findings": [finding(IN_DIFF)] }).to_string(),
    );
    let deep = c.dir.join("deep").join("reports");
    let mut args = full(&c, &[&findings]);
    set_value(&mut args, "--report-dir", deep.to_str().unwrap());
    let (code, _, err) = call(&args);
    assert_eq!(code, 0, "{err}");
    assert!(deep.join(REPORT_FILE).is_file());
}

// ---- run --------------------------------------------------------------

#[test]
fn the_process_entry_reports_bad_usage_as_exit_2() {
    assert_eq!(run([OsString::from("bogus")]), 2);
}
