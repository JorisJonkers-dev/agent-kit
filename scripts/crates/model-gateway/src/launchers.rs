//! How the gateway starts each client and reads what it printed: the exact
//! command of one launch, the isolation its environment carries, and the
//! parser of its output. The builders are pure, so a test reads a command
//! without starting anything.

use crate::extras::{AttachedServer, SandboxProfile};
use crate::json::{self, Value};
use crate::outputs::{agy_output, claude_output, codex_output, hermes_output, open_code_output, vibe_output, ClientOutput};
use crate::ports::ProcessCommand;
use crate::run::{RunClient, RunMode};
use crate::text::join;
use std::collections::BTreeMap;

/// The mark the gateway puts in every delegate's environment, and refuses to run under.
pub const DELEGATION_MARK: &str = "AGENT_KIT_DELEGATION";

/// The Vibe agent a read-only delegation runs through: read tools only, no subagents, nothing outside its workdir.
pub const READ_ONLY_AGENT: &str = "delegate-readonly";

/// The Vibe agent a writing delegation runs through: it reads, searches, writes and edits files, and runs no shell.
pub const WRITE_AGENT: &str = "delegate-write";

/// Seconds the outer timer allows beyond a client's own time limit, so the client's own stop is the one that reports.
const OUTER_TIMEOUT_GRACE_S: f64 = 60.0;

/// What a Vibe run's environment holds: the isolation Vibe reads from `VIBE_*`
/// variables, which sits above the home's `config.toml` and below the agent file.
pub const VIBE_ENV: [(&str, &str); 4] =
    [("VIBE_ENABLE_MCP", "false"), ("VIBE_ENABLE_SUBAGENTS", "false"), ("VIBE_INCLUDE_PROJECT_CONTEXT", "false"), ("VIBE_OUTSIDE_WORKDIR", "deny")];

/// The variables a Vibe launch unsets, so a run started from a `vibe-study` or
/// `vibe-private` session neither spends on the Study account nor runs in the private home.
pub const VIBE_UNSET: [&str; 4] = ["-u", "MISTRAL_API_KEY", "-u", "VIBE_HOME"];

/// The variables a Vibe launch sets itself: the isolation above, the servers it
/// attaches, and the two it unsets. A caller's `extras.env` may not name any of
/// them, so the launcher's own isolation always wins.
pub const VIBE_PROTECTED_ENV: [&str; 7] = [
    "MISTRAL_API_KEY",
    "VIBE_ENABLE_MCP",
    "VIBE_ENABLE_SUBAGENTS",
    "VIBE_HOME",
    "VIBE_INCLUDE_PROJECT_CONTEXT",
    "VIBE_MCP_SERVERS",
    "VIBE_OUTSIDE_WORKDIR",
];

/// How long a client's own MCP server may take to start, and one of its tools to answer.
const MCP_STARTUP_TIMEOUT_S: f64 = 10.0;
const MCP_TOOL_TIMEOUT_S: f64 = 30.0;

/// agy's own tool families, in the order its deny list names them. A read-only
/// run denies every family the caller did not name among the tools it may use.
const AGY_TOOL_FAMILIES: [&str; 8] = ["mcp", "read_url", "execute_url", "command", "write_file", "list_dir", "grep_search", "find_by_name"];

/// Where one launch reads its prompt and writes what the client exports, and
/// the directory the client works in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchPaths {
    pub export_dir: String,
    pub prompt_path: String,
    /// The run's own directory, where a client that writes a report of its own
    /// (Hermes' usage) puts it. A caller that names none gets the export directory.
    pub run_dir: Option<String>,
    /// The directory the client runs in: the repository, a throwaway worktree of it, or an empty directory.
    pub work_dir: String,
    /// The git directories a writing run's client may write besides the worktree.
    /// A read-only run never gets them.
    pub git_writable: Option<Vec<String>>,
}

