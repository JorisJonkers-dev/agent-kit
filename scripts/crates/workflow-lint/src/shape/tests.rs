use serde_json::{json, Value};

use super::*;

fn worker(id: &str) -> Value {
    json!({"type":"worker","id":id,"executionSlotId":format!("slot-{id}"),"profile":"p","goal":"g"})
}

fn validated(id: &str, schema: Value, condition: Value) -> Value {
    let mut node = worker(id);
    node["output"] = json!({"kind":"json","artifact":format!("{id}-out"),"schema":schema});
    node["success_condition"] = condition;
    node
}

fn edge(from: &str, to: &str, on: &str) -> Value {
    json!({"from":from,"to":to,"on":on})
}

fn check(nodes: Vec<Value>, edges: Vec<Value>) -> Vec<String> {
    let def =
        json!({"id":"w","nodes":nodes,"edges":edges,"control":{"max_attempts":1,"max_rounds":1}});
    shape(
        &def,
        def["nodes"].as_array().unwrap(),
        def["edges"].as_array().unwrap(),
    )
}

fn rules(found: &[String]) -> Vec<String> {
    found
        .iter()
        .map(|f| f.splitn(3, ": ").take(2).collect::<Vec<_>>().join(": "))
        .collect()
}

fn one(nodes: Vec<Value>, edges: Vec<Value>) -> Vec<String> {
    rules(&check(nodes, edges))
}

fn linear() -> (Vec<Value>, Vec<Value>) {
    (
        vec![worker("a"), worker("b")],
        vec![edge("a", "b", "success"), edge("b", "$end", "success")],
    )
}

#[test]
fn a_valid_workflow_has_no_findings() {
    let (nodes, edges) = linear();
    assert!(check(nodes, edges).is_empty());
}

#[test]
fn a_check_round_loop_with_backward_failure_edges_has_one_entry() {
    let nodes = vec![
        validated(
            "write",
            json!({"ok":true}),
            json!({"path":"$.ok","equals":true}),
        ),
        {
            let mut n = worker("check");
            n["manual_check"] = json!(true);
            n
        },
        worker("next"),
    ];
    let edges = vec![
        edge("write", "check", "success"),
        edge("check", "next", "success"),
        edge("check", "write", "failure"),
        edge("next", "$end", "success"),
    ];
    assert!(check(nodes, edges).is_empty());
}

#[test]
fn a_check_node_reached_only_by_failure_edges_is_a_second_entry() {
    // The to-spec shape: the check nodes hang off failure edges only.
    let nodes = vec![
        worker("gate"),
        validated(
            "round-1",
            json!({"q":[]}),
            json!({"path":"$.q","equals":[]}),
        ),
        {
            let mut n = worker("check-1");
            n["manual_check"] = json!(true);
            n
        },
        validated(
            "round-2",
            json!({"q":[]}),
            json!({"path":"$.q","equals":[]}),
        ),
    ];
    let edges = vec![
        edge("gate", "round-1", "success"),
        edge("round-1", "round-2", "success"),
        edge("round-1", "check-1", "failure"),
        edge("check-1", "round-2", "success"),
        edge("round-2", "$end", "success"),
    ];
    let found = check(nodes, edges);
    assert_eq!(found.len(), 1);
    assert!(found[0]
        .starts_with("check-1: entry-multiple: Workflow has multiple entry nodes: gate, check-1"));
}

#[test]
fn a_workflow_without_any_entry_is_refused() {
    let mut b = worker("b");
    b["manual_check"] = json!(true);
    let nodes = vec![worker("a"), b];
    let edges = vec![
        edge("a", "b", "success"),
        edge("b", "a", "success"),
        edge("b", "$end", "failure"),
    ];
    assert_eq!(one(nodes, edges), ["workflow: entry-missing"]);
}

#[test]
fn a_success_cycle_node_is_ordered_last() {
    // a -> b -> c -> b: b and c never reach indegree 0; a stays the entry.
    let mut a = worker("a");
    a["manual_check"] = json!(true);
    let nodes = vec![a, worker("b"), worker("c")];
    let edges = vec![
        edge("a", "b", "success"),
        edge("a", "$end", "failure"),
        edge("b", "c", "success"),
        edge("c", "b", "success"),
    ];
    assert!(check(nodes, edges).is_empty());
}

