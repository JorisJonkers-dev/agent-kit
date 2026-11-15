use serde_json::{json, Value};

use super::*;

fn worker(id: &str, agent: &str, model: &str, artifact: Option<&str>) -> Value {
    let mut node = json!({
        "type":"worker","id":id,"executionSlotId":format!("slot-{id}"),
        "provider":agent,"model":model,"profile":"pf-builtin-dev"
    });
    if let Some(artifact) = artifact {
        node["output"] = json!({"kind":"json","artifact":artifact});
    }
    node
}

fn edge(from: &str, to: &str) -> Value {
    json!({"from":from,"to":to,"on":"success"})
}

fn chain(ids: &[&str]) -> Vec<Value> {
    ids.windows(2).map(|w| edge(w[0], w[1])).collect()
}

fn run(nodes: &[Value], edges: &[Value], instance: Instance) -> Vec<String> {
    critique(nodes, edges, &[], instance, None)
}

fn rules(found: &[String]) -> Vec<String> {
    found
        .iter()
        .map(|f| f.splitn(3, ": ").take(2).collect::<Vec<_>>().join(": "))
        .collect()
}

#[test]
fn only_the_declared_artifact_keys_are_writes() {
    for key in [
        "spec-write",
        "spec-draft",
        "grill-summary",
        "docs-update",
        "design-record",
        "implementation",
        "tests-written",
        "repair-2-implementation",
    ] {
        assert!(is_writer_key(key), "{key}");
    }
    for key in [
        "",
        "spec-round-1-open-questions",
        "spec-open-findings",
        "critics-1-findings",
        "gate-x-report",
        "repair-x-findings",
        "repair-1",
        "escalation-judgment",
    ] {
        assert!(!is_writer_key(key), "{key}");
    }
}

#[test]
fn a_record_node_is_the_runner_record_or_a_verdict() {
    for id in [
        "gate-critique-spec-record",
        "gate-critique-x-y-record",
        "verdict-1",
        "verdict-12",
    ] {
        assert!(is_record(id), "{id}");
    }
    for id in [
        "copy-critique-spec-record",
        "verdict-",
        "verdict-a",
        "spec-verdicts",
        "gate-critique-spec-fan-out",
    ] {
        assert!(!is_record(id), "{id}");
    }
}

#[test]
fn a_write_that_reaches_the_end_unreviewed_is_refused() {
    let nodes = [
        worker("w", "vibe", "glm-5-3", Some("spec-write")),
        worker("done", "vibe", "x", None),
    ];
    let found = run(&nodes, &chain(&["w", "done", "$end"]), Instance::Personal);
    assert_eq!(rules(&found), ["w: critic-required"]);
    assert!(found[0].contains("w -> done -> $end"), "{}", found[0]);
}

#[test]
fn a_write_whose_every_path_crosses_a_record_passes() {
    let nodes = [
        worker("w", "vibe", "glm-5-3", Some("spec-write")),
        worker("gate-critique-spec-record", "g", "stub-model", None),
        worker("check", "vibe", "x", None),
    ];
    let mut edges = chain(&["w", "gate-critique-spec-record", "check", "$end"]);
    edges.push(json!({"from":"w","to":"check","on":"failure"}));
    // The failure edge skips the record.
    assert_eq!(
        rules(&run(&nodes, &edges, Instance::Personal)),
        ["w: critic-required"]
    );
    edges.pop();
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
    let verdict = [
        worker("w", "pi", "glm-5.3", Some("implementation")),
        worker("verdict-1", "g", "stub-model", None),
    ];
    assert!(run(
        &verdict,
        &chain(&["w", "verdict-1", "$end"]),
        Instance::Personal
    )
    .is_empty());
}

#[test]
fn a_refused_grant_does_not_reach_the_end_with_the_artifact() {
    let nodes = [
        worker("w", "claude-work", "sonnet", Some("tests-written")),
        worker("grant-refused", "claude-work", "sonnet", None),
        worker("verdict-1", "g", "stub-model", None),
    ];
    let mut edges = chain(&["w", "verdict-1", "$end"]);
    edges.push(edge("w", "grant-refused"));
    edges.push(edge("grant-refused", "$end"));
    assert!(run(&nodes, &edges, Instance::Work).is_empty());
}

#[test]
fn loops_and_dead_ends_do_not_hang_the_walk() {
    let nodes = [
        worker("w", "vibe", "glm-5-3", Some("spec-write")),
        worker("a", "vibe", "x", None),
    ];
    let mut edges = chain(&["w", "a"]);
    edges.push(edge("a", "w"));
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
}

