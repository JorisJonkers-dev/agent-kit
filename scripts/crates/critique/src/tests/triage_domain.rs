//! The pure risk-triage domain: globs, line counts, answers, summaries and ledger lines.

use crate::triage::{
    code_change, count_changed_lines, matches_glob, read_triage_answer, risk_path_hits, risk_signals, signal_json,
    triage_ledger_line, triage_summary, CodeChange, RecordModel, RiskRules, RiskSignal, TriageRecord, TriageSignal,
    BUDGET, FAILED, LOW_RISK, RISKY,
};
use model_gateway::json;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

fn rules<'a>(risk_paths: &'a [String], changed_lines_threshold: f64, labels: &'a [String]) -> RiskRules<'a> {
    RiskRules { risk_paths, changed_lines_threshold, labels }
}

fn record(
    status: &str,
    signal: TriageSignal,
    signals: Vec<RiskSignal>,
    model: Option<RecordModel>,
    prompt_hash: Option<&str>,
) -> TriageRecord {
    TriageRecord {
        status: status.to_string(),
        recorded_at: "2024-01-01T00:00:00.000Z".to_string(),
        repository: "github.com/o/r".to_string(),
        flow: "T-1".to_string(),
        run_id: "run-1".to_string(),
        ticket: "T-1".to_string(),
        signal,
        signals,
        changed_files: 2.0,
        changed_lines: 12.0,
        changed_lines_threshold: 10.0,
        model,
        prompt_hash: prompt_hash.map(|hash| hash.to_string()),
    }
}

#[test]
fn a_change_counts_its_diff_lines() {
    let change = code_change("+a\n-b\n c\n", &strings(&["src/a.rs"]));
    assert_eq!(change, CodeChange { paths: strings(&["src/a.rs"]), changed_lines: 2.0 });
}

#[test]
fn a_header_is_not_counted() {
    let diff = "diff --git a/x b/x\n-- x\n+++ b/x\n+a\n";
    assert_eq!(count_changed_lines(diff), 1.0);
}

#[test]
fn a_hunk_ends_the_header() {
    let diff = "diff --git a/x b/x\n@@ -1 +1 @@\n+a\n-b\n";
    assert_eq!(count_changed_lines(diff), 2.0);
}

#[test]
fn a_diff_without_a_header_counts_every_marked_line() {
    assert_eq!(count_changed_lines("+a\n-b\n+++ c\n"), 3.0);
    assert_eq!(count_changed_lines(""), 0.0);
}

#[test]
fn the_rules_fire_in_order() {
    let risk_paths = strings(&["src/**", "docs/**"]);
    let labels = strings(&["Security", "Docs"]);
    let change = CodeChange { paths: strings(&["src/a.rs", "docs/b.md", "README.md"]), changed_lines: 12.0 };
    let signals = risk_signals(&change, &strings(&["SECURITY"]), &rules(&risk_paths, 10.0, &labels)).expect("globs build");
    assert_eq!(
        signals,
        vec![
            RiskSignal::RiskPath { glob: "src/**".to_string(), path: "src/a.rs".to_string() },
            RiskSignal::RiskPath { glob: "docs/**".to_string(), path: "docs/b.md".to_string() },
            RiskSignal::Size { changed_lines: 12.0, threshold: 10.0 },
            RiskSignal::Label { label: "Security".to_string() },
        ]
    );
}

#[test]
fn a_change_at_the_threshold_is_not_risky() {
    let risk_paths = strings(&["src/**"]);
    let labels = strings(&["Security"]);
    let change = CodeChange { paths: strings(&["README.md"]), changed_lines: 10.0 };
    assert_eq!(
        risk_signals(&change, &strings(&["docs"]), &rules(&risk_paths, 10.0, &labels)),
        Ok(Vec::<RiskSignal>::new())
    );
}

#[test]
fn a_glob_the_pattern_builder_rejects_is_an_error_when_it_is_reached() {
    let risk_paths = strings(&["a{b"]);
    let change = CodeChange { paths: strings(&["a"]), changed_lines: 0.0 };
    let message = "Invalid regular expression: /^a(?:b$/: Unterminated group".to_string();
    assert_eq!(risk_signals(&change, &[], &rules(&risk_paths, 10.0, &[])), Err(message.clone()));
    assert_eq!(risk_path_hits(&change, &risk_paths), Err(message));
    // A glob after a match is never reached, and a change with no paths never reaches any.
    let later = strings(&["a", "a{b"]);
    assert_eq!(risk_path_hits(&change, &later), Ok(1));
    assert_eq!(risk_path_hits(&CodeChange { paths: Vec::new(), changed_lines: 0.0 }, &risk_paths), Ok(0));
}

