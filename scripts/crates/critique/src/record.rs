//! `record`: takes the orchestrator's verdict on every finding, then writes the report and the summary into the run
//! directory and appends the run's metadata to the user's ledger. A run is recorded once. The node runner's
//! `application/record.ts`.

use crate::adjudication::{adjudicate_findings, build_run, check_refutes, checked_verdicts, unasked_answer};
use crate::decode::{read_input, read_json_file};
use crate::error::{CritiqueError, ShapeError};
use crate::fan_out_result::{parse_fan_out_result, FanOutResult};
use crate::feature_gate::{compute_gate, save_gate};
use crate::fields::Fields;
use crate::locations::{findings_path, ledger_path, manifest_path, refute_path, report_path};
use crate::paths::join;
use crate::refute::{parse_refute_file, RefuteFile};
use crate::manifest::parse_manifest;
use crate::report::{render_report, render_summary};
use crate::run_ledger::ledger_lines;
use crate::runtime::Runtime;
use crate::ticket_review::{compute_ticket_review, save_ticket_review};
use crate::verdicts::{candidate_questions, count_verdicts, read_verdict, second_round_due, Verdict};
use model_gateway::json::{Object, Value};
use model_gateway::text::dirname;
use model_gateway::timestamps::to_iso;

struct Input {
    run_dir: String,
    verdicts: Vec<Verdict>,
}

fn read(raw: &Value) -> Result<Input, ShapeError> {
    let fields = Fields::of(raw, "")?;
    Ok(Input { run_dir: fields.string("run_dir")?, verdicts: fields.list("verdicts", read_verdict)? })
}

fn strings(items: impl Iterator<Item = String>) -> Value {
    Value::Arr(items.map(|item| Value::Str(item)).collect())
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input("record", read(raw))?;
    let manifest = read_json_file(runtime, &manifest_path(&input.run_dir), "invalid-run", parse_manifest)?;
    let merged = read_json_file(runtime, &findings_path(&input.run_dir), "invalid-run", parse_fan_out_result)?;
    let verdicts = checked_verdicts(&manifest, &merged, &input.verdicts)?;
    let report = report_path(&input.run_dir);
    if runtime.files.exists(&report).unwrap_or(false) {
        return Err(CritiqueError::new("already-recorded", format!("run {} is already recorded", manifest.run_id)));
    }
    let refuted = refutes(runtime, &input.run_dir, &merged)?;
    let results = refuted.as_ref().map(|file| file.results.clone());
    let adjudicated = adjudicate_findings(&verdicts, &merged.findings, results.as_deref().unwrap_or(&[]))?;
    let gate = if manifest.stage == "feature-review" { Some(compute_gate(runtime, &manifest, &adjudicated)?) } else { None };
    let run = build_run(&manifest, &merged, adjudicated, refuted.as_ref(), gate);
    if let Some(error) = unasked_answer(&run) {
        return Err(error);
    }
    let candidates = candidate_questions(&run.stage, &run.findings);
    let ticket_review = if manifest.stage == "ticket-review" { Some(compute_ticket_review(runtime, &manifest, &run)?) } else { None };
    let summary = render_summary(&run, &report);
    let summary_path = join(&[&input.run_dir, "summary.md"]);
    let ledger = ledger_path(&runtime.env)?;
    let write = |path: &str, text: &str| runtime.files.write_text(path, text).map_err(CritiqueError::from_fs);
    write(&report, &render_report(&run))?;
    write(&summary_path, &summary)?;
    let lines: String = ledger_lines(&run, &to_iso(runtime.clock.now_ms())).iter().map(|line| format!("{line}\n")).collect();
    runtime.files.mkdir_all(dirname(&ledger)).map_err(CritiqueError::from_fs)?;
    runtime.files.append(&ledger, &lines).map_err(CritiqueError::from_fs)?;
    if let Some(gate) = &run.gate {
        save_gate(runtime, &manifest, gate)?;
    }
    if let Some(record) = &ticket_review {
        save_ticket_review(runtime, &manifest, record)?;
    }
    let ids = |positions: &[usize]| strings(positions.iter().map(|position| run.findings[*position].finding.id.clone()));
    let [accepted, hint, needs_human, rejected] = count_verdicts(&run.findings);
    let mut counts = Object::new();
    for (kind, count) in [("accepted", accepted), ("hint", hint), ("needs-human", needs_human), ("rejected", rejected)] {
        counts.set(kind, Value::count(count));
    }
    let mut result = Object::new();
    result.set("counts", Value::Obj(counts));
    if let Some(candidates) = &candidates {
        result.set("deferred", ids(&candidates.deferred));
    }
    result.set("degraded", Value::Bool(run.degraded));
    if let Some(gate) = &run.gate {
        result.set("gate", gate.to_json());
    }
    result.set("ledger_path", Value::str(&ledger));
    result.set("report_path", Value::str(&report));
    if let Some(candidates) = &candidates {
        result.set("questions", ids(&candidates.asked));
    }
    result.set("round", Value::Num(run.round));
    result.set("run_id", Value::str(&manifest.run_id));
    result.set("second_round_due", Value::Bool(second_round_due(&run.stage, run.round, &run.findings)));
    result.set("status", Value::str("recorded"));
    result.set("summary", Value::str(&summary));
    result.set("summary_path", Value::str(&summary_path));
    if let Some(check) = &run.ticket_check {
        result.set("ticket_check", Value::str(&check.status));
    }
    if let Some(record) = &ticket_review {
        result.set("ticket_review", record.to_json());
    }
    Ok(Value::Obj(result))
}

/// The refute round's file, when the run had one; each result must name a finding of the run.
fn refutes(runtime: &Runtime, run_dir: &str, merged: &FanOutResult) -> Result<Option<RefuteFile>, CritiqueError> {
    let path = refute_path(run_dir);
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Ok(None);
    }
    let file = read_json_file(runtime, &path, "invalid-run", parse_refute_file)?;
    check_refutes(&path, &merged.findings, &file)?;
    Ok(Some(file))
}