#[test]
fn duplicate_success_edges_keep_the_order_consistent() {
    let (nodes, mut edges) = linear();
    edges.push(edge("a", "b", "success"));
    assert_eq!(one(nodes, edges), ["a: duplicate-edge-outcome"]);
}

#[test]
fn workflow_level_rules() {
    let def = json!({"nodes":[],"edges":[],"control":{"max_attempts":0,"max_rounds":"0"}});
    let found = shape(&def, &[], &[]);
    assert_eq!(
        rules(&found),
        [
            "workflow: workflow-id-required",
            "workflow: nodes-required",
            "workflow: end-required",
            "workflow: max-attempts-positive",
            "workflow: max-rounds-positive",
        ]
    );
    assert!(found[3].ends_with("Max attempts must be greater than 0."));
}

#[test]
fn control_limits_follow_the_editors_number_coercion() {
    for (value, bad) in [
        (json!(2), false),
        (json!("3"), false),
        (json!(" 4 "), false),
        (json!(true), false),
        (json!(false), true),
        (json!(""), true),
        (json!("x"), true),
        (json!([1]), true),
        (json!(-1), true),
        (json!(0.5), true),
        (json!(null), false),
        (json!("Infinity"), true),
    ] {
        let def = json!({"id":"w","control":{"max_rounds":value}});
        let (nodes, edges) = linear();
        let found = shape(&def, &nodes, &edges);
        assert_eq!(!found.is_empty(), bad, "{value}");
    }
    // A missing control block is fine.
    let (nodes, edges) = linear();
    assert!(shape(&json!({"id":"w"}), &nodes, &edges).is_empty());
}

#[test]
fn node_id_rules() {
    let (_, edges) = linear();
    let nodes = vec![worker("a"), worker("a"), worker("b")];
    let found = one(nodes, edges.clone());
    assert!(found.contains(&"a: duplicate-node-id".to_string()));
    let mut reserved = worker("$end");
    reserved["executionSlotId"] = json!("s");
    for id in ["$end", "$entry", "$new-round"] {
        let mut n = worker(id);
        n["executionSlotId"] = json!(format!("s-{id}"));
        let found = one(vec![n, worker("a")], vec![edge("a", "$end", "success")]);
        assert!(found.contains(&format!("{id}: reserved-node-id")), "{id}");
    }
    let found = one(
        vec![worker(""), worker("a")],
        vec![edge("a", "$end", "success")],
    );
    assert!(found.contains(&"workflow: node-id-required".to_string()));
    assert!(found.contains(&"workflow: dangling-node".to_string()));
}

#[test]
fn a_node_without_an_outgoing_edge_dangles() {
    let nodes = vec![worker("a"), worker("b")];
    let edges = vec![edge("a", "b", "success"), edge("a", "$end", "failure")];
    let found = one(nodes, edges);
    assert!(found.contains(&"b: dangling-node".to_string()));
}

#[test]
fn slot_and_role_rules() {
    let mut a = worker("a");
    a["executionSlotId"] = json!(" ");
    a["profile"] = json!("");
    let mut b = worker("b");
    b["executionSlotId"] = json!("dup");
    let mut c = worker("c");
    c["executionSlotId"] = json!("dup");
    let edges = vec![
        edge("a", "b", "success"),
        edge("b", "c", "success"),
        edge("c", "$end", "success"),
    ];
    let found = one(vec![a, b, c], edges);
    assert_eq!(
        found,
        [
            "a: slot-required",
            "a: profile-required",
            "b: slot-duplicate",
            "c: slot-duplicate"
        ]
    );
}

#[test]
fn ai_dynamic_nodes_skip_the_worker_rules() {
    let node = json!({"type":"ai-dynamic","id":"d"});
    assert!(check(vec![node], vec![edge("d", "$end", "success")]).is_empty());
}

#[test]
fn a_validated_output_needs_a_success_condition() {
    // The owner-reported message.
    let mut n = worker("spec-write");
    n["output"] = json!({"kind":"json","artifact":"spec","schema":{"x":1}});
    let found = check(vec![n], vec![edge("spec-write", "$end", "success")]);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0],
        "spec-write: success-expression-required: spec-write requires a success expression when output validation is enabled."
    );
}

#[test]
fn a_success_condition_alone_needs_an_artifact() {
    let mut n = worker("a");
    n["success_condition"] = json!({"path":"$.x","equals":1});
    let found = one(vec![n], vec![edge("a", "$end", "success")]);
    assert_eq!(found, ["a: output-artifact-required"]);
    for artifact in [json!("  "), json!(3)] {
        let mut n = validated("a", json!({"x":1}), json!({"path":"$.x"}));
        n["output"]["artifact"] = artifact;
        let found = one(vec![n], vec![edge("a", "$end", "success")]);
        assert_eq!(found, ["a: output-artifact-required"]);
    }
}

