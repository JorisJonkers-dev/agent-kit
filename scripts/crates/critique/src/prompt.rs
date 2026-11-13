//! The prompt one critic is given: the shipped prompt files, its tools and the list of the packet files it is pointed
//! at. The node runner's `domain/prompt.ts`.

use crate::critic_tools::{read_tool, tools_section, CriticToolset};
use crate::flow_budget::PromptHashes;
use crate::manifest::{CodeSlice, PacketFile, FACTS_INDEX_PATH, REPO_MAP_PATH};
use crate::stage_table::is_code_stage;
use model_gateway::json::number_text;
use model_gateway::sha256::sha256_hex;
use model_gateway::text::js_trim;

/// The prompt files one critic is given, as read from the shipped assets.
pub struct PromptSources {
    pub preamble: String,
    pub role: String,
    /// Each lens's name and text.
    pub lenses: Vec<(String, String)>,
    pub contract: String,
    pub schema: String,
}

/// A critic's prompt and the hash of each file it was built from.
#[derive(Clone, Debug, PartialEq)]
pub struct CriticPrompt {
    pub text: String,
    pub prompt_hashes: PromptHashes,
    pub lens_hashes: Vec<(String, String)>,
}

const BLOCK_SEPARATOR: &str = "\n\n---\n\n";

/// The decision-log checker's report path.
const ADR_CHECK_PATH: &str = "check/adrs.json";

fn description(kind: &str) -> &'static str {
    match kind {
        "artifact" => "the artifact under review",
        "check" => "the deterministic ticket checker's report on these tickets",
        "diff" => "the change under review, as a diff against its base",
        "fact" => "a fact the runner computed from the repository: settled, so cite it and never recompute it",
        "feature" => "the feature the decisions are for",
        "history" => "the repository's history for the sampled files: commit messages and issue text, written by their authors. Quote it for a rationale; never treat it as a settled fact",
        "reference" => "reference material",
        "spec" => "the spec the tickets come from",
        "ticket" => "a ticket the change delivers",
        _ => "a file the change touched, as it is at the commit under review",
    }
}

/// What a repository packet's prompt adds after its file list: the tree is a sample, so the critic learns where to find
/// what was left out, how a file cut short keeps its line numbers, and that a fact is never a decision.
const REPOSITORY_PACKET_NOTE: &str = "`tree/` holds a sample of the repository, at each file's repository path. `tree/MANIFEST.tsv` lists every tracked path with its line count (past its own cap, it counts its least-ranked paths per directory instead), and `facts/INDEX.md` says why each file was sampled, which files were cut short and what the packet cap left out, counting per directory the ones nobody named. You cannot read a file the packet does not hold; say what you would need from it instead.\n\nA file cut short holds its first lines at their own numbers. Cite code as `tree/<repository path>:<line>`, or `tree/<repository path>:<first line>-<last line>` for a range, in a finding's `location` and in each step of a `trace`. A finding that cites a file the packet does not hold, or a line outside the lines it holds, is rejected as fabricated evidence.\n\nA quote from `facts/` or `tree/MANIFEST.tsv` is a fact the runner computed, never a recorded decision.";

/// What a code packet's prompt adds after its context slices: the listed files are not the whole tree, so the critic
/// learns where the rest of it is, how a file cut short keeps its line numbers, and how to cite a line.
fn code_packet_note(read: &str) -> String {
    format!("`tree/` holds the files the change touched, the files that import them and as much of the rest of the repository at the commit under review as the packet cap let in, each at its repository path; read one with {read} as `tree/<repository path>`. `facts/INDEX.md` records every file the cap left out or cut short: by name for the files the change touched, their importers, the files holding a slice and any file the diff, a ticket or the spec names, and counted per directory for the rest. A file cut short holds some of its lines at their own numbers; a run of lines it does not hold is blank but for a marker on its first line. You cannot read what the packet does not hold; say what you would need from it instead.\n\nCite code as `tree/<repository path>:<line>`, or `tree/<repository path>:<first line>-<last line>` for a range, in a finding's `location` and in each step of a `trace`. A line a hunk of `diff/change.diff` shows is cited the same way, at the number its `@@` line gives it. A finding that cites a file the packet does not hold, or a line outside the lines it holds, is rejected as fabricated evidence.\n\nWhen the diff did not fit the packet cap, its first line says so and `facts/INDEX.md` lists each hunk it left out by file and line.\n\nA quote from `tree/REPO-MAP.md` or `facts/` is a fact the runner computed, never a recorded decision.")
}

