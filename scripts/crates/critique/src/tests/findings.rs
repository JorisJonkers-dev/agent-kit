//! findings' own pieces: a critic's reply read against the output contract, and the finding objects fan-out writes.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::findings::{parse_findings, read_finding_body, read_run_finding, Evidence, FindingBody, FindingsParse, PreVerdict, RunFinding};
use model_gateway::json::{self, Value};

pub fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn with(pairs: &mut Vec<(&'static str, Value)>, key: &'static str, value: &Option<String>) {
    if let Some(text) = value {
        pairs.push((key, Value::str(text)));
    }
}

fn evidence(item: &Evidence) -> Value {
    let mut pairs = vec![("kind", Value::str(&item.kind)), ("value", Value::str(&item.value))];
    with(&mut pairs, "file", &item.file);
    json::object(pairs)
}

/// A body as the node runner's reader builds it: its keys in the order of the object literal.
pub fn body_pairs(body: &FindingBody) -> Vec<(&'static str, Value)> {
    let mut pairs = vec![("category", Value::str(&body.category))];
    with(&mut pairs, "check", &body.check);
    pairs.push(("claim", Value::str(&body.claim)));
    pairs.push(("evidence", Value::Arr(body.evidence.iter().map(evidence).collect())));
    pairs.push(("location", Value::str(&body.location)));
    pairs.push(("severity", Value::str(&body.severity)));
    with(&mut pairs, "suggestion", &body.suggestion);
    pairs
}

pub fn run_finding_value(finding: &RunFinding) -> Value {
    let mut pairs = body_pairs(&finding.body);
    pairs.push(("critic_finding_id", Value::str(&finding.critic_finding_id)));
    pairs.push(("id", Value::str(&finding.id)));
    pairs.push(("raised_by", strings(&finding.raised_by)));
    with(&mut pairs, "declared_severity", &finding.declared_severity);
    if let Some(PreVerdict { verdict, reason_code, reason }) = &finding.pre_verdict {
        pairs.push((
            "pre_verdict",
            json::object(vec![("reason", Value::str(reason)), ("reason_code", Value::str(reason_code)), ("verdict", Value::str(verdict))]),
        ));
    }
    json::object(pairs)
}

pub fn parse_value(result: &FindingsParse) -> Value {
    match result {
        FindingsParse::Ok { findings, no_findings_reason } => {
            let items = findings
                .iter()
                .map(|finding| {
                    let mut pairs = body_pairs(&finding.body);
                    pairs.push(("id", Value::str(&finding.id)));
                    json::object(pairs)
                })
                .collect();
            let mut pairs = vec![("findings", Value::Arr(items)), ("ok", Value::Bool(true))];
            with(&mut pairs, "no_findings_reason", no_findings_reason);
            json::object(pairs)
        }
        FindingsParse::Failed { failure, detail } => {
            json::object(vec![("detail", Value::str(detail)), ("failure", Value::str(failure)), ("ok", Value::Bool(false))])
        }
    }
}

fn shaped<T>(result: Result<T, ShapeError>, render: impl Fn(&T) -> Value) -> Value {
    match result {
        Ok(found) => json::object(vec![("ok", render(&found))]),
        Err(error) => json::object(vec![("error", Value::str(&format!("JsonShapeError: {}", error.0)))]),
    }
}

pub fn run_finding_result(value: &Value) -> Value {
    shaped(Fields::of(value, "").and_then(|fields| read_run_finding(&fields)), run_finding_value)
}

#[test]
fn a_reply_without_a_findings_block_or_with_a_broken_one_fails_with_its_reason() {
    let failed = |text: &str| match parse_findings(text) {
        FindingsParse::Failed { failure, detail } => (failure, detail),
        FindingsParse::Ok { .. } => panic!("parsed"),
    };
    assert_eq!(failed("nothing here"), ("no-findings-block", "the reply has no ```json block".to_string()));
    let (failure, detail) = failed("```json\n{\n```");
    assert_eq!((failure, detail.starts_with("SyntaxError: ")), ("unparseable-findings", true));
    assert_eq!(failed("```json\n[]\n```"), ("invalid-findings", "JsonShapeError: the block must be a JSON object".to_string()));
    assert_eq!(failed("```json\n{}\n```"), ("invalid-findings", "JsonShapeError: the block.findings must be a list".to_string()));
    assert_eq!(
        failed("```json\n{\"findings\":[{\"id\":\"a\"}]}\n```"),
        ("invalid-findings", "JsonShapeError: the block.findings[0].category must be one of unasked-question, contradiction, omission, invention, untestable, failure-mode, security, structure, bug, integration, drift, stale, undocumented-decision".to_string())
    );
    assert_eq!(
        failed("```json\n{\"findings\":[],\"no_findings_reason\":\"  \"}\n```"),
        ("missing-no-findings-reason", "an empty findings list needs no_findings_reason".to_string())
    );
}

#[test]
fn an_empty_list_with_a_reason_and_a_finding_with_an_empty_check_both_read() {
    let FindingsParse::Ok { findings, no_findings_reason } = parse_findings("```json\n{\"findings\":[],\"no_findings_reason\":\" clean \"}\n```") else { panic!("failed") };
    assert_eq!((findings.len(), no_findings_reason.as_deref()), (0, Some("clean")));
    let text = r#"```json
{"findings":[{"id":"f1","severity":"minor","category":"bug","location":"a","claim":"c","check":"  ","suggestion":"","evidence":[{"kind":"quote","value":"v","file":"tree/a"}]}]}
```"#;
    let FindingsParse::Ok { findings, no_findings_reason } = parse_findings(text) else { panic!("failed") };
    assert_eq!(no_findings_reason, None);
    assert_eq!((findings[0].body.check.clone(), findings[0].body.suggestion.clone()), (None, Some(String::new())));
    assert_eq!(findings[0].body.evidence[0].file.as_deref(), Some("tree/a"));
}

#[test]
fn a_suggestion_that_is_not_text_is_a_shape_error_and_a_run_finding_reads_its_pre_verdict() {
    let value = json::parse(r#"{"suggestion":3}"#).expect("json");
    let fields = Fields::of(&value, "").expect("object");
    assert_eq!(read_finding_body(&fields), Err(ShapeError("suggestion must be a string".to_string())));
    let finding = r#"{"id":"f1","critic_finding_id":"c1","raised_by":["a"],"severity":"major","category":"bug","location":"l","claim":"c","evidence":[],"declared_severity":"minor","pre_verdict":{"verdict":"hint","reason_code":"no-check","reason":"r"}}"#;
    let value = json::parse(finding).expect("json");
    let read = read_run_finding(&Fields::of(&value, "").expect("object")).expect("reads");
    assert_eq!(read.pre_verdict.map(|pre| pre.reason_code), Some("no-check".to_string()));
    assert_eq!(read.declared_severity.as_deref(), Some("minor"));
}
