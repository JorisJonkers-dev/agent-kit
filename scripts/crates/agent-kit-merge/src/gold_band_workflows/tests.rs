//! `converge gold-band-workflows` against a temporary workflows directory, registry, keep file and template store,
//! with a stub process table and a fixed stamp; one test installs the kit's own templates.

use crate::cli::run_seeing;
use crate::converge::{Ran, Runner};
use crate::gold_band::{Proc, Processes};
use crate::json::{self, Value};
use crate::Failure;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

static NEXT: AtomicUsize = AtomicUsize::new(0);

const REGISTRY: &str = "gold_band_instances:\n  gold-band:\n    agents: [claude-work, codex]\n";

fn template(id: &str, agent: &str, goal: &str) -> String {
    format!(
        r#"{{"workflow": {{"version": "0.1", "id": "{id}", "entry": "one", "nodes": [{{"type": "worker", "id": "one", "executionSlotId": "slot-{id}", "provider": null, "profile": "pf-builtin-dev", "goal": "{goal}"}}], "edges": [{{"from": "one", "to": "$end", "on": "success"}}]}}, "modelBindings": {{"bindings": [{{"executionSlotId": "slot-{id}", "agentId": "{agent}", "modelId": "m"}}]}}}}"#
    )
}

fn template_value(id: &str, agent: &str, goal: &str) -> Value {
    json::parse(&template(id, agent, goal)).unwrap()
}

const BUILT_IN: &str = r#"{"id": "default", "name": "Default full workflow", "isBuiltIn": true, "workflow": {"id": "default", "nodes": []}, "modelBindings": {"bindings": []}, "createdAt": "1794686101Z", "updatedAt": "1794686101Z"}"#;

struct Table(Mutex<Vec<Vec<Proc>>>);

impl Processes for Table {
    fn list(&self) -> Vec<Proc> {
        let mut lists = self.0.lock().unwrap();
        if lists.len() > 1 {
            lists.remove(0)
        } else {
            lists.first().cloned().unwrap_or_default()
        }
    }
}

fn quiet() -> Table {
    Table(Mutex::new(Vec::new()))
}

struct NoRunner;

impl Runner for NoRunner {
    fn run(&mut self, program: &str, _: &[String], _: &[(String, Option<String>)]) -> Result<Ran, Failure> {
        panic!("gold-band-workflows ran {program}")
    }
}

struct Machine {
    root: PathBuf,
}

