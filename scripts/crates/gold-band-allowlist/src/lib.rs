//! The Gold Band family allowlist check (#352): a repository, its allowlist
//! entry, and the Gold Band workflows run against it — at setup and again at
//! each run start, before any node starts (ADR-0009).
//!
//! The inputs are the repository directory (its `origin` remote names it),
//! the allowlist file (a missing file means no entries), a families file
//! mapping Gold Band agent ids to model families (one `<agent id><TAB><family>`
//! per line), and one or more workflow definition files (a bare definition,
//! or the authoring wrapper whose `modelBindings` bind each worker's agent).
//!
//! The rules, each problem one line
//! `gold-band-allowlist: <workflow file>: <node id or "workflow">: <what>`:
//! * no origin remote: refused (`repository has no remote`); a remote no
//!   `host/owner/repo` key can be derived from is refused as well;
//! * an owner of `nedap`, ASCII case-insensitively: refused for every
//!   workflow, whatever a recorded entry allows;
//! * with an entry for the repository key (`host/owner/repo`, lowercased,
//!   `.git` stripped): every agent a node binds must map to a family in the
//!   entry's `families` — a worker node binds one agent, an `ai-dynamic`
//!   node the agents its `agentStrategy` names (`family <f> is not allowed
//!   by entry <key>`); an agent id missing from the families file is
//!   refused (`agent <id> has no family`); and every workflow with at least
//!   one worker or `ai-dynamic` node sends code, so an entry with
//!   `code: false` refuses it (`entry <key> does not allow code`). The
//!   history permission is never read;
//! * without an entry: the personal default allows every family and code
//!   (ADR-0007).
//!
//! Usage: `gold-band-allowlist check --repo <DIR> --allowlist <FILE>
//! --families <FILE> <WORKFLOW>...` checks and answers;
//! `gold-band-allowlist exec --repo <DIR> --allowlist <FILE> --families
//! <FILE> --workflow <WORKFLOW> -- <COMMAND> [ARGS...]` runs the same check
//! and, only when it passes, the command (typically
//! `gold-band run --workflow <WORKFLOW> ...`), exiting with its exit code.
//! When refused, the command never starts. Exit codes: 0 clean, 1 refused,
//! 2 bad usage or unreadable input. Success prints to stdout, problems to
//! stderr.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;

/// The crate's name, which prefixes every message line.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The outside world the check reaches for: a repository's `origin` remote
/// URL. Tests use a fake; only [`GitRemote`], the real impl, runs git.
pub trait Remote {
    /// The repository's `origin` remote URL, or `None` when it has none.
    fn origin(&mut self, repo: &Path) -> Option<String>;
}

/// The outside world `exec` reaches for: starting the checked command's
/// process. Tests use a fake; only [`Proc`], the real impl, starts one.
pub trait Spawn {
    /// Runs `command` with `args` and returns the exit code it earns.
    fn spawn(&mut self, command: &OsStr, args: &[OsString]) -> std::io::Result<i32>;
}

/// The real remote source: `git remote get-url origin`.
///
/// This impl and `Proc`'s below are the ONLY code the unit tests may not
/// execute — every test reaches git and the process start through the
/// fakes — so they stay these few lines each, and `run`'s two dispatch
/// lines that pass them in go with them.
struct GitRemote;

impl Remote for GitRemote {
    fn origin(&mut self, repo: &Path) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["remote", "get-url", "origin"])
            .output()
            .ok()?;
        let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !url.is_empty()).then_some(url)
    }
}

/// The real process start: the command runs in the foreground, sharing this
/// process's stdio, and its exit code is the answer.
struct Proc;

impl Spawn for Proc {
    fn spawn(&mut self, command: &OsStr, args: &[OsString]) -> std::io::Result<i32> {
        // A signal death has no exit code; 1 is the closest honest answer.
        Ok(std::process::Command::new(command)
            .args(args)
            .status()?
            .code()
            .unwrap_or(1))
    }
}

/// One allowlist entry for a repository key. `history` and `updated_at` are
/// deliberately not fields: the history permission is never read, so whether
/// the file records it changes nothing.
#[derive(Debug, Deserialize)]
struct Entry {
    #[serde(default)]
    code: bool,
    #[serde(default)]
    families: Vec<String>,
}

/// The allowlist file: version 1 and its entries by repository key.
#[derive(Deserialize)]
struct AllowlistFile {
    version: i64,
    #[serde(default)]
    repositories: BTreeMap<String, Entry>,
}

/// One node of a workflow file; only the fields the rules read. Other
/// fields are present in the files but do not reach here.
#[derive(Deserialize)]
struct NodeDsl {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(rename = "executionSlotId", default)]
    slot: Option<String>,
    /// An `ai-dynamic` node's agent strategy, which binds its agents.
    #[serde(rename = "agentStrategy", default)]
    strategy: Option<AgentStrategyDsl>,
}

/// One of an `ai-dynamic` node's available agents: the dynamic mode binds
/// the candidate agents here, each by its `provider`.
#[derive(Deserialize)]
struct DynamicAgentDsl {
    #[serde(default)]
    provider: Option<String>,
}

/// An `ai-dynamic` node's `agentStrategy`: the fixed mode binds its one
/// agent in `provider`, the dynamic mode its candidate agents in
/// `availableAgents`.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AgentStrategyDsl {
    provider: Option<String>,
    available_agents: Vec<DynamicAgentDsl>,
}

/// One binding of the authoring wrapper's `modelBindings`: it names the
/// worker's agent by the node's `executionSlotId`.
#[derive(Deserialize)]
struct BindingDsl {
    #[serde(rename = "executionSlotId")]
    slot: String,
    #[serde(rename = "agentId")]
    agent: String,
}

/// The authoring wrapper's `modelBindings` object.
#[derive(Default, Deserialize)]
#[serde(default)]
struct ModelBindings {
    bindings: Vec<BindingDsl>,
}

/// A workflow definition: the rules read the checked nodes out of `nodes`.
#[derive(Deserialize)]
struct WorkflowDsl {
    nodes: Vec<NodeDsl>,
}

/// What one workflow file is: the authoring wrapper around a definition, or
/// a bare definition (the legacy shape).
#[derive(Deserialize)]
#[serde(untagged)]
enum Authored {
    Wrapper {
        workflow: WorkflowDsl,
        #[serde(rename = "modelBindings", default)]
        bindings: ModelBindings,
    },
    Bare(WorkflowDsl),
}

/// One worker node as the rules see it: its id and the agent id it is bound
/// to (`None` when it binds none). An `ai-dynamic` node that binds several
/// agents contributes one `Worker` per agent, each under the node's id.
#[derive(Debug, PartialEq)]
struct Worker {
    id: String,
    agent: Option<String>,
}

