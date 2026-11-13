//! The tools one critic has (ADR-0006), which its prompt names exactly: the
//! toolset, the names it gives, the prompt's `# Tools` section and the calls
//! setup's gateway proof makes. The node runner's `domain/critic-tools.ts`.

use crate::packet_search::{SearchLimits, SearchToolName};
use model_gateway::json::number_text;

/// How a client starts the runner's search server: a command line, its root among the arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchServerLaunch {
    pub command: String,
    pub args: Vec<String>,
}

/// The tools one critic has (ADR-0006), which its prompt names exactly. A Vibe critic reads with `read_file`, and
/// searches through the runner's search server when the server proved it cannot see past the packet and started on
/// this critic's copy; otherwise it reads alone. An agy critic reads with `view_file`, and searches with agy's own
/// `list_dir`, `grep_search` and `find_by_name` only once setup has proven agy read-only on this machine (#178).
#[derive(Clone, Debug, PartialEq)]
pub enum CriticToolset {
    /// A Vibe critic that reads alone.
    VibeRead,
    /// A Vibe critic that reads and searches through the runner's search server.
    VibeSearch { search: Vec<SearchToolName>, limits: SearchLimits, server: SearchServerLaunch },
    /// An agy critic, with agy's own search tools only once setup proved them read-only.
    Agy { search: bool },
}

/// The tools of a critic that reads alone.
pub const READ_ONLY_TOOLSET: CriticToolset = CriticToolset::VibeRead;

/// One tool call as a critic's journal records it: the tool, how it ended, and the error code when it failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub tool: String,
    pub status: String,
    pub code: Option<String>,
}

/// The name the runner's search server goes by in Vibe; its tools are `tools.mcp_packet.<tool>` there.
pub const VIBE_SEARCH_SERVER: &str = "packet";

/// agy's own read-only tools a critic is told about: one to read and three to list and search.
pub const AGY_TOOLS: [&str; 4] = ["view_file", "list_dir", "grep_search", "find_by_name"];

/// The tool names the critic's prompt gives, in that order.
pub fn tool_names(toolset: &CriticToolset) -> Vec<String> {
    match toolset {
        CriticToolset::Agy { search: true } => AGY_TOOLS.iter().map(|tool| (*tool).to_string()).collect(),
        CriticToolset::Agy { search: false } => vec!["view_file".to_string()],
        CriticToolset::VibeRead => vec!["read_file".to_string()],
        CriticToolset::VibeSearch { search, .. } => {
            let mut names = vec!["read_file".to_string(), "run_typescript".to_string()];
            names.extend(search.iter().map(|tool| format!("mcp_{VIBE_SEARCH_SERVER}.{}", tool.name())));
            names
        }
    }
}

/// The tool a critic reads one file with.
pub fn read_tool(toolset: &CriticToolset) -> &'static str {
    match toolset {
        CriticToolset::VibeRead | CriticToolset::VibeSearch { .. } => "read_file",
        CriticToolset::Agy { .. } => "view_file",
    }
}

const SWITCHED_OFF: &str = "You cannot write, edit, run commands, fetch anything or load skills: those tools are switched off, and calling them only wastes your turns.";

const SEARCH_FIRST: &str =
    "Search before you read: a search answer costs far less than a whole file, so read a file in full only when you need more than the lines around a match.";

/// The signature the prompt shows for one search tool: its doc comment and its declaration.
fn signature(tool: SearchToolName) -> &'static str {
    match tool {
        SearchToolName::Grep => "  /** Every line matching a JavaScript regular expression, as `path:line: text`; `path` limits the search to one file or directory, `glob` to matching paths. */\n  function grep(args: { pattern: string; path?: string; glob?: string; ignore_case?: boolean }): Promise<string>",
        SearchToolName::List => "  /** Every file path matching a glob, such as `tree/src/*.ts`, where `**` is any number of directories and `*` any characters but a slash. */\n  function list(args: { glob: string }): Promise<string>",
        SearchToolName::Symbol => "  /** Where a top-level symbol of `tree/REPO-MAP.md` is defined, as `tree/<path>:<line>`, with its kind and signature. */\n  function symbol(args: { name: string }): Promise<string>",
    }
}