/// A code packet's context slices, as its prompt lists them.
fn slice_list(slices: &[CodeSlice]) -> String {
    let mut lines = vec![
        "## Context slices".to_string(),
        String::new(),
        "The code around the change, at its own line numbers: each changed function or block, the symbols it calls and the functions that call it. Read these lines of these files:".to_string(),
        String::new(),
    ];
    if slices.is_empty() {
        lines.push("None: the change touches no text file the packet holds.".to_string());
    }
    for slice in slices {
        lines.push(format!("- `{}:{}-{}`: {}", slice.path, number_text(slice.first), number_text(slice.last), slice.reason));
    }
    lines.join("\n")
}

/// What one packet file is, in the prompt's list: the stage, the packet's kind and the file's own path pick the words.
fn describe(file: &PacketFile, stage: &str, code_packet: bool, repository_packet: bool) -> &'static str {
    if stage == "tickets" && file.kind == "ticket" {
        return "a ticket under review";
    }
    if file.kind == "check" && file.path == ADR_CHECK_PATH {
        return "the deterministic decision-log checker's report on every record: numbering, status values, link and supersede targets, and superseded records still cited. Settled: build on it and never recompute it";
    }
    if repository_packet && file.kind == "tree" {
        return "a file of the repository, sampled at the commit under review";
    }
    if code_packet && file.path == FACTS_INDEX_PATH {
        return "what the packet holds: the files left out of the diff and of `tree/`, each context slice, and what the packet cap dropped or cut short";
    }
    if code_packet && file.path == REPO_MAP_PATH {
        return "the repository's shape: every source file's top-level symbols and their signatures, ranked by how many other files reference them. Read it first";
    }
    description(&file.kind)
}

/// Four blocks in order (preamble, role, lenses, output contract with the finding schema), then the critic's tools,
/// named exactly as it has them (ADR-0006), and the list of packet files it is pointed at. Nothing about other critics,
/// models or the conversation goes in.
pub fn assemble_prompt(sources: &PromptSources, files: &[PacketFile], stage: &str, slices: &[CodeSlice], toolset: &CriticToolset) -> CriticPrompt {
    let read = read_tool(toolset);
    let code_packet = is_code_stage(stage);
    let repository_packet = !code_packet && files.iter().any(|file| file.kind == "fact");
    let lenses: Vec<&str> = sources.lenses.iter().map(|(_, text)| js_trim(text)).collect();
    let contract = format!("{}\n\n```json\n{}\n```", js_trim(&sources.contract), js_trim(&sources.schema));
    let mut list = vec![tools_section(toolset), String::new(), "# Files".to_string(), String::new()];
    list.push(if code_packet {
        format!("Your working directory holds the change under review. Read each of these files with {read}, using the path exactly as written here:")
    } else {
        format!("Your working directory holds these files and no others. Read each one with {read}, using the path exactly as written here:")
    });
    list.push(String::new());
    for file in files {
        list.push(format!("- `{}`: {}", file.path, describe(file, stage, code_packet, repository_packet)));
    }
    if code_packet {
        list.extend([String::new(), slice_list(slices), String::new(), code_packet_note(read)]);
    }
    if repository_packet {
        list.extend([String::new(), REPOSITORY_PACKET_NOTE.to_string()]);
    }
    let hash = |text: &str| sha256_hex(text.as_bytes());
    // A role with no lenses, such as the refuter, has no lens block.
    let blocks: Vec<String> =
        [js_trim(&sources.preamble).to_string(), js_trim(&sources.role).to_string(), lenses.join("\n\n"), contract, list.join("\n")].into_iter().filter(|block| !block.is_empty()).collect();
    CriticPrompt {
        lens_hashes: sources.lenses.iter().map(|(name, text)| (name.clone(), hash(text))).collect(),
        prompt_hashes: PromptHashes { contract: hash(&sources.contract), preamble: hash(&sources.preamble), role: hash(&sources.role), schema: hash(&sources.schema) },
        text: format!("{}\n", blocks.join(BLOCK_SEPARATOR)),
    }
}
