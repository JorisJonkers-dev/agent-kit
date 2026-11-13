//! One critic's launch: the tools it is given, the files it leaves, the tripwire around it and the run it records.

use super::manifest::manifest_text;
use super::scenarios::Sandbox;
use super::tripwire::{write, Status};
use super::vibe_client::{agy_config, claude_config, codex_config, vibe_config};
use crate::critic_run::{choose_tools, read_frozen_packet, run_critic, Launch, ToolChoice};
use crate::critic_tools::{CriticToolset, READ_ONLY_TOOLSET};
use crate::flow_budget::PromptHashes;
use crate::manifest::{parse_manifest, RunManifest};
use crate::prompt::{assemble_prompt, CriticPrompt, PromptSources};
use crate::roster::RosterCritic;
use crate::routing::CriticClient;
use crate::tripwire::ReadOnlyTripwire;
use crate::Runtime;
use model_gateway::ports::{Process, ProcessCommand, ProcessResult};
use model_gateway::sha256::sha256_hex;
use std::sync::Mutex;

/// A process that answers every launch with one canned result, after writing the files a critic that ignores its
/// sandbox would: each `(directory of the launch or absolute path, text)` pair.
struct Writing {
    result: ProcessResult,
    plants: Vec<(String, String)>,
    seen: Mutex<Vec<ProcessCommand>>,
}

impl Process for Writing {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, _command: &str, _args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        Ok(self.result.clone())
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        for (path, text) in &self.plants {
            write(&path.replace("@CWD@", command.cwd.as_deref().unwrap_or("")), text);
        }
        self.seen.lock().map_err(|error| error.to_string())?.push(command.clone());
        Ok(self.result.clone())
    }
}

const FINDINGS: &str = "{\"findings\":[{\"category\":\"bug\",\"check\":\"c\",\"claim\":\"x\",\"evidence\":[{\"kind\":\"quote\",\"value\":\"A\"}],\"id\":\"f1\",\"location\":\"l\",\"severity\":\"major\"}]}";

