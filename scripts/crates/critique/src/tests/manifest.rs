//! manifest's own pieces: a run manifest's reader, the lines a window holds, and the files a critic's prompt names.

use crate::limits::{AntigravityQuota, CritiqueLimits};
use crate::manifest::{
    held_ranges, listed_files, parse_manifest, spec_reference, ArtifactSource, CodeSlice, CodeSource, DanglingBlocker, DepthChoice,
    DepthSignal, LineRange, MeasuredValue, PacketFile, PacketWindow, RunManifest, TicketCheckSummary, WorkRoster,
};
use crate::packet_search::SearchLimits;
use crate::roster::{DroppedCritic, RosterCritic};
use crate::routing::{CriticClient, VibeCriticClient};

/// Every field a manifest must carry, with a value each reader accepts.
const BASE: [(&str, &str); 19] = [
    ("allowed_families", r#"["mistral"]"#),
    ("artifact", r#"{"labels":[],"reference":"o/r#1","tracker":"github"}"#),
    ("created_at", r#""2026-10-30T12:00:00.000Z""#),
    ("critics", "[]"),
    ("depth", r#""base""#),
    ("depth_choice", r#"{"chosen_by":"argument","measures":{},"signals":[]}"#),
    ("distinct_families", "true"),
    ("dropped", "[]"),
    ("flow", r#""flow-1""#),
    ("limits", r#"{"antigravity_quota":{"daily_cap":20,"reserve_threshold":10,"reserved_stages":["spec"]},"search":{"max_bytes":8192,"max_matches":50}}"#),
    ("orchestrator_family", r#""anthropic""#),
    ("packet_dir", r#""runs/r1/packet""#),
    ("packet_files", "[]"),
    ("repo_root", r#""/repo""#),
    ("repository", r#""o/r""#),
    ("run_dir", r#""runs/r1""#),
    ("run_id", r#""r1""#),
    ("stage", r#""spec""#),
    ("version", "1"),
];

/// A manifest's JSON text: the base fields, with `overrides` replacing or adding one.
pub fn manifest_text(overrides: &[(&str, &str)]) -> String {
    let mut fields: Vec<(String, String)> = BASE.iter().map(|(key, value)| (key.to_string(), value.to_string())).collect();
    for (key, value) in overrides {
        let mut found = false;
        for (name, slot) in fields.iter_mut() {
            if name.as_str() == *key {
                *slot = value.to_string();
                found = true;
            }
        }
        if !found {
            fields.push((key.to_string(), value.to_string()));
        }
    }
    format!("{{{}}}", fields.iter().map(|(key, value)| format!("\"{key}\":{value}")).collect::<Vec<_>>().join(","))
}

fn error(overrides: &[(&str, &str)]) -> String {
    parse_manifest(&manifest_text(overrides)).expect_err("error")
}

const CRITIC_LIST: &str = concat!("[", r#"{"client":"vibe-main","role":"critic","lenses":["a"],"max_price_usd":2.5,"time_limit_s":600,"critic_id":"c1-vibe-main","client_config":{"kind":"vibe","binary":"vibe","agent":"critic-m","model":"m","family":"mistral"},"fallbacks":[]}"#, "]");

const DROPPED_LIST: &str = concat!("[", r#"{"client":"agy-1","family":"google","reason":"quota","detail":"out of quota","replaced_by":"c2-vibe","deep":true}"#, "]");

const PACKET_FILES: &str = r#"[
    {"kind":"fact","path":"tree/REPO-MAP.md","sha256":"h1","source":"repo-map"},
    {"kind":"diff","path":"diff/change.diff","sha256":"h2","source":"abc..def"},
    {"kind":"ticket","path":"tickets/T1.md","sha256":"h3","source":"o/r#1"},
    {"kind":"spec","path":"spec/spec.md","sha256":"h4","source":"o/r#2"},
    {"kind":"fact","path":"facts/INDEX.md","sha256":"h5","source":"facts"},
    {"kind":"tree","path":"tree/src/a.rs","sha256":"h6","source":"src/a.rs","window":{"last":10,"lines":100,"ranges":[{"first":1,"last":10}]}}
]"#;

const CODE: &str = r#"{"base":"main","base_commit":"abc","head":"feat","head_commit":"def","changed_files":["src/a.rs"],"slices":[{"first":1,"last":9,"path":"tree/src/a.rs","reason":"touches f"}],"staged_diff":"hash"}"#;

const TICKET_CHECK: &str = r#"{"status":"passed","ticket_ids":["T1"],"cycles":[["T1","T2"]],"dangling_blockers":[{"blocker":"T1","ticket":"T2"}],"uncited_stories":[1,2]}"#;

const DEPTH_CHOICE: &str = r#"{"chosen_by":"signals","measures":{"words":{"threshold":100,"value":250}},"signals":[{"kind":"threshold","measure":"words","threshold":100,"value":250},{"kind":"risk-term","term":"auth"},{"kind":"label","label":"risk"}]}"#;

fn packet_file(kind: &str, path: &str, sha256: &str, source: &str) -> PacketFile {
    PacketFile { path: path.to_string(), source: source.to_string(), kind: kind.to_string(), sha256: sha256.to_string(), window: None }
}

fn full_manifest() -> RunManifest {
    RunManifest {
        version: 1.0,
        run_id: "r1".to_string(),
        created_at: "2026-10-30T12:00:00.000Z".to_string(),
        stage: "ticket-review".to_string(),
        depth: "deep".to_string(),
        round: 2.0,
        previous_run_id: Some("r0".to_string()),
        depth_choice: DepthChoice {
            chosen_by: "signals".to_string(),
            measures: vec![("words".to_string(), MeasuredValue { value: 250.0, threshold: 100.0 })],
            signals: vec![
                DepthSignal::Threshold { measure: "words".to_string(), threshold: 100.0, value: 250.0 },
                DepthSignal::RiskTerm { term: "auth".to_string() },
                DepthSignal::Label { label: "risk".to_string() },
            ],
        },
        orchestrator_family: "anthropic".to_string(),
        repository: "o/r".to_string(),
        repo_root: "/repo".to_string(),
        run_dir: "runs/r1".to_string(),
        packet_dir: "runs/r1/packet".to_string(),
        allowed_families: vec!["mistral".to_string(), "google".to_string()],
        artifact: ArtifactSource { tracker: "github".to_string(), reference: "o/r#1".to_string(), labels: vec!["bug".to_string()] },
        flow: "flow-1".to_string(),
        limits: CritiqueLimits {
            antigravity_quota: AntigravityQuota { daily_cap: 20.0, reserve_threshold: 10.0, reserved_stages: vec!["spec".to_string()] },
            search: SearchLimits { max_matches: 50.0, max_bytes: 8192.0 },
        },
        plan_warnings: Some(vec!["plan at 80%".to_string()]),
        packet_files: vec![
            packet_file("fact", "tree/REPO-MAP.md", "h1", "repo-map"),
            packet_file("diff", "diff/change.diff", "h2", "abc..def"),
            packet_file("ticket", "tickets/T1.md", "h3", "o/r#1"),
            packet_file("spec", "spec/spec.md", "h4", "o/r#2"),
            packet_file("fact", "facts/INDEX.md", "h5", "facts"),
            PacketFile {
                path: "tree/src/a.rs".to_string(),
                source: "src/a.rs".to_string(),
                kind: "tree".to_string(),
                sha256: "h6".to_string(),
                window: Some(PacketWindow { last: 10.0, lines: 100.0, ranges: Some(vec![LineRange { first: 1.0, last: 10.0 }]) }),
            },
        ],
        code: Some(CodeSource {
            base: "main".to_string(),
            base_commit: "abc".to_string(),
            head: "feat".to_string(),
            head_commit: "def".to_string(),
            changed_files: vec!["src/a.rs".to_string()],
            slices: Some(vec![CodeSlice {
                path: "tree/src/a.rs".to_string(),
                first: 1.0,
                last: 9.0,
                reason: "touches f".to_string(),
            }]),
            staged_diff: Some("hash".to_string()),
        }),
        ticket_check: Some(TicketCheckSummary {
            status: "passed".to_string(),
            ticket_ids: vec!["T1".to_string()],
            cycles: vec![vec!["T1".to_string(), "T2".to_string()]],
            dangling_blockers: vec![DanglingBlocker { blocker: "T1".to_string(), ticket: "T2".to_string() }],
            uncited_stories: Some(vec![1.0, 2.0]),
            coverage_reason: None,
        }),
        distinct_families: true,
        critics: vec![RosterCritic {
            client: "vibe-main".to_string(),
            role: "critic".to_string(),
            lenses: vec!["a".to_string()],
            max_price_usd: 2.5,
            time_limit_s: 600.0,
            critic_id: "c1-vibe-main".to_string(),
            client_config: CriticClient::Vibe(VibeCriticClient {
                binary: "vibe".to_string(),
                agent: "critic-m".to_string(),
                model: "m".to_string(),
                family: "mistral".to_string(),
                prices: None,
                gateway: None,
            }),
            fallbacks: Vec::new(),
        }],
        dropped: vec![DroppedCritic {
            client: "agy-1".to_string(),
            family: "google".to_string(),
            reason: "quota".to_string(),
            detail: Some("out of quota".to_string()),
            replaced_by: Some("c2-vibe".to_string()),
            deep: Some(true),
        }],
        work: Some(WorkRoster { grant_workflow: Some("critique".to_string()) }),
    }
}

fn full_text() -> String {
    manifest_text(&[
        ("allowed_families", r#"["mistral","google"]"#),
        ("artifact", r#"{"labels":["bug"],"reference":"o/r#1","tracker":"github"}"#),
        ("code", CODE),
        ("critics", CRITIC_LIST),
        ("depth", r#""deep""#),
        ("depth_choice", DEPTH_CHOICE),
        ("dropped", DROPPED_LIST),
        ("packet_files", PACKET_FILES),
        ("plan_warnings", r#"["plan at 80%"]"#),
        ("previous_run_id", r#""r0""#),
        ("round", "2"),
        ("stage", r#""ticket-review""#),
        ("ticket_check", TICKET_CHECK),
        ("work", r#"{"grant_workflow":"critique"}"#),
    ])
}

#[test]
fn a_full_manifest_round_trips() {
    let manifest = parse_manifest(&full_text()).expect("manifest");
    assert_eq!(manifest, full_manifest());
}

#[test]
fn a_code_packet_names_its_repo_map_the_change_and_the_facts_index() {
    let manifest = full_manifest();
    let listed: Vec<String> = listed_files(&manifest).into_iter().map(|file| file.path).collect();
    assert_eq!(listed, ["tree/REPO-MAP.md", "diff/change.diff", "tickets/T1.md", "spec/spec.md", "facts/INDEX.md"]);
}

#[test]
fn a_document_packet_names_every_file() {
    let mut manifest = full_manifest();
    manifest.code = None;
    assert_eq!(listed_files(&manifest), manifest.packet_files);
}

#[test]
fn a_ticket_review_takes_its_spec_from_the_packet() {
    let mut manifest = full_manifest();
    assert_eq!(spec_reference(&manifest), "o/r#2");
    manifest.stage = "spec".to_string();
    assert_eq!(spec_reference(&manifest), "o/r#1");
    manifest.stage = "ticket-review".to_string();
    manifest.packet_files.retain(|file| file.kind != "spec");
    assert_eq!(spec_reference(&manifest), "o/r#1");
}

#[test]
fn a_window_holds_its_ranges_or_its_first_lines() {
    let whole = PacketWindow { last: 10.0, lines: 100.0, ranges: None };
    assert_eq!(held_ranges(&whole), vec![LineRange { first: 1.0, last: 10.0 }]);
    let sliced = PacketWindow { last: 10.0, lines: 100.0, ranges: Some(vec![LineRange { first: 3.0, last: 4.0 }]) };
    assert_eq!(held_ranges(&sliced), vec![LineRange { first: 3.0, last: 4.0 }]);
}

#[test]
fn a_manifest_without_its_optional_parts_reads_their_defaults() {
    let manifest = parse_manifest(&manifest_text(&[])).expect("manifest");
    assert_eq!(manifest.version, 1.0);
    assert_eq!(manifest.stage, "spec");
    assert_eq!(manifest.depth, "base");
    assert_eq!(manifest.round, 1.0);
    assert_eq!(manifest.previous_run_id, None);
    assert_eq!(manifest.code, None);
    assert_eq!(manifest.work, None);
    assert_eq!(manifest.plan_warnings, None);
    assert_eq!(manifest.ticket_check, None);
    assert_eq!(manifest.critics, Vec::new());
    assert_eq!(manifest.dropped, Vec::new());
    assert_eq!(manifest.packet_files, Vec::new());
    assert_eq!(manifest.depth_choice, DepthChoice { chosen_by: "argument".to_string(), measures: Vec::new(), signals: Vec::new() });
    assert_eq!(manifest.artifact, ArtifactSource { tracker: "github".to_string(), reference: "o/r#1".to_string(), labels: Vec::new() });
    assert_eq!(
        manifest.limits,
        CritiqueLimits {
            antigravity_quota: AntigravityQuota { daily_cap: 20.0, reserve_threshold: 10.0, reserved_stages: vec!["spec".to_string()] },
            search: SearchLimits { max_matches: 50.0, max_bytes: 8192.0 },
        }
    );
    assert_eq!(spec_reference(&manifest), "o/r#1");
    assert_eq!(listed_files(&manifest), Vec::new());
}

#[test]
fn a_first_round_is_one_and_a_work_roster_may_name_no_grant() {
    let manifest = parse_manifest(&manifest_text(&[("round", "1"), ("work", "{}")])).expect("manifest");
    assert_eq!(manifest.round, 1.0);
    assert_eq!(manifest.previous_run_id, None);
    assert_eq!(manifest.work, Some(WorkRoster { grant_workflow: None }));
}

#[test]
fn a_code_source_without_slices_or_a_staged_change_has_neither() {
    let code = r#"{"base":"main","base_commit":"abc","head":"feat","head_commit":"def","changed_files":[]}"#;
    let manifest = parse_manifest(&manifest_text(&[("code", code)])).expect("manifest");
    let source = manifest.code.expect("code");
    assert_eq!(source.slices, None);
    assert_eq!(source.staged_diff, None);
    assert_eq!(source.changed_files, Vec::<String>::new());
}

#[test]
fn a_packet_file_without_a_window_has_none() {
    let files = r#"[{"kind":"artifact","path":"artifact/spec.md","sha256":"h","source":"o/r#1"}]"#;
    let manifest = parse_manifest(&manifest_text(&[("packet_files", files)])).expect("manifest");
    assert_eq!(manifest.packet_files[0].window, None);
}

#[test]
fn a_ticket_check_that_could_not_check_coverage_carries_its_reason() {
    let check = r#"{"status":"failed","ticket_ids":[],"cycles":[],"dangling_blockers":[],"uncited_stories":null,"coverage_reason":"no stories"}"#;
    let manifest = parse_manifest(&manifest_text(&[("ticket_check", check)])).expect("manifest");
    let summary = manifest.ticket_check.expect("check");
    assert_eq!(summary.uncited_stories, None);
    assert_eq!(summary.coverage_reason, Some("no stories".to_string()));
    assert_eq!(summary.status, "failed");
}

#[test]
fn json_that_does_not_parse_or_is_no_object_is_an_error() {
    assert_eq!(parse_manifest("").expect_err("error"), "Unexpected end of JSON input");
    assert_eq!(parse_manifest("[]").expect_err("error"), "the value must be a JSON object");
}

#[test]
fn a_manifest_names_the_first_wrong_field() {
    assert_eq!(error(&[("allowed_families", r#"["nope"]"#)]), "allowed_families[0] must be one of mistral, zhipu, deepseek, google, anthropic, openai");
    assert_eq!(error(&[("artifact", "5")]), "artifact must be a JSON object");
    assert_eq!(error(&[("artifact", r#"{"labels":[],"reference":"o/r#1","tracker":"jira"}"#)]), "artifact.tracker must be one of github, local-markdown, none");
    assert_eq!(error(&[("artifact", r#"{"labels":[1],"reference":"o/r#1","tracker":"github"}"#)]), "artifact.labels[0] must be a string");
    assert_eq!(error(&[("artifact", r#"{"labels":[],"reference":"","tracker":"github"}"#)]), "artifact.reference must be a non-empty string");
    assert_eq!(error(&[("created_at", r#"" ""#)]), "created_at must be a non-empty string");
    assert_eq!(error(&[("critics", "[5]")]), "critics[0] must be a JSON object");
    assert_eq!(error(&[("depth", r#""mid""#)]), "depth must be one of base, deep");
    assert_eq!(error(&[("distinct_families", "1")]), "distinct_families must be true or false");
    assert_eq!(error(&[("depth_choice", "[]")]), "depth_choice must be a JSON object");
    assert_eq!(error(&[("dropped", "[5]")]), "dropped[0] must be a JSON object");
    assert_eq!(error(&[("flow", "null")]), "flow must be a non-empty string");
    assert_eq!(error(&[("limits", "[]")]), "limits must be a JSON object");
    assert_eq!(error(&[("orchestrator_family", r#""x""#)]), "orchestrator_family must be one of anthropic, openai, mistral, zhipu, deepseek, google");
    assert_eq!(error(&[("packet_dir", "0")]), "packet_dir must be a non-empty string");
    assert_eq!(error(&[("code", "[]")]), "code must be a JSON object");
    assert_eq!(error(&[("work", "[]")]), "work must be a JSON object");
    assert_eq!(error(&[("work", r#"{"grant_workflow":""}"#)]), "work.grant_workflow must be a non-empty string");
    assert_eq!(error(&[("plan_warnings", "{}")]), "plan_warnings must be a list");
    assert_eq!(error(&[("packet_files", "[5]")]), "packet_files[0] must be a JSON object");
    assert_eq!(error(&[("ticket_check", "[]")]), "ticket_check must be a JSON object");
    assert_eq!(error(&[("repo_root", "false")]), "repo_root must be a non-empty string");
    assert_eq!(error(&[("repository", "false")]), "repository must be a non-empty string");
    assert_eq!(error(&[("run_dir", "false")]), "run_dir must be a non-empty string");
    assert_eq!(error(&[("run_id", "false")]), "run_id must be a non-empty string");
    assert_eq!(
        error(&[("stage", r#""nope""#)]),
        "stage must be one of decisions, spec, tickets, ticket-review, feature-review, adr-review, decision-recovery, architecture-review, plan"
    );
    assert_eq!(error(&[("version", "2")]), "version must be 1");
}

#[test]
fn a_round_is_one_or_two() {
    assert_eq!(error(&[("round", "3")]), "round must be 1");
    assert_eq!(error(&[("round", r#""2""#)]), "round must be a number");
    assert_eq!(error(&[("round", "2")]), "previous_run_id must be a non-empty string");
}

#[test]
fn a_packet_file_names_its_wrong_field() {
    let kinds = "packet_files[0].kind must be one of artifact, reference, tree, diff, ticket, spec, check, feature, fact, history";
    assert_eq!(error(&[("packet_files", r#"[{"kind":"nope","path":"p","sha256":"h","source":"s"}]"#)]), kinds);
    assert_eq!(error(&[("packet_files", r#"[{"path":"p","sha256":"h","source":"s"}]"#)]), kinds);
    assert_eq!(error(&[("packet_files", r#"[{"kind":"tree","sha256":"h","source":"s"}]"#)]), "packet_files[0].path must be a non-empty string");
    assert_eq!(error(&[("packet_files", r#"[{"kind":"tree","path":"p","source":"s"}]"#)]), "packet_files[0].sha256 must be a non-empty string");
    assert_eq!(error(&[("packet_files", r#"[{"kind":"tree","path":"p","sha256":"h"}]"#)]), "packet_files[0].source must be a non-empty string");
    let window = |window: &str| format!(r#"[{{"kind":"tree","path":"p","sha256":"h","source":"s","window":{window}}}]"#);
    assert_eq!(error(&[("packet_files", window("[]").as_str())]), "packet_files[0].window must be a JSON object");
    assert_eq!(error(&[("packet_files", window(r#"{"lines":1}"#).as_str())]), "packet_files[0].window.last must be a number");
    assert_eq!(error(&[("packet_files", window(r#"{"last":1}"#).as_str())]), "packet_files[0].window.lines must be a number");
    assert_eq!(
        error(&[("packet_files", window(r#"{"last":1,"lines":1,"ranges":[{"first":0,"last":1}]}"#).as_str())]),
        "packet_files[0].window.ranges[0].first must be a whole number of at least 1"
    );
    assert_eq!(
        error(&[("packet_files", window(r#"{"last":1,"lines":1,"ranges":[{"first":1}]}"#).as_str())]),
        "packet_files[0].window.ranges[0].last must be a number"
    );
}

#[test]
fn a_code_source_names_its_wrong_field() {
    assert_eq!(error(&[("code", r#"{"base_commit":"abc","head":"feat","head_commit":"def","changed_files":[]}"#)]), "code.base must be a non-empty string");
    assert_eq!(error(&[("code", r#"{"base":"main","head":"feat","head_commit":"def","changed_files":[]}"#)]), "code.base_commit must be a non-empty string");
    assert_eq!(error(&[("code", r#"{"base":"main","base_commit":"abc","head":"feat","head_commit":"def"}"#)]), "code.changed_files must be a list");
    assert_eq!(error(&[("code", r#"{"base":"main","base_commit":"abc","head_commit":"def","changed_files":[]}"#)]), "code.head must be a non-empty string");
    assert_eq!(error(&[("code", r#"{"base":"main","base_commit":"abc","head":"feat","changed_files":[]}"#)]), "code.head_commit must be a non-empty string");
    let slices = |slices: &str| format!(r#"{{"base":"main","base_commit":"abc","head":"feat","head_commit":"def","changed_files":[],"slices":{slices}}}"#);
    assert_eq!(
        error(&[("code", slices(r#"[{"first":0,"last":1,"path":"p","reason":"r"}]"#).as_str())]),
        "code.slices[0].first must be a whole number of at least 1"
    );
    assert_eq!(
        error(&[("code", slices(r#"[{"first":1,"last":1,"reason":"r"}]"#).as_str())]),
        "code.slices[0].path must be a non-empty string"
    );
    assert_eq!(
        error(&[("code", slices(r#"[{"first":1,"last":1,"path":"p"}]"#).as_str())]),
        "code.slices[0].reason must be a non-empty string"
    );
    let staged = r#"{"base":"main","base_commit":"abc","head":"feat","head_commit":"def","changed_files":[],"staged_diff":""}"#;
    assert_eq!(error(&[("code", staged)]), "code.staged_diff must be a non-empty string");
}

#[test]
fn a_ticket_check_names_its_wrong_field() {
    assert_eq!(error(&[("ticket_check", r#"{"coverage_reason":""}"#)]), "ticket_check.coverage_reason must be a non-empty string");
    assert_eq!(error(&[("ticket_check", r#"{"cycles":{}}"#)]), "ticket_check.cycles must be a list");
    assert_eq!(error(&[("ticket_check", r#"{"cycles":[5]}"#)]), "ticket_check.cycles[0] must be a list of strings");
    assert_eq!(error(&[("ticket_check", r#"{"cycles":[["a",5]]}"#)]), "ticket_check.cycles[0] must be a list of strings");
    let blockers = |blockers: &str| format!(r#"{{"cycles":[],"dangling_blockers":{blockers}}}"#);
    assert_eq!(
        error(&[("ticket_check", blockers(r#"[{"blocker":"T1"}]"#).as_str())]),
        "ticket_check.dangling_blockers[0].ticket must be a non-empty string"
    );
    assert_eq!(
        error(&[("ticket_check", blockers(r#"[{"ticket":"T2"}]"#).as_str())]),
        "ticket_check.dangling_blockers[0].blocker must be a non-empty string"
    );
    assert_eq!(
        error(&[("ticket_check", r#"{"cycles":[],"dangling_blockers":[],"status":"maybe"}"#)]),
        "ticket_check.status must be one of passed, failed"
    );
    assert_eq!(
        error(&[("ticket_check", r#"{"cycles":[],"dangling_blockers":[],"status":"passed","ticket_ids":[5]}"#)]),
        "ticket_check.ticket_ids[0] must be a string"
    );
    assert_eq!(
        error(&[("ticket_check", r#"{"cycles":[],"dangling_blockers":[],"status":"passed","ticket_ids":[],"uncited_stories":["a"]}"#)]),
        "ticket_check.uncited_stories[0] must be a number"
    );
}

#[test]
fn a_depth_choice_names_its_wrong_field() {
    assert_eq!(error(&[("depth_choice", r#"{"chosen_by":"guess"}"#)]), "depth_choice.chosen_by must be one of argument, signals");
    assert_eq!(error(&[("depth_choice", r#"{"chosen_by":"argument","measures":[]}"#)]), "depth_choice.measures must be a JSON object");
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{"words":{"value":1}}}"#)]),
        "depth_choice.measures.words.threshold must be a number"
    );
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{"words":{"threshold":1}}}"#)]),
        "depth_choice.measures.words.value must be a number"
    );
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{},"signals":[{"kind":"nope"}]}"#)]),
        "depth_choice.signals[0].kind must be one of threshold, risk-term, label"
    );
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{},"signals":[{"kind":"threshold"}]}"#)]),
        "depth_choice.signals[0].measure must be a non-empty string"
    );
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{},"signals":[{"kind":"threshold","measure":"m"}]}"#)]),
        "depth_choice.signals[0].threshold must be a number"
    );
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{},"signals":[{"kind":"threshold","measure":"m","threshold":1}]}"#)]),
        "depth_choice.signals[0].value must be a number"
    );
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{},"signals":[{"kind":"risk-term"}]}"#)]),
        "depth_choice.signals[0].term must be a non-empty string"
    );
    assert_eq!(
        error(&[("depth_choice", r#"{"chosen_by":"argument","measures":{},"signals":[{"kind":"label"}]}"#)]),
        "depth_choice.signals[0].label must be a non-empty string"
    );
}