#[test]
fn gates_and_unbound_workers_are_not_writers() {
    let nodes = [
        worker("gate", "gate", "stub-model", Some("spec-write")),
        worker("lone", "", "", Some("spec-write")),
        json!({"type":"ai-dynamic","id":"dyn","output":{"artifact":"spec-write"}}),
        json!({"type":"worker","id":"bare"}),
    ];
    let edges = [
        edge("gate", "$end"),
        edge("lone", "$end"),
        edge("dyn", "$end"),
        edge("bare", "$end"),
    ];
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
}

#[test]
fn the_wrapper_binding_names_the_writer_when_the_node_has_none() {
    let node = json!({"type":"worker","id":"w","executionSlotId":"s","profile":"p","output":{"artifact":"spec-draft"}});
    let bindings = [
        json!({"executionSlotId":"other","agentId":"codex","modelId":"gpt-5.5"}),
        json!({"executionSlotId":"s","agentId":"vibe","modelId":"mistral-large-4-0"}),
    ];
    let found = critique(&[node], &[edge("w", "$end")], &bindings, Instance::Personal, None);
    assert_eq!(rules(&found), ["w: critic-required"]);
}

fn dynamic(id: &str, agents: Value) -> Value {
    json!({"type":"ai-dynamic","id":id,"agentStrategy":{"availableAgents":agents}})
}

#[test]
fn an_ai_dynamic_critic_of_the_writers_family_is_refused() {
    let nodes = [
        worker(
            "w",
            "pi",
            "litellm-vibe-study/deepseek-v4.1-flash",
            Some("implementation"),
        ),
        dynamic(
            "critics",
            json!([
                {"provider":"pi","model":"litellm-vibe/glm-5.3"},
                {"provider":"pi","model":"litellm-vibe/deepseek-v4.1-flash"},
                {"provider":"pi","model":"unknown-model"}
            ]),
        ),
        worker("verdict-1", "g", "stub-model", None),
    ];
    let found = run(
        &nodes,
        &chain(&["w", "critics", "verdict-1", "$end"]),
        Instance::Personal,
    );
    assert_eq!(rules(&found), ["critics: critic-family"]);
    assert!(found[0].contains("pi/litellm-vibe/deepseek-v4.1-flash is of family deepseek"));
    assert!(found[0].contains(
        "writer w (pi/litellm-vibe-study/deepseek-v4.1-flash), whose implementation it reviews"
    ));
}

#[test]
fn a_dynamic_critic_without_candidates_is_not_compared() {
    let nodes = [
        worker("w", "pi", "glm-5.3", Some("implementation")),
        json!({"type":"ai-dynamic","id":"critics"}),
        worker("verdict-1", "g", "stub-model", None),
    ];
    let edges = chain(&["w", "critics", "verdict-1", "$end"]);
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
}

#[test]
fn a_review_profile_worker_is_a_critic_with_its_own_binding() {
    let mut critic = worker("review", "pi", "glm-5.3", None);
    critic["profile"] = json!("pf-diff-reviewer");
    let mut unbound = worker("review-2", "x", "stub-model", None);
    unbound["profile"] = json!("pf-diff-reviewer");
    let nodes = [
        worker("w", "pi", "litellm-vibe/glm-5.3", Some("implementation")),
        critic,
        unbound,
        worker("verdict-1", "g", "stub-model", None),
    ];
    let edges = chain(&["w", "review", "review-2", "verdict-1", "$end"]);
    let found = run(&nodes, &edges, Instance::Personal);
    assert_eq!(rules(&found), ["review: critic-family"]);
}

#[test]
fn the_runner_fan_out_is_a_critic_with_the_stages_roster() {
    let nodes = [
        worker("w", "vibe", "mistral-large-4-0", Some("spec-draft")),
        worker("gate-critique-spec-fan-out", "g", "stub-model", None),
        worker("gate-critique-spec-record", "g", "stub-model", None),
    ];
    let edges = chain(&[
        "w",
        "gate-critique-spec-fan-out",
        "gate-critique-spec-record",
        "$end",
    ]);
    let found = run(&nodes, &edges, Instance::Personal);
    assert_eq!(rules(&found), ["gate-critique-spec-fan-out: critic-family"]);
    assert!(
        found[0].contains("routing client pi-vibe-mistral-large is of family mistral"),
        "{}",
        found[0]
    );
    // The work instance routes the stage to Codex and Claude: a Mistral writer is clear of it.
    assert!(run(&nodes, &edges, Instance::Work).is_empty());
    // A stage the routing file does not know has no roster.
    let mut other = nodes.to_vec();
    other[1]["id"] = json!("gate-critique-nothing-fan-out");
    let edges = chain(&[
        "w",
        "gate-critique-nothing-fan-out",
        "gate-critique-spec-record",
        "$end",
    ]);
    assert!(run(&other, &edges, Instance::Personal).is_empty());
}