impl Machine {
    fn new(sources: &[(&str, String)], keep: &str) -> Machine {
        let root = std::env::temp_dir().join(format!("agent-kit-workflows-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir_all(root.join("workflows")).unwrap();
        for (id, body) in sources {
            fs::write(root.join("workflows").join(format!("{id}.json")), body).unwrap();
        }
        fs::write(root.join("keep.yaml"), keep).unwrap();
        fs::write(root.join("registry.yaml"), REGISTRY).unwrap();
        Machine { root }
    }

    fn plain() -> Machine {
        Machine::new(&[("alpha", template("alpha", "claude", "do it")), ("beta", template("beta", "pi", "do it"))], "keep: []\n")
    }

    fn store(&self) -> PathBuf {
        self.root.join("home/.gold-band/context/workflows.json")
    }

    fn seed(&self, store: &str) {
        fs::create_dir_all(self.store().parent().unwrap()).unwrap();
        fs::write(self.store(), store).unwrap();
    }

    fn read(&self) -> Value {
        json::parse(&fs::read_to_string(self.store()).unwrap()).unwrap()
    }

    fn ids(&self) -> Vec<String> {
        match self.read().as_object().unwrap().get("templates") {
            Some(Value::Arr(templates)) => templates
                .iter()
                .map(|t| match t.as_object().unwrap().get("id") {
                    Some(Value::Str(id)) => id.clone(),
                    _ => "?".to_string(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn args(&self, extra: &[&str]) -> Vec<String> {
        let path = |name: &str| self.root.join(name).display().to_string();
        let store = self.store().display().to_string();
        let mut args: Vec<String> = ["converge", "gold-band-workflows", "--keep", &path("keep.yaml"), "--store", &store, "--workflows", &path("workflows"), "--registry", &path("registry.yaml")]
            .iter()
            .map(|text| text.to_string())
            .collect();
        args.extend(extra.iter().map(|text| text.to_string()));
        args
    }

    fn run(&self, extra: &[&str]) -> (i32, String) {
        self.run_with(&self.args(extra), &quiet(), &[("AGENT_KIT_GOLD_BAND_RUNNING", "0")])
    }

    fn run_with(&self, args: &[String], table: &dyn Processes, vars: &[(&str, &str)]) -> (i32, String) {
        let home = self.root.join("home").display().to_string();
        let mut vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        vars.push(("AGENT_KIT_NOW".to_string(), "100Z".to_string()));
        let env = move |name: &str| if name == "HOME" { Some(home.clone()) } else { vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()) };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let status = {
            let mut stdin: &[u8] = &[];
            let mut io = crate::Io { stdin: &mut stdin, out: &mut out, err: &mut err };
            run_seeing(args, &mut io, &env, &mut NoRunner, table)
        };
        (status, String::from_utf8(out).unwrap() + &String::from_utf8(err).unwrap())
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn template_of(store: &Value, id: &str) -> json::Object {
    match store.as_object().unwrap().get("templates") {
        Some(Value::Arr(templates)) => templates.iter().filter_map(Value::as_object).find(|t| t.get("id") == Some(&Value::Str(id.to_string()))).cloned().unwrap(),
        _ => panic!("no templates"),
    }
}

#[test]
fn a_missing_store_is_created_owner_only_and_a_second_run_writes_nothing() {
    let machine = Machine::plain();
    assert_eq!(machine.run(&[]), (0, "added alpha\nadded beta\n".to_string()));
    use std::os::unix::fs::PermissionsExt;
    for path in [machine.root.join("home/.gold-band"), machine.root.join("home/.gold-band/context")] {
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700, "{}", path.display());
    }
    assert_eq!(fs::metadata(machine.store()).unwrap().permissions().mode() & 0o777, 0o600);
    let alpha = template_of(&machine.read(), "alpha");
    assert_eq!(alpha.keys(), ["id", "name", "isBuiltIn", "workflow", "modelBindings", "createdAt", "updatedAt"]);
    assert_eq!(alpha.get("createdAt"), Some(&Value::Str("100Z".to_string())));
    let text = fs::read_to_string(machine.store()).unwrap();
    assert!(text.starts_with("{\n  \"version\": \"0.1\",\n  \"lastUsedTemplateId\": null,") && text.ends_with("}\n"), "{text}");
    assert_eq!(machine.run(&[]), (0, "workflows unchanged\n".to_string()));
    assert!(!machine.store().with_file_name("workflows.json.agent-kit-tmp").exists());
}

#[test]
fn drift_is_replaced_in_place_and_the_rest_of_the_store_is_left_alone() {
    let machine = Machine::new(&[("alpha", template("alpha", "claude", "new goal")), ("beta", template("beta", "pi", "do it"))], "keep: []\n");
    // alpha drifted (a different goal); beta differs only by a null field the app drops and sits in a slot named
    // otherwise; gamma is the owner's own.
    let old_alpha = template_value("alpha", "claude", "old goal");
    let held_alpha = format!(
        r#"{{"id": "alpha", "name": "My alpha", "isBuiltIn": false, "workflow": {}, "modelBindings": {{"bindings": []}}, "createdAt": "1Z", "updatedAt": "1Z", "extra": "kept"}}"#,
        json::dumps(old_alpha.as_object().unwrap().get("workflow").unwrap())
    );
    let beta = template_value("beta", "pi", "do it");
    let beta_text = json::dumps(beta.as_object().unwrap().get("workflow").unwrap()).replace(r#""provider": null,"#, "");
    let held_beta = format!(
        r#"{{"id": "renamed-slot", "name": "Beta", "isBuiltIn": false, "workflow": {beta_text}, "modelBindings": {}, "createdAt": "2Z", "updatedAt": "2Z"}}"#,
        json::dumps(beta.as_object().unwrap().get("modelBindings").unwrap())
    );
    let gamma = r#"{"id": "gamma", "name": "Mine", "isBuiltIn": false, "workflow": {"id": "gamma"}, "modelBindings": {}, "createdAt": "3Z", "updatedAt": "3Z"}"#;
    machine.seed(&format!(r#"{{"version": "0.1", "lastUsedTemplateId": "default", "lastCreatedWorkflow": null, "templates": [{BUILT_IN}, {held_alpha}, {held_beta}, {gamma}], "other": {{"keep": true}}}}"#));
    let before = fs::read_to_string(machine.store()).unwrap();
    assert_eq!(machine.run(&[]), (0, "changed alpha\n".to_string()));
    assert_eq!(machine.ids(), ["default", "alpha", "renamed-slot", "gamma"]);
    let alpha = template_of(&machine.read(), "alpha");
    let field = |key: &str| alpha.get(key).cloned();
    assert_eq!((field("name"), field("createdAt"), field("updatedAt"), field("extra")), (Some(Value::Str("My alpha".into())), Some(Value::Str("1Z".into())), Some(Value::Str("100Z".into())), Some(Value::Str("kept".into()))));
    assert_eq!(machine.read().as_object().unwrap().get("other").cloned(), json::parse(r#"{"keep": true}"#).ok());
    assert_eq!(fs::read_to_string(machine.store().with_file_name("workflows.json.bak")).unwrap(), before);
}

#[test]
fn a_binding_change_alone_is_drift_and_check_writes_nothing() {
    let machine = Machine::plain();
    assert_eq!(machine.run(&[]).0, 0);
    fs::write(machine.root.join("workflows/alpha.json"), template("alpha", "pi-vibe", "do it")).unwrap();
    let before = fs::read_to_string(machine.store()).unwrap();
    assert_eq!(machine.run(&["--check"]), (0, "would change alpha\n".to_string()));
    assert_eq!(fs::read_to_string(machine.store()).unwrap(), before);
    assert_eq!(machine.run(&[]), (0, "changed alpha\n".to_string()));
    let fresh = Machine::plain();
    assert_eq!(fresh.run(&["--check"]), (0, "would add alpha\nwould add beta\n".to_string()));
    assert!(!fresh.store().exists());
}

#[test]
fn the_keep_file_and_a_work_agent_binding_leave_a_template_out() {
    let sources = [
        ("alpha", template("alpha", "claude", "do it")),
        ("kept-one", template("kept-one", "claude", "do it")),
        ("kept-two", template("kept-two", "claude", "do it")),
        ("codex-bound-work", template("codex-bound-work", "codex", "do it")),
        ("worker-bound", template("worker-bound", "claude", "do it").replace(r#""provider": null"#, r#""provider": "claude-work""#)),
    ];
    let machine = Machine::new(&sources, "keep: [kept-one, kept-two.json]\n");
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 0, "{out}");
    assert_eq!(out, "added alpha\nskipped codex-bound-work (bound to the work instance's codex)\nkept kept-one (keep file)\nkept kept-two (keep file)\nskipped worker-bound (bound to the work instance's claude-work)\n");
    assert_eq!(machine.ids(), ["alpha"]);
    let only_kept = Machine::new(&[("kept-one", template("kept-one", "claude", "x"))], "keep: [kept-one]\n");
    assert_eq!(only_kept.run(&[]), (0, "kept kept-one (keep file)\nworkflows unchanged\n".to_string()));
    let bad_keep = Machine::plain();
    fs::write(bad_keep.root.join("keep.yaml"), "keep: 5\n").unwrap();
    assert_eq!(bad_keep.run(&[]).0, 1);
    assert!(!bad_keep.store().exists());
}

#[test]
fn a_running_app_stops_the_run_before_the_write() {
    let machine = Machine::plain();
    let (status, out) = machine.run_with(&machine.args(&[]), &quiet(), &[("AGENT_KIT_GOLD_BAND_RUNNING", "1")]);
    assert_eq!(status, 4);
    assert_eq!(out, "gold-band-workflows: Gold Band instance gold-band-personal (set by AGENT_KIT_GOLD_BAND_RUNNING) is running; quit it and re-run setup (nothing was written)\n");
    let later = Table(Mutex::new(vec![vec![], vec![Proc { pid: 3, exe: "gold-band-desktop".into(), data: None }]]));
    let (status, out) = machine.run_with(&machine.args(&[]), &later, &[]);
    assert_eq!(status, 4);
    assert!(out.ends_with("gold-band-workflows: written: nothing\ngold-band-workflows: not yet written: workflows.json\n"), "{out}");
    assert!(!machine.store().exists());
}

#[test]
fn a_store_that_is_not_a_store_or_a_builtin_under_a_shipped_id_is_refused() {
    for store in ["[]", r#"{"templates": 5}"#, r#"{"templates": [1]}"#, "not json"] {
        let machine = Machine::plain();
        machine.seed(store);
        let (status, out) = machine.run(&[]);
        assert_eq!(status, 3, "{store}");
        assert!(out.starts_with("gold-band-workflows: cannot converge: "), "{out}");
        assert_eq!(fs::read_to_string(machine.store()).unwrap(), store);
    }
    let machine = Machine::plain();
    machine.seed(&format!(r#"{{"templates": [{}]}}"#, BUILT_IN.replace(r#""id": "default", "name""#, r#""id": "alpha", "name""#)));
    assert_eq!(machine.run(&[]), (1, "gold-band-workflows: alpha: the existing template is a built-in; nothing was written\n".to_string()));
}

#[test]
fn a_bad_source_registry_or_directory_refuses_the_whole_run() {
    let cases: [(&str, &str, &str); 9] = [
        ("broken", "{", "broken.json cannot be read: "),
        ("halves", r#"{"workflow": {"id": "halves"}}"#, "must hold a `workflow` object and a `modelBindings` object"),
        ("named", r#"{"workflow": {"id": "other"}, "modelBindings": {}}"#, "the workflow id must be the file's own name ('named'), got 'other'"),
        ("unnamed", r#"{"workflow": {"id": 7}, "modelBindings": {}}"#, "got 7"),
        ("noid", r#"{"workflow": {}, "modelBindings": {}}"#, "got None"),
        ("flag", r#"{"workflow": {"id": true}, "modelBindings": {}}"#, "got True"),
        ("listy", r#"{"workflow": {"id": ["x"]}, "modelBindings": {}}"#, "got ["),
        ("default", r#"{"workflow": {"id": "default"}, "modelBindings": {}}"#, "'default' is the id of a built-in template of the app"),
        ("listed", "[]", "must hold a `workflow` object"),
    ];
    for (id, body, message) in cases {
        let machine = Machine::new(&[("alpha", template("alpha", "claude", "x")), (id, body.to_string())], "keep: []\n");
        let (status, out) = machine.run(&[]);
        assert_eq!(status, 3, "{id}");
        assert!(out.contains(message), "{out}");
        assert!(!machine.store().exists());
    }
    let machine = Machine::plain();
    fs::write(machine.root.join("workflows/undecodable.json"), [0xff, 0xfe]).unwrap();
    assert!(machine.run(&[]).1.contains("undecodable.json cannot be read"));
    let machine = Machine::plain();
    fs::remove_dir_all(machine.root.join("workflows")).unwrap();
    assert!(machine.run(&[]).1.contains("workflows is not a directory"));
    let machine = Machine::plain();
    for registry in ["gold_band_instances: {gold-band: {agents: 5}}", "[", "other: 1"] {
        fs::write(machine.root.join("registry.yaml"), registry).unwrap();
        let (status, out) = machine.run(&[]);
        assert_eq!(status, 3);
        assert!(out.contains("registry.yaml"), "{out}");
    }
    fs::remove_file(machine.root.join("registry.yaml")).unwrap();
    assert!(machine.run(&[]).1.contains("registry.yaml cannot be read"));
}

#[test]
fn the_shape_rules_refuse_unless_the_template_is_a_known_failure() {
    let broken = r#"{"workflow": {"version": "0.1", "id": "broken", "nodes": [], "edges": []}, "modelBindings": {"bindings": []}}"#;
    let machine = Machine::new(&[("alpha", template("alpha", "claude", "x")), ("broken", broken.to_string())], "keep: []\n");
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 3);
    assert!(out.contains("a template fails Gold Band's workflow shape rules:\n  broken.json: ") && out.contains("known-shape-failures.txt"), "{out}");
    fs::write(machine.root.join("workflows/known-shape-failures.txt"), "# tolerated\n\nbroken.json\n").unwrap();
    let (_, out) = machine.run(&[]);
    assert!(out.contains("gold-band-workflows: warning: broken fails the editor's shape rules ("), "{out}");
}

#[test]
fn the_critique_rules_refuse_unless_the_template_is_a_known_failure() {
    // A Codex binding on the personal instance breaks the base lint the critique gate runs.
    let codex = template("banned", "codex-acp", "x");
    let machine = Machine::new(&[("alpha", template("alpha", "claude", "x")), ("banned", codex)], "keep: []\n");
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 3, "{out}");
    assert!(out.contains("a template breaks the critique rules (critic-required, critic-family):\n  workflow-lint: ") && out.contains("banned.json: "), "{out}");
    fs::write(machine.root.join("workflows/known-critique-failures.txt"), "# known\nbanned.json #999\n").unwrap();
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("gold-band-workflows: warning: banned breaks the critique rules ("), "{out}");
    // A template the lint cannot read as a workflow: the rules were not checked, and the run goes on.
    let odd = r#"{"workflow": {"version": "0.1", "id": "odd", "entry": "one", "nodes": 5, "edges": []}, "modelBindings": {}}"#;
    let machine = Machine::new(&[("odd", odd.to_string())], "keep: []\n");
    fs::write(machine.root.join("workflows/known-shape-failures.txt"), "odd\n").unwrap();
    let (_, out) = machine.run(&[]);
    assert!(out.contains("gold-band-workflows: warning: the critique rules (#793) were not checked: workflow-lint: "), "{out}");
}

#[test]
fn a_known_failures_file_that_cannot_be_read_refuses_the_run() {
    for file in ["known-shape-failures.txt", "known-critique-failures.txt"] {
        let machine = Machine::plain();
        fs::create_dir_all(machine.root.join("workflows").join(file)).unwrap();
        let machine2 = Machine::plain();
        fs::write(machine2.root.join("workflows").join(file), [0xff, 0xfe]).unwrap();
        let (status, out) = machine2.run(&[]);
        assert_eq!(status, 3, "{out}");
        assert!(out.contains(&format!("{file} cannot be read")), "{out}");
        // A directory under the name is no file: it is not read.
        assert_eq!(machine.run(&[]).0, 0);
    }
}

#[test]
fn a_store_that_cannot_be_replaced_leaves_the_old_one_and_no_staged_file() {
    let machine = Machine::plain();
    machine.seed(r#"{"templates": []}"#);
    use std::os::unix::fs::PermissionsExt;
    let context = machine.store().parent().unwrap().to_path_buf();
    fs::set_permissions(&context, fs::Permissions::from_mode(0o500)).unwrap();
    let (status, out) = machine.run(&[]);
    fs::set_permissions(&context, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(status, 3, "{out}");
    assert!(out.contains("; the old store is in place"), "{out}");
    assert_eq!(fs::read_to_string(machine.store()).unwrap(), r#"{"templates": []}"#);
    let names: Vec<String> = fs::read_dir(&context).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["workflows.json"]);
    // A parent that cannot be made fails before anything is staged.
    let machine = Machine::plain();
    fs::write(machine.root.join("home"), "a file").unwrap();
    assert_eq!(machine.run(&[]).0, 3);
}

#[test]
fn the_defaults_are_the_personal_instances_store_and_the_kits_own_templates() {
    let machine = Machine::plain();
    let kit = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let registry = kit.join("registry/estate-tooling.yaml").display().to_string();
    let keep = machine.root.join("keep.yaml").display().to_string();
    let args: Vec<String> = ["converge", "gold-band-workflows", "--keep", &keep, "--registry", &registry].iter().map(|t| t.to_string()).collect();
    let band = machine.root.join("band").display().to_string();
    let vars = [("GOLD_BAND_HOME", band.as_str()), ("AGENT_KIT_GOLD_BAND_RUNNING", "0")];
    let (status, out) = machine.run_with(&args, &quiet(), &vars);
    assert_eq!(status, 0, "{out}");
    assert!(out.contains("skipped workflow-b-work (bound to the work instance's"), "{out}");
    assert!(machine.root.join("band/.gold-band/context/workflows.json").is_file());
    assert!(machine.run_with(&args, &quiet(), &vars).1.ends_with("workflows unchanged\n"));
    let (status, out) = machine.run_with(&["converge".to_string(), "gold-band-workflows".to_string()], &quiet(), &vars);
    assert_eq!((status, out.as_str()), (2, "agent-kit-merge: the following arguments are required: --keep, --registry\n"));
    assert!(super::now_stamp().ends_with('Z') && super::now_stamp().len() > 5);
}

#[test]
fn unicode_is_written_as_it_is() {
    assert_eq!(json::dumps_unicode(&Value::Str("é — \u{1}".into())), "\"é — \\u0001\"");
    assert_eq!(json::dumps(&Value::Str("é".into())), "\"\\u00e9\"");
}

#[test]
fn a_literal_only_python_reads_a_store_path_that_is_a_directory_and_a_blocked_backup_are_refused() {
    let nan = r#"{"workflow": {"id": "nan", "x": 1e400}, "modelBindings": {}}"#;
    let machine = Machine::new(&[("nan", nan.to_string())], "keep: []\n");
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 3);
    assert!(out.contains("nan.json cannot be read: "), "{out}");
    let machine = Machine::plain();
    fs::create_dir_all(machine.store()).unwrap();
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 3);
    assert!(out.contains("workflows.json cannot be read: "), "{out}");
    let machine = Machine::plain();
    machine.seed(r#"{"templates": []}"#);
    let blocked = machine.store().with_file_name("workflows.json.bak.agent-kit-tmp");
    fs::create_dir_all(blocked.join("inside")).unwrap();
    let (status, out) = machine.run(&[]);
    assert_eq!(status, 3, "{out}");
    assert!(out.contains("; the old store is in place"), "{out}");
    assert_eq!(fs::read_to_string(machine.store()).unwrap(), r#"{"templates": []}"#);
}

#[test]
fn the_kits_known_failures_name_exactly_the_failing_templates_each_with_a_ticket() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../installer/gold-band/workflows");
    let sources = match super::sources(&directory) {
        Ok(sources) => sources,
        Err(super::Cannot(message)) => panic!("{message}"),
    };
    let failing: std::collections::BTreeSet<String> = sources.iter().filter(|s| !workflow_lint::shape_of_definition(s.file.get("workflow").unwrap()).is_empty()).map(|s| s.id.clone()).collect();
    let known = match super::known(&directory, super::SHAPE_KNOWN, |line| super::without_json(line.trim())) {
        Ok(known) => known,
        Err(super::Cannot(message)) => panic!("{message}"),
    };
    assert_eq!(failing, known);
    let critique = fs::read_to_string(directory.join(super::CRITIQUE_KNOWN)).unwrap();
    let lines: Vec<&str> = critique.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')).collect();
    assert!(!lines.is_empty());
    for line in lines {
        let file = line.split_whitespace().next().unwrap();
        assert!(directory.join(file).is_file(), "{line}");
        assert!(line.split_whitespace().any(|word| word.strip_prefix('#').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))), "{line}");
    }
}

#[test]
fn a_store_the_app_resaved_with_its_builtins_first_and_nulls_dropped_is_unchanged() {
    let machine = Machine::new(&[("critique", template("critique", "claude", "x"))], "keep: []\n");
    machine.seed(&format!(r#"{{"version": "0.1", "lastUsedTemplateId": "default", "lastCreatedWorkflow": null, "templates": [{BUILT_IN}]}}"#));
    assert_eq!(machine.run(&["--check"]), (0, "would add critique\n".to_string()));
    assert_eq!(machine.run(&[]).0, 0);
    let resaved = fs::read_to_string(machine.store()).unwrap().replace("\"provider\": null,", "");
    fs::write(machine.store(), resaved).unwrap();
    assert_eq!(machine.run(&["--check"]), (0, "workflows unchanged\n".to_string()));
}
