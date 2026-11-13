//! The runner's own search server wired into the critics a run launches: the client that starts and stops it, the reach
//! proof, the tool choice and a Vibe critic's launch with it. The node runner's `adapters/search/client.ts`,
//! `application/critic-tools.ts` and the Vibe adapter's search reading.

use super::critic_run::{run_in, tools_of, Run};
use super::tripwire::write;
use super::vibe_client::vibe_config;
use crate::critic_run::{run_critic, CriticTools, Launch, ToolChoice};
use crate::critic_tools::{CriticToolset, ToolCall, READ_ONLY_TOOLSET};
use crate::packet_search::{default_search_limits, reach_problems, reach_proof_calls, SearchToolName};
use crate::prompt::{assemble_prompt, PromptSources};
use crate::roster::RosterCritic;
use crate::routing::CriticClient;
use crate::search_client::{prove_search_reach, server_launch, session, stop, SearchCommand, SearchSession};
use crate::tripwire::ReadOnlyTripwire;
use crate::vibe_search::{agent_file_search_problem, home_config_problem, search_failure, tool_calls, vibe_extras};
use model_gateway::json::{self, Value};
use model_gateway::ports::{Process, ProcessCommand, ProcessResult};
use std::sync::{Arc, Mutex};

const OUTSIDE_ANSWER: &str = "Nothing: that path is outside the packet, or is not in it.";

type Handler = Box<dyn Fn(&ProcessCommand) -> Result<ProcessResult, String> + Send + Sync>;
type Runs = Arc<Mutex<Vec<ProcessCommand>>>;
type Kills = Arc<Mutex<Vec<Vec<String>>>>;

/// A process that hands every launch to a handler and records the launches and the `pkill` patterns.
struct Scripted {
    handler: Handler,
    runs: Runs,
    kills: Kills,
}

impl Process for Scripted {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }

    fn exec(&self, command: &str, args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        let mut words = vec![command.to_string()];
        words.extend(args.iter().map(|arg| (*arg).to_string()));
        self.kills.lock().map_err(|error| error.to_string())?.push(words);
        Ok(ok(""))
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.runs.lock().map_err(|error| error.to_string())?.push(command.clone());
        (self.handler)(command)
    }
}

fn ok(stdout: &str) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false }
}

/// What an MCP server answers to the requests on stdin: every search tool, and for each call the reach proof's honest
/// answer (`leak` makes call number `leak` name a file outside the packet).
fn serve(stdin: &str, leak: Option<usize>) -> String {
    let mut out = String::new();
    for line in stdin.lines() {
        let Ok(Value::Obj(request)) = json::parse(line) else { continue };
        let Some(Value::Num(id)) = request.get("id") else { continue };
        let id = *id as usize;
        let result = match id {
            1 => json::object(Vec::new()),
            2 => json::object(vec![(
                "tools",
                Value::Arr(["grep", "list", "symbol", "extra"].iter().map(|name| json::object(vec![("name", Value::str(name))])).collect()),
            )]),
            _ => {
                let text = if id == 3 {
                    "tree/inside.md:1: reach proof: inside the packet".to_string()
                } else if leak == Some(id - 2) {
                    "escape/secret.md:1: reach proof".to_string()
                } else {
                    OUTSIDE_ANSWER.to_string()
                };
                json::object(vec![("content", Value::Arr(vec![json::object(vec![("text", Value::str(&text))])]))])
            }
        };
        out.push_str(&json::compact(&json::object(vec![("id", Value::count(id)), ("jsonrpc", Value::str("2.0")), ("result", result)])));
        out.push('\n');
    }
    out
}

fn scripted(handler: Handler) -> (Box<Scripted>, Runs, Kills) {
    let runs = Arc::new(Mutex::new(Vec::new()));
    let kills = Arc::new(Mutex::new(Vec::new()));
    (Box::new(Scripted { handler, runs: runs.clone(), kills: kills.clone() }), runs, kills)
}