/// One workflow file, as the rules see it: the path shown in messages and
/// its checked nodes in declaration order.
struct File {
    shown: String,
    workers: Vec<Worker>,
}

/// Reads the allowlist: a missing file means no entries, so nothing is
/// recorded for any repository and the personal default applies.
fn load_allowlist(path: &OsStr) -> Result<BTreeMap<String, Entry>, String> {
    let shown = path.to_string_lossy();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("{shown}: cannot read: {e}")),
    };
    let allowlist: AllowlistFile =
        serde_json::from_str(&text).map_err(|e| format!("{shown}: not valid JSON: {e}"))?;
    if allowlist.version != 1 {
        return Err(format!("{shown}: version is not 1"));
    }
    Ok(allowlist.repositories)
}

/// Reads the families file: one `<agent id><TAB><family>` per line, blank
/// and `#` lines ignored.
fn load_families(path: &OsStr) -> Result<BTreeMap<String, String>, String> {
    let shown = path.to_string_lossy();
    let text = fs::read_to_string(path).map_err(|e| format!("{shown}: cannot read: {e}"))?;
    let mut families = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (agent, family) = line.split_once('\t').ok_or_else(|| {
            format!(
                "{shown}: line {}: is not <agent id><TAB><family>",
                index + 1
            )
        })?;
        families.insert(agent.trim().to_string(), family.trim().to_string());
    }
    Ok(families)
}

/// The workflow's checked nodes in declaration order: every worker node and
/// every `ai-dynamic` node, each as the bindings it contributes.
fn workers(workflow: &WorkflowDsl, bindings: &[BindingDsl]) -> Vec<Worker> {
    workflow
        .nodes
        .iter()
        .filter(|node| node.kind == "worker" || node.kind == "ai-dynamic")
        .flat_map(|node| bound_workers(node, bindings))
        .collect()
}

/// The bindings one node contributes. A worker node binds one agent: the
/// wrapper's binding for its `executionSlotId`, or else its `provider`. An
/// `ai-dynamic` node binds the agents its `agentStrategy` names — the fixed
/// mode's `provider`, then the dynamic mode's `availableAgents` — and a node
/// that names no agent still sends code, like an unbound worker.
fn bound_workers(node: &NodeDsl, bindings: &[BindingDsl]) -> Vec<Worker> {
    if node.kind == "ai-dynamic" {
        let mut agents = Vec::new();
        if let Some(strategy) = &node.strategy {
            if let Some(provider) = &strategy.provider {
                agents.extend(agent_id(provider));
            }
            for available in &strategy.available_agents {
                agents.extend(available.provider.as_deref().and_then(agent_id));
            }
        }
        if agents.is_empty() {
            return vec![Worker {
                id: node.id.clone(),
                agent: None,
            }];
        }
        return agents
            .into_iter()
            .map(|agent| Worker {
                id: node.id.clone(),
                agent: Some(agent),
            })
            .collect();
    }
    vec![Worker {
        id: node.id.clone(),
        agent: bound_agent(node, bindings).or_else(|| node.provider.as_deref().and_then(agent_id)),
    }]
}

/// The agent id the wrapper binds to the node's `executionSlotId`, when one
/// is bound.
fn bound_agent(node: &NodeDsl, bindings: &[BindingDsl]) -> Option<String> {
    node.slot
        .as_deref()
        .and_then(|slot| bindings.iter().find(|binding| binding.slot == slot))
        .map(|binding| binding.agent.as_str())
        .and_then(agent_id)
}

/// A value as a worker's agent id: trimmed, and `None` when it is blank.
fn agent_id(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Parses workflow JSON into the node bindings the rules check, applying
/// the wrapper's bindings when it is a wrapper.
fn parse_workflow(text: &str) -> Result<Vec<Worker>, serde_json::Error> {
    let authored: Authored = serde_json::from_str(text)?;
    Ok(match authored {
        Authored::Wrapper { workflow, bindings } => workers(&workflow, &bindings.bindings),
        Authored::Bare(workflow) => workers(&workflow, &[]),
    })
}

/// Reads one workflow file into the node bindings the rules check: an
/// authoring wrapper (its bindings bind the agents) or a bare workflow
/// definition (its `provider` and `agentStrategy` fields do).
fn load_workflow(path: &OsStr) -> Result<Vec<Worker>, String> {
    let shown = path.to_string_lossy();
    let text = fs::read_to_string(path).map_err(|e| format!("{shown}: cannot read: {e}"))?;
    parse_workflow(&text).map_err(|e| format!("{shown}: not a Gold Band workflow: {e}"))
}

/// The repository key derived from an origin remote URL: `host/owner/repo`
/// lowercased, a trailing `.git` stripped, for https and ssh forms — so
/// `git@github.com:JorisJonkers-dev/agent-kit.git` gives
/// `github.com/jorisjonkers-dev/agent-kit`.
fn repo_key(url: &str) -> Option<String> {
    let path = scheme_path(url).or_else(|| scp_path(url))?;
    let bare = path.trim_end_matches('/');
    let bare = bare.strip_suffix(".git").unwrap_or(bare);
    let segments: Vec<&str> = bare.split('/').collect();
    // host/owner/repo: fewer than three segments, or an empty one, names no
    // repository.
    if segments.len() < 3 || segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    Some(bare.to_ascii_lowercase())
}

/// The `host/owner/repo` path of a scheme URL (`https://`, `http://`,
/// `ssh://`), with any `user@` before the host stripped.
fn scheme_path(url: &str) -> Option<String> {
    let rest = ["https://", "http://", "ssh://"]
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix))?;
    let rest = match rest.find('@') {
        // `ssh://git@host/...`: the user ends before the host's first slash
        Some(user) if user < rest.find('/').unwrap_or(rest.len()) => &rest[user + 1..],
        _ => rest,
    };
    // `host:22/owner/repo`: a port is not part of the repository key, and a
    // key left with one would miss the recorded entry and fall through to
    // the permissive personal default.
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(':').next().unwrap_or(host);
    Some(format!("{host}/{path}"))
}

/// The `host/owner/repo` path of an scp-form URL, `git@host:owner/repo`.
fn scp_path(url: &str) -> Option<String> {
    let (user_host, path) = url.split_once(':')?;
    if user_host.contains('/') {
        return None; // a `/` before the `:` is not scp
    }
    let host = user_host.rsplit_once('@')?.1;
    Some(format!("{host}/{path}"))
}

/// Whether the key's owner segment is `nedap`, ASCII case-insensitively.
fn owned_by_nedap(key: &str) -> bool {
    key.split('/')
        .nth(1)
        .is_some_and(|owner| owner.eq_ignore_ascii_case("nedap"))
}

