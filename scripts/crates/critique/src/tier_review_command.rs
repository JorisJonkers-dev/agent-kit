//! `tier-review`: the spec critique on the tiers, from one command. It prepares a run on the tier-review roster (three
//! families from the chain of the `tier-review` alias, none of the writer's), fans it out, checks that the reviewers that
//! reviewed still span three families, and writes the synthesis the coordinator rules from: the reviewers with the routing
//! of each, then the findings by severity with who raised them. The coordinator, not the runner, rules on the findings and
//! writes the final synthesis (`record` takes its verdicts), so nothing here decides what the spec says.

use crate::error::{CritiqueError, ShapeError};
use crate::fields::Fields;
use crate::locations::findings_path;
use crate::paths::join;
use crate::runtime::Runtime;
use crate::tier_review::{MIN_FAMILIES, TIER};
use model_gateway::json::{self, Object, Value};

/// The stages the tier review runs: the spec critique.
const STAGES: [&str; 1] = ["spec"];

/// The writer's family when the caller names none: Claude Code writes.
const WRITER_FAMILY: &str = "anthropic";

/// The severities, most serious first.
const SEVERITIES: [&str; 3] = ["critical", "major", "minor"];

fn read_stage(raw: &Value) -> Result<(String, Object), ShapeError> {
    let fields = Fields::of(raw, "")?;
    let stage = if fields.has("stage") { fields.one_of("stage", &STAGES)? } else { "spec".to_string() };
    if fields.has("roster") {
        return Err(ShapeError("roster: tier-review picks the roster, so leave it out".to_string()));
    }
    let mut input = raw.as_object().cloned().unwrap_or_default();
    input.set("stage", Value::str(&stage));
    input.set("roster", Value::str("tier-review"));
    if !fields.has("orchestrator_family") {
        input.set("orchestrator_family", Value::str(WRITER_FAMILY));
    }
    Ok((stage, input))
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let (_, input) = read_stage(raw).map_err(|error| error.into_error("invalid-input", "tier-review input"))?;
    let prepared = crate::prepare::run(&Value::Obj(input), runtime)?;
    let ready = prepared.as_object().filter(|result| result.get("status").and_then(Value::as_str) == Some("ready"));
    let Some(prepared) = ready else {
        return Ok(prepared);
    };
    let run_dir = text(prepared.get("run_dir"));
    let reviewed = crate::fan_out_command::run(&json::object(vec![("run_dir", Value::str(&run_dir))]), runtime)?;
    let reviewed = reviewed.as_object().cloned().unwrap_or_default();
    let critics = list(reviewed.get("critics"));
    let findings = list(reviewed.get("findings"));
    let reviewers: Vec<Value> = critics.iter().filter_map(Value::as_object).map(reviewer).collect();
    let families = families_that_reviewed(&critics);
    let spanned = families.len() >= MIN_FAMILIES;
    let artifact = prepared.get("artifact").and_then(Value::as_object).map_or(String::new(), |artifact| text(artifact.get("reference")));
    let synthesis_path = join(&[&run_dir, "synthesis.md"]);
    runtime.files.write_text(&synthesis_path, &synthesis(&artifact, &text(prepared.get("run_id")), &critics, &findings, &families)).map_err(CritiqueError::from_fs)?;
    let mut result = Object::new();
    result.set("dropped", prepared.get("dropped").cloned().unwrap_or(Value::Null));
    result.set("families", Value::Arr(families.iter().map(|family| Value::str(family)).collect()));
    result.set("findings_path", Value::str(&findings_path(&run_dir)));
    result.set("finding_count", Value::count(findings.len()));
    if !spanned {
        result.set("message", Value::Str(format!("only {} of {MIN_FAMILIES} families reviewed ({}), so this is not a {TIER} review; the reviewers' routing is in synthesis.md", families.len(), families.join(", "))));
    }
    result.set("reviewers", Value::Arr(reviewers));
    result.set("run_dir", Value::str(&run_dir));
    result.set("run_id", Value::str(&text(prepared.get("run_id"))));
    result.set("stage", Value::str(&text(prepared.get("stage").or(reviewed.get("stage")))));
    result.set("status", Value::str(if spanned { "reviewed" } else { "failed" }));
    result.set("synthesis_path", Value::str(&synthesis_path));
    Ok(Value::Obj(result))
}