/// What one launch needs of its input: the model, what the run may do, the
/// provider a client that runs on one uses, and the limits the client enforces itself.
#[derive(Clone, Debug, PartialEq)]
pub struct LaunchRequest {
    pub model: String,
    pub time_limit_s: f64,
    pub max_price_usd: Option<f64>,
    /// What the run may do: a writing run's client gets its editing profile.
    pub mode: Option<RunMode>,
    /// The provider a client that runs on one uses; Hermes names it on its command line.
    pub provider: Option<String>,
}

/// What a caller adds to one launch, with the client's home servers already resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchExtras {
    pub mcp_servers: Vec<AttachedServer>,
    /// The MCP servers the client's home declares, which a run with `disable_home_mcp` switches off by name.
    pub home_servers: Vec<String>,
    pub sandbox: Option<SandboxProfile>,
    pub agent: Option<String>,
    pub tools: Vec<String>,
    pub schema: Option<String>,
    pub env: BTreeMap<String, String>,
}

/// How the gateway starts one client and reads what it printed.
pub struct ClientLauncher {
    /// The extras this client can apply; one it cannot apply is refused rather than dropped.
    pub extras: &'static [&'static str],
    /// The extras this client applies only under a sandbox profile; one named without `sandbox` is refused rather than dropped.
    pub sandboxed_extras: &'static [&'static str],
    /// The variables this launcher sets itself, which a caller's `extras.env` may not name.
    pub protected_env: &'static [&'static str],
    /// The client's non-interactive launch for one run; `prompt` is the wrapped prompt text.
    pub command: fn(&str, &LaunchRequest, &LaunchPaths, &str, Option<&LaunchExtras>) -> ProcessCommand,
    /// What the client's output (and the report it wrote, where it writes one) says about the run.
    pub read: fn(&str, Option<&Value>) -> ClientOutput,
    /// The file the client writes beside its output that `read` takes as its second argument: Vibe's
    /// `export.json`, Hermes' usage report. `None` for a client that writes none.
    pub report_path: Option<fn(&LaunchPaths) -> String>,
}

/// The launcher of every client a run can be handed to.
pub fn launcher(client: RunClient) -> &'static ClientLauncher {
    match client {
        RunClient::Vibe => &VIBE,
        RunClient::Agy => &AGY,
        RunClient::Claude => &CLAUDE,
        RunClient::Opencode => &OPENCODE,
        RunClient::Hermes => &HERMES,
        RunClient::Codex => &CODEX,
    }
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

/// The outer timer for a client with no time limit of its own: the run's limit.
fn limit_ms(input: &LaunchRequest) -> Option<u64> {
    Some((input.time_limit_s * 1000.0) as u64)
}

/// The outer timer for a client that stops itself at the limit: the limit and the grace beyond it.
fn graced_ms(input: &LaunchRequest) -> Option<u64> {
    Some(((input.time_limit_s + OUTER_TIMEOUT_GRACE_S) * 1000.0) as u64)
}

/// The caller's own variables, or none.
fn caller_env(extras: Option<&LaunchExtras>) -> BTreeMap<String, String> {
    extras.map(|extras| extras.env.clone()).unwrap_or_default()
}

/// The MCP servers a Vibe launch configures: the caller's, in its packet, and
/// each of the home's switched off by name. Vibe joins the servers of every
/// layer by name, so a home server the caller attaches itself is replaced by
/// the caller's, and every other one is replaced with a switched-off entry
/// that never starts.
fn vibe_servers(extras: &LaunchExtras, work_dir: &str) -> Vec<Value> {
    let own = extras.mcp_servers.iter().map(|server| {
        json::object(vec![
            ("args", Value::Arr(server.args.iter().map(|arg| Value::str(arg)).collect())),
            ("command", Value::str(&server.command)),
            ("cwd", Value::str(work_dir)),
            ("name", Value::str(&server.name)),
            ("startup_timeout_sec", Value::Num(MCP_STARTUP_TIMEOUT_S)),
            ("tool_timeout_sec", Value::Num(MCP_TOOL_TIMEOUT_S)),
            ("transport", Value::str("stdio")),
        ])
    });
    let off = extras
        .home_servers
        .iter()
        .filter(|name| !extras.mcp_servers.iter().any(|server| &server.name == *name))
        .map(|name| json::object(vec![("command", Value::str("false")), ("disabled", Value::Bool(true)), ("name", Value::str(name)), ("transport", Value::str("stdio"))]));
    own.chain(off).collect()
}