#[test]
fn the_hits_count_the_paths_that_match() {
    let change = CodeChange { paths: strings(&["src/a.rs", "docs/b.md", "README.md"]), changed_lines: 0.0 };
    assert_eq!(risk_path_hits(&change, &strings(&["src/**", "docs/**"])), Ok(2));
    assert_eq!(risk_path_hits(&change, &strings(&["src/**"])), Ok(1));
}

#[test]
fn a_double_star_prefix_matches_any_depth() {
    assert_eq!(matches_glob("x", "**/x"), Ok(true));
    assert_eq!(matches_glob("a/x", "**/x"), Ok(true));
    assert_eq!(matches_glob("a/b/x", "**/x"), Ok(true));
    assert_eq!(matches_glob("ax", "**/x"), Ok(false));
    assert_eq!(matches_glob("a/xy", "**/x"), Ok(false));
}

#[test]
fn a_double_star_suffix_matches_any_depth() {
    assert_eq!(matches_glob("a", "a/**"), Ok(true));
    assert_eq!(matches_glob("a/", "a/**"), Ok(true));
    assert_eq!(matches_glob("a/b/c", "a/**"), Ok(true));
    assert_eq!(matches_glob("ab", "a/**"), Ok(false));
    assert_eq!(matches_glob("a/b", "a/**/b"), Ok(true));
    assert_eq!(matches_glob("a/x/y/b", "a/**/b"), Ok(true));
    assert_eq!(matches_glob("a/x/y/c", "a/**/b"), Ok(false));
}

#[test]
fn a_double_star_matches_anything_but_a_line_terminator() {
    assert_eq!(matches_glob("", "**"), Ok(true));
    assert_eq!(matches_glob("a/b", "**"), Ok(true));
    assert_eq!(matches_glob("a\nb", "**"), Ok(false));
    assert_eq!(matches_glob("a\rb", "**"), Ok(false));
    assert_eq!(matches_glob("a\u{2028}b", "**"), Ok(false));
    assert_eq!(matches_glob("a\u{2029}b", "**"), Ok(false));
    assert_eq!(matches_glob("a\nb/x", "**/x"), Ok(false));
    assert_eq!(matches_glob("a/b\nc", "a/**"), Ok(false));
}

#[test]
fn a_single_star_matches_anything_but_a_slash() {
    assert_eq!(matches_glob("", "*"), Ok(true));
    assert_eq!(matches_glob("abc", "*"), Ok(true));
    assert_eq!(matches_glob("a\nb", "*"), Ok(true));
    assert_eq!(matches_glob("a/b", "*"), Ok(false));
}

#[test]
fn a_question_mark_matches_one_utf16_unit() {
    assert_eq!(matches_glob("a", "?"), Ok(true));
    assert_eq!(matches_glob("ab", "?"), Ok(false));
    assert_eq!(matches_glob("/", "?"), Ok(false));
    assert_eq!(matches_glob("\u{1f600}", "?"), Ok(false));
    assert_eq!(matches_glob("\u{1f600}", "??"), Ok(true));
}

#[test]
fn braces_offer_alternatives() {
    assert_eq!(matches_glob("a/c", "{a,b}/c"), Ok(true));
    assert_eq!(matches_glob("b/c", "{a,b}/c"), Ok(true));
    assert_eq!(matches_glob("c/c", "{a,b}/c"), Ok(false));
    assert_eq!(matches_glob("b/d", "{a,{b,c}}/d"), Ok(true));
    assert_eq!(matches_glob("c/d", "{a,{b,c}}/d"), Ok(true));
    assert_eq!(matches_glob("d/d", "{a,{b,c}}/d"), Ok(false));
}

