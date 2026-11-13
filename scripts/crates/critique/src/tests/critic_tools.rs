//! critic-tools' own pieces: the prompt's `# Tools` section for every toolset, the names it gives, and the gateway
//! proof's problems. The expected text is the node runner's, copied from `domain/critic-tools.ts`.

use crate::critic_tools::{
    gateway_problems, read_tool, tool_names, tools_section, CriticToolset, SearchServerLaunch, ToolCall, AGY_TOOLS,
    GATEWAY_PROOF, READ_ONLY_TOOLSET, VIBE_SEARCH_SERVER,
};
use crate::packet_search::{default_search_limits, SearchLimits, SearchToolName, SEARCH_TOOLS};

fn vibe_search(search: &[SearchToolName]) -> CriticToolset {
    CriticToolset::VibeSearch {
        search: search.to_vec(),
        limits: default_search_limits(),
        server: SearchServerLaunch { command: "critique".to_string(), args: vec!["search".to_string()] },
    }
}

#[test]
fn a_critic_that_reads_alone_is_told_which_tool_it_has() {
    assert_eq!(
        tools_section(&READ_ONLY_TOOLSET),
        r#"# Tools

You have one tool: `read_file`, which reads one file of your working directory, given its path as listed below. You cannot search: no search tool is switched on for you. You cannot write, edit, run commands, fetch anything or load skills: those tools are switched off, and calling them only wastes your turns."#
    );
    assert_eq!(
        tools_section(&CriticToolset::Agy { search: false }),
        r#"# Tools

You have one tool: `view_file`, which reads one file of your working directory, given its path as listed below. You cannot search: listing directories and searching files are switched off for you. You cannot write, edit, run commands, fetch anything or load skills: those tools are switched off, and calling them only wastes your turns."#
    );
    assert_eq!(
        tools_section(&CriticToolset::Agy { search: true }),
        r#"# Tools

Your tools read and search your working directory and see nothing outside it: `view_file` reads one file, `list_dir` lists a directory, `grep_search` searches the contents of files and `find_by_name` finds files by name. Search before you read: a search answer costs far less than a whole file, so read a file in full only when you need more than the lines around a match.

You cannot write, edit, run commands, fetch anything or load skills: those tools are switched off, and calling them only wastes your turns."#
    );
}

