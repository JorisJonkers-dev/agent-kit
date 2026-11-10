use super::*;

const EDGE: &str = r#"{"from":"dev","to":"$end"}"#;

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn lint_def(nodes: &str, edges: &str) -> Vec<String> {
    let text = format!(r#"{{"nodes":[{nodes}],"edges":[{edges}]}}"#);
    lint(&parse(&text), Instance::Personal).unwrap()
}

fn lint_nodes(nodes: &str) -> Vec<String> {
    lint_def(nodes, EDGE)
}

fn worker(id: &str, provider: &str, model: &str) -> String {
    format!(r#"{{"id":"{id}","provider":"{provider}","model":"{model}"}}"#)
}

fn tmp(tag: &str, text: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("wf-lint-{tag}-{}", std::process::id()));
    fs::write(&path, text).unwrap();
    path
}

fn go(files: Vec<OsString>) -> i32 {
    let mut args = vec![
        OsString::from("--instance"),
        OsString::from("gold-band-personal"),
    ];
    args.extend(files);
    run(args)
}

fn rules(problems: &[String]) -> Vec<String> {
    problems
        .iter()
        .map(|p| p.splitn(3, ": ").take(2).collect::<Vec<_>>().join(": "))
        .collect()
}

#[test]
fn codex_is_banned_and_names_the_node() {
    let found = lint_nodes(&worker("a", "CodeX-CLI", "gpt-5"));
    assert_eq!(rules(&found), ["a: banned-binding"]);
}

#[test]
fn openrouter_and_opencode_bindings_pass() {
    for model in ["openrouter/x", "opencode/zen", "OpenCode/Zen"] {
        assert!(lint_nodes(&worker("a", "acp", model)).is_empty());
    }
}

#[test]
fn opus_passes_only_on_the_three_judgment_suffixes() {
    for id in ["x-escalation", "x-acceptance", "x-planning"] {
        assert!(lint_nodes(&worker(id, "acp", "Opus-4")).is_empty(), "{id}");
    }
    for id in ["a", "x-escalations", "acceptance", "x-review"] {
        let found = lint_nodes(&worker(id, "acp", "opus-4"));
        assert_eq!(rules(&found), [format!("{id}: opus-outside-judgment")]);
    }
    let found = lint_nodes(&worker("a", "opus-crew", "m"));
    assert_eq!(rules(&found), ["a: opus-outside-judgment"]);
}

#[test]
fn memory_write_passes_only_on_acceptance_and_escalation() {
    for id in ["x-acceptance", "x-escalation"] {
        assert!(lint_nodes(&worker(id, "acp+Memory-Write", "m")).is_empty());
    }
    for id in ["a", "x-planning"] {
        let found = lint_nodes(&worker(id, "acp+memory-write", "m"));
        assert_eq!(
            rules(&found),
            [format!("{id}: memory-write-outside-judgment")]
        );
    }
}

#[test]
fn the_marker_only_counts_in_an_agent_id_not_a_model() {
    assert!(lint_nodes(&worker("a", "acp", "memory-write")).is_empty());
}

#[test]
fn one_node_can_break_three_rules() {
    let found = lint_nodes(&worker("a", "codex+memory-write", "opus"));
    assert_eq!(
        rules(&found),
        [
            "a: banned-binding",
            "a: opus-outside-judgment",
            "a: memory-write-outside-judgment"
        ]
    );
}

#[test]
fn a_wrapper_binding_applies_where_its_slot_joins_it() {
    let text = r#"{"workflow":{"nodes":[
        {"id":"dev","executionSlotId":"s"},{"id":"other","executionSlotId":"t"}],
        "edges":[]},
      "modelBindings":{"bindings":[
        {"executionSlotId":"s","agentId":"a+memory-write","modelId":"opus"},
        {"executionSlotId":"u","agentId":"codex"}]}}"#;
    let found = lint(&parse(text), Instance::Personal).unwrap();
    assert_eq!(
        rules(&found),
        [
            "dev: opus-outside-judgment",
            "dev: memory-write-outside-judgment"
        ]
    );
}