#[test]
fn an_unclosed_brace_is_an_invalid_pattern() {
    assert_eq!(
        matches_glob("a", "a{b"),
        Err("Invalid regular expression: /^a(?:b$/: Unterminated group".to_string())
    );
    assert_eq!(
        matches_glob("a", "a{b,c"),
        Err("Invalid regular expression: /^a(?:b|c$/: Unterminated group".to_string())
    );
}

#[test]
fn a_brace_or_comma_outside_a_group_is_a_literal() {
    assert_eq!(matches_glob("a}b", "a}b"), Ok(true));
    assert_eq!(matches_glob("a,b", "a,b"), Ok(true));
    assert_eq!(matches_glob("a,b", "a{b,c}"), Ok(false));
}

#[test]
fn a_pattern_character_is_a_literal() {
    assert_eq!(matches_glob("a.b", "a.b"), Ok(true));
    assert_eq!(matches_glob("axb", "a.b"), Ok(false));
    assert_eq!(matches_glob("a+b", "a+b"), Ok(true));
    assert_eq!(matches_glob("aab", "a+b"), Ok(false));
    assert_eq!(matches_glob("(x)", "(x)"), Ok(true));
    assert_eq!(matches_glob("[x]", "[x]"), Ok(true));
    assert_eq!(matches_glob("a|b", "a|b"), Ok(true));
    assert_eq!(matches_glob("a", "a|b"), Ok(false));
    assert_eq!(matches_glob("a^b", "a^b"), Ok(true));
    assert_eq!(matches_glob("a$b", "a$b"), Ok(true));
    assert_eq!(matches_glob("a\\b", "a\\b"), Ok(true));
    assert_eq!(matches_glob("A", "a"), Ok(false));
}

#[test]
fn an_answer_is_yes_or_no_through_markup() {
    assert_eq!(read_triage_answer("yes"), Some("yes"));
    assert_eq!(read_triage_answer("  YES  "), Some("yes"));
    assert_eq!(read_triage_answer("**no**"), Some("no"));
    assert_eq!(read_triage_answer("`yes`."), Some("yes"));
    assert_eq!(read_triage_answer("yes!"), Some("yes"));
    assert_eq!(read_triage_answer("\u{feff}no"), Some("no"));
    assert_eq!(read_triage_answer("maybe"), None);
    assert_eq!(read_triage_answer(""), None);
    assert_eq!(read_triage_answer("yes no"), None);
}

#[test]
fn a_summary_names_the_signal() {
    let client = "vibe";
    let path = TriageSignal::Risk(RiskSignal::RiskPath { glob: "src/**".to_string(), path: "src/a.rs".to_string() });
    assert_eq!(triage_summary(RISKY, &path, client), "Triage: risky: src/a.rs matches the risk path src/**.");
    let size = TriageSignal::Risk(RiskSignal::Size { changed_lines: 12.5, threshold: 10.0 });
    assert_eq!(triage_summary(RISKY, &size, client), "Triage: risky: 12.5 changed lines, above the threshold of 10.");
    let label = TriageSignal::Risk(RiskSignal::Label { label: "Security".to_string() });
    assert_eq!(triage_summary(RISKY, &label, client), "Triage: risky: the ticket carries the label Security.");
    let answered = TriageSignal::Model { answer: "yes" };
    assert_eq!(
        triage_summary(RISKY, &answered, client),
        "Triage: risky: no risk rule fired, and the triage model (vibe) answered yes."
    );
    let skipped = TriageSignal::Model { answer: "no" };
    assert_eq!(
        triage_summary(LOW_RISK, &skipped, client),
        "Triage: low-risk: no risk rule fired, and the triage model (vibe) answered no, so the review is skipped."
    );
    let absent = TriageSignal::ModelUnavailable { reason: BUDGET, detail: "the flow budget is spent".to_string() };
    assert_eq!(
        triage_summary(RISKY, &absent, client),
        "Triage: risky: no risk rule fired, and the triage model (vibe) could not answer: the flow budget is spent."
    );
}