fn text(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or("").to_string()
}

fn list(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Arr(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// The distinct families of the reviewers that reviewed, in the order they ran.
pub(crate) fn families_that_reviewed(critics: &[Value]) -> Vec<String> {
    let mut families: Vec<String> = Vec::new();
    for critic in critics.iter().filter_map(Value::as_object).filter(|critic| text(critic.get("status")) == "ok") {
        let family = text(critic.get("family"));
        if !families.contains(&family) {
            families.push(family);
        }
    }
    families
}

/// One reviewer as the result names it: who it was, how it ended and the routing record of its launch.
fn reviewer(critic: &Object) -> Value {
    let mut object = Object::new();
    for key in ["critic_id", "family", "model", "status", "failure", "retry_of", "routing"] {
        if let Some(value) = critic.get(key) {
            object.set(key, value.clone());
        }
    }
    Value::Obj(object)
}

/// What the coordinator rules from: the reviewers and their routing, then the findings by severity.
fn synthesis(artifact: &str, run_id: &str, critics: &[Value], findings: &[Value], families: &[String]) -> String {
    let mut lines = vec![
        format!("# Tier review of {artifact}"),
        String::new(),
        format!("Run {run_id}. Reviewers from {TIER} that reviewed: {} ({} families).", families.join(", "), families.len()),
        String::new(),
        "## Reviewers".to_string(),
        String::new(),
        "| Critic | Family | Asked for | Served | Fallbacks | Capture | Status |".to_string(),
        "|---|---|---|---|---|---|---|".to_string(),
    ];
    for critic in critics.iter().filter_map(Value::as_object) {
        let routing = critic.get("routing").and_then(Value::as_object);
        let routed = |key: &str| routing.and_then(|routing| routing.get(key)).map_or("-".to_string(), shown);
        let status = match critic.get("failure").and_then(Value::as_str) {
            Some(failure) => format!("{} ({failure})", text(critic.get("status"))),
            None => text(critic.get("status")),
        };
        lines.push(format!("| {} | {} | {} | {} | {} | {} | {status} |", text(critic.get("critic_id")), text(critic.get("family")), text(critic.get("model")), routed("served_model"), routed("attempted_fallbacks"), routed("routing_capture")));
    }
    lines.push(String::new());
    lines.push(format!("## Findings ({})", findings.len()));
    for severity in SEVERITIES {
        for finding in findings.iter().filter_map(Value::as_object).filter(|finding| text(finding.get("severity")) == severity) {
            lines.extend(finding_lines(finding, critics));
        }
    }
    lines.push(String::new());
    lines.push("## For the coordinator".to_string());
    lines.push(String::new());
    lines.push("Every line above is a reviewer's claim, from a model that never saw the conversation. Rule on each finding from the packet alone, as /review-spec steps 3 to 7 describe, and write the synthesis from your verdicts.".to_string());
    format!("{}\n", lines.join("\n"))
}

/// A value as a table cell shows it.
fn shown(value: &Value) -> String {
    match value {
        Value::Str(text) => text.clone(),
        other => json::compact(other),
    }
}

pub(crate) fn finding_lines(finding: &Object, critics: &[Value]) -> Vec<String> {
    let family_of = |id: &str| critics.iter().filter_map(Value::as_object).find(|critic| text(critic.get("critic_id")) == id).map_or("?".to_string(), |critic| text(critic.get("family")));
    let raised: Vec<String> = list(finding.get("raised_by")).iter().filter_map(Value::as_str).map(|id| format!("{id} ({})", family_of(id))).collect();
    let mut lines = vec![String::new(), format!("### {} [{}] {}", text(finding.get("id")), text(finding.get("severity")), text(finding.get("location"))), String::new(), text(finding.get("claim"))];
    lines.push(String::new());
    lines.push(format!("Raised by {}.", raised.join(", ")));
    if let Some(pre) = finding.get("pre_verdict").and_then(Value::as_object) {
        lines.push(format!("Pre-verdict: {} ({}): {}", text(pre.get("verdict")), text(pre.get("reason_code")), text(pre.get("reason"))));
    }
    if let Some(suggestion) = finding.get("suggestion").and_then(Value::as_str) {
        lines.push(format!("Suggestion: {suggestion}"));
    }
    lines
}