fn own() -> SearchCommand {
    SearchCommand { command: "/opt/critique".to_string(), args: vec!["--flag".to_string()] }
}

fn stdin_of(command: &ProcessCommand) -> &str {
    command.stdin.as_deref().unwrap_or("")
}

fn honest() -> Handler {
    Box::new(|command| Ok(ok(&serve(stdin_of(command), None))))
}

/// A run whose search server is `handler`, with the Vibe agent file setup installs in its home.
fn searching(handler: Handler) -> (Run, Runs, Kills) {
    let (process, runs, kills) = scripted(handler);
    let mut run = run_in(process, vec![], &[]);
    run.runtime.search = Some(own());
    write(&format!("{}/home/.vibe/agents/critic-mistral-medium.toml", run.sandbox.text()), AGENT);
    (run, runs, kills)
}

const AGENT: &str = r#"enabled_tools = ["read_file", "packet_grep", "packet_list", "packet_symbol"]
[tools.packet_grep]
permission = "always"
[tools.packet_list]
permission = "always"
[tools.packet_symbol]
permission = "always"
"#;

fn vibe_critic() -> RosterCritic {
    RosterCritic {
        client: "vibe-medium".to_string(),
        role: "bug-hunter".to_string(),
        lenses: vec![],
        max_price_usd: 2.0,
        time_limit_s: 300.0,
        critic_id: "c1-vibe".to_string(),
        client_config: CriticClient::Vibe(vibe_config()),
        fallbacks: vec![],
    }
}

#[test]
fn the_server_is_the_runners_own_command_confined_to_a_root() {
    let launch = server_launch(&own(), "/p", &default_search_limits());
    assert_eq!(launch.command, "/opt/critique");
    assert_eq!(launch.args, ["--flag", "search-server", "--root", "/p", "--max-matches", "50", "--max-bytes", "8192"]);
}