#[test]
fn manual_check_with_validation_is_refused() {
    let mut n = validated("a", json!({"x":1}), json!({"path":"$.x"}));
    n["manual_check"] = json!(true);
    let found = one(vec![n], vec![edge("a", "$end", "success")]);
    assert_eq!(found, ["a: manual-check-exclusive"]);
}

#[test]
fn success_expressions_must_parse_and_be_declared_in_the_schema() {
    let ok = [
        json!({"path":"$.a[1]","equals":1}),
        json!({"path":"a"}),
        json!({"path":"$a"}),
        json!({"path":"$.a[0].b"}),
        json!({"expression":"$.a >= 1"}),
        json!({"expression":"$.a == 1"}),
        json!({"expression":"$.a < 1"}),
    ];
    let schema = json!({"a":[{"b":"s"}, 1]});
    for condition in ok {
        let n = validated("n", schema.clone(), condition.clone());
        let found = check(vec![n], vec![edge("n", "$end", "success")]);
        assert!(found.is_empty(), "{condition}: {found:?}");
    }
    let bad = [
        json!({"path":"$"}),
        json!({"path":""}),
        json!({}),
        json!({"path":null}),
        json!({"path":3}),
        json!({"path":"$.a..b"}),
        json!({"path":".a"}),
        json!({"path":"$.a["}),
        json!({"path":"$.a[x]"}),
        json!({"path":"$.a[]"}),
        json!({"path":"$.a[0]b"}),
        json!({"expression":"$.a"}),
        json!({"expression":"a == 1"}),
        json!({"expression":5}),
        json!({"expression":"$ == 1"}),
        json!("text"),
        json!(true),
    ];
    for condition in bad {
        let n = validated("n", schema.clone(), condition.clone());
        let found = one(vec![n], vec![edge("n", "$end", "success")]);
        assert_eq!(found, ["n: success-expression-invalid"], "{condition}");
    }
}

#[test]
fn the_schema_must_contain_the_path() {
    let cases = [
        (json!({"a":1}), json!({"path":"$.b"}), true),
        (json!({"a":1}), json!({"path":"$.a.b"}), true),
        (json!({"a":[]}), json!({"path":"$.a[0]"}), true),
        (json!({"a":1}), json!({"path":"$.a[0]"}), true),
        (json!({"a":[null]}), json!({"path":"$.a[3]"}), false),
        (json!({"a":[{"k":1}]}), json!({"path":"$.a[2].k"}), false),
        (json!({"a":[{"k":1}]}), json!({"path":"$.a[0].z"}), true),
        (json!({"a":[{"k":1},null]}), json!({"path":"$.a[1]"}), false),
        (json!({"a":[null]}), json!({"path":"$.a[0]"}), false),
        (
            json!({"a":[1]}),
            json!({"path":"$.a[99999999999999999999999]"}),
            false,
        ),
    ];
    for (schema, condition, missing) in cases {
        let n = validated("n", schema.clone(), condition.clone());
        let found = one(vec![n], vec![edge("n", "$end", "success")]);
        let expected: Vec<&str> = if missing {
            vec!["n: schema-missing-path"]
        } else {
            vec![]
        };
        assert_eq!(found, expected, "{schema} {condition}");
    }
}

#[test]
fn a_legacy_json_schema_is_refused() {
    for schema in [
        json!({"type":"object","properties":{"a":{"type":"string"}}}),
        json!({"required":["a"]}),
    ] {
        let n = validated("n", schema, json!({"path":"$.a"}));
        let found = one(vec![n], vec![edge("n", "$end", "success")]);
        assert_eq!(found, ["n: legacy-schema"]);
    }
    // A schema that is not an object is not legacy, and a falsy one is skipped.
    for schema in [json!(["a"]), json!(null), json!(0), json!(false), json!("")] {
        let n = validated("n", schema, json!({"path":"$.a"}));
        let found = one(vec![n], vec![edge("n", "$end", "success")]);
        assert!(found.iter().all(|f| !f.contains("legacy")), "{found:?}");
    }
}

