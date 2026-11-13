//! What the goldens cannot reach in `setup`: a Vibe client's read-only proof, which starts the runner's own search server
//! (the node runner's is `node critique.mjs`, so its launch cannot be the same bytes), a probe whose launch is refused,
//! and the hook and client readers' edges. The node runner's `application/setup.ts`, `application/agent-hooks.ts` and
//! `domain/agent-hooks.ts`.

use super::critic_run::{run_in, Run};
use super::search_wiring::{honest, ok, searching, serve, stdin_of, vibing, Handler, AGENT};
use super::tripwire::write;
use super::vibe_client::{claude_config, codex_config, exit, FakeProcess};
use crate::agent_hooks::{codex_hook_states, state_table_key, wired_hooks};
use crate::clients::{client_binary, client_sign_in, client_version};
use crate::routing::CriticClient;
use crate::setup;
use model_gateway::json::{self, Value};
use model_gateway::ports::ProcessResult;

const DENIED: &str = r#",{"type":"effect","detail":{"toolName":"file_system.bash"},"state":{"status":"failed","error":{"code":"tool_denied","message":"no"}}},{"type":"effect","detail":{"toolName":"file_system.write_file"},"state":{"status":"failed","error":{"code":"tool_denied","message":"no"}}},{"type":"effect","detail":{"toolName":"process.start"},"state":{"status":"failed","error":{"code":"tool_denied","message":"no"}}},{"type":"effect","detail":{"toolName":"mcp_packet.grep"},"state":{"status":"completed"}}"#;

fn input(text: &str) -> Value {
    json::parse(text).expect("input JSON")
}

fn field<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(value, |value, key| value.as_object().and_then(|object| object.get(key)))
}

fn text<'a>(value: &'a Value, path: &[&str]) -> &'a str {
    field(value, path).and_then(Value::as_str).unwrap_or_else(|| panic!("no text at {path:?} in {}", json::compact(value)))
}