/// One problem line: the workflow file as named on the command line, the
/// node — `workflow` for the workflow itself — and what is wrong.
fn problem(file: &str, node: &str, what: &str) -> String {
    format!("{file}: {node}: {what}")
}

/// Every rule problem, one line per problem, in the files' command-line
/// order: no remote, a nedap owner, a family the entry does not allow, an
/// agent with no family, and an entry without code for a workflow that
/// sends code. A repository without a recorded entry gets the personal
/// default (every family, code) and refuses nothing.
fn evaluate(
    remote: &mut dyn Remote,
    repo: &Path,
    allowlist: &BTreeMap<String, Entry>,
    families: &BTreeMap<String, String>,
    files: &[File],
) -> Vec<String> {
    let url = remote.origin(repo);
    let mut problems = Vec::new();
    for file in files {
        let Some(key) = url.as_deref().and_then(repo_key) else {
            let what = match url.as_deref() {
                Some(url) => format!("repository remote {url} is not a repository URL"),
                None => "repository has no remote".to_string(),
            };
            problems.push(problem(&file.shown, "workflow", &what));
            continue;
        };
        if owned_by_nedap(&key) {
            problems.push(problem(
                &file.shown,
                "workflow",
                "nedap repositories are refused for Gold Band workflows",
            ));
            continue;
        }
        // Without a recorded entry the personal default applies and nothing
        // is refused; with one, the entry decides.
        let Some(entry) = allowlist.get(&key) else {
            continue;
        };
        for worker in &file.workers {
            let Some(agent) = worker.agent.as_deref() else {
                continue;
            };
            let Some(family) = families.get(agent) else {
                problems.push(problem(
                    &file.shown,
                    &worker.id,
                    &format!("agent {agent} has no family"),
                ));
                continue;
            };
            if !entry.families.iter().any(|allowed| allowed == family) {
                problems.push(problem(
                    &file.shown,
                    &worker.id,
                    &format!("family {family} is not allowed by entry {key}"),
                ));
            }
        }
        // Every workflow with a worker or ai-dynamic node sends code;
        // history is never required.
        if !file.workers.is_empty() && !entry.code {
            problems.push(problem(
                &file.shown,
                "workflow",
                &format!("entry {key} does not allow code"),
            ));
        }
    }
    problems
}

/// Loads every input and answers the rules for the repository: the problems
/// when refused, or `Err` on unreadable input (exit 2 territory).
fn evaluate_files(
    remote: &mut dyn Remote,
    repo: &OsStr,
    allowlist_path: &OsStr,
    families_path: &OsStr,
    workflows: &[OsString],
) -> Result<Vec<String>, String> {
    let allowlist = load_allowlist(allowlist_path)?;
    let families = load_families(families_path)?;
    let mut files = Vec::new();
    for path in workflows {
        let workers = load_workflow(path)?;
        files.push(File {
            shown: path.to_string_lossy().into_owned(),
            workers,
        });
    }
    Ok(evaluate(
        remote,
        Path::new(repo),
        &allowlist,
        &families,
        &files,
    ))
}

/// Prints every problem line, prefixed, and returns 1.
fn refused(problems: &[String]) -> i32 {
    for problem in problems {
        eprintln!("{NAME}: {problem}");
    }
    1
}

/// Prints one unreadable-input line, prefixed, and returns 2.
fn unreadable(line: &str) -> i32 {
    eprintln!("{NAME}: {line}");
    2
}

/// The `check` command line: prints the problems and returns 1, prints
/// `gold-band-allowlist: ok` and returns 0, or reports unreadable input
/// with 2.
fn check_command(
    remote: &mut dyn Remote,
    repo: &OsStr,
    allowlist: &OsStr,
    families: &OsStr,
    workflows: &[OsString],
) -> i32 {
    match evaluate_files(remote, repo, allowlist, families, workflows) {
        Ok(problems) if problems.is_empty() => {
            println!("{NAME}: ok");
            0
        }
        Ok(problems) => refused(&problems),
        Err(line) => unreadable(&line),
    }
}

/// The `exec` command line: the same check, and only when it passes does the
/// command start; its exit code is the answer. A refused or unreadable
/// workflow never starts it (exit 1, exit 2), so an entry narrowed or a
/// remote changed after setup stops the run before any node starts.
fn exec_command(
    remote: &mut dyn Remote,
    spawn: &mut dyn Spawn,
    repo: &OsStr,
    allowlist: &OsStr,
    families: &OsStr,
    workflow: &OsString,
    command: &[OsString],
) -> i32 {
    match evaluate_files(
        remote,
        repo,
        allowlist,
        families,
        std::slice::from_ref(workflow),
    ) {
        Ok(problems) if problems.is_empty() => match spawn.spawn(&command[0], &command[1..]) {
            Ok(code) => code,
            Err(e) => {
                eprintln!("{}: cannot run {}: {e}", NAME, command[0].to_string_lossy());
                2
            }
        },
        Ok(problems) => refused(&problems),
        Err(line) => unreadable(&line),
    }
}

/// What one leading `--` argument is.
enum Flag {
    Repo,
    Allowlist,
    Families,
    Workflow,
    Values,
    Positional,
    Unknown,
}

/// Classifies one argument: the four value flags, the `--` that ends flag
/// parsing, a positional value, or an unknown flag.
fn flag(arg: &OsStr) -> Flag {
    match arg.to_string_lossy().as_ref() {
        "--repo" => Flag::Repo,
        "--allowlist" => Flag::Allowlist,
        "--families" => Flag::Families,
        "--workflow" => Flag::Workflow,
        "--" => Flag::Values,
        other if other.starts_with('-') => Flag::Unknown,
        _ => Flag::Positional,
    }
}

/// Stores the flag's next argument as its value, refusing a duplicate flag
/// or one without a value.
fn set<I>(slot: &mut Option<OsString>, args: &mut I) -> Result<(), ()>
where
    I: Iterator<Item = OsString>,
{
    if slot.is_some() {
        return Err(());
    }
    *slot = Some(args.next().ok_or(())?);
    Ok(())
}

/// The parsed command line: one subcommand's arguments, in its own shape.
enum CommandLine {
    Check {
        repo: OsString,
        allowlist: OsString,
        families: OsString,
        workflows: Vec<OsString>,
    },
    Exec {
        repo: OsString,
        allowlist: OsString,
        families: OsString,
        workflow: OsString,
        command: Vec<OsString>,
    },
}