#[test]
fn strategy_bindings_are_checked_in_fixed_and_dynamic_modes() {
    let nodes = concat!(
        r#"{"id":"f","agentStrategy":{"mode":"fixed","provider":"codex","model":"m"}},"#,
        r#"{"id":"d","agentStrategy":{"mode":"dynamic","#,
        r#""bootstrapProvider":"a+memory-write","bootstrapModel":"opencode/zen","#,
        r#""acceptanceModel":"opus","routingPrompt":"r","#,
        r#""availableAgents":[{"provider":"codex","model":"openrouter/x"}]}},"#,
        r#"{"id":"p","agentStrategy":{"mode":"dynamic","#,
        r#""bootstrapModel":"opencode/zen","availableAgents":[{"model":"openrouter/x"}]}}"#
    );
    assert_eq!(
        rules(&lint_nodes(nodes)),
        [
            "f: banned-binding",
            "d: banned-binding",
            "d: opus-outside-judgment",
            "d: memory-write-outside-judgment"
        ]
    );
}

#[test]
fn a_strategy_without_a_known_mode_cannot_hide_a_binding() {
    let nodes = r#"{"id":"o","agentStrategy":{"provider":"codex","model":"opus-4"}}"#;
    assert_eq!(
        rules(&lint_nodes(nodes)),
        ["o: banned-binding", "o: opus-outside-judgment"]
    );
}

#[test]
fn a_node_without_an_id_is_gold_bands_problem() {
    assert!(lint_nodes(r#"{"provider":"codex"}"#).is_empty());
}

const TWO: &str = concat!(
    r#"{"id":"a","provider":"acp","model":"m"},"#,
    r#"{"id":"b","provider":"acp","model":"m"}"#
);

#[test]
fn a_back_edge_is_a_loop() {
    let edges = r#"{"from":"a","to":"b"},{"from":"b","to":"a"}"#;
    assert_eq!(rules(&lint_def(TWO, edges)), ["a: loop-not-unrolled"]);
}

#[test]
fn a_back_edge_is_a_loop_even_with_a_limit_declared() {
    let text = format!(
        r#"{{"control":{{"max_attempts":2,"max_rounds":3}},"nodes":[{TWO}],
        "edges":[{{"from":"a","to":"b"}},{{"from":"b","to":"a"}}]}}"#
    );
    let found = lint(&parse(&text), Instance::Personal).unwrap();
    assert_eq!(rules(&found), ["a: loop-not-unrolled"]);
}