/// A Vibe run's environment: the caller's own variables, then its isolation and the servers it may reach, which win.
fn vibe_env(extras: Option<&LaunchExtras>, work_dir: &str) -> BTreeMap<String, String> {
    let servers = extras.map(|extras| vibe_servers(extras, work_dir)).unwrap_or_default();
    let mut env = caller_env(extras);
    for (name, value) in VIBE_ENV {
        env.insert(name.to_string(), value.to_string());
    }
    if !servers.is_empty() {
        env.insert("VIBE_ENABLE_MCP".to_string(), "true".to_string());
        env.insert("VIBE_MCP_SERVERS".to_string(), json::compact(&Value::Arr(servers)));
    }
    env
}

/// Vibe: the wrapped prompt from a file, the agent the caller names (the
/// read-only one, or the writing one for a writing run), the tools the caller
/// allows, and Vibe's own time and price limits. A run that names no agent also
/// names the client model by the alias `models` lists. A run that names an
/// agent leaves the model to that agent's file: Vibe refuses `--model` beside an
/// agent that pins one, so the policy check and the ledger line carry the model
/// the routing named, and the agent file is what runs it. Every
/// launch is the read-only profile: `MISTRAL_API_KEY` and `VIBE_HOME` unset,
/// MCP, subagents and the project's `AGENTS.md` pointer off, and nothing
/// outside the workdir.
fn vibe_command(binary: &str, input: &LaunchRequest, paths: &LaunchPaths, _prompt: &str, extras: Option<&LaunchExtras>) -> ProcessCommand {
    let agent = extras.and_then(|extras| extras.agent.as_deref());
    let mut args = strings(&VIBE_UNSET);
    args.extend(strings(&[binary, "--prompt-file", &paths.prompt_path]));
    if agent.is_none() {
        args.extend(strings(&["--model", &input.model]));
    }
    let default_agent = if input.mode == Some(RunMode::Write) { WRITE_AGENT } else { READ_ONLY_AGENT };
    args.extend(strings(&["--agent", agent.unwrap_or(default_agent), "--trust", "--workdir", &paths.work_dir, "--output", "json"]));
    for tool in extras.map(|extras| extras.tools.as_slice()).unwrap_or_default() {
        args.extend(strings(&["--enabled-tools", tool]));
    }
    args.extend(strings(&["--time-limit", &json::number_text(input.time_limit_s)]));
    if let Some(price) = input.max_price_usd {
        args.extend(strings(&["--max-price", &json::number_text(price)]));
    }
    args.extend(strings(&["--output-dir", &paths.export_dir]));
    ProcessCommand {
        command: "env".to_string(),
        args,
        cwd: Some(paths.work_dir.clone()),
        env: vibe_env(extras, &paths.work_dir),
        timeout_ms: graced_ms(input),
        stdin: None,
    }
}

fn vibe_read(stdout: &str, exported: Option<&Value>) -> ClientOutput {
    vibe_output(stdout, exported)
}

fn vibe_report(paths: &LaunchPaths) -> String {
    join(&[&paths.export_dir, "export.json"])
}

static VIBE: ClientLauncher = ClientLauncher {
    extras: &["agent", "disable_home_mcp", "env", "mcp_servers", "tools"],
    sandboxed_extras: &[],
    protected_env: &VIBE_PROTECTED_ENV,
    command: vibe_command,
    read: vibe_read,
    report_path: Some(vibe_report),
};

/// The reasoning effort a model id names by its last word (`gemini-3.8-flash-high`); medium for an id that names none.
fn effort_of(model: &str) -> &'static str {
    ["low", "medium", "high"].into_iter().find(|effort| model.ends_with(&format!("-{effort}"))).unwrap_or("medium")
}

/// The session settings a read-only agy run gets: every tool family the caller did not name is denied.
fn read_only_settings(tools: &[String]) -> String {
    let deny = AGY_TOOL_FAMILIES.iter().filter(|family| !tools.iter().any(|tool| tool == *family)).map(|family| Value::Str(format!("{family}(*)"))).collect();
    json::compact(&json::object(vec![("permissions", json::object(vec![("deny", Value::Arr(deny))]))]))
}