/// Parses one subcommand's arguments into its command line, or `Err` for any
/// bad usage: an unknown flag, a flag without its value or a duplicate, or a
/// shape the subcommand refuses (`check` takes its workflows as arguments;
/// `exec` takes one `--workflow` and its command after `--`).
fn parse(sub: &str, args: Vec<OsString>) -> Result<CommandLine, ()> {
    let mut repo: Option<OsString> = None;
    let mut allowlist: Option<OsString> = None;
    let mut families: Option<OsString> = None;
    let mut workflow: Option<OsString> = None;
    let mut loose: Vec<OsString> = Vec::new();
    let mut command: Vec<OsString> = Vec::new();
    let mut values = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if values {
            command.push(arg);
            continue;
        }
        match flag(&arg) {
            Flag::Values => values = true,
            Flag::Repo => set(&mut repo, &mut args)?,
            Flag::Allowlist => set(&mut allowlist, &mut args)?,
            Flag::Families => set(&mut families, &mut args)?,
            Flag::Workflow => set(&mut workflow, &mut args)?,
            Flag::Unknown => return Err(()),
            Flag::Positional => loose.push(arg),
        }
    }
    let repo = repo.ok_or(())?;
    let allowlist = allowlist.ok_or(())?;
    let families = families.ok_or(())?;
    if sub == "check" {
        if workflow.is_some() || loose.is_empty() || !command.is_empty() {
            return Err(());
        }
        Ok(CommandLine::Check {
            repo,
            allowlist,
            families,
            workflows: loose,
        })
    } else {
        let workflow = workflow.ok_or(())?;
        if !loose.is_empty() || command.is_empty() {
            return Err(());
        }
        Ok(CommandLine::Exec {
            repo,
            allowlist,
            families,
            workflow,
            command,
        })
    }
}

/// The usage text a bad command line prints on stderr.
fn usage() -> String {
    format!(
        "usage: {NAME} check --repo <DIR> --allowlist <FILE> --families <FILE> <WORKFLOW>...\n       {NAME} exec --repo <DIR> --allowlist <FILE> --families <FILE> --workflow <WORKFLOW> -- <COMMAND> [ARGS...]"
    )
}

/// Runs one command line and returns its exit code: 0 clean, 1 refused,
/// 2 bad usage or unreadable input. Success prints to stdout, problems to
/// stderr.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let sub = args.next();
    let rest: Vec<OsString> = args.collect();
    let line = match sub.as_deref() {
        Some(s) if s == OsStr::new("check") => parse("check", rest),
        Some(s) if s == OsStr::new("exec") => parse("exec", rest),
        _ => Err(()),
    };
    match line {
        Ok(CommandLine::Check {
            repo,
            allowlist,
            families,
            workflows,
        }) => check_command(&mut GitRemote, &repo, &allowlist, &families, &workflows),
        Ok(CommandLine::Exec {
            repo,
            allowlist,
            families,
            workflow,
            command,
        }) => exec_command(
            &mut GitRemote,
            &mut Proc,
            &repo,
            &allowlist,
            &families,
            &workflow,
            &command,
        ),
        Err(()) => {
            eprintln!("{}", usage());
            2
        }
    }
}

// The unit tests below execute every line of this file except the real-impl
// lines named in GitRemote's and Proc's doc comments and the two dispatch
// lines of run that pass those impls in; every rule and both commands run
// through the fakes, and the fixtures live under std::env::temp_dir().
#[cfg(test)]
mod tests {
    use super::*;

    /// The real allowlist file's one repository key.
    const KEY: &str = "github.com/jorisjonkers-dev/standards";

    /// The real allowlist file, as data: the standards repository's entry.
    const ALLOWLIST: &str = r#"{"version":1,"repositories":{"github.com/jorisjonkers-dev/standards":{"code":true,"families":["deepseek","zhipu","mistral","google"],"history":false,"updated_at":"2026-02-06"}}}"#;

    /// A families file: a comment, a blank line, three agents.
    const FAMILIES: &str =
        "# generated from the registry later\nseeker\tdeepseek\n\nreviewer\tmistral\nhermes\tgemini\n";

    /// A bare workflow definition: one worker node bound by `provider`.
    fn bare(node: &str, provider: &str) -> String {
        format!(
            r#"{{"version":"0.1","id":"flow","entry":"{node}","nodes":[{{"id":"{node}","type":"worker","provider":"{provider}","model":"m","permission_mode":"bypassPermissions","auto_accept":true}}],"edges":[{{"from":"{node}","to":"$end","on":"success","session":null}}]}}"#
        )
    }

    /// The authoring wrapper: the worker binds its agent through the
    /// `executionSlotId` binding.
    fn wrapper(node: &str, slot: &str, agent: &str) -> String {
        format!(
            r#"{{"workflow":{{"version":"0.1","id":"flow","entry":"{node}","nodes":[{{"id":"{node}","type":"worker","executionSlotId":"{slot}"}}],"edges":[{{"from":"{node}","to":"$end","on":"success","session":null}}]}},"modelBindings":{{"bindings":[{{"executionSlotId":"{slot}","agentId":"{agent}"}}]}}}}"#
        )
    }

    /// The fake remote: always the URL it was built with, recording the
    /// repository it was asked for.
    struct FakeRemote {
        url: Option<String>,
        seen: Vec<String>,
    }

    impl Remote for FakeRemote {
        fn origin(&mut self, repo: &Path) -> Option<String> {
            self.seen.push(repo.display().to_string());
            self.url.clone()
        }
    }

    /// The fake process: records what it was asked to run.
    struct FakeSpawn {
        calls: Vec<(OsString, Vec<OsString>)>,
        code: i32,
        fails: bool,
    }

    impl Spawn for FakeSpawn {
        fn spawn(&mut self, command: &OsStr, args: &[OsString]) -> std::io::Result<i32> {
            self.calls.push((command.to_os_string(), args.to_vec()));
            if self.fails {
                Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"))
            } else {
                Ok(self.code)
            }
        }
    }

    fn remote(url: Option<&str>) -> FakeRemote {
        FakeRemote {
            url: url.map(str::to_string),
            seen: Vec::new(),
        }
    }

    /// The standards entry, as the real file records it.
    fn standards(code: bool, families: &[&str]) -> BTreeMap<String, Entry> {
        BTreeMap::from([(
            KEY.to_string(),
            Entry {
                code,
                families: families
                    .iter()
                    .map(|family| (*family).to_string())
                    .collect(),
            },
        )])
    }