fn agy_answer(structured: &str) -> String {
    format!(r#"{{"event":"result","result":{{"response":"x","structured_output":{structured}}}}}"#)
}

struct Run {
    sandbox: Sandbox,
    runtime: Runtime,
    manifest: RunManifest,
}

/// A run in a sandbox: a packet of two files, a repository beside it, and a runtime whose clients are `process`.
fn run_in(process: Box<dyn Process>, status: Vec<crate::git::PorcelainEntry>, env: &[(&str, &str)]) -> Run {
    let sandbox = Sandbox::new();
    let root = sandbox.text();
    write(&format!("{root}/run/packet/a.md"), "A");
    write(&format!("{root}/run/packet/sub/b.md"), "B");
    let files = format!(
        r#"[{{"kind":"artifact","path":"a.md","sha256":"{}","source":"s"}},{{"kind":"reference","path":"sub/b.md","sha256":"{}","source":"s"}}]"#,
        sha256_hex(b"A"),
        sha256_hex(b"B")
    );
    let text = manifest_text(&[
        ("packet_dir", &format!("\"{root}/run/packet\"")),
        ("repo_root", &format!("\"{root}/repo\"")),
        ("run_dir", &format!("\"{root}/run\"")),
        ("packet_files", &files),
    ]);
    let mut runtime = super::cli::runtime("");
    runtime.files = Box::new(model_gateway::adapters::fs::SystemFiles { cwd: root.clone() });
    runtime.git = Box::new(Status(Mutex::new(Ok(status))));
    runtime.gateway.files = Box::new(model_gateway::adapters::fs::SystemFiles { cwd: root.clone() });
    runtime.gateway.process = process;
    runtime.env = std::iter::once(("HOME".to_string(), format!("{root}/home"))).chain(env.iter().map(|(key, value)| (key.to_string(), value.to_string()))).collect();
    runtime.gateway.env = runtime.env.clone();
    runtime.assets_root = format!("{root}/assets");
    Run { manifest: parse_manifest(&text).expect("a manifest"), sandbox, runtime }
}

fn critic(config: CriticClient) -> RosterCritic {
    RosterCritic {
        client: "agy-main".to_string(),
        role: "bug-hunter".to_string(),
        lenses: vec!["lens-a".to_string()],
        max_price_usd: 2.0,
        time_limit_s: 300.0,
        critic_id: "c1-agy-main".to_string(),
        client_config: config,
        fallbacks: vec![],
    }
}

fn prompt(toolset: &CriticToolset) -> CriticPrompt {
    let sources = PromptSources {
        preamble: "pre".to_string(),
        role: "role".to_string(),
        lenses: vec![("lens-a".to_string(), "lens text".to_string())],
        contract: "contract".to_string(),
        schema: "{}".to_string(),
    };
    assemble_prompt(&sources, &[], "spec", &[], toolset)
}

fn writing(stdout: &str, exit_code: i32, plants: Vec<(String, String)>) -> Box<Writing> {
    Box::new(Writing { result: ProcessResult { stdout: stdout.to_string(), stderr: "warn".to_string(), exit_code, timed_out: false }, plants, seen: Mutex::new(Vec::new()) })
}

fn launch_one(run: &Run, critic: &RosterCritic, retry_of: Option<&str>) -> crate::critic_run::CriticLaunchResult {
    let tripwire = ReadOnlyTripwire::load(&run.runtime, &run.manifest).expect("a tripwire");
    let dir = format!("{}/run/critics/c1", run.sandbox.text());
    run_critic(&run.runtime, &Launch { tripwire: &tripwire, critic, critic_dir: &dir, prompt: &prompt, retry_of: retry_of.map(str::to_string) })
        .unwrap_or_else(|error| panic!("{}", error.message))
}

#[test]
fn a_critic_that_gives_a_review_has_its_findings_and_leaves_its_files() {
    let run = run_in(writing(&agy_answer(FINDINGS), 0, vec![]), vec![], &[]);
    let routed = critic(CriticClient::Agy(agy_config()));
    let launched = launch_one(&run, &routed, Some("c0"));
    assert_eq!(launched.findings.len(), 1);
    let record = &launched.run;
    assert_eq!((record.status.as_str(), record.finding_count, record.retry_of.as_deref()), ("ok", 1.0, Some("c0")));
    assert_eq!((record.critic_id.as_str(), record.family.as_str(), record.model.as_str()), ("c1-agy-main", "google", "gemini-3"));
    assert_eq!((record.charged_usd, record.exit_code, record.cost_usd), (0.0, Some(0.0), None));
    assert_eq!(record.tools.as_deref(), Some(&["view_file".to_string()][..]));
    assert!(record.search_failure.as_deref().is_some_and(|failure| failure.starts_with("agy-search-unproven: agy-main's read-only proof")));
    assert_eq!(record.prompt_hashes, PromptHashes { contract: sha256_hex(b"contract"), preamble: sha256_hex(b"pre"), role: sha256_hex(b"role"), schema: sha256_hex(b"{}") });
    let dir = format!("{}/run/critics/c1", run.sandbox.text());
    assert_eq!(record.prompt_path, format!("{dir}/prompt.md"));
    assert_eq!(record.stdout_path.as_deref(), Some(format!("{dir}/stdout.json").as_str()));
    assert_eq!(std::fs::read_to_string(format!("{dir}/stderr.txt")).expect("stderr"), "warn");
    assert_eq!(std::fs::read_to_string(format!("{dir}/packet/sub/b.md")).expect("the critic's copy"), "B");
    assert!(std::fs::read_to_string(format!("{dir}/prompt.md")).expect("prompt").contains("`view_file`"));
}

#[test]
fn a_critic_that_writes_to_what_it_may_only_read_is_voided() {
    let root_marker = "@CWD@/planted.txt".to_string();
    let run = run_in(writing(&agy_answer(FINDINGS), 0, vec![(root_marker, "x".to_string())]), vec![], &[]);
    let launched = launch_one(&run, &critic(CriticClient::Agy(agy_config())), None);
    assert!(launched.findings.is_empty());
    let record = &launched.run;
    assert_eq!((record.status.as_str(), record.finding_count, record.failure.as_deref()), ("violated-readonly", 0.0, None));
    assert_eq!(record.detail.as_deref(), Some("read-only violated: packet: added planted.txt"));
    // The copy is put back as it was frozen.
    assert!(!std::path::Path::new(&format!("{}/run/critics/c1/packet/planted.txt", run.sandbox.text())).exists());
}

#[test]
fn a_critic_whose_run_failed_records_why() {
    let run = run_in(writing("", 3, vec![]), vec![], &[]);
    let launched = launch_one(&run, &critic(CriticClient::Agy(agy_config())), None);
    assert!(launched.findings.is_empty());
    let record = &launched.run;
    assert_eq!((record.status.as_str(), record.failure.as_deref()), ("failed", Some("exit-3")));
    assert_eq!(record.detail.as_deref(), Some("agy exited 3: warn"));
    let empty = run_in(writing(&agy_answer(r#"{"findings":[],"no_findings_reason":"clean"}"#), 0, vec![]), vec![], &[]);
    let reviewed = launch_one(&empty, &critic(CriticClient::Agy(agy_config())), None);
    assert_eq!((reviewed.run.status.as_str(), reviewed.run.no_findings_reason.as_deref()), ("ok", Some("clean")));
}

#[test]
fn every_kind_of_critic_runs_through_its_own_client() {
    let codex = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"```json\\n{\\\"findings\\\":[],\\\"no_findings_reason\\\":\\\"clean\\\"}\\n```\"}}\n";
    let run = run_in(writing(codex, 0, vec![]), vec![], &[]);
    let launched = launch_one(&run, &critic(CriticClient::Codex(codex_config())), None);
    assert_eq!((launched.run.status.as_str(), launched.run.charged_usd), ("ok", 0.0));
    assert!(launched.run.tokens.is_some() && launched.run.vibe_cost_usd.is_none() && launched.run.search_failure.is_none());
    let claude = launch_one(&run, &critic(CriticClient::Claude(claude_config())), None);
    // Codex's output is not Claude's: the run says so.
    assert_eq!((claude.run.status.as_str(), claude.run.failure.as_deref()), ("failed", Some("unparseable-output")));
    // A Vibe critic with no export wrote nothing the runner can trust.
    let vibe = launch_one(&run, &critic(CriticClient::Vibe(vibe_config())), None);
    assert_eq!((vibe.run.status.as_str(), vibe.run.failure.as_deref()), ("failed", Some("missing-export")));
    assert_eq!(vibe.run.vibe_cost_usd, Some(None));
}

#[test]
fn an_agy_critic_searches_only_once_setup_proved_it_read_only() {
    let run = run_in(writing("", 0, vec![]), vec![], &[]);
    let routed = critic(CriticClient::Agy(agy_config()));
    let home = format!("{}/home", run.sandbox.text());
    let unproven = |choice: ToolChoice| (choice.toolset == CriticToolset::Agy { search: false }) && choice.search_failure.is_some();
    // No results file, a failing result and another client's pass prove nothing.
    assert!(unproven(choose_tools(&run.runtime, &routed).expect("a choice")));
    let results = |checks: &str| format!(r#"{{"version":1,"checks":{checks}}}"#);
    let entry = |result: &str| format!(r#"{{"checked_at":"t","detail":"d","inputs":{{}},"result":"{result}"}}"#);
    let path = format!("{home}/.local/share/agent-kit/critique/machine-setup.json");
    write(&path, &results(&format!(r#"{{"read-only:agy-main":{},"read-only:other":{}}}"#, entry("fail"), entry("pass"))));
    assert!(unproven(choose_tools(&run.runtime, &routed).expect("a choice")));
    write(&path, &results(&format!(r#"{{"read-only:agy-main":{}}}"#, entry("pass"))));
    assert_eq!(choose_tools(&run.runtime, &routed), Ok(ToolChoice { toolset: CriticToolset::Agy { search: true }, search_failure: None }));
    write(&path, "not json");
    assert!(choose_tools(&run.runtime, &routed).is_err());
    // Every other critic reads alone, and says nothing about it.
    for config in [CriticClient::Vibe(vibe_config()), CriticClient::Codex(codex_config()), CriticClient::Claude(claude_config())] {
        assert_eq!(choose_tools(&run.runtime, &critic(config)), Ok(ToolChoice { toolset: READ_ONLY_TOOLSET, search_failure: None }));
    }
    let mut homeless = run_in(writing("", 0, vec![]), vec![], &[]);
    homeless.runtime.env.remove("HOME");
    assert!(choose_tools(&homeless.runtime, &routed).is_err());
}

#[test]
fn the_frozen_packet_is_read_before_any_critic_runs() {
    let run = run_in(writing("", 0, vec![]), vec![], &[]);
    let packet = read_frozen_packet(&run.runtime, &run.manifest).expect("a packet");
    assert_eq!(packet.iter().map(|file| (file.kind.as_str(), file.path.as_str(), file.text.as_str())).collect::<Vec<_>>(), [("artifact", "a.md", "A"), ("reference", "sub/b.md", "B")]);
    std::fs::remove_file(format!("{}/run/packet/a.md", run.sandbox.text())).expect("remove");
    assert!(read_frozen_packet(&run.runtime, &run.manifest).is_err());
}

#[test]
fn a_critic_that_cannot_start_or_be_watched_is_the_runs_error() {
    // Git that cannot say what the working tree holds stops the critic before it starts.
    let mut run = run_in(writing("", 0, vec![]), vec![], &[]);
    run.runtime.git = Box::new(Status(Mutex::new(Err("git is gone".to_string()))));
    let tripwire = ReadOnlyTripwire::load(&run.runtime, &run.manifest).expect("a tripwire");
    let dir = format!("{}/run/critics/c1", run.sandbox.text());
    let routed = critic(CriticClient::Agy(agy_config()));
    let launch = Launch { tripwire: &tripwire, critic: &routed, critic_dir: &dir, prompt: &prompt, retry_of: None };
    assert_eq!(run_critic(&run.runtime, &launch).err().map(|error| error.message), Some("git is gone".to_string()));
    // A client that cannot run is the error its launch gave.
    let refusing = run_in(Box::new(super::vibe_client::FakeProcess::refusing("spawn agy EACCES")), vec![], &[]);
    let tripwire = ReadOnlyTripwire::load(&refusing.runtime, &refusing.manifest).expect("a tripwire");
    let launch = Launch { tripwire: &tripwire, critic: &routed, critic_dir: &dir, prompt: &prompt, retry_of: None };
    assert_eq!(run_critic(&refusing.runtime, &launch).err().map(|error| error.message), Some("spawn agy EACCES".to_string()));
}