/// Antigravity. A run with the read-only sandbox profile prints a stream in a
/// fresh sandboxed session, with slash commands off, the schema the caller
/// gives and every tool family it did not name denied. A run without it prints
/// one JSON object in plan mode (accept-edits for a writing run), with the
/// effort its model id names. Its stdin is closed either way.
fn agy_command(binary: &str, input: &LaunchRequest, paths: &LaunchPaths, prompt: &str, extras: Option<&LaunchExtras>) -> ProcessCommand {
    let print_timeout = format!("{}s", json::number_text(input.time_limit_s));
    let args = match extras {
        Some(extras) if extras.sandbox == Some(SandboxProfile::ReadOnly) => {
            let mut args = strings(&["-p", prompt, "--output-format", "stream-json", "--model", &input.model, "--print-timeout", &print_timeout, "--disable-slash-commands", "--sandbox"]);
            if let Some(schema) = &extras.schema {
                args.extend(strings(&["--json-schema", schema]));
            }
            args.extend(strings(&["--settings", &read_only_settings(&extras.tools)]));
            args
        }
        _ => {
            let mode = if input.mode == Some(RunMode::Write) { "accept-edits" } else { "plan" };
            strings(&["--print", prompt, "--model", &input.model, "--output-format", "json", "--effort", effort_of(&input.model), "--mode", mode, "--print-timeout", &print_timeout])
        }
    };
    ProcessCommand { command: binary.to_string(), args, cwd: Some(paths.work_dir.clone()), env: caller_env(extras), timeout_ms: graced_ms(input), stdin: None }
}

fn agy_read(stdout: &str, _exported: Option<&Value>) -> ClientOutput {
    agy_output(stdout)
}

static AGY: ClientLauncher = ClientLauncher {
    extras: &["env", "sandbox", "schema", "tools"],
    sandboxed_extras: &["schema", "tools"],
    protected_env: &[],
    command: agy_command,
    read: agy_read,
    report_path: None,
};

/// Claude Code: headless print mode with the prompt on stdin, in plan mode and
/// with the read tools alone, or for a writing run in accept-edits mode with
/// the edit tools besides. It has no time limit of its own, so the outer
/// timer is the limit.
fn claude_command(binary: &str, input: &LaunchRequest, paths: &LaunchPaths, prompt: &str, extras: Option<&LaunchExtras>) -> ProcessCommand {
    let write = input.mode == Some(RunMode::Write);
    let args = strings(&[
        "-p",
        "--model",
        &input.model,
        "--output-format",
        "json",
        "--permission-mode",
        if write { "acceptEdits" } else { "plan" },
        "--allowedTools",
        if write { "Read,Grep,Glob,Edit,Write" } else { "Read,Grep,Glob" },
    ]);
    ProcessCommand { command: binary.to_string(), args, cwd: Some(paths.work_dir.clone()), env: caller_env(extras), stdin: Some(prompt.to_string()), timeout_ms: limit_ms(input) }
}

fn claude_read(stdout: &str, _exported: Option<&Value>) -> ClientOutput {
    claude_output(stdout)
}

static CLAUDE: ClientLauncher = ClientLauncher {
    extras: &["env"],
    sandboxed_extras: &[],
    protected_env: &[],
    command: claude_command,
    read: claude_read,
    report_path: None,
};

/// OpenCode: `opencode run` with the provider/model, JSON events and the
/// prompt as its message. `--standalone` keeps it on a private server rooted
/// in the directory it is started in: left to the background service, a run
/// lands in whatever checkout that service was started in. It has no time
/// limit of its own, so the outer timer is the limit.
fn opencode_command(binary: &str, input: &LaunchRequest, paths: &LaunchPaths, prompt: &str, extras: Option<&LaunchExtras>) -> ProcessCommand {
    let args = strings(&["run", "--standalone", "--model", &input.model, "--format", "json", prompt]);
    ProcessCommand { command: binary.to_string(), args, cwd: Some(paths.work_dir.clone()), env: caller_env(extras), timeout_ms: limit_ms(input), stdin: None }
}