    /// A uniquely named directory under the temp directory, emptied first.
    fn fixture(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("gold-band-allowlist-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Writes `text` under `root` and returns the file's path.
    fn write(root: &Path, name: &str, text: &str) -> OsString {
        let path = root.join(name);
        fs::write(&path, text).unwrap();
        path.into_os_string()
    }

    /// The problems the rules find for one repository's one workflow, built
    /// straight from the pieces the rules read.
    fn problems_of(
        url: Option<&str>,
        allowlist: &BTreeMap<String, Entry>,
        families: &[(&str, &str)],
        shown: &str,
        workers: &[(&str, Option<&str>)],
    ) -> Vec<String> {
        let families: BTreeMap<String, String> = families
            .iter()
            .map(|(agent, family)| ((*agent).to_string(), (*family).to_string()))
            .collect();
        let files = [File {
            shown: shown.to_string(),
            workers: workers
                .iter()
                .map(|(id, agent)| Worker {
                    id: (*id).to_string(),
                    agent: agent.map(str::to_string),
                })
                .collect(),
        }];
        evaluate(
            &mut remote(url),
            Path::new("/repo"),
            allowlist,
            &families,
            &files,
        )
    }

    #[test]
    fn an_entry_allows_an_allowed_family_and_code() {
        let problems = problems_of(
            Some("https://github.com/JorisJonkers-dev/standards.git"),
            &standards(true, &["deepseek", "mistral"]),
            &[("seeker", "deepseek")],
            "flow.json",
            &[("dev-test", Some("seeker"))],
        );
        assert!(problems.is_empty());
    }

    #[test]
    fn a_forbidden_family_is_refused_naming_node_family_and_entry() {
        let problems = problems_of(
            Some("git@github.com:JorisJonkers-dev/standards.git"),
            &standards(true, &["deepseek", "mistral"]),
            &[("seeker", "deepseek"), ("hermes", "gemini")],
            "flow.json",
            &[("dev-test", Some("hermes"))],
        );
        assert_eq!(
            problems,
            vec![format!(
                "flow.json: dev-test: family gemini is not allowed by entry {KEY}"
            )]
        );
    }

    #[test]
    fn an_agent_missing_from_the_families_file_is_refused() {
        let problems = problems_of(
            Some("https://github.com/JorisJonkers-dev/standards"),
            &standards(true, &["deepseek"]),
            &[("seeker", "deepseek")],
            "flow.json",
            &[("dev-test", Some("ghost"))],
        );
        assert_eq!(
            problems,
            vec!["flow.json: dev-test: agent ghost has no family".to_string()]
        );
    }

    #[test]
    fn an_entry_without_code_refuses_a_workflow_that_sends_code() {
        let problems = problems_of(
            Some("https://github.com/JorisJonkers-dev/standards"),
            &standards(false, &["deepseek"]),
            &[("seeker", "deepseek")],
            "flow.json",
            &[("dev-test", Some("seeker"))],
        );
        assert_eq!(
            problems,
            vec![format!(
                "flow.json: workflow: entry {KEY} does not allow code"
            )]
        );
    }

    #[test]
    fn an_unbound_worker_still_sends_code_and_skips_the_family_rules() {
        let problems = problems_of(
            Some("https://github.com/JorisJonkers-dev/standards"),
            &standards(false, &[]),
            &[],
            "flow.json",
            &[("dev-test", None)],
        );
        assert_eq!(
            problems,
            vec![format!(
                "flow.json: workflow: entry {KEY} does not allow code"
            )]
        );
    }

    #[test]
    fn a_workflow_without_worker_nodes_needs_no_code_permission() {
        let problems = problems_of(
            Some("https://github.com/JorisJonkers-dev/standards"),
            &standards(false, &[]),
            &[],
            "flow.json",
            &[],
        );
        assert!(problems.is_empty());
    }

    #[test]
    fn a_repository_without_an_entry_gets_the_personal_default() {
        let problems = problems_of(
            Some("git@github.com:JorisJonkers-dev/agent-kit.git"),
            &BTreeMap::new(),
            &[],
            "flow.json",
            &[("dev-test", Some("any-agent"))],
        );
        assert!(problems.is_empty());
    }

    #[test]
    fn every_worker_node_is_checked_in_order() {
        let problems = problems_of(
            Some("https://github.com/JorisJonkers-dev/standards"),
            &standards(false, &["deepseek"]),
            &[("seeker", "deepseek"), ("hermes", "gemini")],
            "flow.json",
            &[
                ("a", Some("seeker")),
                ("b", Some("hermes")),
                ("c", Some("ghost")),
            ],
        );
        assert_eq!(
            problems,
            vec![
                format!("flow.json: b: family gemini is not allowed by entry {KEY}"),
                "flow.json: c: agent ghost has no family".to_string(),
                format!("flow.json: workflow: entry {KEY} does not allow code"),
            ]
        );
    }

    #[test]
    fn the_problems_of_two_workflows_keep_command_line_order() {
        let mut files = Vec::new();
        for (shown, agent) in [("a.json", "seeker"), ("b.json", "hermes")] {
            files.push(File {
                shown: shown.to_string(),
                workers: vec![Worker {
                    id: "dev-test".to_string(),
                    agent: Some(agent.to_string()),
                }],
            });
        }
        let families: BTreeMap<String, String> = BTreeMap::from([
            ("seeker".to_string(), "deepseek".to_string()),
            ("hermes".to_string(), "gemini".to_string()),
        ]);
        let problems = evaluate(
            &mut remote(Some("git@github.com:JorisJonkers-dev/standards.git")),
            Path::new("/repo"),
            &standards(true, &["deepseek"]),
            &families,
            &files,
        );
        assert_eq!(
            problems,
            vec![format!(
                "b.json: dev-test: family gemini is not allowed by entry {KEY}"
            )]
        );
    }

    #[test]
    fn a_repository_without_a_remote_is_refused_for_every_workflow() {
        let files = ["a.json", "b.json"]
            .iter()
            .map(|shown| File {
                shown: (*shown).to_string(),
                workers: Vec::new(),
            })
            .collect::<Vec<_>>();
        let problems = evaluate(
            &mut remote(None),
            Path::new("/repo"),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &files,
        );
        assert_eq!(
            problems,
            vec![
                "a.json: workflow: repository has no remote".to_string(),
                "b.json: workflow: repository has no remote".to_string(),
            ]
        );
    }

    #[test]
    fn a_remote_that_names_no_repository_is_refused() {
        let problems = problems_of(
            Some("/srv/git/repo"),
            &BTreeMap::new(),
            &[],
            "flow.json",
            &[("dev-test", None)],
        );
        assert_eq!(
            problems,
            vec![
                "flow.json: workflow: repository remote /srv/git/repo is not a repository URL"
                    .to_string()
            ]
        );
    }

    #[test]
    fn a_nedap_repository_is_refused_whatever_a_matching_entry_allows() {
        let allowances = BTreeMap::from([(
            "github.com/nedap/proprietary".to_string(),
            Entry {
                code: true,
                families: vec!["deepseek".to_string()],
            },
        )]);
        let problems = problems_of(
            Some("https://github.com/Nedap/proprietary.git"),
            &allowances,
            &[("seeker", "deepseek")],
            "flow.json",
            &[("dev-test", Some("seeker"))],
        );
        assert_eq!(
            problems,
            vec![
                "flow.json: workflow: nedap repositories are refused for Gold Band workflows"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn repo_key_derives_https_and_ssh_forms_lowercased() {
        let key = "github.com/jorisjonkers-dev/agent-kit";
        for url in [
            "git@github.com:JorisJonkers-dev/agent-kit.git",
            "https://github.com/JorisJonkers-dev/agent-kit.git",
            "https://github.com/JorisJonkers-dev/agent-kit",
            "ssh://git@github.com/JorisJonkers-dev/agent-kit.git",
            "https://github.com/JorisJonkers-dev/agent-kit/",
        ] {
            assert_eq!(repo_key(url), Some(key.to_string()), "for {url}");
        }
        assert_eq!(
            repo_key("git@gitlab.example:team/sub/project.git"),
            Some("gitlab.example/team/sub/project".to_string())
        );
    }

    #[test]
    fn repo_key_refuses_what_it_cannot_derive() {
        for url in [
            "",
            "github.com",
            "/srv/git/repo",
            "file:///srv/git/repo",
            "example.com/path:x/y",
            "https://github.com",
            "https://github.com/only",
            "https://github.com//repo.git",
            "git@github.com:not-a-repository",
            "git@github.com:a//b",
        ] {
            assert_eq!(repo_key(url), None, "for {url}");
        }
    }

    #[test]
    fn the_owner_segment_decides_nedap_case_insensitively() {
        assert!(owned_by_nedap("github.com/nedap/x"));
        assert!(owned_by_nedap("github.com/NEDAP/x"));
        assert!(!owned_by_nedap("github.com/x/nedap"));
        assert!(!owned_by_nedap("nedap.github.com/x"));
    }

    #[test]
    fn blank_bindings_have_no_agent_id() {
        assert_eq!(agent_id(" seeker "), Some("seeker".to_string()));
        assert_eq!(agent_id(""), None);
        assert_eq!(agent_id("   "), None);
    }

    #[test]
    fn a_worker_binds_through_provider_or_the_wrapper_binding() {
        assert_eq!(
            parse_workflow(&bare("dev-test", "seeker")).unwrap(),
            vec![Worker {
                id: "dev-test".to_string(),
                agent: Some("seeker".to_string()),
            }]
        );
        assert_eq!(
            parse_workflow(&wrapper("dev-test", "slot-dev", "reviewer")).unwrap(),
            vec![Worker {
                id: "dev-test".to_string(),
                agent: Some("reviewer".to_string()),
            }]
        );
        // a binding for another slot is not the node's binding
        let stray = r#"{"workflow":{"version":"0.1","id":"flow","entry":"dev-test","nodes":[{"id":"dev-test","type":"worker","provider":"seeker","executionSlotId":"slot-dev"}],"edges":[{"from":"dev-test","to":"$end","on":"success","session":null}]},"modelBindings":{"bindings":[{"executionSlotId":"slot-other","agentId":"hermes"}]}}"#;
        assert_eq!(
            parse_workflow(stray).unwrap(),
            vec![Worker {
                id: "dev-test".to_string(),
                agent: Some("seeker".to_string()),
            }]
        );
        // the binding wins over the node's provider
        let winner = r#"{"workflow":{"version":"0.1","id":"flow","entry":"dev-test","nodes":[{"id":"dev-test","type":"worker","provider":"seeker","executionSlotId":"slot-dev"}],"edges":[{"from":"dev-test","to":"$end","on":"success","session":null}]},"modelBindings":{"bindings":[{"executionSlotId":"slot-dev","agentId":"hermes"}]}}"#;
        assert_eq!(
            parse_workflow(winner).unwrap(),
            vec![Worker {
                id: "dev-test".to_string(),
                agent: Some("hermes".to_string()),
            }]
        );
    }

    #[test]
    fn an_ai_dynamic_node_binding_a_forbidden_family_is_refused() {
        let flow = r#"{"version":"0.1","id":"flow","entry":"dyn","nodes":[{"id":"dyn","type":"ai-dynamic","agentStrategy":{"mode":"fixed","provider":"hermes"}}],"edges":[]}"#;
        let workers = parse_workflow(flow).unwrap();
        let families = BTreeMap::from([("hermes".to_string(), "gemini".to_string())]);
        let problems = evaluate(
            &mut remote(Some("https://github.com/JorisJonkers-dev/standards")),
            Path::new("/repo"),
            &standards(true, &["deepseek"]),
            &families,
            &[File {
                shown: "flow.json".to_string(),
                workers,
            }],
        );
        assert_eq!(
            problems,
            vec![format!(
                "flow.json: dyn: family gemini is not allowed by entry {KEY}"
            )]
        );
    }

    #[test]
    fn an_ai_dynamic_node_binding_a_forbidden_family_in_dynamic_mode_is_refused() {
        let flow = r#"{"version":"0.1","id":"flow","entry":"dyn","nodes":[{"id":"dyn","type":"ai-dynamic","agentStrategy":{"mode":"dynamic","availableAgents":[{"provider":"seeker"},{"provider":"hermes"}]}}],"edges":[]}"#;
        let workers = parse_workflow(flow).unwrap();
        let families = BTreeMap::from([
            ("seeker".to_string(), "deepseek".to_string()),
            ("hermes".to_string(), "gemini".to_string()),
        ]);
        let problems = evaluate(
            &mut remote(Some("https://github.com/JorisJonkers-dev/standards")),
            Path::new("/repo"),
            &standards(true, &["deepseek"]),
            &families,
            &[File {
                shown: "flow.json".to_string(),
                workers,
            }],
        );
        assert_eq!(
            problems,
            vec![format!(
                "flow.json: dyn: family gemini is not allowed by entry {KEY}"
            )]
        );
    }

    #[test]
    fn an_ai_dynamic_node_binding_an_allowed_family_passes() {
        let families = BTreeMap::from([("seeker".to_string(), "deepseek".to_string())]);
        for flow in [
            // the fixed mode binds its agent in the strategy's provider
            r#"{"version":"0.1","id":"flow","entry":"dyn","nodes":[{"id":"dyn","type":"ai-dynamic","agentStrategy":{"mode":"fixed","provider":"seeker"}}],"edges":[]}"#,
            // the dynamic mode binds it in the available agents' providers
            r#"{"version":"0.1","id":"flow","entry":"dyn","nodes":[{"id":"dyn","type":"ai-dynamic","agentStrategy":{"mode":"dynamic","availableAgents":[{"provider":"seeker"}]}}],"edges":[]}"#,
        ] {
            let workers = parse_workflow(flow).unwrap();
            assert_eq!(
                workers,
                vec![Worker {
                    id: "dyn".to_string(),
                    agent: Some("seeker".to_string()),
                }],
                "for {flow}"
            );
            let problems = evaluate(
                &mut remote(Some("https://github.com/JorisJonkers-dev/standards")),
                Path::new("/repo"),
                &standards(true, &["deepseek"]),
                &families,
                &[File {
                    shown: "flow.json".to_string(),
                    workers,
                }],
            );
            assert!(problems.is_empty(), "for {flow}");
        }
    }

    #[test]
    fn an_ai_dynamic_node_binding_an_agent_without_a_family_is_refused() {
        let flow = r#"{"version":"0.1","id":"flow","entry":"dyn","nodes":[{"id":"dyn","type":"ai-dynamic","agentStrategy":{"mode":"fixed","provider":"ghost"}}],"edges":[]}"#;
        let workers = parse_workflow(flow).unwrap();
        let families = BTreeMap::from([("seeker".to_string(), "deepseek".to_string())]);
        let problems = evaluate(
            &mut remote(Some("https://github.com/JorisJonkers-dev/standards")),
            Path::new("/repo"),
            &standards(true, &["deepseek"]),
            &families,
            &[File {
                shown: "flow.json".to_string(),
                workers,
            }],
        );
        assert_eq!(
            problems,
            vec!["flow.json: dyn: agent ghost has no family".to_string()]
        );
    }

    #[test]
    fn an_ai_dynamic_node_sends_code_like_a_worker_node() {
        // the family is allowed; the entry without code refuses the workflow
        let flow = r#"{"version":"0.1","id":"flow","entry":"dyn","nodes":[{"id":"dyn","type":"ai-dynamic","agentStrategy":{"mode":"fixed","provider":"seeker"}}],"edges":[]}"#;
        let workers = parse_workflow(flow).unwrap();
        let families = BTreeMap::from([("seeker".to_string(), "deepseek".to_string())]);
        let problems = evaluate(
            &mut remote(Some("https://github.com/JorisJonkers-dev/standards")),
            Path::new("/repo"),
            &standards(false, &["deepseek"]),
            &families,
            &[File {
                shown: "flow.json".to_string(),
                workers,
            }],
        );
        assert_eq!(
            problems,
            vec![format!(
                "flow.json: workflow: entry {KEY} does not allow code"
            )]
        );
    }

    #[test]
    fn an_ai_dynamic_node_binding_no_agent_still_sends_code() {
        let flow = r#"{"version":"0.1","id":"flow","entry":"dyn","nodes":[{"id":"dyn","type":"ai-dynamic"}],"edges":[]}"#;
        assert_eq!(
            parse_workflow(flow).unwrap(),
            vec![Worker {
                id: "dyn".to_string(),
                agent: None,
            }]
        );
    }

    #[test]
    fn a_workflow_that_is_no_workflow_is_an_input_error() {
        assert!(parse_workflow("{").is_err());
        assert!(parse_workflow(r#"{"version":"0.1","id":"flow"}"#).is_err());
        assert!(parse_workflow(r#"{"workflow":{}}"#).is_err());
        let root = fixture("no-workflow");
        let ghost = root.join("ghost.json");
        assert!(load_workflow(ghost.as_os_str())
            .unwrap_err()
            .contains("cannot read"));
    }

    #[test]
    fn the_allowlist_file_has_every_reading() {
        let root = fixture("allowlist");
        // a missing file means no entries
        let missing = root.join("missing.json");
        assert!(load_allowlist(missing.as_os_str()).unwrap().is_empty());
        // the real file's data: one entry, history ignored
        let real = write(&root, "real.json", ALLOWLIST);
        let entries = load_allowlist(&real).unwrap();
        let entry = entries.get(KEY).unwrap();
        assert!(entry.code);
        assert_eq!(entry.families, ["deepseek", "zhipu", "mistral", "google"]);
        // a version other than 1, and text that is not JSON
        let v2 = write(&root, "v2.json", r#"{"version":2,"repositories":{}}"#);
        assert!(load_allowlist(&v2)
            .unwrap_err()
            .contains("version is not 1"));
        let broken = write(&root, "broken.json", "{oops");
        assert!(load_allowlist(&broken)
            .unwrap_err()
            .contains("not valid JSON"));
        // an entry with missing fields defaults closed
        let partial = write(
            &root,
            "partial.json",
            r#"{"version":1,"repositories":{"github.com/x/y":{}}}"#,
        );
        let entries = load_allowlist(&partial).unwrap();
        assert!(!entries["github.com/x/y"].code);
        assert!(entries["github.com/x/y"].families.is_empty());
        // unreadable: a directory
        fs::create_dir_all(root.join("dir.json")).unwrap();
        assert!(load_allowlist(root.join("dir.json").as_os_str())
            .unwrap_err()
            .contains("cannot read"));
    }

    #[test]
    fn the_families_file_is_one_agent_one_family_per_line() {
        let root = fixture("families");
        let missing = root.join("missing.tsv");
        assert!(load_families(missing.as_os_str())
            .unwrap_err()
            .contains("cannot read"));
        let families = write(&root, "families.tsv", FAMILIES);
        let families = load_families(&families).unwrap();
        assert_eq!(families.len(), 3);
        assert_eq!(families["seeker"], "deepseek");
        assert_eq!(families["reviewer"], "mistral");
        assert_eq!(families["hermes"], "gemini");
        // a line without the tab names its number
        let bad = write(&root, "bad.tsv", "seeker deepseek\n");
        assert!(load_families(&bad)
            .unwrap_err()
            .contains("is not <agent id><TAB><family>"));
    }

    #[test]
    fn check_answers_clean_refused_and_unreadable() {
        let root = fixture("check");
        let allowlist = write(&root, "allowlist.json", ALLOWLIST);
        let families = write(&root, "families.tsv", FAMILIES);
        let repo = OsString::from("/repo");
        let url = Some("https://github.com/JorisJonkers-dev/standards.git");
        let flow = write(&root, "flow.json", &bare("dev-test", "seeker"));
        // clean: seeker serves deepseek, the entry allows it
        let mut clean = remote(url);
        assert_eq!(
            check_command(
                &mut clean,
                &repo,
                &allowlist,
                &families,
                std::slice::from_ref(&flow)
            ),
            0
        );
        assert_eq!(clean.seen, vec!["/repo"]);
        // refused: hermes serves gemini, the entry does not allow it
        let hermes = write(&root, "hermes.json", &bare("dev-test", "hermes"));
        assert_eq!(
            check_command(&mut remote(url), &repo, &allowlist, &families, &[hermes]),
            1
        );
        // an unreadable workflow is bad input, not a refusal
        let ghost = root.join("ghost.json").into_os_string();
        let mut broken = remote(None);
        assert_eq!(
            check_command(&mut broken, &repo, &allowlist, &families, &[ghost]),
            2
        );
        assert!(broken.seen.is_empty()); // the inputs are read before the remote
                                         // a missing allowlist file means no entries: the personal default
        let missing = root.join("missing.json").into_os_string();
        assert_eq!(
            check_command(&mut remote(url), &repo, &missing, &families, &[flow]),
            0
        );
    }

    #[test]
    fn exec_runs_the_command_only_when_the_check_passes() {
        let root = fixture("exec");
        let allowlist = write(&root, "allowlist.json", ALLOWLIST);
        let families = write(&root, "families.tsv", FAMILIES);
        let repo = OsString::from("/repo");
        let url = Some("https://github.com/JorisJonkers-dev/standards.git");
        let flow = write(&root, "flow.json", &bare("dev-test", "seeker"));
        let hermes = write(&root, "hermes.json", &bare("dev-test", "hermes"));
        let command: Vec<OsString> = vec!["gold-band".into(), "run".into(), flow.clone()];
        // clean: the command runs and its exit code is the answer
        let mut spawn = FakeSpawn {
            calls: Vec::new(),
            code: 5,
            fails: false,
        };
        assert_eq!(
            exec_command(
                &mut remote(url),
                &mut spawn,
                &repo,
                &allowlist,
                &families,
                &flow,
                &command
            ),
            5
        );
        assert_eq!(
            spawn.calls,
            vec![(
                OsString::from("gold-band"),
                vec![OsString::from("run"), flow.clone()]
            )]
        );
        // refused: the command never starts
        assert_eq!(
            exec_command(
                &mut remote(url),
                &mut spawn,
                &repo,
                &allowlist,
                &families,
                &hermes,
                &command
            ),
            1
        );
        assert_eq!(spawn.calls.len(), 1);
        // unreadable input: the command never starts either
        let ghost = root.join("ghost.json").into_os_string();
        assert_eq!(
            exec_command(
                &mut remote(None),
                &mut spawn,
                &repo,
                &allowlist,
                &families,
                &ghost,
                &command
            ),
            2
        );
        assert_eq!(spawn.calls.len(), 1);
        // a command that fails to start is bad usage
        let mut missing = FakeSpawn {
            calls: Vec::new(),
            code: 0,
            fails: true,
        };
        assert_eq!(
            exec_command(
                &mut remote(url),
                &mut missing,
                &repo,
                &allowlist,
                &families,
                &flow,
                &command
            ),
            2
        );
        assert_eq!(missing.calls.len(), 1);
    }

    #[test]
    fn both_subcommands_have_one_command_line_shape() {
        let made = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
        match parse(
            "check",
            made(&[
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "one.json",
                "two.json",
            ]),
        )
        .unwrap()
        {
            CommandLine::Check {
                repo,
                allowlist,
                families,
                workflows,
            } => {
                assert_eq!(repo, OsString::from("r"));
                assert_eq!(allowlist, OsString::from("a"));
                assert_eq!(families, OsString::from("f"));
                assert_eq!(
                    workflows,
                    vec![OsString::from("one.json"), OsString::from("two.json")]
                );
            }
            CommandLine::Exec { .. } => panic!("check parsed as exec"),
        }
        match parse(
            "exec",
            made(&[
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "--workflow",
                "w.json",
                "--",
                "gold-band",
                "run",
            ]),
        )
        .unwrap()
        {
            CommandLine::Exec {
                repo,
                allowlist,
                families,
                workflow,
                command,
            } => {
                assert_eq!(repo, OsString::from("r"));
                assert_eq!(allowlist, OsString::from("a"));
                assert_eq!(families, OsString::from("f"));
                assert_eq!(workflow, OsString::from("w.json"));
                assert_eq!(
                    command,
                    vec![OsString::from("gold-band"), OsString::from("run")]
                );
            }
            CommandLine::Check { .. } => panic!("exec parsed as check"),
        }
    }

    #[test]
    fn bad_command_lines_are_bad_usage() {
        let answer = |args: &[&str]| run(args.iter().map(OsString::from));
        assert_eq!(answer(&[]), 2);
        assert_eq!(answer(&["dance"]), 2);
        assert_eq!(answer(&["--version"]), 2);
        // a check line needs all three flags and at least one workflow
        assert_eq!(answer(&["check"]), 2);
        assert_eq!(answer(&["check", "--repo", "r", "--allowlist", "a"]), 2);
        assert_eq!(
            answer(&[
                "check",
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f"
            ]),
            2
        );
        // a flag without its value, or a duplicate one, is refused
        assert_eq!(answer(&["check", "--repo"]), 2);
        assert_eq!(
            answer(&[
                "check",
                "--repo",
                "r",
                "--repo",
                "s",
                "--allowlist",
                "a",
                "--families",
                "f",
                "w.json"
            ]),
            2
        );
        // an unknown flag is refused
        assert_eq!(
            answer(&[
                "check",
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "--wat"
            ]),
            2
        );
        // check takes its workflows as arguments; --workflow is exec's
        assert_eq!(
            answer(&[
                "check",
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "--workflow",
                "w.json"
            ]),
            2
        );
        // an exec line needs one --workflow and a command after --
        assert_eq!(answer(&["exec"]), 2);
        assert_eq!(
            answer(&[
                "exec",
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "--"
            ]),
            2
        );
        assert_eq!(
            answer(&[
                "exec",
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "--",
                "x"
            ]),
            2
        );
        assert_eq!(
            answer(&[
                "exec",
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "--workflow",
                "w.json",
                "stray",
                "--",
                "gold-band"
            ]),
            2
        );
        // what follows -- belongs to the command
        assert_eq!(
            answer(&[
                "check",
                "--repo",
                "r",
                "--allowlist",
                "a",
                "--families",
                "f",
                "w.json",
                "--",
                "x"
            ]),
            2
        );
    }

    #[test]
    fn regression_a_port_in_the_origin_url_is_not_part_of_the_key() {
        assert_eq!(
            repo_key("ssh://git@github.com:22/JorisJonkers-dev/standards.git"),
            Some("github.com/jorisjonkers-dev/standards".to_string())
        );
        assert_eq!(
            repo_key("https://github.com:443/JorisJonkers-dev/standards"),
            Some("github.com/jorisjonkers-dev/standards".to_string())
        );
    }
}
