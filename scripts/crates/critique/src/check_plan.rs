//! `check-plan`: the deterministic refactoring-plan checker. It reads the plan file and
//! reports what a script can judge; with a repository it also reads the decision log, to
//! find the ADRs a plan anchors to absent or superseded. It sends nothing to any critic and
//! writes nothing; a repository whose remotes are refused is still refused.

use crate::adr_log::{check_adr_log, AdrSource};
use crate::check_adrs::log_records;
use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::refactoring_plan::{check_plan, plan_check_status, PlanAdrs, PlanCheck};
use crate::repository::{resolve, Resolution};
use crate::routing::stage_routing;
use crate::routing_source::load_routing;
use crate::runtime::Runtime;
use model_gateway::json::{Object, Value};
use model_gateway::text::{join, resolve as resolve_path};
use std::collections::HashSet;

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let (plan, repo) = Fields::of(raw, "")
        .and_then(|fields| Ok((fields.string("plan")?, fields.optional_string("repo")?)))
        .map_err(|error| error.into_error("invalid-input", "check-plan input"))?;
    let root = match &repo {
        None => None,
        Some(repo) => match resolve(runtime, repo)? {
            Resolution::Refused(refusal) => return Ok(refusal),
            Resolution::Resolved(repository) => Some(repository.root),
        },
    };
    let path = match &root {
        None => plan,
        Some(root) => resolve_path(root, &plan),
    };
    if !runtime.files.exists(&path).unwrap_or(false) {
        return Err(CritiqueError::new("invalid-input", format!("check-plan input: {path} does not exist")));
    }
    let text = runtime.files.read_text(&path).map_err(CritiqueError::from_fs)?;
    let routing = load_routing(runtime)?;
    let size = &stage_routing(&routing, "tickets")?.ticket_size;
    let adrs = match &root {
        None => None,
        Some(root) => Some(decision_log(runtime, root)?),
    };
    Ok(report(&check_plan(&text, size, adrs.as_ref())))
}

/// The numbers the decision log holds, and those of its records that are superseded.
fn decision_log(runtime: &Runtime, root: &str) -> Result<PlanAdrs, CritiqueError> {
    let mut sources = Vec::new();
    for path in log_records(runtime, root) {
        let text = runtime.files.read_text(&join(&[root, &path])).map_err(CritiqueError::from_fs)?;
        sources.push(AdrSource { path, text });
    }
    let log = check_adr_log(&sources, &HashSet::new());
    let numbered: Vec<(u32, &str)> = log.records.iter().filter_map(|record| record.number.map(|number| (number, record.path.as_str()))).collect();
    Ok(PlanAdrs {
        present: numbered.iter().map(|(number, _)| *number).collect(),
        superseded: numbered.iter().filter(|(_, path)| log.superseded.iter().any(|name| name == path)).map(|(number, _)| *number).collect(),
    })
}

fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

/// The check as the runner prints it: `{status, ...check}`.
fn report(check: &PlanCheck) -> Value {
    let mut result = Object::new();
    result.set("status", Value::str(plan_check_status(check)));
    let errors = check
        .errors
        .iter()
        .map(|error| {
            let mut item = Object::new();
            item.set("message", Value::str(&error.message));
            item.set("rule", Value::str(&error.rule));
            if let Some(step) = &error.step {
                item.set("step", Value::str(step));
            }
            Value::Obj(item)
        })
        .collect();
    result.set("errors", Value::Arr(errors));
    result.set("notes", strings(&check.notes));
    let steps = check
        .steps
        .iter()
        .map(|step| {
            let mut item = Object::new();
            item.set("blocked_by", strings(&step.blocked_by));
            item.set("id", Value::str(&step.id));
            item.set("kind", Value::str(&step.kind));
            item.set("title", Value::str(&step.title));
            Value::Obj(item)
        })
        .collect();
    result.set("steps", Value::Arr(steps));
    result.set("stored_data_steps", strings(&check.stored_data_steps));
    Value::Obj(result)
}