/// The prompt's `# Tools` section: exactly the tools this critic has, and how to call them.
pub fn tools_section(toolset: &CriticToolset) -> String {
    match toolset {
        CriticToolset::VibeRead => format!(
            "# Tools\n\nYou have one tool: `read_file`, which reads one file of your working directory, given its path as listed below. You cannot search: no search tool is switched on for you. {SWITCHED_OFF}"
        ),
        CriticToolset::Agy { search: false } => format!(
            "# Tools\n\nYou have one tool: `view_file`, which reads one file of your working directory, given its path as listed below. You cannot search: listing directories and searching files are switched off for you. {SWITCHED_OFF}"
        ),
        CriticToolset::Agy { search: true } => format!(
            "# Tools\n\nYour tools read and search your working directory and see nothing outside it: `view_file` reads one file, `list_dir` lists a directory, `grep_search` searches the contents of files and `find_by_name` finds files by name. {SEARCH_FIRST}\n\n{SWITCHED_OFF}"
        ),
        CriticToolset::VibeSearch { search, limits, .. } => {
            let server = format!("mcp_{VIBE_SEARCH_SERVER}");
            let signatures: Vec<&str> = search.iter().map(|tool| signature(*tool)).collect();
            format!(
                "# Tools\n\nYou have `read_file`, which reads one file of your working directory given its path, and these search functions over your working directory, which you call from `run_typescript` as `tools.{server}.<function>`:\n\n```typescript\ndeclare namespace tools.{server} {{\n{}\n}}\n```\n\nFor example, call `run_typescript` with `{{\"code\": \"async function main() {{\\n  return await tools.{server}.grep({{ pattern: 'TODO', path: 'tree' }})\\n}}\"}}`.\n\nThe functions see nothing outside your working directory. An answer holds at most {} matches and {} bytes; past that it ends by saying how many were left out, so search narrower rather than wider. {SEARCH_FIRST}\n\n{SWITCHED_OFF}",
                signatures.join("\n"),
                number_text(limits.max_matches),
                number_text(limits.max_bytes)
            )
        }
    }
}

/// What setup's read-only proof has a Vibe critic try through `run_typescript` (ADR-0006, amended): a shell command,
/// a file write and a process start, each of which must come back `tool_denied`, and one search, which must complete.
/// The search shows the gateway works; the refusals show it reaches nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayProof {
    pub code: &'static str,
    pub refused: [&'static str; 3],
    pub searched: &'static str,
}

pub const GATEWAY_PROOF: GatewayProof = GatewayProof {
    code: "async function main() {\n  return await Promise.allSettled([\n    tools.file_system.bash({ command: 'echo probe > probe.txt' }),\n    tools.file_system.write_file({ path: 'probe.txt', content: 'probe' }),\n    tools.process.start({ command: 'echo probe' }),\n    tools.mcp_packet.grep({ pattern: 'probe' }),\n  ])\n}",
    refused: ["file_system.bash", "file_system.write_file", "process.start"],
    searched: "mcp_packet.grep",
};

/// Why the critic's tool calls do not show the gateway refusing all but search; none when they do.
pub fn gateway_problems(calls: &[ToolCall]) -> Vec<String> {
    let mut problems = Vec::new();
    for tool in GATEWAY_PROOF.refused {
        match calls.iter().find(|entry| entry.tool == tool) {
            None => problems.push(format!("{tool} was not tried, so its refusal is not shown")),
            Some(call) => {
                if call.status != "failed" || call.code.as_deref() != Some("tool_denied") {
                    let code = call.code.as_ref().map(|code| format!(" ({code})")).unwrap_or_default();
                    problems.push(format!("{tool} came back {}{code}, not tool_denied", call.status));
                }
            }
        }
    }
    let searched = calls.iter().any(|entry| entry.tool == GATEWAY_PROOF.searched && entry.status == "completed");
    if !searched {
        problems.push(format!("{} did not complete through run_typescript", GATEWAY_PROOF.searched));
    }
    problems
}