#[test]
fn edge_endpoint_rules() {
    let nodes = vec![worker("a")];
    let edges = vec![
        edge("", "$end", "success"),
        edge("ghost", "$end", "success"),
        edge("a", "", "success"),
        edge("a", "ghost", "success"),
        edge("a", "$end", "success"),
    ];
    let found = one(nodes, edges);
    assert_eq!(
        found,
        [
            "workflow: edge-source-required",
            "ghost: edge-source-missing",
            "a: edge-target-required",
            "a: duplicate-edge-outcome",
            "a: edge-target-missing",
        ]
    );
}

#[test]
fn edge_outcome_rules() {
    let mut check_node = worker("c");
    check_node["manual_check"] = json!(true);
    let nodes = vec![worker("a"), check_node];
    let edges = vec![
        edge("a", "c", "success"),
        edge("a", "$end", "maybe"),
        edge("a", "c", "failure"),
        edge("c", "$end", "success"),
        edge("c", "a", "failure"),
        edge("c", "$end", "failure"),
    ];
    let found = one(nodes, edges);
    assert_eq!(
        found,
        [
            "a: edge-outcome-invalid",
            "a: failure-edge-unsupported",
            "c: duplicate-edge-outcome",
        ]
    );
}

#[test]
fn duplicate_edge_outcome_reports_once_with_the_count() {
    let (nodes, mut edges) = linear();
    edges.push(edge("a", "$end", "success"));
    edges.push(edge("a", "b", "success"));
    let found = check(nodes, edges);
    assert_eq!(found.len(), 1);
    assert!(found[0].ends_with("a has 3 success edges; each outcome type can only have one edge."));
}

#[test]
fn new_round_edge_rules() {
    let nodes = vec![worker("a"), {
        let mut n = worker("b");
        n["manual_check"] = json!(true);
        n
    }];
    let mut ok = edge("b", "$new-round", "failure");
    ok["new_round_entry"] = json!("$entry");
    let mut named = edge("b", "$new-round", "failure");
    named["new_round_entry"] = json!("a");
    let mut missing = edge("b", "$new-round", "failure");
    missing["new_round_entry"] = json!("ghost");
    let base = vec![edge("a", "b", "success"), edge("b", "$end", "success")];
    let with = |extra: Vec<Value>| {
        let mut edges = base.clone();
        edges.extend(extra);
        one(nodes.clone(), edges)
    };
    assert!(with(vec![ok]).is_empty());
    assert!(with(vec![named]).is_empty());
    assert_eq!(with(vec![missing]), ["b: new-round-entry-missing"]);
    assert_eq!(
        with(vec![edge("b", "$new-round", "failure")]),
        ["b: new-round-entry-required"]
    );
    let success = with(vec![edge("a", "$new-round", "success")]);
    assert!(success.contains(&"a: success-new-round".to_string()));
}

#[test]
fn terminal_sources_and_continue_sessions_are_refused() {
    let (nodes, mut edges) = linear();
    edges.push(edge("$end", "a", "success"));
    edges.push(edge("$new-round", "a", "success"));
    let found = one(nodes.clone(), edges);
    assert!(found.contains(&"$end: terminal-edge-source".to_string()));
    assert!(found.contains(&"$new-round: terminal-edge-source".to_string()));
    let (_, mut edges) = linear();
    edges[1]["session"] = json!("continue");
    assert_eq!(one(nodes.clone(), edges), ["b: continue-terminal-target"]);
    let (_, mut edges) = linear();
    edges[0]["session"] = json!("continue");
    assert!(one(nodes, edges).is_empty());
}

#[test]
fn a_non_string_binding_cannot_hide_a_failure_edge_rule() {
    let mut n = worker("a");
    n["output"] = json!(0);
    n["manual_check"] = json!(0);
    let found = one(vec![n], vec![edge("a", "$end", "failure")]);
    assert_eq!(found, ["a: failure-edge-unsupported"]);
    let found = one(vec![json!({"id":"x"})], vec![edge("x", "$end", "failure")]);
    assert!(found.contains(&"x: failure-edge-unsupported".to_string()));
}

#[test]
fn a_path_may_start_with_an_index() {
    assert_eq!(
        parse_json_path("$[0].a").unwrap(),
        [Seg::Index(0), Seg::Key("a".into())]
    );
    assert_eq!(
        parse_json_path("$.a[1]").unwrap(),
        [Seg::Key("a".into()), Seg::Index(1)]
    );
    assert!(parse_json_path(".").is_err());
}
