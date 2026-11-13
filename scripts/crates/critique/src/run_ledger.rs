//! The lines record appends to critique's own ledger: one per critic run and one per adjudicated finding. They hold
//! metadata only: identifiers, enums, numbers and hashes. Claims, locations, evidence, suggestions and free-text reasons
//! stay in the report, so the ledger can be kept indefinitely. The node runner's `ledgerLines`.

use crate::adjudication::{families_of, AdjudicatedRun};
use crate::critic::CriticRun;
use crate::manifest::DepthSignal;
use model_gateway::json::{self, Object, Value};

/// Whether a check a finding names has the shape of a check id (`/^[a-z0-9]+(?:-[a-z0-9]+)*$/`), rather than free text.
pub fn is_check_id(check: &str) -> bool {
    check.split('-').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit()))
}

fn text_or_null(text: Option<&String>) -> Value {
    text.map_or(Value::Null, |text| Value::str(text))
}

fn number_or_null(number: Option<f64>) -> Value {
    number.map_or(Value::Null, Value::Num)
}

fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

fn signal(signal: &DepthSignal) -> Value {
    match signal {
        DepthSignal::Threshold { measure, threshold, value } => json::object(vec![
            ("kind", Value::str("threshold")),
            ("measure", Value::str(measure)),
            ("threshold", Value::Num(*threshold)),
            ("value", Value::Num(*value)),
        ]),
        DepthSignal::RiskTerm { term } => json::object(vec![("kind", Value::str("risk-term")), ("term", Value::str(term))]),
        DepthSignal::Label { label } => json::object(vec![("kind", Value::str("label")), ("label", Value::str(label))]),
    }
}

/// What every line says about its run.
fn common(run: &AdjudicatedRun, recorded_at: &str) -> Vec<(&'static str, Value)> {
    let mut measures = Object::new();
    for (name, measure) in &run.depth_choice.measures {
        measures.set(name, json::object(vec![("threshold", Value::Num(measure.threshold)), ("value", Value::Num(measure.value))]));
    }
    vec![
        ("depth", Value::str(&run.depth)),
        ("flow", Value::str(&run.flow)),
        ("depth_chosen_by", Value::str(&run.depth_choice.chosen_by)),
        ("depth_measures", Value::Obj(measures)),
        ("depth_signals", Value::Arr(run.depth_choice.signals.iter().map(signal).collect())),
        ("recorded_at", Value::str(recorded_at)),
        ("repository", Value::str(&run.repository)),
        ("round", Value::Num(run.round)),
        ("run_id", Value::str(&run.run_id)),
        ("spec", Value::str(&run.spec)),
        ("stage", Value::str(&run.stage)),
    ]
}

/// Who the critic is, as the lines name it; a critic id the run does not have names only itself.
fn critic_fields(run: &AdjudicatedRun, critic_id: &str) -> Vec<(&'static str, Value)> {
    let Some(critic) = run.critics.iter().find(|critic| critic.critic_id == critic_id) else {
        return vec![("critic_id", Value::str(critic_id))];
    };
    vec![
        ("client", Value::str(&critic.client)),
        ("critic_id", Value::str(critic_id)),
        ("family", Value::str(&critic.family)),
        ("lens_hashes", lens_hashes(critic)),
        ("lenses", strings(&critic.lenses)),
        ("model", Value::str(&critic.model)),
        ("prompt_hashes", critic.prompt_hashes.to_json()),
        ("role", Value::str(&critic.role)),
    ]
}

fn lens_hashes(critic: &CriticRun) -> Value {
    let mut hashes = Object::new();
    for (name, hash) in &critic.lens_hashes {
        hashes.set(name, Value::str(hash));
    }
    Value::Obj(hashes)
}

fn line(pairs: Vec<Vec<(&'static str, Value)>>) -> String {
    let mut object = Object::new();
    for (key, value) in pairs.into_iter().flatten() {
        object.set(key, value);
    }
    json::compact(&Value::Obj(object))
}

/// The ledger lines of a recorded run: its critics' runs, then its findings.
pub fn ledger_lines(run: &AdjudicatedRun, recorded_at: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for critic in &run.critics {
        let mut own = vec![
            ("charged_usd", Value::Num(critic.charged_usd)),
            ("cost_usd", number_or_null(critic.cost_usd)),
            ("duration_ms", Value::Num(critic.duration_ms)),
            ("exit_code", number_or_null(critic.exit_code)),
            ("failure", text_or_null(critic.failure.as_ref())),
            ("finding_count", Value::Num(critic.finding_count)),
            ("retry_of", text_or_null(critic.retry_of.as_ref())),
            ("status", Value::str(&critic.status)),
        ];
        if let Some(vibe) = critic.vibe_cost_usd {
            own.push(("vibe_cost_usd", number_or_null(vibe)));
        }
        lines.push(line(vec![vec![("type", Value::str("run"))], common(run, recorded_at), critic_fields(run, &critic.critic_id), own]));
    }
    for adjudicated in &run.findings {
        let finding = &adjudicated.finding;
        let verdict = &adjudicated.verdict;
        let check = finding.body.check.as_ref().filter(|check| is_check_id(check));
        let own = vec![
            ("category", Value::str(&finding.body.category)),
            ("check", text_or_null(check)),
            ("evidence_kinds", Value::Arr(finding.body.evidence.iter().map(|e| Value::str(&e.kind)).collect())),
            ("finding_id", Value::str(&finding.id)),
            ("pre_verdict", finding.pre_verdict.as_ref().map_or(Value::Null, |pre| Value::str(&pre.reason_code))),
            ("answered", Value::Bool(verdict.answered == Some(true))),
            ("raised_by", strings(&finding.raised_by)),
            ("raised_by_families", strings(&families_of(&run.critics, finding))),
            ("reason_code", text_or_null(verdict.reason_code.as_ref())),
            ("severity", Value::str(&finding.body.severity)),
            ("declared_severity", text_or_null(finding.declared_severity.as_ref())),
            ("severity_override", text_or_null(verdict.severity.as_ref())),
            ("severity_reason", text_or_null(verdict.severity_reason.as_ref())),
            ("verdict", Value::str(&verdict.verdict)),
        ];
        let raiser = finding.raised_by.first().map_or("", String::as_str);
        lines.push(line(vec![vec![("type", Value::str("finding"))], common(run, recorded_at), critic_fields(run, raiser), own]));
    }
    lines
}