/// A Vibe client's read-only check, with the agent file setup installs in the sandbox's home.
fn proof(run: &Run, confirmed: bool) -> Result<Value, crate::error::CritiqueError> {
    write(&format!("{}/home/.vibe/agents/critic-mistral-small.toml", run.sandbox.text()), AGENT);
    let confirm = if confirmed { r#","confirmed":true"# } else { "" };
    setup::run(&input(&format!(r#"{{"action":"check","check":"read-only:vibe-mistral-small"{confirm}}}"#)), &run.runtime)
}

/// The proof's recorded detail.
fn detail(result: &Value) -> &str {
    text(result, &["item", "last", "detail"])
}

/// Vibe as the proof launches it, with `plant` written into its packet when it runs.
fn planting(effects: &str, plant: Option<&'static str>) -> Handler {
    let inner = vibing(effects);
    Box::new(move |command| {
        if let (Some(file), false) = (plant, command.command == "/opt/critique") {
            write(&format!("{}/{file}", command.cwd.clone().unwrap_or_default()), "planted");
        }
        inner(command)
    })
}

#[test]
fn a_vibe_client_is_proven_read_only_once_search_stays_in_the_packet_and_run_typescript_refuses_the_rest() {
    let (run, runs, kills) = searching(planting(DENIED, None));
    // It spends, so it waits for the owner's word.
    let waiting = proof(&run, false).expect("a result");
    assert_eq!(text(&waiting, &["status"]), "needs-confirmation");
    assert!(runs.lock().expect("runs").iter().all(|launch| launch.stdin.as_deref().is_some_and(|stdin| stdin.contains("tools/call"))));
    let result = proof(&run, true).expect("a result");
    assert_eq!((text(&result, &["status"]), text(&result, &["item", "last", "result"])), ("checked", "pass"));
    assert_eq!(
        detail(&result),
        "search cannot reach past the packet, and through run_typescript file_system.bash, file_system.write_file, process.start came back tool_denied while mcp_packet.grep completed (charged $0.0500); agent-kit#178's live smoke check adds the rest of the read-only proof"
    );
    // The server stopped with the critic, and the launch is on the ledger as a setup line.
    let proof_packet = format!("{}/home/.local/share/agent-kit/critique/setup-checks/vibe-mistral-small/read-only/packet", run.sandbox.text());
    assert!(kills.lock().expect("kills").iter().any(|kill| kill[0] == "pkill" && kill[3].starts_with(&format!("search-server --root {}", proof_packet.replace('.', "\\.")))));
    let ledger = std::fs::read_to_string(format!("{}/home/.local/share/agent-kit/critique/ledger.jsonl", run.sandbox.text())).expect("the ledger");
    assert!(ledger.starts_with(r#"{"charged_usd":0.05,"client":"vibe-mistral-small","critic_id":"setup-vibe-mistral-small","family":"mistral","flow":"machine-setup","kind":"vibe","#), "{ledger}");
    assert!(ledger.contains(r#""repository":"machine-setup","status":"ok","type":"invocation"}"#));
    // Only the note is left in the packet.
    assert_eq!(std::fs::read_to_string(format!("{proof_packet}/note.md")).expect("the note"), "This note is agent-kit's setup check. It has nothing in it to review.\n");
}

#[test]
fn a_proof_that_cannot_be_made_fails_before_anything_is_spent() {
    // No search server on this runner.
    let (mut bare, _, _) = searching(honest());
    bare.runtime.search = None;
    let result = proof(&bare, true).expect("a result");
    assert_eq!(detail(&result), "search is not proven to stay inside the packet: this runner has no search server");
    // A server that reaches past the packet.
    let (leaking, _, _) = searching(Box::new(|command| Ok(ok(&serve(stdin_of(command), Some(2))))));
    assert_eq!(detail(&proof(&leaking, true).expect("a result")), "search is not proven to stay inside the packet: call 2 returned text from outside the packet");
    // An agent file that keeps the critic from searching.
    let (blocked, _, _) = searching(honest());
    write(&format!("{}/home/.vibe/agents/critic-mistral-small.toml", blocked.sandbox.text()), "enable_mcp = false\n");
    let runtime = &blocked.runtime;
    let result = setup::run(&input(r#"{"action":"check","check":"read-only:vibe-mistral-small","confirmed":true}"#), runtime).expect("a result");
    assert!(detail(&result).starts_with("agent-file-disables-mcp: "), "{}", detail(&result));
    // A server that proves itself but will not start on the critic's own copy.
    let (down, _, _) = searching(Box::new(|command| {
        if command.cwd.as_deref().is_some_and(|cwd| cwd.ends_with("read-only/packet")) {
            Err("boom".to_string())
        } else {
            Ok(ok(&serve(stdin_of(command), None)))
        }
    }));
    assert_eq!(detail(&proof(&down, true).expect("a result")), "search is not proven to stay inside the packet: the search server did not start: boom");
}

#[test]
fn a_run_that_does_not_show_the_refusals_or_changes_its_packet_is_not_proven() {
    let (partial, _, _) = searching(planting(r#",{"type":"effect","detail":{"toolName":"file_system.bash"},"state":{"status":"completed"}}"#, None));
    assert_eq!(
        detail(&proof(&partial, true).expect("a result")),
        "run_typescript is not proven to reach only the search tools: file_system.bash came back completed, not tool_denied; file_system.write_file was not tried, so its refusal is not shown; process.start was not tried, so its refusal is not shown; mcp_packet.grep did not complete through run_typescript (charged $0.0500)"
    );
    let (planted, _, _) = searching(planting(DENIED, Some("stray.md")));
    assert_eq!(
        detail(&proof(&planted, true).expect("a result")),
        "run_typescript is not proven to reach only the search tools: the critic changed its packet (charged $0.0500)"
    );
    // A packet left over from an earlier check is emptied first.
    let (stale, _, _) = searching(planting(DENIED, None));
    write(&format!("{}/home/.local/share/agent-kit/critique/setup-checks/vibe-mistral-small/read-only/packet/old/leftover.md", stale.sandbox.text()), "old");
    assert_eq!(text(&proof(&stale, true).expect("a result"), &["item", "last", "result"]), "pass");
}

#[test]
fn a_run_that_fails_or_cannot_start_is_the_checks_failure_or_the_runs_error() {
    let failing: Handler = Box::new(|command| {
        if command.command == "/opt/critique" {
            Ok(ok(&serve(stdin_of(command), None)))
        } else {
            Ok(ProcessResult { exit_code: 3, stderr: "boom\n".to_string(), ..ok("") })
        }
    });
    let (failed, _, _) = searching(failing);
    let result = proof(&failed, true).expect("a result");
    assert_eq!(text(&result, &["item", "last", "result"]), "fail");
    assert!(detail(&result).starts_with("stopped-early: "), "{}", detail(&result));
    assert!(detail(&result).ends_with("(charged $0.0500)"), "{}", detail(&result));
    let refusing: Handler = Box::new(|command| if command.command == "/opt/critique" { Ok(ok(&serve(stdin_of(command), None))) } else { Err("spawn vibe EACCES".to_string()) });
    let (refused, _, kills) = searching(refusing);
    assert_eq!(proof(&refused, true).err().map(|error| error.message), Some("spawn vibe EACCES".to_string()));
    // The server is stopped even though the launch failed.
    assert!(kills.lock().expect("kills").iter().any(|kill| kill[0] == "pkill"));
}

#[test]
fn a_probe_whose_client_cannot_be_launched_is_the_runs_error() {
    let (process, _, _) = super::search_wiring::scripted(Box::new(|_| Err("spawn vibe EACCES".to_string())));
    let mut run = run_in(process, vec![], &[]);
    let root = run.sandbox.text();
    write(&format!("{root}/bin/vibe"), "");
    write(&format!("{root}/home/.vibe/.env"), "MISTRAL_API_KEY=sk\n");
    write(&format!("{root}/home/.vibe/agents/critic-mistral-small.toml"), "# agent\n");
    run.runtime.env.insert("PATH".to_string(), format!("{root}/bin"));
    let result = setup::run(&input(r#"{"action":"check","check":"critic:vibe-mistral-small","confirmed":true}"#), &run.runtime);
    assert_eq!(result.err().map(|error| error.message), Some("spawn vibe EACCES".to_string()));
}

#[test]
fn a_client_is_known_by_its_binary_and_its_version_line() {
    let codex = CriticClient::Codex(codex_config());
    let claude = CriticClient::Claude(claude_config());
    assert_eq!((client_binary(&codex), client_binary(&claude)), ("codex", "claude"));
    let version = |stdout: &str, code: i32, timed_out: bool| {
        let process = FakeProcess::answering(ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: code, timed_out });
        let run = run_in(Box::new(process), vec![], &[]);
        client_version(&run.runtime, &codex)
    };
    assert_eq!(version(" codex 1.2.3 \nmore\n", 0, false).as_deref(), Some("codex 1.2.3"));
    assert_eq!(version("\n", 0, false), None);
    assert_eq!(version("codex 1\n", 1, false), None);
    assert_eq!(version("codex 1\n", 0, true), None);
    let refusing = run_in(Box::new(FakeProcess::refusing("spawn codex ENOENT")), vec![], &[]);
    assert_eq!(client_version(&refusing.runtime, &claude), None);
    // A work client that is not on PATH is not signed in.
    let missing = run_in(Box::new(FakeProcess::answering(exit(0))), vec![], &[]);
    for config in [&codex, &claude] {
        let readiness = client_sign_in(&missing.runtime, config).expect("a readiness");
        assert_eq!((readiness.ready, readiness.detail.is_some_and(|detail| detail.ends_with("is not on PATH"))), (false, true));
    }
}

#[test]
fn codex_records_a_hooks_trust_under_the_key_it_names_it_by() {
    assert_eq!(state_table_key(r#"[hooks.state."a/b.json:pre_tool_use:0:0"]"#), Some("a/b.json:pre_tool_use:0:0"));
    assert_eq!(state_table_key(r#"[ hooks.state."k"  ]"#), Some("k"));
    assert_eq!(state_table_key(r#"[hooks.state."a\"b\\"]"#), Some(r#"a\"b\\"#));
    for line in [r#"hooks.state."k"]"#, r#"[hooks.state."k""#, r#"[hooks.state."k" x]"#, r#"[hooks.state."k\"]"#, "[hooks.state.\"k\\", "[hooks.state.\"k\\\u{2028}\"]", r#"[other."k"]"#, r#"[hooks.state."k]"#] {
        assert_eq!(state_table_key(line), None, "{line}");
    }
    let states = codex_hook_states(
        "[hooks.state.\"a\"]\ntrusted_hash = \"sha256:x\"\nenabled = true\n[other]\ntrusted_hash = \"lost\"\n[hooks.state.\"b\"]\nenabled = false\nenabled = truex\ntrusted_hash = unquoted\ntrusted_hash   =   \"\"\n[hooks.state.\"c\"]\nenabled = true_\n",
    );
    let a = &states["a"];
    assert_eq!((a.trusted_hash.as_deref(), a.enabled), (Some("sha256:x"), Some(true)));
    let b = &states["b"];
    assert_eq!((b.trusted_hash.as_deref(), b.enabled), (Some(""), Some(false)));
    assert_eq!(states["c"].enabled, None);
    assert_eq!(states.len(), 3);
}

#[test]
fn a_hook_is_wired_by_the_script_its_command_names() {
    let file = input(
        r#"{"hooks":{"PreToolUse":[1,{"matcher":"Bash","hooks":5},{"matcher":"Bash","hooks":[{"command":"x"},{"command":"\"/opt/h/critique-pr-gate.sh\" --flag"}]},{"hooks":[{"command":"critique-commit-gate.sh"}]}],"SessionStart":[{"hooks":[{"command":"a/critique-session-start.sh"}]}]}}"#,
    );
    let wired: Vec<String> = wired_hooks(&file).iter().map(|hook| format!("{hook:?}")).collect();
    assert_eq!(wired.len(), 2, "{wired:?}");
    assert!(wired[0].contains("critique-pr-gate") && wired[0].contains("group_index: 2") && wired[0].contains("handler_index: 1") && wired[0].contains("/opt/h/critique-pr-gate.sh"));
    assert!(wired[1].contains("critique-session-start") && wired[1].contains("matcher: None"));
    assert!(wired_hooks(&Value::Null).is_empty());
    assert!(wired_hooks(&input(r#"{"hooks":5}"#)).is_empty());
}

#[test]
fn a_runner_file_that_cannot_be_read_stops_the_listing() {
    let run = run_in(Box::new(FakeProcess::answering(exit(0))), vec![], &[]);
    std::fs::create_dir_all(format!("{}/assets/critique.mjs", run.sandbox.text())).expect("a directory where the runner goes");
    let result = setup::run(&input("{}"), &run.runtime);
    assert_eq!(result.err().map(|error| error.message), Some(format!("cannot read {}/assets/critique.mjs", run.sandbox.text())));
}