#[test]
fn a_session_asks_the_calls_and_reads_the_answers() {
    let (run, runs, _) = searching(honest());
    let calls = reach_proof_calls("/o");
    let SearchSession::Started { tools, answers } = session(&run.runtime, &own(), "/p", &default_search_limits(), &calls) else {
        panic!("the session starts")
    };
    // The server also listed a tool the runner does not know.
    assert_eq!(tools, [SearchToolName::Grep, SearchToolName::List, SearchToolName::Symbol]);
    assert_eq!(answers.len(), 9);
    assert!(answers[0].contains("inside.md") && answers[1] == OUTSIDE_ANSWER);
    let seen = runs.lock().expect("runs")[0].clone();
    assert_eq!((seen.cwd.as_deref(), seen.timeout_ms), (Some("/p"), Some(20_000)));
    let stdin = seen.stdin.expect("stdin");
    let lines: Vec<&str> = stdin.lines().collect();
    assert_eq!(lines.len(), 3 + 9);
    assert_eq!(
        lines[0],
        r#"{"id":1,"jsonrpc":"2.0","method":"initialize","params":{"capabilities":{},"clientInfo":{"name":"critique","version":"1"},"protocolVersion":"2025-06-18"}}"#
    );
    assert_eq!(lines[1], r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert_eq!(lines[2], r#"{"id":2,"jsonrpc":"2.0","method":"tools/list"}"#);
    assert_eq!(lines[3], r#"{"id":3,"jsonrpc":"2.0","method":"tools/call","params":{"arguments":{"pattern":"reach proof"},"name":"grep"}}"#);
}

fn failing(handler: Handler) -> String {
    let (run, _, _) = searching(handler);
    match session(&run.runtime, &own(), "/p", &default_search_limits(), &[]) {
        SearchSession::Failed { detail } => detail,
        SearchSession::Started { .. } => panic!("the session fails"),
    }
}

#[test]
fn a_session_that_goes_wrong_says_how() {
    assert_eq!(failing(Box::new(|_| Err("spawn ENOENT".to_string()))), "the search server did not start: spawn ENOENT");
    assert_eq!(failing(Box::new(|_| Ok(ProcessResult { timed_out: true, ..ok("") }))), "the search server did not answer in time");
    assert_eq!(
        failing(Box::new(|_| Ok(ProcessResult { exit_code: 1, stderr: " critique search-server: no root\nmore\n".to_string(), ..ok("") }))),
        "the search server exited 1: critique search-server: no root"
    );
    assert_eq!(failing(Box::new(|_| Ok(ProcessResult { exit_code: 2, ..ok("") }))), "the search server exited 2");
    // Lines that are not replies, replies without a numeric id and a list without search tools offer nothing.
    let noise = "not json\n[1]\n{\"id\":\"2\",\"result\":{\"tools\":[{\"name\":\"grep\"}]}}\n\n{\"id\":2,\"result\":{\"tools\":[{\"name\":\"other\"},5]}}\n";
    assert_eq!(failing(Box::new(move |_| Ok(ok(noise)))), "the search server listed no search tools");
    assert_eq!(failing(Box::new(|_| Ok(ok("")))), "the search server listed no search tools");
    assert_eq!(failing(Box::new(|_| Ok(ok("{\"id\":2,\"result\":{\"tools\":\"grep\"}}\n")))), "the search server listed no search tools");
}

#[test]
fn an_answer_is_its_text_or_what_went_wrong_with_it() {
    let reply = |id: usize, body: &str| format!("{{\"id\":{id},{body}}}\n");
    let text = [
        "{\"id\":2,\"result\":{\"tools\":[{\"name\":\"grep\"}]}}\n".to_string(),
        reply(3, "\"error\":{\"message\":\"bad pattern\"}"),
        reply(4, "\"error\":{}"),
        reply(5, "\"result\":{\"content\":[{\"text\":\"a\"},{\"type\":\"image\"},{\"text\":\"b\"}]}"),
        reply(6, "\"result\":{\"content\":\"flat\"}"),
    ]
    .concat();
    let (run, _, _) = searching(Box::new(move |_| Ok(ok(&text))));
    let calls = reach_proof_calls("/o");
    let SearchSession::Started { answers, .. } = session(&run.runtime, &own(), "/p", &default_search_limits(), &calls[..6]) else {
        panic!("the session starts")
    };
    assert_eq!(answers, ["error: bad pattern", "error: undefined", "ab", "error: no answer", "error: no answer", "error: no answer"]);
}

#[test]
fn stopping_names_the_root_with_its_regular_expression_characters_escaped() {
    let (run, _, kills) = searching(honest());
    stop(&run.runtime, "/a.b/(c)[d]");
    assert_eq!(kills.lock().expect("kills")[0], ["pkill", "-f", "--", "search-server --root /a\\.b/\\(c\\)\\[d\\]( |$)"]);
}

#[test]
fn the_proof_calls_try_every_way_out() {
    let calls = reach_proof_calls("/o");
    let texts: Vec<String> = calls.iter().map(|call| format!("{}{}", call.tool.name(), json::compact(&call.arguments))).collect();
    assert_eq!(
        texts,
        [
            r#"grep{"pattern":"reach proof"}"#,
            r#"grep{"pattern":"reach proof","path":"../outside"}"#,
            r#"grep{"pattern":"reach proof","path":"/o"}"#,
            r#"grep{"pattern":"reach proof","path":"escape"}"#,
            r#"grep{"pattern":"reach proof","path":"escape.md"}"#,
            r#"grep{"pattern":"reach proof","glob":"../**"}"#,
            r#"list{"glob":"**"}"#,
            r#"list{"glob":"../**"}"#,
            r#"list{"glob":"/o/**"}"#,
        ]
    );
    let answers = |first: &str, later: &str| vec![first.to_string(), later.to_string(), "OUTSIDE-THE-PACKET".to_string()];
    assert!(reach_problems(&answers("inside.md", "")).contains(&"call 3 returned text from outside the packet".to_string()));
    assert_eq!(reach_problems(&answers("nothing", "secret.md")).len(), 3);
    assert_eq!(reach_problems(&[]).len(), 1);
}

#[test]
fn the_proof_lays_out_a_packet_with_its_way_out_and_proves_a_server_that_stays_inside() {
    let (run, _, _) = searching(honest());
    let dir = format!("{}/proof", run.sandbox.text());
    assert_eq!(prove_search_reach(&run.runtime, &dir, &default_search_limits()), Ok(None));
    let text = |path: &str| std::fs::read_to_string(format!("{dir}/{path}")).expect("a file");
    assert_eq!(
        (text("packet/inside.md").as_str(), text("outside/secret.md").as_str()),
        ("reach proof: inside the packet\n", "reach proof: OUTSIDE-THE-PACKET\n")
    );
    assert_eq!(std::fs::read_link(format!("{dir}/packet/escape")).expect("a link").to_string_lossy(), "../outside");
    assert_eq!(std::fs::read_link(format!("{dir}/packet/escape.md")).expect("a link").to_string_lossy(), "../outside/secret.md");
    // Proving again replaces the links, a directory in their place included.
    std::fs::remove_file(format!("{dir}/packet/escape")).expect("remove");
    std::fs::create_dir_all(format!("{dir}/packet/escape/deep")).expect("a directory");
    assert_eq!(prove_search_reach(&run.runtime, &dir, &default_search_limits()), Ok(None));
}

#[test]
fn the_proof_fails_for_a_server_that_leaks_does_not_search_or_is_not_there() {
    let (run, _, _) = searching(Box::new(|command| Ok(ok(&serve(stdin_of(command), Some(4))))));
    let dir = format!("{}/proof", run.sandbox.text());
    assert_eq!(
        prove_search_reach(&run.runtime, &dir, &default_search_limits()),
        Ok(Some("call 4 returned text from outside the packet".to_string()))
    );
    let (down, _, _) = searching(Box::new(|_| Err("spawn ENOENT".to_string())));
    assert_eq!(
        prove_search_reach(&down.runtime, &format!("{}/proof", down.sandbox.text()), &default_search_limits()),
        Ok(Some("the search server did not start: spawn ENOENT".to_string()))
    );
    let mut bare = run_in(scripted(honest()).0, vec![], &[]);
    assert_eq!(prove_search_reach(&bare.runtime, "/x", &default_search_limits()), Ok(Some("this runner has no search server".to_string())));
    // A packet the file system will not make is a proof that could not be made, whichever step fails.
    bare.runtime.search = Some(own());
    write(&format!("{}/proof/packet", bare.sandbox.text()), "a file where the packet goes");
    assert!(prove_search_reach(&bare.runtime, &format!("{}/proof", bare.sandbox.text()), &default_search_limits()).is_err());
    write(&format!("{}/proof2", bare.sandbox.text()), "a file");
    assert!(prove_search_reach(&bare.runtime, &format!("{}/proof2/x", bare.sandbox.text()), &default_search_limits()).is_err());
    // The links fail where the files could be written.
    let linked = format!("{}/proof3", bare.sandbox.text());
    assert_eq!(prove_search_reach(&bare.runtime, &linked, &default_search_limits()), Ok(None));
    let lock = |mode: u32| std::fs::set_permissions(format!("{linked}/packet"), std::os::unix::fs::PermissionsExt::from_mode(mode)).expect("permissions");
    lock(0o500);
    let unlinkable = prove_search_reach(&bare.runtime, &linked, &default_search_limits());
    lock(0o700);
    assert!(unlinkable.is_err());
}

#[test]
fn the_symlink_port_cannot_link_where_a_file_blocks_its_directory() {
    use crate::dirs::{Dirs, SystemDirs};
    let sandbox = super::scenarios::Sandbox::new();
    write(&format!("{}/blocker", sandbox.text()), "x");
    assert!(SystemDirs.symlink("t", &format!("{}/blocker/link", sandbox.text())).is_err());
    assert!(SystemDirs.symlink("t", &format!("{}/blocker", sandbox.text())).is_ok());
}

fn proven_servers(runs: &Runs) -> usize {
    runs.lock().expect("runs").iter().filter(|run| stdin_of(run).contains("tools/call")).count()
}

#[test]
fn a_vibe_critic_searches_once_the_agent_file_the_proof_and_the_server_allow_it() {
    let (run, runs, _) = searching(honest());
    let tools = tools_of(&run);
    let packet = format!("{}/run/critics/c1/packet", run.sandbox.text());
    let choice = tools.choose(&run.runtime, &vibe_critic(), &packet).expect("a choice");
    let CriticToolset::VibeSearch { search, limits, server } = &choice.toolset else { panic!("a searching toolset: {choice:?}") };
    assert_eq!((search.len(), limits, choice.search_failure.as_ref()), (3, &default_search_limits(), None));
    assert_eq!(server, &server_launch(&own(), &packet, &default_search_limits()));
    // The proof is made once for the command, whatever number of critics ask.
    tools.choose(&run.runtime, &vibe_critic(), &packet).expect("a choice");
    assert_eq!(proven_servers(&runs), 1);
    assert_eq!(runs.lock().expect("runs").len(), 3);
}

fn alone(run: &Run, why: &str) -> bool {
    let choice = tools_of(run).choose(&run.runtime, &vibe_critic(), "/packet").expect("a choice");
    choice.toolset == READ_ONLY_TOOLSET && choice.search_failure.as_deref().is_some_and(|failure| failure.starts_with(why))
}

#[test]
fn a_vibe_critic_reads_alone_and_says_why() {
    let (run, _, _) = searching(Box::new(|command| {
        let leak = if stdin_of(command).contains("tools/call") { Some(2) } else { None };
        Ok(ok(&serve(stdin_of(command), leak)))
    }));
    // The server's second answer names a file outside the packet.
    assert!(alone(&run, "search is not proven to stay inside the packet: "));
    let agent = format!("{}/home/.vibe/agents/critic-mistral-medium.toml", run.sandbox.text());
    write(&agent, "enable_mcp = false\n");
    assert!(alone(&run, "agent-file-disables-mcp: ~/.vibe/agents/critic-mistral-medium.toml sets"));
    std::fs::remove_file(&agent).expect("remove");
    assert!(alone(&run, "agent-file-missing: ~/.vibe/agents/critic-mistral-medium.toml is not installed; re-run agent-kit setup"));
    write(&agent, AGENT);
    write(&format!("{}/home/.vibe/config.toml", run.sandbox.text()), "key = \n");
    assert!(alone(&run, "home-config-unreadable: ~/.vibe/config.toml"));
    // A server that will not start for this critic's copy leaves it reading alone, in the server's own words.
    let (down, _, _) = searching(Box::new(|command| {
        if command.args.contains(&"/packet".to_string()) {
            Ok(ProcessResult { exit_code: 1, stderr: "no root\n".to_string(), ..ok("") })
        } else {
            Ok(ok(&serve(stdin_of(command), None)))
        }
    }));
    assert!(alone(&down, "the search server exited 1: no root"));
    // Without a server every Vibe critic reads alone, silently; a missing HOME is the run's error.
    let mut bare = run_in(scripted(honest()).0, vec![], &[]);
    assert_eq!(
        tools_of(&bare).choose(&bare.runtime, &vibe_critic(), "/p"),
        Ok(ToolChoice { toolset: READ_ONLY_TOOLSET, search_failure: None })
    );
    bare.runtime.search = Some(own());
    bare.runtime.env.remove("HOME");
    assert!(tools_of(&bare).choose(&bare.runtime, &vibe_critic(), "/p").is_err());
}

#[test]
fn an_unreadable_agent_or_home_file_is_the_runs_error() {
    let (run, _, _) = searching(honest());
    let agent = format!("{}/home/.vibe/agents/critic-mistral-medium.toml", run.sandbox.text());
    std::fs::remove_file(&agent).expect("remove");
    std::fs::create_dir_all(&agent).expect("a directory where the file goes");
    assert!(tools_of(&run).choose(&run.runtime, &vibe_critic(), "/p").is_err());
    std::fs::remove_dir(&agent).expect("remove");
    write(&agent, AGENT);
    let config = format!("{}/home/.vibe/config.toml", run.sandbox.text());
    std::fs::create_dir_all(&config).expect("a directory where the file goes");
    assert!(tools_of(&run).choose(&run.runtime, &vibe_critic(), "/p").is_err());
}

#[test]
fn a_proof_that_could_not_be_made_is_tried_again_by_the_next_critic() {
    let (run, runs, _) = searching(honest());
    let proof = format!("{}/run/search-proof", run.sandbox.text());
    write(&format!("{proof}/packet"), "a file where the packet goes");
    let tools = CriticTools::new(default_search_limits(), proof.clone());
    let first = tools.choose(&run.runtime, &vibe_critic(), "/packet").expect("a choice");
    assert!(first.search_failure.is_some_and(|failure| failure.starts_with("search is not proven to stay inside the packet: ")));
    std::fs::remove_file(format!("{proof}/packet")).expect("remove");
    assert!(matches!(tools.choose(&run.runtime, &vibe_critic(), "/packet").expect("a choice").toolset, CriticToolset::VibeSearch { .. }));
    assert_eq!(proven_servers(&runs), 1);
}

#[test]
fn a_proof_that_failed_is_kept_for_the_command() {
    let (run, runs, _) = searching(Box::new(|_| Err("spawn ENOENT".to_string())));
    let tools = tools_of(&run);
    for _ in 0..2 {
        let choice = tools.choose(&run.runtime, &vibe_critic(), "/packet").expect("a choice");
        assert_eq!(
            choice.search_failure.as_deref(),
            Some("search is not proven to stay inside the packet: the search server did not start: spawn ENOENT")
        );
    }
    assert_eq!(runs.lock().expect("runs").len(), 1);
}

#[test]
fn an_agent_file_must_let_a_critic_search() {
    let fix = "re-run agent-kit setup to install the current critic agents";
    let file = "~/.vibe/agents/a.toml";
    assert_eq!(agent_file_search_problem(AGENT, "a"), None);
    let problem = |toml: &str| agent_file_search_problem(toml, "a").expect("a problem");
    assert!(problem("x = ").starts_with(&format!("agent-file-unreadable: {file} is not TOML the runner can read (")));
    assert!(problem("x = ").ends_with(&format!("); {fix}")));
    assert_eq!(problem("enable_mcp = false"), format!("agent-file-disables-mcp: {file} sets enable_mcp = false, which the launch cannot override; {fix}"));
    assert_eq!(
        problem("[[mcp_servers]]\nname = \"a\"\n[[x.y]]\nmcp_servers = [{ name = \"b\" }, { }, 5]\n[z]\nmcp_servers = 1\n"),
        format!("agent-file-declares-servers: {file} declares MCP servers (a, b, ?, ?, ?) that would run beside the search server; {fix}")
    );
    let lacking = format!("agent-file-lacks-search-tools: {file} does not enable and approve packet_grep, packet_list, packet_symbol; {fix}");
    assert_eq!(problem("enabled_tools = [\"read_file\", \"packet_grep\"]\n[tools.packet_grep]\npermission = \"ask\"\n"), lacking);
    assert_eq!(problem("enabled_tools = 3\ntools = 4\n"), lacking);
    assert_eq!(home_config_problem("a = 1"), None);
    assert!(home_config_problem("a =")
        .is_some_and(|text| text.starts_with("home-config-unreadable: ~/.vibe/config.toml is not TOML the runner can read (")));
}

const JOURNAL: &str = r#"[
{"type":"message","role":"assistant","content":[{"text":"x"}]},
{"type":"effect","detail":{"toolName":"file_system.bash"},"state":{"status":"failed","error":{"code":"tool_denied","message":"no"}}},
{"type":"effect","detail":{"toolName":"mcp_packet.grep"},"state":{"status":"completed"}},
{"type":"effect","detail":{"toolName":"mcp_packet.list"},"state":{"status":"failed","error":{"code":"transport","message":"closed"}}},
{"type":"effect","detail":{"toolName":"mcp_packet.symbol"},"state":{"status":"failed","error":{"code":5}}},
{"type":"effect","detail":7},
{"type":"effect"}
]"#;

#[test]
fn the_journal_says_which_tools_were_called_and_which_searches_failed() {
    let call = |tool: &str, status: &str, code: Option<&str>| ToolCall { tool: tool.to_string(), status: status.to_string(), code: code.map(str::to_string) };
    assert_eq!(
        tool_calls(JOURNAL),
        [
            call("file_system.bash", "failed", Some("tool_denied")),
            call("mcp_packet.grep", "completed", None),
            call("mcp_packet.list", "failed", Some("transport")),
            call("mcp_packet.symbol", "failed", None),
            call("undefined", "undefined", None),
            call("undefined", "undefined", None),
        ]
    );
    assert_eq!(search_failure(JOURNAL).as_deref(), Some("2 of 3 search calls failed, the first with transport: closed"));
    let bare = "[{\"type\":\"effect\",\"detail\":{\"toolName\":\"mcp_packet.grep\"},\"state\":{\"status\":\"failed\"}}]";
    assert_eq!(search_failure(bare).as_deref(), Some("1 of 1 search calls failed, the first with undefined: undefined"));
    assert_eq!(search_failure("[]"), None);
    assert_eq!(search_failure("nope"), None);
    assert_eq!(tool_calls("nope"), []);
}

#[test]
fn a_searching_launch_attaches_the_server_and_a_reading_one_does_not() {
    let server = server_launch(&own(), "/p", &default_search_limits());
    let reading = vibe_extras("a", None);
    assert_eq!((reading.disable_home_mcp, reading.mcp_servers.is_none(), reading.tools), (None, true, Some(vec!["read_file".to_string()])));
    let searching = vibe_extras("a", Some((&[SearchToolName::Grep, SearchToolName::Symbol], &server)));
    assert_eq!(searching.tools, Some(vec!["read_file".to_string(), "packet_grep".to_string(), "packet_symbol".to_string()]));
    assert_eq!(searching.disable_home_mcp, Some(true));
    assert_eq!(searching.env.and_then(|env| env.get("VIBE_ENABLE_CONNECTORS").cloned()).as_deref(), Some("false"));
    let attached = searching.mcp_servers.expect("a server");
    assert_eq!((attached[0].name.as_str(), attached[0].command.as_str(), attached[0].args.len()), ("packet", "/opt/critique", 8));
}

fn prompt(toolset: &CriticToolset) -> crate::prompt::CriticPrompt {
    let sources = PromptSources {
        preamble: "pre".to_string(),
        role: "role".to_string(),
        lenses: vec![],
        contract: "contract".to_string(),
        schema: "{}".to_string(),
    };
    assemble_prompt(&sources, &[], "spec", &[], toolset)
}

/// Vibe as a critic in the sandbox sees it: it writes the export beside its packet copy and prints a journal.
fn vibing(searches: &str) -> Handler {
    let journal = format!(
        r#"[{{"type":"message","role":"assistant","content":[{{"text":"```json\n{{\"findings\":[],\"no_findings_reason\":\"clean\"}}\n```"}}]}}{searches}]"#
    );
    Box::new(move |command| {
        if command.command == "/opt/critique" {
            return Ok(ok(&serve(stdin_of(command), None)));
        }
        let packet = command.cwd.clone().unwrap_or_default();
        let config = format!(
            r#"{{"config":{{"enable_mcp":true,"mcp_servers":[{{"name":"packet","command":"/opt/critique","cwd":"{packet}"}}],"enabled_tools":["read_file","packet_grep","packet_list","packet_symbol"],"include_project_context":false,"outside_workdir":"deny"}}}}"#
        );
        write(&format!("{packet}/../export/export.json"), &config);
        Ok(ok(&journal))
    })
}

fn launched(run: &Run) -> crate::critic_run::CriticLaunchResult {
    let tripwire = ReadOnlyTripwire::load(&run.runtime, &run.manifest).expect("a tripwire");
    let dir = format!("{}/run/critics/c1", run.sandbox.text());
    let critic = vibe_critic();
    run_critic(&run.runtime, &Launch { tripwire: &tripwire, critic: &critic, critic_dir: &dir, prompt: &prompt, retry_of: None, tools: &tools_of(run) })
        .unwrap_or_else(|error| panic!("{}", error.message))
}

#[test]
fn a_searching_critic_runs_with_the_server_in_its_copy_and_the_server_stops_with_it() {
    let failed = r#",{"type":"effect","detail":{"toolName":"mcp_packet.grep"},"state":{"status":"failed","error":{"code":"transport","message":"closed"}}}"#;
    let (run, runs, kills) = searching(vibing(failed));
    let launched = launched(&run);
    let record = &launched.run;
    assert_eq!((record.status.as_str(), record.failure.as_deref()), ("ok", None));
    let names = ["read_file", "run_typescript", "mcp_packet.grep", "mcp_packet.list", "mcp_packet.symbol"].map(String::from);
    assert_eq!(record.tools.as_deref(), Some(&names[..]));
    assert_eq!(record.search_failure.as_deref(), Some("1 of 1 search calls failed, the first with transport: closed"));
    let packet = format!("{}/run/critics/c1/packet", run.sandbox.text());
    assert_eq!(kills.lock().expect("kills")[0][3], format!("search-server --root {}( |$)", packet.replace('.', "\\.")));
    let vibe = runs.lock().expect("runs").iter().find(|run| run.command != "/opt/critique").cloned().expect("a vibe launch");
    assert_eq!(vibe.cwd.as_deref(), Some(packet.as_str()));
    assert_eq!(vibe.env.get("VIBE_ENABLE_CONNECTORS").map(String::as_str), Some("false"));
    // The prompt was written for the tools the critic has.
    let written = std::fs::read_to_string(format!("{}/run/critics/c1/prompt.md", run.sandbox.text())).expect("prompt");
    assert!(written.contains("tools.mcp_packet.<function>"));
}

#[test]
fn a_critic_that_searched_without_a_failure_records_none_and_one_that_could_not_search_says_why() {
    let (run, _, kills) = searching(vibing(""));
    assert_eq!(launched(&run).run.search_failure, None);
    assert_eq!(kills.lock().expect("kills").len(), 1);
    // Reading alone starts no server, so there is none to stop; the agent file's problem is the run's note.
    let (blocked, _, kills) = searching(vibing(""));
    write(&format!("{}/home/.vibe/agents/critic-mistral-medium.toml", blocked.sandbox.text()), "enable_mcp = false\n");
    let record = launched(&blocked).run;
    assert!(record.search_failure.as_deref().is_some_and(|failure| failure.starts_with("agent-file-disables-mcp: ")));
    assert_eq!(record.tools.as_deref(), Some(&["read_file".to_string()][..]));
    assert!(kills.lock().expect("kills").is_empty());
}