#[test]
fn a_self_loop_is_a_loop() {
    let node = worker("a", "acp", "m");
    let found = lint_def(&node, r#"{"from":"a","to":"a"}"#);
    assert_eq!(rules(&found), ["a: loop-not-unrolled"]);
}

#[test]
fn a_diamond_and_edges_to_unknown_nodes_are_not_loops() {
    let nodes = r#"{"id":"a"},{"id":"b"},{"id":"c"},{"id":"d"}"#;
    let edges = concat!(
        r#"{"from":"a","to":"b"},{"from":"a","to":"c"},"#,
        r#"{"from":"b","to":"d"},{"from":"c","to":"d"},"#,
        r#"{"from":"d","to":"$end"},{"from":"d","to":"ghost"},{"from":"ghost","to":"a"},{}"#
    );
    assert!(lint_def(nodes, edges).is_empty());
}

#[test]
fn a_new_round_edge_is_a_loop() {
    let node = worker("a", "acp", "m");
    let found = lint_def(&node, r#"{"from":"a","to":"$new-round"}"#);
    assert_eq!(rules(&found), ["a: loop-not-unrolled"]);
    assert!(found[0].contains("`$new-round`"));
}

#[test]
fn a_new_round_edge_without_a_source_names_the_workflow() {
    let node = worker("a", "acp", "m");
    let found = lint_def(&node, r#"{"to":"$new-round"}"#);
    assert_eq!(rules(&found), ["workflow: loop-not-unrolled"]);
}

#[test]
fn shapes_the_lint_cannot_read_are_errors() {
    let err = |text: &str| lint(&parse(text), Instance::Work).unwrap_err();
    assert_eq!(err("[]"), "the file is not a JSON object");
    assert_eq!(err(r#"{"workflow":1}"#), "workflow is not a JSON object");
    assert_eq!(err("{}"), "nodes is not an array");
    assert_eq!(err(r#"{"nodes":1}"#), "nodes is not an array");
    assert_eq!(err(r#"{"nodes":[]}"#), "edges is not an array");
    assert_eq!(err(r#"{"nodes":[],"edges":1}"#), "edges is not an array");
}

#[test]
fn bad_usage_exits_two() {
    assert_eq!(run(std::iter::empty::<OsString>()), 2);
}

#[test]
fn an_unreadable_file_exits_two() {
    let path = std::env::temp_dir().join(format!("wf-lint-absent-{}", std::process::id()));
    let _ = fs::remove_file(&path);
    assert_eq!(go(vec![path.into_os_string()]), 2);
}

#[test]
fn invalid_json_and_invalid_workflows_exit_two() {
    assert_eq!(go(vec![tmp("j", "{{").into_os_string()]), 2);
    assert_eq!(go(vec![tmp("wf", "[]").into_os_string()]), 2);
}

#[test]
fn many_files_exit_the_worst_outcome() {
    let neat = tmp("m-clean", r#"{"nodes":[],"edges":[]}"#).into_os_string();
    let text = format!(r#"{{"nodes":[{}],"edges":[]}}"#, worker("a", "codex", "m"));
    let wrong = tmp("m-bad", &text).into_os_string();
    let shape = tmp("m-shape", "[]").into_os_string();
    assert_eq!(go(vec![neat.clone(), neat.clone()]), 0);
    assert_eq!(go(vec![neat, wrong.clone()]), 1);
    assert_eq!(go(vec![wrong, shape]), 2);
}

#[test]
fn codex_is_allowed_in_the_work_instance_and_refused_in_the_personal_one() {
    let text = format!(
        r#"{{"nodes":[{}],"edges":[]}}"#,
        worker("a", "codex", "gpt-5")
    );
    let root = parse(&text);
    assert!(lint(&root, Instance::Work).unwrap().is_empty());
    let found = lint(&root, Instance::Personal).unwrap();
    assert_eq!(rules(&found), ["a: banned-binding"]);
    assert!(found[0].contains("personal instance"));
}

#[test]
fn the_instance_flag_is_required_and_must_name_an_instance() {
    let file = tmp("inst", r#"{"nodes":[],"edges":[]}"#).into_os_string();
    let flag = |s: &str| OsString::from(s);
    assert_eq!(run(vec![file.clone()]), 2);
    assert_eq!(run(vec![flag("--instance")]), 2);
    assert_eq!(run(vec![flag("--instance"), flag("nope"), file.clone()]), 2);
    assert_eq!(run(vec![flag("--instance=nope"), file.clone()]), 2);
    assert_eq!(run(vec![flag("--instance=gold-band")]), 2);
    // Repeated --instance and an unknown flag are bad usage.
    assert_eq!(
        run(vec![flag("--instance=gold-band"), flag("--instance=gold-band"), file.clone()]),
        2
    );
    assert_eq!(run(vec![flag("--instance=gold-band"), flag("--bogus"), file.clone()]), 2);
    assert_eq!(run(vec![flag("--instance=gold-band"), file.clone()]), 0);
    assert_eq!(
        run(vec![file, flag("--instance"), flag("gold-band-personal")]),
        0
    );
}