#[test]
fn only_the_nearest_writer_is_the_one_a_critic_reviews() {
    let nodes = [
        worker("first", "vibe", "mistral-large-4-0", Some("spec-draft")),
        worker("second", "vibe", "glm-5-3", Some("tickets-draft")),
        dynamic(
            "critics",
            json!([{"provider":"pi","model":"mistral-large"}]),
        ),
        worker("verdict-1", "g", "stub-model", None),
    ];
    let edges = chain(&["first", "second", "critics", "verdict-1", "$end"]);
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
    // A critic with no writer upstream, and a critic that sits on a loop, review nothing.
    let lone = [dynamic(
        "critics",
        json!([{"provider":"pi","model":"mistral-large"}]),
    )];
    let mut loops = chain(&["critics", "x"]);
    loops.push(edge("x", "critics"));
    assert!(run(&lone, &loops, Instance::Personal).is_empty());
}

#[test]
fn the_group_runs_from_the_command_line() {
    let text = json!({"nodes":[worker("w","vibe","glm-5-3",Some("spec-write"))],"edges":[edge("w","$end")]})
        .to_string();
    let path = std::env::temp_dir().join(format!("wf-lint-critique-{}", std::process::id()));
    std::fs::write(&path, text).unwrap();
    let go = |flags: &[&str]| {
        let mut args: Vec<std::ffi::OsString> = flags.iter().map(Into::into).collect();
        args.push("--instance".into());
        args.push("gold-band-personal".into());
        args.push(path.clone().into_os_string());
        crate::run(args)
    };
    assert_eq!(go(&[]), 0);
    assert_eq!(go(&["--critique"]), 1);
    std::fs::remove_file(&path).unwrap();
    assert!(crate::lint_critique(&json!([]), Instance::Work).is_err());
}

fn fixture_findings(group: &str, name: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(group)
        .join(format!("{name}.json"));
    let root: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    crate::lint_critique(&root, Instance::Personal).unwrap()
}

#[test]
fn the_valid_fixtures_pass() {
    for name in ["reviewed-by-the-runner", "reviewed-by-dynamic-critics"] {
        assert!(
            fixture_findings("critique-valid", name).is_empty(),
            "{name}"
        );
    }
}

#[test]
fn each_refused_fixture_breaks_its_rule() {
    for (name, rule) in [
        ("write-without-critic", "critic-required"),
        ("critic-of-the-writers-family", "critic-family"),
        ("runner-roster-of-the-writers-family", "critic-family"),
    ] {
        let found = fixture_findings("critique-refused", name);
        assert!(!found.is_empty(), "{name}");
        assert!(rules(&found).iter().all(|r| r.ends_with(rule)), "{name}");
    }
}

fn spec_review(writer: (&str, &str), orchestrator: Option<(&str, &str)>) -> (Vec<Value>, Vec<Value>) {
    let mut nodes = vec![
        worker("w", writer.0, writer.1, Some("spec-draft")),
        worker("gate-critique-spec-fan-out", "g", "stub-model", None),
    ];
    let mut ids = vec!["w", "gate-critique-spec-fan-out"];
    if let Some((agent, model)) = orchestrator {
        nodes.push(worker("spec-verdicts", agent, model, Some("spec-open-findings")));
        ids.push("spec-verdicts");
    }
    nodes.push(worker("gate-critique-spec-record", "g", "stub-model", None));
    ids.extend(["gate-critique-spec-record", "$end"]);
    (nodes, chain(&ids))
}

