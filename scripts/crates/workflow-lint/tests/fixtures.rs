//! The Gold Band fixture workflows: valid ones pass, each one that breaks a
//! rule is refused with a message that names the node, the rule and the fix.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

fn fixture(kind: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(kind)
        .join(format!("{name}.json"))
}

fn problems(kind: &str, name: &str) -> Vec<String> {
    let text = fs::read_to_string(fixture(kind, name)).unwrap();
    let root: Value = serde_json::from_str(&text).unwrap();
    workflow_lint::lint(&root).unwrap()
}

fn exit_code(kind: &str, name: &str) -> i32 {
    workflow_lint::run([fixture(kind, name).into_os_string()])
}

#[test]
fn valid_workflows_pass() {
    for name in ["unrolled-review", "wrapped-personal"] {
        assert_eq!(problems("valid", name), Vec::<String>::new(), "{name}");
        assert_eq!(exit_code("valid", name), 0, "{name}");
    }
}

#[test]
fn opus_outside_a_judgment_node_is_refused() {
    let found = problems("refused", "opus-outside-judgment");
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("dev-build: opus-outside-judgment: binds Opus"));
    assert!(found[0].contains("-escalation, -acceptance or -planning"));
    assert_eq!(exit_code("refused", "opus-outside-judgment"), 1);
}

#[test]
fn a_memory_write_outside_acceptance_or_escalation_is_refused() {
    let found = problems("refused", "memory-write-outside-judgment");
    assert_eq!(found.len(), 1);
    assert!(
        found[0].starts_with("dev-build: memory-write-outside-judgment: binds a memory-writing")
    );
    assert!(found[0].contains("ADR-0012"));
    assert_eq!(exit_code("refused", "memory-write-outside-judgment"), 1);
}

#[test]
fn a_planning_node_may_not_write_memory() {
    let found = problems("refused", "memory-write-in-planning");
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("dev-planning: memory-write-outside-judgment"));
}

#[test]
fn a_loop_that_is_not_unrolled_is_refused() {
    let found = problems("refused", "repair-loop");
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("dev-build: loop-not-unrolled: sits on a cycle"));
    assert!(found[0].contains("unroll the loop into fixed nodes"));
    assert_eq!(exit_code("refused", "repair-loop"), 1);
}

#[test]
fn a_new_round_edge_is_refused() {
    let found = problems("refused", "new-round");
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("dev-build: loop-not-unrolled: has an edge to `$new-round`"));
    assert_eq!(exit_code("refused", "new-round"), 1);
}
