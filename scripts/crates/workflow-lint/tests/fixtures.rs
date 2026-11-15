//! The Gold Band fixture workflows: valid ones pass, each one that breaks a
//! rule is refused with a message that names the node, the rule and the fix.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use workflow_lint::Instance;

fn fixture(kind: &str, name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(kind)
        .join(format!("{name}.json"))
}

fn problems(kind: &str, name: &str) -> Vec<String> {
    problems_for(kind, name, Instance::Personal)
}

fn problems_for(kind: &str, name: &str, instance: Instance) -> Vec<String> {
    let text = fs::read_to_string(fixture(kind, name)).unwrap();
    let root: Value = serde_json::from_str(&text).unwrap();
    workflow_lint::lint(&root, instance).unwrap()
}

fn exit_code(kind: &str, name: &str) -> i32 {
    run_for(kind, name, "gold-band-personal")
}

fn run_for(kind: &str, name: &str, instance: &str) -> i32 {
    workflow_lint::run([
        "--instance".into(),
        instance.into(),
        fixture(kind, name).into_os_string(),
    ])
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

#[test]
fn codex_passes_in_the_work_instance() {
    assert!(problems_for("valid", "work-codex", Instance::Work).is_empty());
    assert_eq!(run_for("valid", "work-codex", "gold-band"), 0);
}

#[test]
fn codex_is_refused_in_the_personal_instance() {
    let found = problems("refused", "personal-codex");
    assert_eq!(found.len(), 1);
    assert!(found[0].starts_with("dev-build: banned-binding"));
    assert_eq!(
        run_for("refused", "personal-codex", "gold-band-personal"),
        1
    );
    assert_eq!(run_for("valid", "work-codex", "gold-band-personal"), 1);
}

const SHAPE_REFUSED: [(&str, &str); 26] = [
    ("multiple-entries", "entry-multiple"),
    (
        "validated-output-without-success-condition",
        "success-expression-required",
    ),
    ("no-entry", "entry-missing"),
    ("no-end", "end-required,dangling-node"),
    ("no-artifact", "output-artifact-required"),
    (
        "unparseable-success-condition",
        "success-expression-invalid",
    ),
    ("schema-missing-path", "schema-missing-path"),
    ("legacy-json-schema", "legacy-schema"),
    ("manual-check-with-validation", "manual-check-exclusive"),
    ("dangling-node", "dangling-node"),
    ("edge-source-missing", "edge-source-missing"),
    ("edge-target-missing", "edge-target-missing"),
    ("edge-outcome-invalid", "edge-outcome-invalid"),
    (
        "failure-edge-without-result-decision",
        "failure-edge-unsupported",
    ),
    ("duplicate-edge-outcome", "duplicate-edge-outcome"),
    ("reserved-node-id", "reserved-node-id"),
    ("duplicate-node-id", "duplicate-node-id"),
    ("max-attempts-not-positive", "max-attempts-positive"),
    ("max-rounds-not-positive", "max-rounds-positive"),
    ("slot-missing", "slot-required"),
    ("slot-duplicate", "slot-duplicate"),
    ("profile-missing", "profile-required"),
    (
        "terminal-edge-source",
        "terminal-edge-source,edge-source-missing",
    ),
    ("success-edge-to-new-round", "success-new-round"),
    ("new-round-without-entry", "new-round-entry-required"),
    ("continue-into-end", "continue-terminal-target"),
];

fn shape_findings(kind: &str, name: &str) -> Vec<String> {
    let text = fs::read_to_string(fixture(kind, name)).unwrap();
    let root: Value = serde_json::from_str(&text).unwrap();
    workflow_lint::lint_shape(&root).unwrap()
}

fn run_shape(kind: &str, name: &str) -> i32 {
    workflow_lint::run([
        "--shape".into(),
        "--instance".into(),
        "gold-band-personal".into(),
        fixture(kind, name).into_os_string(),
    ])
}

#[test]
fn the_valid_shape_fixture_passes() {
    assert_eq!(
        shape_findings("shape-valid", "check-round-loop"),
        Vec::<String>::new()
    );
    assert_eq!(run_shape("shape-valid", "check-round-loop"), 0);
}

#[test]
fn a_failure_target_sorted_before_its_source_is_not_an_entry() {
    assert_eq!(
        shape_findings("shape-valid", "failure-target"),
        Vec::<String>::new()
    );
}

#[test]
fn each_refused_shape_fixture_breaks_exactly_its_rule() {
    for (name, rule) in SHAPE_REFUSED {
        let found = shape_findings("shape-refused", name);
        assert!(!found.is_empty(), "{name} passed");
        for line in &found {
            let mut parts = line.splitn(3, ": ");
            let (subject, found_rule) = (parts.next().unwrap(), parts.next().unwrap());
            assert!(!subject.is_empty(), "{name}: {line}");
            assert!(rule.split(',').any(|r| r == found_rule), "{name}: {line}");
        }
        assert_eq!(run_shape("shape-refused", name), 1, "{name}");
    }
}

#[test]
fn the_owner_reported_messages_are_reproduced() {
    let multiple = shape_findings("shape-refused", "multiple-entries");
    assert_eq!(multiple.len(), 1);
    assert!(multiple[0]
        .starts_with("check: entry-multiple: Workflow has multiple entry nodes: gate, check."));
    let output = shape_findings(
        "shape-refused",
        "validated-output-without-success-condition",
    );
    assert_eq!(
        output,
        ["write: success-expression-required: write requires a success expression when output validation is enabled."]
    );
}

#[test]
fn without_the_flag_the_shape_is_not_checked() {
    assert_eq!(exit_code("shape-refused", "multiple-entries"), 0);
}