#[test]
fn a_searching_critic_is_told_the_functions_it_calls() {
    let toolset = vibe_search(&[SearchToolName::Grep, SearchToolName::List, SearchToolName::Symbol]);
    assert_eq!(
        tools_section(&toolset),
        r#"# Tools

You have `read_file`, which reads one file of your working directory given its path, and these search functions over your working directory, which you call from `run_typescript` as `tools.mcp_packet.<function>`:

```typescript
declare namespace tools.mcp_packet {
  /** Every line matching a JavaScript regular expression, as `path:line: text`; `path` limits the search to one file or directory, `glob` to matching paths. */
  function grep(args: { pattern: string; path?: string; glob?: string; ignore_case?: boolean }): Promise<string>
  /** Every file path matching a glob, such as `tree/src/*.ts`, where `**` is any number of directories and `*` any characters but a slash. */
  function list(args: { glob: string }): Promise<string>
  /** Where a top-level symbol of `tree/REPO-MAP.md` is defined, as `tree/<path>:<line>`, with its kind and signature. */
  function symbol(args: { name: string }): Promise<string>
}
```

For example, call `run_typescript` with `{"code": "async function main() {\n  return await tools.mcp_packet.grep({ pattern: 'TODO', path: 'tree' })\n}"}`.

The functions see nothing outside your working directory. An answer holds at most 50 matches and 8192 bytes; past that it ends by saying how many were left out, so search narrower rather than wider. Search before you read: a search answer costs far less than a whole file, so read a file in full only when you need more than the lines around a match.

You cannot write, edit, run commands, fetch anything or load skills: those tools are switched off, and calling them only wastes your turns."#
    );
}

#[test]
fn a_searching_critic_is_told_only_the_functions_it_has() {
    let toolset = CriticToolset::VibeSearch {
        search: vec![SearchToolName::List],
        limits: SearchLimits { max_matches: 3.0, max_bytes: 1000.0 },
        server: SearchServerLaunch { command: "critique".to_string(), args: Vec::new() },
    };
    assert_eq!(
        tools_section(&toolset),
        r#"# Tools

You have `read_file`, which reads one file of your working directory given its path, and these search functions over your working directory, which you call from `run_typescript` as `tools.mcp_packet.<function>`:

```typescript
declare namespace tools.mcp_packet {
  /** Every file path matching a glob, such as `tree/src/*.ts`, where `**` is any number of directories and `*` any characters but a slash. */
  function list(args: { glob: string }): Promise<string>
}
```

For example, call `run_typescript` with `{"code": "async function main() {\n  return await tools.mcp_packet.grep({ pattern: 'TODO', path: 'tree' })\n}"}`.

The functions see nothing outside your working directory. An answer holds at most 3 matches and 1000 bytes; past that it ends by saying how many were left out, so search narrower rather than wider. Search before you read: a search answer costs far less than a whole file, so read a file in full only when you need more than the lines around a match.

You cannot write, edit, run commands, fetch anything or load skills: those tools are switched off, and calling them only wastes your turns."#
    );
}

#[test]
fn the_names_a_critic_is_given_follow_its_toolset() {
    assert_eq!(tool_names(&READ_ONLY_TOOLSET), ["read_file"]);
    assert_eq!(tool_names(&CriticToolset::Agy { search: false }), ["view_file"]);
    assert_eq!(tool_names(&CriticToolset::Agy { search: true }), AGY_TOOLS);
    assert_eq!(
        tool_names(&vibe_search(&[SearchToolName::Grep, SearchToolName::Symbol])),
        ["read_file", "run_typescript", "mcp_packet.grep", "mcp_packet.symbol"]
    );
}

#[test]
fn a_critic_reads_with_the_tool_its_client_has() {
    assert_eq!(read_tool(&READ_ONLY_TOOLSET), "read_file");
    assert_eq!(read_tool(&vibe_search(&[SearchToolName::Grep])), "read_file");
    assert_eq!(read_tool(&CriticToolset::Agy { search: true }), "view_file");
    assert_eq!(read_tool(&CriticToolset::Agy { search: false }), "view_file");
}

#[test]
fn the_search_server_and_its_tools_are_named_as_the_node_runner_names_them() {
    assert_eq!(VIBE_SEARCH_SERVER, "packet");
    assert_eq!(SEARCH_TOOLS, ["grep", "list", "symbol"]);
    assert_eq!(SearchToolName::Grep.name(), "grep");
    assert_eq!(SearchToolName::List.name(), "list");
    assert_eq!(SearchToolName::Symbol.name(), "symbol");
}

#[test]
fn the_gateway_proof_is_the_node_runners() {
    assert_eq!(
        GATEWAY_PROOF.code,
        "async function main() {\n  return await Promise.allSettled([\n    tools.file_system.bash({ command: 'echo probe > probe.txt' }),\n    tools.file_system.write_file({ path: 'probe.txt', content: 'probe' }),\n    tools.process.start({ command: 'echo probe' }),\n    tools.mcp_packet.grep({ pattern: 'probe' }),\n  ])\n}"
    );
    assert_eq!(GATEWAY_PROOF.refused, ["file_system.bash", "file_system.write_file", "process.start"]);
    assert_eq!(GATEWAY_PROOF.searched, "mcp_packet.grep");
}

fn call(tool: &str, status: &str, code: Option<&str>) -> ToolCall {
    ToolCall { tool: tool.to_string(), status: status.to_string(), code: code.map(str::to_string) }
}

#[test]
fn a_gateway_that_refused_all_but_search_has_no_problems() {
    let calls = [
        call("file_system.bash", "failed", Some("tool_denied")),
        call("file_system.write_file", "failed", Some("tool_denied")),
        call("process.start", "failed", Some("tool_denied")),
        call("mcp_packet.grep", "completed", None),
    ];
    assert!(gateway_problems(&calls).is_empty());
}

#[test]
fn a_gateway_that_let_a_tool_through_says_which() {
    let calls = [
        call("file_system.bash", "completed", None),
        call("file_system.write_file", "failed", Some("tool_error")),
        call("process.start", "failed", None),
        call("mcp_packet.grep", "completed", None),
    ];
    assert_eq!(
        gateway_problems(&calls),
        [
            "file_system.bash came back completed, not tool_denied",
            "file_system.write_file came back failed (tool_error), not tool_denied",
            "process.start came back failed, not tool_denied",
        ]
    );
}

#[test]
fn a_gateway_that_was_not_tried_or_did_not_search_says_which() {
    let calls = [
        call("file_system.bash", "failed", Some("tool_denied")),
        call("file_system.write_file", "failed", Some("tool_denied")),
        call("mcp_packet.grep", "failed", None),
    ];
    assert_eq!(
        gateway_problems(&calls),
        [
            "process.start was not tried, so its refusal is not shown",
            "mcp_packet.grep did not complete through run_typescript",
        ]
    );
}
