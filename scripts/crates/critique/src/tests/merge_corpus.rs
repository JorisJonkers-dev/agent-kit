//! The merge-commit rules over generated manifests, conflicts and diffs, compared with the node runner's
//! `domain/merge-commit.ts`.

use crate::merge_commit::{diff_of_paths, differs_only_in_pins, is_manifest};
use model_gateway::json::{self, Value};

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).expect("a field")
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key).as_str().expect("a string")
}

#[test]
fn merge_corpus() {
    let source = match std::env::var("CRITIQUE_MERGE_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/merge-commit.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        let expected = field(case, "expected");
        let Value::Arr(paths) = field(case, "paths") else { panic!("paths") };
        let paths: Vec<String> = paths.iter().map(|path| path.as_str().expect("path").to_string()).collect();
        assert_eq!(Value::Bool(differs_only_in_pins(text(case, "auto"), text(case, "merged"))), *field(expected, "pins"), "case {index}: pins differ");
        assert_eq!(diff_of_paths(text(case, "diff"), &paths), text(expected, "diff"), "case {index}: diff sections differ");
        assert_eq!(Value::Bool(is_manifest(text(case, "path"))), *field(expected, "manifest"), "case {index}: manifest differs");
    }
}
