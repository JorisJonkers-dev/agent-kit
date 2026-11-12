//! The three workflow commands: `workflow start`, `workflow end` and
//! `workflow list`, and the commands the Rust gateway does not carry yet.

use crate::error::GatewayError;
use crate::json::{Object, Value};
use crate::ports::Runtime;
use crate::state_file::Workflows;
use crate::workflows;

/// The subcommands the gateway has, as the command line names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Batch,
    GrantList,
    GrantWrite,
    Models,
    Run,
    Usage,
    WorkflowEnd,
    WorkflowList,
    WorkflowStart,
    WorktreesClean,
}

/// Runs one subcommand over its parsed input.
pub fn run(command: Command, name: &str, input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    match command {
        Command::WorkflowStart => workflow_start(input, runtime),
        Command::WorkflowEnd => workflow_end(input, runtime),
        Command::WorkflowList => workflow_list(runtime),
        _ => Err(GatewayError::new("not-ported", format!("{name} is not ported to the Rust gateway yet; use the node runner"))),
    }
}

/// Starts a workflow, or reports the one that is already running.
fn workflow_start(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let name = read_workflow_input(input)?;
    let now = runtime.clock.now_ms();
    let workflow = Workflows { runtime }.update(|current| workflows::start(current, &name, now))?;
    Ok(ok_object("workflow", workflows::view(&workflow, now)))
}

/// Ends a workflow at this moment; one that already ended keeps the moment it ended.
fn workflow_end(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let name = read_workflow_input(input)?;
    let now = runtime.clock.now_ms();
    let workflow = Workflows { runtime }.update(|current| workflows::end(current, &name, now))?;
    Ok(ok_object("workflow", workflows::view(&workflow, now)))
}

/// Lists every workflow this machine has started, with the 24-hour end worked out from the clock.
fn workflow_list(runtime: &Runtime) -> Result<Value, GatewayError> {
    let records = Workflows { runtime }.read()?;
    let now = runtime.clock.now_ms();
    Ok(ok_object("workflows", Value::Arr(records.iter().map(|record| workflows::view(record, now)).collect())))
}

/// `workflow start` and `workflow end` take the name of the workflow they act on.
fn read_workflow_input(input: &Object) -> Result<String, GatewayError> {
    workflows::require_name(input.get("name"), "a workflow command must name its workflow")
}

/// `{"status": "ok", <key>: <value>}`.
fn ok_object(key: &str, value: Value) -> Value {
    let mut object = Object::new();
    object.set("status", Value::str("ok"));
    object.set(key, value);
    Value::Obj(object)
}