fn opencode_read(stdout: &str, _exported: Option<&Value>) -> ClientOutput {
    open_code_output(stdout)
}

static OPENCODE: ClientLauncher = ClientLauncher {
    extras: &["env"],
    sandboxed_extras: &[],
    protected_env: &[],
    command: opencode_command,
    read: opencode_read,
    report_path: None,
};

/// Where a client that writes a report of its own puts it: the run's directory, or the export directory when the caller names none.
fn report_dir(paths: &LaunchPaths) -> &str {
    paths.run_dir.as_deref().unwrap_or(&paths.export_dir)
}

/// Hermes, one-shot: the wrapped prompt as its argument, the client model and
/// the provider this machine allows, and the usage file it writes its own
/// tokens and dollars to. Hermes loads the working directory's rules and memory
/// as it always does, so the run carries the mark that stops it delegating on
/// and nothing else. It has no time limit of its own, so the outer timer is the
/// limit.
fn hermes_command(binary: &str, input: &LaunchRequest, paths: &LaunchPaths, prompt: &str, extras: Option<&LaunchExtras>) -> ProcessCommand {
    // The node launcher writes `String(input.provider)`, so a run that names none passes the word `undefined`.
    let provider = input.provider.as_deref().unwrap_or("undefined");
    let args = strings(&["-z", prompt, "--usage-file", &hermes_report(paths), "-m", &input.model, "--provider", provider]);
    ProcessCommand { command: binary.to_string(), args, cwd: Some(paths.work_dir.clone()), env: caller_env(extras), timeout_ms: graced_ms(input), stdin: None }
}

fn hermes_read(stdout: &str, usage: Option<&Value>) -> ClientOutput {
    hermes_output(stdout, usage)
}

fn hermes_report(paths: &LaunchPaths) -> String {
    join(&[report_dir(paths), "usage.json"])
}

static HERMES: ClientLauncher = ClientLauncher {
    extras: &["env"],
    sandboxed_extras: &[],
    protected_env: &[],
    command: hermes_command,
    read: hermes_read,
    report_path: Some(hermes_report),
};

/// Codex, `exec --json`: non-interactive, the wrapped prompt on stdin, the
/// client model by `-m`, the reasoning effort its id names, and Codex's own
/// sandbox: read-only for a read-only run, workspace-write for a writing one.
/// A writing run also gets `--add-dir` for each git directory the caller names,
/// so its sandbox can write the worktree's git directory and the repository's
/// shared objects and refs, which sit outside the worktree it works in.
/// `--skip-git-repo-check` lets an inline run work in its empty directory. It
/// has no time limit of its own, so the outer timer is the limit.
fn codex_command(binary: &str, input: &LaunchRequest, paths: &LaunchPaths, prompt: &str, extras: Option<&LaunchExtras>) -> ProcessCommand {
    let write = input.mode == Some(RunMode::Write);
    let effort = format!("model_reasoning_effort=\"{}\"", effort_of(&input.model));
    let mut args = strings(&[
        "exec",
        "--json",
        "--skip-git-repo-check",
        "-m",
        &input.model,
        "-c",
        &effort,
        "-s",
        if write { "workspace-write" } else { "read-only" },
        "-C",
        &paths.work_dir,
    ]);
    if write {
        for dir in paths.git_writable.as_deref().unwrap_or_default() {
            args.extend(strings(&["--add-dir", dir]));
        }
    }
    ProcessCommand { command: binary.to_string(), args, cwd: Some(paths.work_dir.clone()), env: caller_env(extras), stdin: Some(prompt.to_string()), timeout_ms: limit_ms(input) }
}

fn codex_read(stdout: &str, _exported: Option<&Value>) -> ClientOutput {
    codex_output(stdout)
}

static CODEX: ClientLauncher = ClientLauncher {
    extras: &["env"],
    sandboxed_extras: &[],
    protected_env: &[],
    command: codex_command,
    read: codex_read,
    report_path: None,
};
