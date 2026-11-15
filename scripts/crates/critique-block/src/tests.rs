use std::path::{Path, PathBuf};

use model_gateway::json::{self, Value};

use super::*;

fn kit() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../installer/gold-band")
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("critique-block-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for stage in ["spec", "tickets"] {
        let name = format!("critique-{stage}.json");
        fs::copy(kit().join("workflows").join(&name), dir.join(name)).unwrap();
    }
    dir
}

const TEMPLATE: &str = r#"{"workflow":{"id":"t","entry":"w","nodes":[
  {"type":"worker","id":"w","executionSlotId":"slot-w","provider":"vibe","model":"glm-5-3","profile":"pf-builtin-dev",
   "goal":"write","output":{"kind":"json","artifact":"spec-draft","schema":{"ok":true}},"success_condition":{"path":"$.ok","equals":true}},
  {"type":"worker","id":"after","executionSlotId":"slot-after","provider":"vibe","model":"glm-5-3","profile":"pf-builtin-dev","goal":"next"}],
 "edges":[{"from":"w","to":"after","on":"success"},{"from":"w","to":"w-fix","on":"failure"},{"from":"after","to":"$end","on":"success"}]},
 "modelBindings":{"bindings":[{"executionSlotId":"slot-w","agentId":"vibe","modelId":"glm-5-3"},{"executionSlotId":"slot-after","agentId":"vibe","modelId":"glm-5-3"}]}}"#;

fn recipe(extra: &str) -> String {
    format!(
        r#"{{"version":1,"blocks":[{{"template":"t.json","stage":"spec","writer":"w","next":"after","label":"the spec review","artifact":"the spec",
        "orchestrator":{{"provider":"vibe","model":"glm-5-3"}},"escalation":{{"provider":"claude","model":"opus"}},
        "escalation_check":{{"provider":"vibe","model":"glm-5-3"}}{extra}}}]}}"#
    )
}

fn generate(dir: &Path, recipes: &str) -> Result<Vec<(String, String)>, String> {
    generated(recipes, dir)
}

fn nodes_and_edges(text: &str) -> (Vec<Value>, Vec<(String, String, String)>) {
    let root = json::parse(text).unwrap();
    let workflow = root.as_object().unwrap().get("workflow").unwrap();
    let (Some(Value::Arr(nodes)), Some(Value::Arr(edges))) = (
        workflow.as_object().unwrap().get("nodes"),
        workflow.as_object().unwrap().get("edges"),
    ) else {
        panic!("a workflow has nodes and edges")
    };
    let edges = edges.iter().map(|e| (text_of(e, "from"), text_of(e, "to"), text_of(e, "on"))).collect();
    (nodes.clone(), edges)
}

fn text_of(value: &Value, key: &str) -> String {
    super::text(value, key).to_string()
}

fn reaches(edges: &[(String, String, String)], from: &str, to: &str) -> bool {
    let mut seen = vec![from.to_string()];
    let mut at = 0;
    while at < seen.len() {
        for (f, t, _) in edges {
            if *f == seen[at] && !seen.contains(t) {
                seen.push(t.clone());
            }
        }
        at += 1;
    }
    seen.iter().any(|n| n == to)
}

#[test]
fn the_block_has_two_rounds_and_one_way_into_the_next_node() {
    let dir = scratch("shape");
    fs::write(dir.join("t.json"), TEMPLATE).unwrap();
    let out = generate(&dir, &recipe("")).unwrap();
    let (nodes, edges) = nodes_and_edges(&out[0].1);
    let ids: Vec<String> = nodes.iter().map(|n| text_of(n, "id")).collect();
    for id in ["accept-spec-1", "accept-spec-2", "spec-revise-1", "spec-verdicts-2", "spec-escalation", "spec-escalation-check", "gate-critique-spec-record-2"] {
        assert!(ids.contains(&id.to_string()), "{id}");
    }
    // Only an accept node reaches the next node; the writer's own edge leads to the first round.
    let into: Vec<&str> = edges.iter().filter(|e| e.1 == "after").map(|e| e.0.as_str()).collect();
    assert_eq!(into, ["accept-spec-1", "accept-spec-2"]);
    assert!(edges.contains(&("w".into(), "gate-critique-spec-prepare".into(), "success".into())));
    // The writer's failure edge is left alone.
    assert!(edges.contains(&("w".into(), "w-fix".into(), "failure".into())));
    // A failed first round is revised, then critiqued again; a failed second round escalates to the end.
    assert!(edges.contains(&("accept-spec-1".into(), "spec-revise-1".into(), "failure".into())));
    assert!(edges.contains(&("spec-revise-1".into(), "gate-critique-spec-prepare-2".into(), "success".into())));
    assert!(edges.contains(&("accept-spec-2".into(), "spec-escalation".into(), "failure".into())));
    assert!(!reaches(&edges, "spec-escalation", "after"));
    assert!(reaches(&edges, "spec-escalation-check", "$end"));
    assert!(reaches(&edges, "spec-revise-1", "gate-critique-spec-record-2"));
    // Round 2 asks for fresh critics over the first round's run directory.
    let goal = nodes.iter().find(|n| text_of(n, "id") == "gate-critique-spec-prepare-2").map(|n| text_of(n, "goal")).unwrap();
    assert!(goal.contains("previous_run_dir"), "{goal}");
    // The revise node is the writer's own binding; the judge is the recipe's.
    let revise = nodes.iter().find(|n| text_of(n, "id") == "spec-revise-1").unwrap();
    assert_eq!((text_of(revise, "provider"), text_of(revise, "model")), ("vibe".into(), "glm-5-3".into()));
    let judge = nodes.iter().find(|n| text_of(n, "id") == "spec-escalation").unwrap();
    assert_eq!((text_of(judge, "provider"), text_of(judge, "model")), ("claude".into(), "opus".into()));
}

