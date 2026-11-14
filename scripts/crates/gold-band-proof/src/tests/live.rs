//! Tests for the live run home: the pins file, the shim lookup and the home it
//! builds from an instance's settings.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{json, Value};

use crate::harness::inspect;
use crate::live::{compose, parse_pins, shim_command, LiveInputs, Pins};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-live-{}-{}-{}",
        std::process::id(),
        tick,
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A workflow with a model node, one gate pair in two rounds, a verdict and a
/// manual check.
const WORKFLOW: &str = r#"{"workflow":{"id":"live-wf","entry":"w","nodes":[
  {"type":"worker","id":"w","provider":"pi","model":"litellm-vibe/glm-5.3","profile":"pf-builtin-dev","output":{"kind":"json","schema":{"ok":true}}},
  {"type":"worker","id":"gate-build","provider":"gate-build","model":"pinned-command","profile":"pf-builtin-dev"},
  {"type":"worker","id":"copy-build","provider":"copy-build","model":"pinned-command","profile":"pf-builtin-dev"},
  {"type":"worker","id":"gate-build-r1","provider":"gate-build-r1","model":"pinned-command","profile":"pf-builtin-dev"},
  {"type":"worker","id":"gate-findings-2","provider":"gate-findings-2","model":"pinned-command","profile":"pf-builtin-dev"},
  {"type":"worker","id":"verdict-2","provider":"verdict-2","model":"pinned-command","profile":"pf-builtin-dev"}
],"edges":[{"from":"w","to":"$end","on":"success"}]}}"#;

