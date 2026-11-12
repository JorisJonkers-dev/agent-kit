//! `worktrees clean`: removes the writing worktrees a workflow's runs kept, with
//! their branches, for the workflow that ended or for all of them.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::kept_file::KeptStore;
use crate::ports::Runtime;
use crate::workflows;

/// Which kept worktrees to clean.
#[derive(Debug, PartialEq, Eq)]
pub enum Selection {
    Workflow(String),
    All,
}

/// Reads the selection: the workflow that ended, or `"all": true`, not both and not neither.
pub fn read_selection(input: &Object) -> Result<Selection, GatewayError> {
    let named = input.get("workflow").is_some();
    let all = input.get("all").is_some();
    if named && all {
        return Err(GatewayError::invalid_input("worktrees clean takes the workflow that ended, or all, not both"));
    }
    if named {
        let workflow = workflows::require_name(input.get("workflow"), "workflow must be the non-empty name of the workflow that ended")?;
        return Ok(Selection::Workflow(workflow));
    }
    if input.get("all") != Some(&Value::Bool(true)) {
        return Err(GatewayError::invalid_input(r#"worktrees clean needs "workflow", the workflow that ended, or "all": true"#));
    }
    Ok(Selection::All)
}

/// One kept worktree's fields, each as the file holds it when it is text.
fn text<'a>(entry: &'a Value, key: &str) -> Option<&'a str> {
    entry.as_object().and_then(|object| object.get(key)).and_then(Value::as_str)
}

/// `String(value)` for what a kept entry may hold: text as it is, a missing member as `undefined`.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::Str(text)) => text.clone(),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Num(number)) => json::number_text(*number),
        Some(Value::Arr(items)) => items.iter().map(|item| if matches!(item, Value::Null) { String::new() } else { js_string(Some(item)) }).collect::<Vec<_>>().join(","),
        Some(Value::Obj(_)) => "[object Object]".to_string(),
    }
}

/// Removes the chosen worktrees and their branches, and forgets the ones that were removed.
pub fn clean(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let selection = read_selection(input)?;
    let store = KeptStore { runtime };
    let chosen: Vec<Value> = store
        .list()?
        .into_iter()
        .filter(|entry| match &selection {
            Selection::All => true,
            Selection::Workflow(name) => text(entry, "workflow") == Some(name.as_str()),
        })
        .collect();
    let mut removed: Vec<Value> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    let mut branch_kept: Vec<Value> = Vec::new();
    let mut removed_ids: Vec<Option<Value>> = Vec::new();
    for entry in &chosen {
        let member = |key: &str| entry.as_object().and_then(|object| object.get(key)).cloned();
        let run_id = member("run_id");
        let Some(path) = text(entry, "path") else {
            // The node runner cannot delete a worktree with no path by hand either; it never guesses one.
            let reason = Value::str("the kept worktree names no path");
            failed.push(report(&[("reason", Some(reason)), ("run_id", run_id), ("worktree", member("path"))]));
            continue;
        };
        // What the node runner hands git for a field that is not text: JavaScript's own spelling of it.
        let repo = js_string(member("repo").as_ref());
        if let Err(reason) = runtime.git.remove_worktree(&repo, path) {
            failed.push(report(&[("reason", Some(Value::Str(reason))), ("run_id", run_id), ("worktree", Some(Value::str(path)))]));
            continue;
        }
        let branch = js_string(member("branch").as_ref());
        removed.push(report(&[("branch", member("branch")), ("run_id", run_id.clone()), ("workflow", member("workflow")), ("worktree", Some(Value::str(path)))]));
        removed_ids.push(run_id.clone());
        if let Err(reason) = runtime.git.delete_branch(&repo, &branch) {
            branch_kept.push(report(&[("branch", member("branch")), ("reason", Some(Value::Str(reason))), ("run_id", run_id)]));
        }
    }
    store.remove(&removed_ids)?;
    let mut object = Object::new();
    object.set("branch_kept", Value::Arr(branch_kept));
    object.set("failed", Value::Arr(failed));
    object.set("removed", Value::Arr(removed));
    object.set("status", Value::str("ok"));
    Ok(Value::Obj(object))
}

/// One object of the result; a member that is not there is left out, as `JSON.stringify` leaves out `undefined`.
fn report(members: &[(&str, Option<Value>)]) -> Value {
    let mut object = Object::new();
    for (key, value) in members {
        if let Some(value) = value {
            object.set(key, value.clone());
        }
    }
    Value::Obj(object)
}