#[test]
fn the_runner_leaves_the_orchestrators_family_out_of_the_roster() {
    // A Mistral writer meets the Mistral critic of the spec roster unless the orchestrator is Mistral too.
    let (nodes, edges) = spec_review(("vibe", "mistral-large-4-0"), None);
    assert_eq!(rules(&run(&nodes, &edges, Instance::Personal)), ["gate-critique-spec-fan-out: critic-family"]);
    let (nodes, edges) = spec_review(("vibe", "mistral-large-4-0"), Some(("vibe", "mistral-large-4-0")));
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
    // An orchestrator of another family does not help.
    let (nodes, edges) = spec_review(
        ("vibe", "mistral-large-4-0"),
        Some(("vibe-study", "deepseek-v4-1-flash-1017")),
    );
    assert_eq!(rules(&run(&nodes, &edges, Instance::Personal)), ["gate-critique-spec-fan-out: critic-family"]);
    // The work instance: a Codex writer and Codex orchestrator leave the Claude critic.
    let (nodes, edges) = spec_review(("codex", "gpt-5.5"), Some(("codex", "gpt-5.5")));
    assert!(run(&nodes, &edges, Instance::Work).is_empty());
    let (nodes, edges) = spec_review(("codex", "gpt-5.5"), Some(("claude-work", "sonnet")));
    assert_eq!(rules(&run(&nodes, &edges, Instance::Work)), ["gate-critique-spec-fan-out: critic-family"]);
}

#[test]
fn a_machine_override_changes_the_roster_the_check_runs_over() {
    let (nodes, edges) = spec_review(("vibe", "glm-5-3"), Some(("vibe", "glm-5-3")));
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
    // The override adds a writer-family critic to the base spec roster.
    let patch = json!({"stages":{"spec":{"base":[{"client":"pi-vibe-glm","role":"spec-redteam","lenses":["fidelity"]}]}}});
    // The orchestrator's own family is still dropped, so the writer meets no critic of it.
    assert!(critique(&nodes, &edges, &[], Instance::Personal, Some(&patch)).is_empty());
    let (nodes, edges) = spec_review(("vibe", "glm-5-3"), Some(("vibe-study", "deepseek-v4-1-flash-1017")));
    let found = critique(&nodes, &edges, &[], Instance::Personal, Some(&patch));
    // The patched base slot and the shipped deep slot are both of the writer's family.
    assert_eq!(rules(&found), ["gate-critique-spec-fan-out: critic-family"; 2]);
    assert!(found[0].contains("routing client pi-vibe-glm is of family zhipu"), "{}", found[0]);
    // A patch that removes every client leaves nothing to compare.
    let none = json!({"clients":null});
    assert!(critique(&nodes, &edges, &[], Instance::Personal, Some(&none)).is_empty());
}

#[test]
fn the_critic_family_findings_leave_out_the_other_rules() {
    let (nodes, mut edges) = spec_review(("vibe", "mistral-large-4-0"), None);
    edges.retain(|e| e["to"] != "gate-critique-spec-record");
    edges.push(edge("gate-critique-spec-fan-out", "$end"));
    let root = json!({"nodes": nodes, "edges": edges});
    let all = crate::lint_critique(&root, Instance::Personal).unwrap();
    assert_eq!(rules(&all), ["w: critic-required", "gate-critique-spec-fan-out: critic-family"]);
    let family = crate::critic_family_findings(&root, Instance::Personal, None).unwrap();
    assert_eq!(rules(&family), ["gate-critique-spec-fan-out: critic-family"]);
    assert!(crate::critic_family_findings(&json!([]), Instance::Work, None).is_err());
}

#[test]
fn a_later_round_of_a_stage_is_the_same_stage_with_the_same_roster() {
    assert_eq!(without_round("gate-critique-spec-fan-out-2"), ("gate-critique-spec-fan-out", "-2"));
    assert_eq!(without_round("gate-critique-spec-fan-out"), ("gate-critique-spec-fan-out", ""));
    assert_eq!(without_round("spec-verdicts-"), ("spec-verdicts-", ""));
    assert_eq!(without_round("plain"), ("plain", ""));
    assert!(is_record("gate-critique-spec-record-2"));
    assert!(!is_record("gate-critique-spec-record-x"));
    // The second round's fan-out is checked, and its orchestrator is the round's own.
    let nodes = [
        worker("w", "vibe", "glm-5-3", Some("spec-draft")),
        worker("gate-critique-spec-fan-out-2", "g", "stub-model", None),
        worker("spec-verdicts-2", "vibe", "glm-5-3", Some("spec-open-findings-2")),
        worker("gate-critique-spec-record-2", "g", "stub-model", None),
    ];
    let edges = chain(&["w", "gate-critique-spec-fan-out-2", "spec-verdicts-2", "gate-critique-spec-record-2", "$end"]);
    assert!(run(&nodes, &edges, Instance::Personal).is_empty());
    let mut other = nodes.to_vec();
    other[2] = worker("spec-verdicts", "vibe", "glm-5-3", Some("spec-open-findings"));
    let found = run(&other, &edges, Instance::Personal);
    assert_eq!(rules(&found), ["gate-critique-spec-fan-out-2: critic-family"; 1]);
}