fn pins() -> Pins {
    parse_pins(r#"{"version":1,"timeout_secs":120,"shims":{"build":["cargo","build","{repo}"],"findings":["check","{requirement}"]}}"#)
        .unwrap()
}

/// An instance home holding `settings` and a profiles directory.
fn instance(settings: &Value) -> PathBuf {
    let home = temp_dir("instance");
    let data = home.join(".gold-band");
    fs::create_dir_all(data.join("context/profiles")).unwrap();
    fs::write(data.join("settings.json"), settings.to_string()).unwrap();
    home
}

fn build(
    info_json: &str,
    pins: &Pins,
    instance_home: &Path,
) -> (PathBuf, Result<PathBuf, Vec<String>>) {
    let info = inspect(info_json).unwrap();
    let run_home = temp_dir("run").join("home");
    let mut values = BTreeMap::new();
    values.insert("tools".to_string(), "/tools".to_string());
    let result = compose(&LiveInputs {
        info: &info,
        pins,
        instance_home,
        run_home: &run_home,
        exe: Path::new("/bin/gold-band-proof"),
        values: &values,
    });
    (run_home, result)
}

#[test]
fn a_pins_file_reads_its_shims_and_its_time_limit() {
    let pins = pins();
    assert_eq!(pins.timeout_secs, 120);
    assert_eq!(pins.shims["build"], vec!["cargo", "build", "{repo}"]);
    let defaulted = parse_pins(r#"{"version":1,"shims":{}}"#).unwrap();
    assert_eq!(defaulted.timeout_secs, 3600);
}

#[test]
fn a_bad_pins_file_is_refused() {
    let cases = [
        ("nope", "not JSON"),
        ("[1]", "not a JSON object"),
        (
            r#"{"version":1,"shims":{},"extra":1}"#,
            "unknown key `extra`",
        ),
        (r#"{"version":2,"shims":{}}"#, "version 1"),
        (
            r#"{"version":1,"timeout_secs":0,"shims":{}}"#,
            "timeout_secs",
        ),
        (r#"{"version":1}"#, "`shims` must be an object"),
        (r#"{"version":1,"shims":{"a":"true"}}"#, "array of strings"),
        (r#"{"version":1,"shims":{"a":[]}}"#, "names no command"),
    ];
    for (text, expected) in cases {
        let problem = parse_pins(text).unwrap_err();
        assert!(problem.contains(expected), "{problem} lacks {expected}");
    }
}

#[test]
fn a_shim_is_found_by_name_then_without_its_repair_or_round() {
    let pins =
        parse_pins(r#"{"version":1,"shims":{"build":["b"],"build-r1":["b1"],"findings":["f"]}}"#)
            .unwrap();
    assert_eq!(shim_command(&pins, "build-r1").unwrap(), &vec!["b1"]);
    assert_eq!(shim_command(&pins, "build-r2").unwrap(), &vec!["b"]);
    assert_eq!(shim_command(&pins, "findings-3").unwrap(), &vec!["f"]);
    assert!(shim_command(&pins, "lint").is_none());
    assert!(shim_command(&pins, "build-rx").is_none());
    assert!(shim_command(&pins, "build-r").is_none());
    assert!(shim_command(&pins, "lint-r1").is_none());
}

#[test]
fn the_run_home_adds_a_pinned_agent_for_every_shim_and_keeps_the_instance() {
    let home = instance(&json!({
        "settingsSchemaVersion": 12,
        "agents": { "pi": { "adapter": { "command": "pi-acp" } } },
        "contextServers": [],
    }));
    let (run_home, result) = build(WORKFLOW, &pins(), &home);
    let reports = result.unwrap();
    assert_eq!(reports, run_home.join("reports"));
    assert!(reports.is_dir());
    let settings: Value = serde_json::from_str(
        &fs::read_to_string(run_home.join(".gold-band/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["settingsSchemaVersion"], 12);
    assert_eq!(settings["contextServers"], json!([]));
    let agents = settings["agents"].as_object().unwrap();
    let ids: Vec<&String> = agents.keys().collect();
    assert_eq!(
        ids,
        vec![
            "copy-build",
            "gate-build",
            "gate-build-r1",
            "gate-findings-2",
            "pi",
            "verdict-2"
        ]
    );
    assert_eq!(agents["pi"], json!({ "adapter": { "command": "pi-acp" } }));
    let gate = &agents["gate-build-r1"]["adapter"];
    assert_eq!(gate["command"], "/bin/gold-band-proof");
    assert_eq!(gate["args"], json!(["pinned-acp"]));
    assert_eq!(gate["env"]["PINNED_ACP_ROLE"], "gate:build-r1");
    assert_eq!(
        gate["env"]["PINNED_ACP_COMMAND"],
        r#"["cargo","build","{repo}"]"#
    );
    assert_eq!(gate["env"]["PINNED_ACP_TIMEOUT_SECS"], "120");
    assert_eq!(gate["env"]["PINNED_ACP_MODELS"], "pinned-command");
    assert_eq!(gate["env"]["PINNED_ACP_VALUES"], r#"{"tools":"/tools"}"#);
    assert_eq!(
        gate["env"]["PINNED_ACP_REPORT_ROOT"],
        reports.to_string_lossy().as_ref()
    );
    assert_eq!(
        agents["copy-build"]["adapter"]["env"]["PINNED_ACP_ROLE"],
        "copy:build"
    );
    assert_eq!(
        agents["gate-findings-2"]["adapter"]["env"]["PINNED_ACP_COMMAND"],
        r#"["check","{requirement}"]"#
    );
    let verdict = &agents["verdict-2"]["adapter"]["env"];
    assert_eq!(verdict["PINNED_ACP_ROLE"], "verdict:2");
    assert_eq!(verdict["PINNED_ACP_COMMAND"], "[]");
    let context = run_home.join(".gold-band/context");
    assert_eq!(
        fs::read_link(&context).unwrap(),
        home.join(".gold-band/context")
    );
    // The instance's own settings are not written.
    let original: Value =
        serde_json::from_str(&fs::read_to_string(home.join(".gold-band/settings.json")).unwrap())
            .unwrap();
    assert_eq!(original["agents"].as_object().unwrap().len(), 1);
}

#[test]
fn an_instance_without_profiles_gets_no_context_link() {
    let home = temp_dir("bare");
    fs::create_dir_all(home.join(".gold-band")).unwrap();
    fs::write(
        home.join(".gold-band/settings.json"),
        r#"{"agents":{"pi":{}}}"#,
    )
    .unwrap();
    let (run_home, result) = build(WORKFLOW, &pins(), &home);
    result.unwrap();
    assert!(!run_home.join(".gold-band/context").exists());
    // An instance without a schema version gets the pinned Gold Band's.
    let settings: Value = serde_json::from_str(
        &fs::read_to_string(run_home.join(".gold-band/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(settings["settingsSchemaVersion"], 12);
}

#[test]
fn every_reason_a_run_cannot_be_bound_is_reported() {
    let home = instance(&json!({ "agents": { "gate-build": {} } }));
    let workflow = WORKFLOW.replace(
        r#"{"type":"worker","id":"verdict-2""#,
        r#"{"type":"worker","id":"grant-check-writer","provider":"grant-writer","model":"m"},{"type":"worker","id":"lint","provider":"gate-lint","model":"m"},{"type":"worker","id":"verdict-2""#,
    );
    let (_, result) = build(&workflow, &pins(), &home);
    let problems = result.unwrap_err();
    let text = problems.join("\n");
    assert!(
        text.contains("agent gate-build: the instance already declares it"),
        "{text}"
    );
    assert!(
        text.contains("agent gate-lint: the pins file names no command for `lint`"),
        "{text}"
    );
    assert!(
        text.contains("agent grant-writer: a grant check belongs to the work instance"),
        "{text}"
    );
    assert!(
        text.contains("agent pi: not in the instance's agent map"),
        "{text}"
    );
    assert_eq!(problems.len(), 4, "{text}");
}

#[test]
fn an_unreadable_instance_is_refused() {
    let missing = temp_dir("missing");
    let (_, result) = build(WORKFLOW, &pins(), &missing);
    assert!(result.unwrap_err()[0].contains("cannot read"));
    let not_object = instance(&json!([1]));
    let (_, result) = build(WORKFLOW, &pins(), &not_object);
    assert!(result.unwrap_err()[0].contains("is not a JSON object"));
    let no_agents = instance(&json!({ "agents": [] }));
    let (_, result) = build(WORKFLOW, &pins(), &no_agents);
    assert!(result.unwrap_err()[0].contains("has no agents map"));
}

#[test]
fn a_run_home_that_cannot_be_built_is_refused() {
    let home = instance(&json!({ "agents": { "pi": {} } }));
    let info = inspect(WORKFLOW).unwrap();
    let blocker = temp_dir("blocker").join("file");
    fs::write(&blocker, "").unwrap();
    let values = BTreeMap::new();
    let pins = pins();
    let result = compose(&LiveInputs {
        info: &info,
        pins: &pins,
        instance_home: &home,
        run_home: &blocker.join("home"),
        exe: Path::new("/bin/gold-band-proof"),
        values: &values,
    });
    assert!(result.unwrap_err()[0].contains("cannot build the run home"));
}

#[test]
fn the_checked_in_pins_bind_every_shim_of_workflow_b() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../installer/gold-band");
    let workflow = fs::read_to_string(root.join("workflows/workflow-b.json")).unwrap();
    let pins =
        parse_pins(&fs::read_to_string(root.join("pins/workflow-b-agent-kit.json")).unwrap())
            .unwrap();
    let home = instance(&json!({ "agents": { "pi": {}, "claude": {} } }));
    let (run_home, result) = build(&workflow, &pins, &home);
    result.unwrap_or_else(|problems| panic!("{}", problems.join("\n")));
    let settings: Value = serde_json::from_str(
        &fs::read_to_string(run_home.join(".gold-band/settings.json")).unwrap(),
    )
    .unwrap();
    let pinned = settings["agents"]
        .as_object()
        .unwrap()
        .keys()
        .filter(|id| !["pi", "claude"].contains(&id.as_str()))
        .count();
    // Fifteen gates and their copiers (four per build round, one finding
    // check per critics round) and three verdicts.
    assert_eq!(pinned, 15 * 2 + 3);
}