#[test]
fn generating_twice_changes_nothing_and_check_reads_the_same() {
    let dir = scratch("again");
    fs::write(dir.join("t.json"), TEMPLATE).unwrap();
    let recipes_file = dir.join("recipes.json");
    fs::write(&recipes_file, recipe("")).unwrap();
    let go = |verb: &str| {
        run([verb, "--recipes", recipes_file.to_str().unwrap(), "--dir", dir.to_str().unwrap()].map(OsString::from))
    };
    assert_eq!(go("check"), 1);
    assert_eq!(go("generate"), 0);
    let first = fs::read_to_string(dir.join("t.json")).unwrap();
    assert_eq!(go("check"), 0);
    assert_eq!(go("generate"), 0);
    assert_eq!(fs::read_to_string(dir.join("t.json")).unwrap(), first);
}

#[test]
fn the_precheck_the_grant_checks_and_the_overrides_are_generated_when_the_recipe_asks() {
    let dir = scratch("extras");
    let mut template = TEMPLATE.replace(r#"{"type":"worker","id":"after""#, r#"{"type":"worker","id":"grant-check-w","executionSlotId":"slot-grant-check-w","provider":"grant-w","model":"stub-model","profile":"pf-builtin-dev","goal":"Run the grant-check wrapper for the Codex node w (--node w)."},
      {"type":"worker","id":"refused","executionSlotId":"slot-refused","provider":"vibe","model":"glm-5-3","profile":"pf-builtin-dev","goal":"refused","manual_check":true},
      {"type":"worker","id":"after""#);
    template = template.replace(r#""provider":"vibe","model":"glm-5-3","profile":"pf-builtin-dev",
   "goal":"write""#, r#""provider":"codex","model":"gpt-5.5","profile":"pf-builtin-dev",
   "goal":"write""#);
    fs::write(dir.join("t.json"), template).unwrap();
    let extra = r#","needs_human":{"provider":"claude-work","model":"sonnet"},"precheck":{"command":"critique check-tickets --strict","purpose":"edges"},
        "grant":{"refusal":"refused","prototype":"grant-check-w"},"routing_note":"Routed to the work roster.""#;
    let recipes_text = recipe(extra).replace(r#""orchestrator":{"provider":"vibe","model":"glm-5-3"}"#, r#""orchestrator":{"provider":"codex","model":"gpt-5.5"}"#);
    let out = generate(&dir, &recipes_text).unwrap();
    let (nodes, edges) = nodes_and_edges(&out[0].1);
    let get = |id: &str| nodes.iter().find(|n| text_of(n, "id") == id).unwrap_or_else(|| panic!("{id}"));
    // The pre-check runs before each round's critics, as the gate shim and its copier.
    assert!(text_of(get("gate-spec-precheck"), "goal").contains("`critique check-tickets --strict`"));
    assert!(edges.contains(&("copy-spec-precheck".into(), "gate-critique-spec-prepare".into(), "success".into())));
    assert!(edges.contains(&("spec-revise-1".into(), "gate-spec-precheck-2".into(), "success".into())));
    assert!(text_of(get("copy-spec-precheck-2"), "goal").contains("verify copier"));
    // Every Codex node (the orchestrators and the reviser) sits behind a grant check that falls to the refusal.
    for id in ["spec-verdicts", "spec-verdicts-2", "spec-revise-1"] {
        let grant = format!("grant-check-{id}");
        assert!(edges.contains(&(grant.clone(), id.into(), "success".into())), "{id}");
        assert!(edges.contains(&(grant.clone(), "refused".into(), "failure".into())), "{id}");
        assert_eq!(edges.iter().filter(|e| e.1 == id).map(|e| e.0.clone()).collect::<Vec<_>>(), [grant.clone()], "{id}");
        assert!(text_of(get(&grant), "goal").contains(&format!("(--node {id})")), "{id}");
        assert_eq!(text_of(get(&grant), "provider"), format!("grant-{id}"));
    }
    assert!(!ids_of(&nodes).contains(&"grant-check-spec-escalation".to_string()));
    // The needs-human override and the routing note.
    assert_eq!(text_of(get("spec-needs-human"), "provider"), "claude-work");
    assert!(text_of(get("gate-critique-spec-fan-out"), "goal").ends_with("Routed to the work roster."));
    assert!(!text_of(get("copy-critique-spec-fan-out"), "goal").contains("work roster"));
    // The bindings follow the nodes.
    let root = json::parse(&out[0].1).unwrap();
    let Some(Value::Obj(wrapper)) = root.as_object().unwrap().get("modelBindings") else { panic!() };
    let Some(Value::Arr(bindings)) = wrapper.get("bindings") else { panic!() };
    // The template's own grant-check and refusal nodes have no binding in this fixture.
    assert_eq!(bindings.len(), nodes.len() - 2);
}

fn ids_of(nodes: &[Value]) -> Vec<String> {
    nodes.iter().map(|n| text_of(n, "id")).collect()
}

#[test]
fn a_template_without_bindings_is_generated_without_them() {
    let dir = scratch("bare");
    let bare = TEMPLATE.split(",\n \"modelBindings\"").next().unwrap().to_string() + "}";
    fs::write(dir.join("t.json"), bare).unwrap();
    let out = generate(&dir, &recipe("")).unwrap();
    assert!(!out[0].1.contains("modelBindings"));
}

#[test]
fn two_blocks_of_one_template_share_one_output() {
    let dir = scratch("two");
    fs::write(dir.join("t.json"), TEMPLATE).unwrap();
    let text = recipe("");
    let second = text.replace(r#""stage":"spec""#, r#""stage":"tickets""#).replace(r#""writer":"w""#, r#""writer":"after""#).replace(r#""next":"after""#, r#""next":"$end""#);
    let blocks = format!("{},{}", &text[..text.len() - 2], &second[second.find("{\"template\"").unwrap()..]);
    let out = generate(&dir, &blocks).unwrap();
    assert_eq!(out.len(), 1);
    let (nodes, _) = nodes_and_edges(&out[0].1);
    let ids = ids_of(&nodes);
    assert!(ids.contains(&"accept-spec-2".to_string()) && ids.contains(&"accept-tickets-2".to_string()));
}

#[test]
fn a_recipe_that_cannot_be_read_is_an_error_that_says_why() {
    let dir = scratch("errors");
    fs::write(dir.join("t.json"), TEMPLATE).unwrap();
    let err = |text: &str| generate(&dir, text).unwrap_err();
    assert!(!err("{").is_empty());
    assert_eq!(err("{}"), "the recipes file needs a blocks list");
    assert_eq!(err(r#"{"blocks":[{}]}"#), "blocks[0]: template is required");
    assert_eq!(err(&recipe("").replace(r#""stage":"spec""#, r#""stage":5"#)), "blocks[0]: stage must be a string");
    assert_eq!(err(&recipe("").replace(r#""model":"opus""#, r#""model":3"#)), "blocks[0].escalation: model must be a string");
    assert_eq!(err(&recipe(r#","precheck":{"command":"x"}"#)), "blocks[0]: purpose is required");
    assert_eq!(err(&recipe(r#","routing_note":4"#)), "blocks[0]: routing_note must be a string");
    assert!(err(&recipe("").replace(r#""writer":"w""#, r#""writer":"nobody""#)).contains("has no node nobody"));
    assert!(err(&recipe("").replace(r#""stage":"spec""#, r#""stage":"ticket-review""#)).contains("critique-ticket-review.json"));
    assert!(err(&recipe(r#","grant":{"refusal":"r","prototype":"nobody"}"#)).contains("has no node nobody"));
    assert!(err(&recipe("").replace("t.json", "missing.json")).contains("missing.json: cannot read"));
    fs::write(dir.join("broken.json"), "[1]").unwrap();
    assert_eq!(err(&recipe("").replace("t.json", "broken.json")), "the template has no workflow.nodes list");
    fs::write(dir.join("noedges.json"), r#"{"workflow":{"nodes":[{"id":"w"}]}}"#).unwrap();
    assert_eq!(err(&recipe("").replace("t.json", "noedges.json")), "the template has no workflow.edges list");
    fs::write(dir.join("critique-spec.json"), "[]").unwrap();
    assert_eq!(err(&recipe("")), "the template has no workflow.nodes list");
    fs::write(dir.join("critique-spec.json"), "nope").unwrap();
    assert!(err(&recipe("")).contains("not JSON"));
}

#[test]
fn the_command_line_names_a_verb_and_both_files() {
    let go = |args: &[&str]| run(args.iter().map(OsString::from));
    assert_eq!(go(&[]), 2);
    assert_eq!(go(&["frobnicate"]), 2);
    assert_eq!(go(&["check", "--recipes"]), 2);
    assert_eq!(go(&["check", "--dir", "x"]), 2);
    assert_eq!(go(&["check", "--other", "x"]), 2);
    assert_eq!(go(&["check", "--recipes", "/nonexistent/recipes.json", "--dir", "/tmp"]), 2);
    let dir = scratch("cli");
    fs::write(dir.join("recipes.json"), recipe("")).unwrap();
    let (recipes_file, dir_arg) = (dir.join("recipes.json"), dir.to_str().unwrap().to_string());
    // The template the recipe names is missing: an input error, not a stale template.
    assert_eq!(go(&["check", "--recipes", recipes_file.to_str().unwrap(), "--dir", &dir_arg]), 2);
    // A directory that cannot hold the output is an error.
    fs::write(dir.join("t.json"), TEMPLATE).unwrap();
    let mut readonly = fs::metadata(dir.join("t.json")).unwrap().permissions();
    readonly.set_readonly(true);
    fs::set_permissions(dir.join("t.json"), readonly).unwrap();
    let denied = go(&["generate", "--recipes", recipes_file.to_str().unwrap(), "--dir", &dir_arg]);
    // A root user can write a read-only file, so only the exit code of a refused write is checked.
    assert!(denied == 2 || denied == 0);
}

#[test]
fn the_shipped_templates_hold_what_the_recipes_generate() {
    let recipes_file = kit().join("critique-blocks.json");
    let dir = kit().join("workflows");
    let go = |verb: &str| run([verb, "--recipes", recipes_file.to_str().unwrap(), "--dir", dir.to_str().unwrap()].map(OsString::from));
    assert_eq!(go("check"), 0);
}

#[test]
fn the_json_the_generator_writes_is_the_json_the_templates_are_kept_in() {
    for name in ["to-spec", "to-tickets", "workflow-c", "workflow-c-work", "workflow-b", "workflow-b-work"] {
        let text = fs::read_to_string(kit().join("workflows").join(format!("{name}.json"))).unwrap();
        assert_eq!(format!("{}\n", json::pretty(&json::parse(&text).unwrap())), text, "{name}");
    }
}

#[test]
fn every_shipped_block_meets_the_rules_the_issue_names() {
    for name in ["to-spec", "to-tickets", "workflow-c", "workflow-c-work", "workflow-b", "workflow-b-work"] {
        let text = fs::read_to_string(kit().join("workflows").join(format!("{name}.json"))).unwrap();
        let (nodes, edges) = nodes_and_edges(&text);
        let ids = ids_of(&nodes);
        for stage in ["spec", "tickets", "ticket-review"].into_iter().filter(|s| ids.contains(&format!("accept-{s}-1"))) {
            // Every revise is followed by a critic before any exit.
            let revise = format!("{stage}-revise-1");
            assert!(reaches(&edges, &revise, &format!("gate-critique-{stage}-record-2")), "{name} {stage}");
            // The edges out of the accept nodes are the only ones that leave the block towards the next stage.
            for round in 1..=2 {
                let accept = format!("accept-{stage}-{round}");
                assert!(edges.iter().any(|e| e.0 == accept && e.2 == "success"), "{name} {accept}");
            }
            let escalation = format!("{stage}-escalation-check");
            assert_eq!(edges.iter().filter(|e| e.0 == escalation).count(), 1, "{name}");
            assert!(edges.contains(&(escalation, "$end".to_string(), "success".to_string())));
        }
    }
}

#[test]
fn the_helpers_leave_a_value_that_is_not_an_object_alone() {
    let mut scalar = Value::Null;
    set(&mut scalar, "key", Value::str("x"));
    assert_eq!(scalar, Value::Null);
    assert!(nested(&mut scalar, "a", "b").is_none());
    let mut shallow = json::parse(r#"{"a":1}"#).unwrap();
    assert!(nested(&mut shallow, "a", "b").is_none());
    assert!(Template { root: Value::Null }.list(&["workflow"]).is_none());
    let mut template = Template { root: json::parse(r#"{"workflow":{"nodes":{}}}"#).unwrap() };
    assert!(template.list(&["workflow", "nodes"]).is_none());
    assert!(template.list(&["workflow", "nodes", "deeper"]).is_none());
}