#[test]
fn a_signal_writes_its_whole_object() {
    let cases = [
        (
            TriageSignal::Risk(RiskSignal::RiskPath { glob: "src/**".to_string(), path: "src/a.rs".to_string() }),
            r#"{"glob":"src/**","kind":"risk-path","path":"src/a.rs"}"#,
        ),
        (
            TriageSignal::Risk(RiskSignal::Size { changed_lines: 12.0, threshold: 10.0 }),
            r#"{"changed_lines":12,"kind":"size","threshold":10}"#,
        ),
        (
            TriageSignal::Risk(RiskSignal::Label { label: "Security".to_string() }),
            r#"{"kind":"label","label":"Security"}"#,
        ),
        (TriageSignal::Model { answer: "yes" }, r#"{"answer":"yes","kind":"model"}"#),
        (
            TriageSignal::ModelUnavailable { reason: FAILED, detail: "the model timed out".to_string() },
            r#"{"detail":"the model timed out","kind":"model-unavailable","reason":"failed"}"#,
        ),
    ];
    for (signal, expected) in cases {
        assert_eq!(json::compact(&signal_json(&signal)), expected);
    }
}

#[test]
fn a_risky_ledger_line_names_the_model() {
    let line = triage_ledger_line(&record(
        RISKY,
        TriageSignal::Model { answer: "yes" },
        vec![RiskSignal::RiskPath { glob: "src/**".to_string(), path: "src/a.rs".to_string() }],
        Some(RecordModel {
            client: "vibe".to_string(),
            model: "mistral-small".to_string(),
            cost_usd: Some(0.5),
            charged_usd: 0.25,
            answer: Some("yes"),
        }),
        Some("abc"),
    ));
    assert_eq!(
        json::compact(&line),
        concat!(
            r#"{"type":"triage","recorded_at":"2024-01-01T00:00:00.000Z","repository":"github.com/o/r","#,
            r#""flow":"T-1","run_id":"run-1","stage":"ticket-review","status":"risky","ticket":"T-1","#,
            r#""triage_signal":{"answer":"yes","kind":"model"},"risk_signals":[{"glob":"src/**","kind":"risk-path"}],"#,
            r#""changed_files":2,"changed_lines":12,"changed_lines_threshold":10,"client":"vibe","#,
            r#""model":"mistral-small","answer":"yes","cost_usd":0.5,"charged_usd":0.25,"prompt_hash":"abc"}"#
        )
    );
}

#[test]
fn a_low_risk_ledger_line_is_a_skipped_run() {
    let line = triage_ledger_line(&record(LOW_RISK, TriageSignal::Model { answer: "no" }, Vec::new(), None, None));
    assert_eq!(
        json::compact(&line),
        concat!(
            r#"{"type":"run","recorded_at":"2024-01-01T00:00:00.000Z","repository":"github.com/o/r","#,
            r#""flow":"T-1","run_id":"run-1","stage":"ticket-review","status":"skipped-low-risk","ticket":"T-1","#,
            r#""triage_signal":{"answer":"no","kind":"model"},"risk_signals":[],"changed_files":2,"#,
            r#""changed_lines":12,"changed_lines_threshold":10,"client":null,"model":null,"answer":null,"#,
            r#""cost_usd":null,"charged_usd":0,"prompt_hash":null}"#
        )
    );
}

#[test]
fn a_ledger_line_reduces_its_signals() {
    let line = triage_ledger_line(&record(
        RISKY,
        TriageSignal::ModelUnavailable { reason: FAILED, detail: "the model timed out".to_string() },
        vec![
            RiskSignal::Label { label: "Security".to_string() },
            RiskSignal::Size { changed_lines: 12.0, threshold: 10.0 },
        ],
        Some(RecordModel {
            client: "vibe".to_string(),
            model: "m".to_string(),
            cost_usd: None,
            charged_usd: 0.75,
            answer: None,
        }),
        Some("h"),
    ));
    assert_eq!(
        json::compact(&line),
        concat!(
            r#"{"type":"triage","recorded_at":"2024-01-01T00:00:00.000Z","repository":"github.com/o/r","#,
            r#""flow":"T-1","run_id":"run-1","stage":"ticket-review","status":"risky","ticket":"T-1","#,
            r#""triage_signal":{"kind":"model-unavailable","reason":"failed"},"#,
            r#""risk_signals":[{"kind":"label","label":"Security"},{"changed_lines":12,"kind":"size","threshold":10}],"#,
            r#""changed_files":2,"changed_lines":12,"changed_lines_threshold":10,"client":"vibe","model":"m","#,
            r#""answer":null,"cost_usd":null,"charged_usd":0.75,"prompt_hash":"h"}"#
        )
    );
}
