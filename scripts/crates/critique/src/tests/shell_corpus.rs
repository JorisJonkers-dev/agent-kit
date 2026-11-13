//! The shell command reader over generated command lines, compared with what the node runner's
//! `domain/shell-command.ts` reads from them.

use crate::shell_command::{gated_invocations, simple_commands, GatedInvocation, PushTargets};
use model_gateway::json::{self, Value};

fn text(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::str)
}

fn strings(items: &[String]) -> Value {
    Value::Arr(items.iter().map(|item| Value::str(item)).collect())
}

/// An invocation as `JSON.stringify` writes the node runner's object.
fn invocation_json(invocation: &GatedInvocation) -> Value {
    match invocation {
        GatedInvocation::Commit { cwd, widened } => json::object(vec![("cwd", text(cwd.as_deref())), ("kind", Value::str("commit")), ("widened", Value::str(widened.name()))]),
        GatedInvocation::Merge { cwd, revisions, fast_forward, head_moves } => json::object(vec![
            ("cwd", text(cwd.as_deref())),
            ("head_moves", Value::Bool(*head_moves)),
            ("kind", Value::str("merge")),
            ("fast_forward", Value::Bool(*fast_forward)),
            ("revisions", strings(revisions)),
        ]),
        GatedInvocation::Push { cwd, targets, head_moves } => {
            let targets = match targets {
                PushTargets::All => Value::str("all"),
                PushTargets::Some(targets) => Value::Arr(
                    targets.iter().map(|target| json::object(vec![("branch", text(target.branch.as_deref())), ("source", text(target.source.as_deref()))])).collect(),
                ),
            };
            json::object(vec![("cwd", text(cwd.as_deref())), ("head_moves", Value::Bool(*head_moves)), ("kind", Value::str("push")), ("targets", targets)])
        }
        GatedInvocation::PrCreate { cwd, head, head_moves } => {
            json::object(vec![("cwd", text(cwd.as_deref())), ("head", text(head.as_deref())), ("head_moves", Value::Bool(*head_moves)), ("kind", Value::str("pr-create"))])
        }
    }
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).expect("a field")
}

#[test]
fn shell_corpus() {
    let source = match std::env::var("CRITIQUE_SHELL_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/shell-commands.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        let line = field(case, "line").as_str().expect("line");
        let cwd = field(case, "cwd").as_str().expect("cwd");
        let home = field(case, "home").as_str();
        let expected = field(case, "expected");
        let commands = Value::Arr(simple_commands(line).iter().map(|words| strings(words)).collect());
        assert_eq!(json::compact(&commands), json::compact(field(expected, "commands")), "case {index}: commands differ\n  line: {line:?}");
        let gated = Value::Arr(gated_invocations(line, cwd, home).iter().map(invocation_json).collect());
        assert_eq!(json::compact(&gated), json::compact(field(expected, "gated")), "case {index}: invocations differ\n  line: {line:?}");
    }
}
