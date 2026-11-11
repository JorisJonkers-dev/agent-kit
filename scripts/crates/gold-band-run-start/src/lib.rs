//! The run-start shim every Gold Band workflow opens with (#444, spec #392).
//!
//! The first node of a workflow is a cheap, read-only worker that runs this
//! one pinned wrapper. It decides from the working copy and the owner's
//! records alone, never from model output, whether the run may go on:
//!
//! * `repository-rule`: the instance's rule against EVERY remote of the
//!   repository (`git remote -v`). The work instance (`gold-band`) runs only
//!   on a repository whose every remote is owned by `nedap` (owner compared
//!   ASCII case-insensitively, https, ssh and scp URL forms) and refuses a
//!   repository with no remote. The personal instance (`gold-band-personal`)
//!   refuses any `nedap`-owned remote and allows no remote at all, so a new
//!   repository can be bootstrapped (ADR-0011);
//! * `family-allowlist`: the family of every agent id the workflow binds (a
//!   worker's `provider`, the authoring wrapper's `modelBindings` joined by
//!   `executionSlotId`, an `ai-dynamic` node's `agentStrategy`), against the
//!   repository's allowlist entry, including its `code` permission. A
//!   workflow with a worker or `ai-dynamic` node sends code. The `history`
//!   permission is never required. A repository with no entry gets the
//!   personal default (every family, code). On a `nedap` repository the
//!   entry file is not consulted at all (ADR-0007): only the agent ids
//!   `claude-work` and `codex` are allowed, whatever an entry says. An agent
//!   id missing from the families file is refused;
//! * `grant`: in the work instance only, a live gateway grant for this run
//!   for every bound Codex agent id (an agent id containing `codex`, case
//!   insensitive). The run is named `<workflow>@<repository>@<run id>`, read
//!   from the gateway's own grants record (ADR-0008, ADR-0014). The cap is
//!   counted in Codex node starts by the per-node grant check, not here.
//!
//! Usage:
//!
//! ```text
//! gold-band-run-start check --instance gold-band|gold-band-personal
//!     --repo DIR --workflow-name NAME --run-id ID --attempt N
//!     --workflow FILE --families FILE [--allowlist FILE] [--grants FILE]
//!     [--now RFC3339] --report-dir DIR
//! ```
//!
//! The report (`gold-band-report`) goes to `<report-dir>/run-start.json`:
//! the command line, the exit code, the SHA-256 of the printed output, the
//! time and, when refused, the failed check. Problems go to stderr one per
//! line prefixed `gold-band-run-start:`; a pass prints `gold-band-run-start:
//! ok` on stdout. Exit 0 clean, 1 refused, 2 bad usage or unreadable input
//! (which also tries to write a failed report).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use gold_band_report::{parse_rfc3339, rfc3339, Clock, Report, SystemClock};
use serde_json::Value;

/// The crate's name, which prefixes every message line.
pub const NAME: &str = "gold-band-run-start";
/// The file the report is written to, inside the report directory.
pub const REPORT_FILE: &str = "run-start.json";

const CHECK_REPOSITORY: &str = "repository-rule";
const CHECK_FAMILY: &str = "family-allowlist";
const CHECK_GRANT: &str = "grant";
const CHECK_USAGE: &str = "usage";
const CHECK_INPUT: &str = "input";

/// The owner whose repositories the work instance serves.
const WORK_OWNER: &str = "nedap";
/// The agent ids a `nedap` repository may reach (ADR-0007, ADR-0011).
const NEDAP_AGENTS: [&str; 2] = ["claude-work", "codex"];
/// The gateway client a Codex agent id runs as.
const CODEX_CLIENT: &str = "codex";

/// The outside world the shim reaches for. Tests fake it; [`RealHost`] is
/// the real one.
pub trait Host: Clock {
    /// The repository's remotes as (name, URL) pairs, in `git remote -v`
    /// order without repeats. `Err` when the directory is not a repository.
    fn remotes(&self, repo: &Path) -> Result<Vec<(String, String)>, String>;
    /// One environment variable.
    fn env(&self, key: &str) -> Option<String>;
    /// Prints text on stdout, or on stderr when `stderr` is set.
    fn print(&self, stderr: bool, text: &str);
}

/// The real host: git, the process environment and the system clock.
pub struct RealHost {
    /// The git executable to run.
    pub git: &'static str,
}

impl RealHost {
    /// The real host, running `git` from `PATH`.
    pub fn new() -> RealHost {
        RealHost { git: "git" }
    }
}

impl Default for RealHost {
    fn default() -> Self {
        RealHost::new()
    }
}

impl Clock for RealHost {
    fn unix_seconds(&self) -> u64 {
        SystemClock.unix_seconds()
    }
}

impl Host for RealHost {
    fn remotes(&self, repo: &Path) -> Result<Vec<(String, String)>, String> {
        let out = std::process::Command::new(self.git)
            .arg("-C")
            .arg(repo)
            .args(["remote", "-v"])
            .output()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(parse_remotes(&String::from_utf8_lossy(&out.stdout)))
    }

    fn env(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }

    fn print(&self, stderr: bool, text: &str) {
        if stderr {
            eprint!("{text}");
        } else {
            print!("{text}");
        }
    }
}

/// `git remote -v` output as (name, URL) pairs: `name<TAB>url (fetch)` and
/// `(push)` lines, each pair once, `origin` first.
fn parse_remotes(text: &str) -> Vec<(String, String)> {
    let mut remotes: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(name), Some(url)) = (parts.next(), parts.next()) {
            let pair = (name.to_string(), url.to_string());
            if !remotes.contains(&pair) {
                remotes.push(pair);
            }
        }
    }
    remotes.sort_by_key(|(name, _)| name != "origin");
    remotes
}

/// Which instance the check runs for (ADR-0011).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Instance {
    Work,
    Personal,
}

/// Everything the flags carry.
struct Options {
    instance: Instance,
    repo: String,
    workflow_name: String,
    run_id: String,
    attempt: u32,
    workflow: String,
    families: String,
    allowlist: Option<String>,
    grants: Option<String>,
    now: Option<u64>,
    report_dir: String,
}

/// A problem line: the check that raised it and what to change.
struct Problem {
    check: &'static str,
    text: String,
}

fn problem(check: &'static str, text: String) -> Problem {
    Problem { check, text }
}

/// The value flags `check` takes.
const FLAGS: [&str; 11] = [
    "instance",
    "repo",
    "workflow-name",
    "run-id",
    "attempt",
    "workflow",
    "families",
    "allowlist",
    "grants",
    "now",
    "report-dir",
];

/// The usage text a bad command line prints on stderr.
fn usage() -> String {
    format!(
        "usage: {NAME} check --instance gold-band|gold-band-personal --repo DIR --workflow-name NAME --run-id ID --attempt N --workflow FILE --families FILE [--allowlist FILE] [--grants FILE] [--now RFC3339] --report-dir DIR"
    )
}

/// Splits the command line into its value flags, refusing a missing
/// subcommand, an unknown or repeated flag and a flag without a value.
fn flags(args: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut rest = args.iter();
    if rest.next().map(String::as_str) != Some("check") {
        return Err("the one subcommand is check".to_string());
    }
    let mut found = BTreeMap::new();
    while let Some(arg) = rest.next() {
        let name = arg
            .strip_prefix("--")
            .filter(|name| FLAGS.contains(name))
            .ok_or_else(|| format!("unexpected argument {arg}"))?;
        let value = rest
            .next()
            .ok_or_else(|| format!("--{name} needs a value"))?;
        if found.insert(name.to_string(), value.clone()).is_some() {
            return Err(format!("--{name} given twice"));
        }
    }
    Ok(found)
}

/// The typed options out of the flags.
fn options(found: &BTreeMap<String, String>) -> Result<Options, String> {
    let get = |name: &str| {
        found
            .get(name)
            .cloned()
            .ok_or_else(|| format!("--{name} is required"))
    };
    let instance = match get("instance")?.as_str() {
        "gold-band" => Instance::Work,
        "gold-band-personal" => Instance::Personal,
        other => {
            return Err(format!(
                "--instance {other} is not gold-band or gold-band-personal"
            ))
        }
    };
    let attempt = get("attempt")?
        .parse()
        .map_err(|_| "--attempt must be a non-negative integer".to_string())?;
    let now =
        match found.get("now") {
            None => None,
            Some(text) => Some(parse_rfc3339(text).ok_or_else(|| {
                "--now must be RFC 3339 UTC like 2026-11-11T09:30:00Z".to_string()
            })?),
        };
    Ok(Options {
        instance,
        repo: get("repo")?,
        workflow_name: get("workflow-name")?,
        run_id: get("run-id")?,
        attempt,
        workflow: get("workflow")?,
        families: get("families")?,
        allowlist: found.get("allowlist").cloned(),
        grants: found.get("grants").cloned(),
        now,
        report_dir: get("report-dir")?,
    })
}

/// Runs one command line against the real world and returns its exit code:
/// 0 clean, 1 refused, 2 bad usage or unreadable input.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_with(args, &RealHost::new())
}

/// [`run`] against any [`Host`].
pub fn run_with<I>(args: I, host: &dyn Host) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let command = format!("{NAME} {}", args.join(" "));
    let found = match flags(&args) {
        Ok(found) => found,
        Err(why) => return bad_input(host, &command, None, &found_of(&args), &why, true),
    };
    let opts = match options(&found) {
        Ok(opts) => opts,
        Err(why) => return bad_input(host, &command, None, &found, &why, true),
    };
    match check(&opts, host) {
        Err(why) => bad_input(host, &command, Some(&opts), &found, &why, false),
        Ok(problems) => finish(host, &command, &opts, &problems),
    }
}

/// The flags that could be read from a command line that did not parse in
/// full, so a failed report can still go to its `--report-dir`.
fn found_of(args: &[String]) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for pair in args.windows(2) {
        if let Some(name) = pair[0].strip_prefix("--").filter(|n| FLAGS.contains(n)) {
            found
                .entry(name.to_string())
                .or_insert_with(|| pair[1].clone());
        }
    }
    found
}

/// Exit 2: prints the reason (and the usage for a bad command line) and
/// tries to leave a failed report in the report directory.
fn bad_input(
    host: &dyn Host,
    command: &str,
    opts: Option<&Options>,
    found: &BTreeMap<String, String>,
    why: &str,
    show_usage: bool,
) -> i32 {
    let mut output = format!("{NAME}: {why}\n");
    if show_usage {
        output.push_str(&format!("{}\n", usage()));
    }
    host.print(true, &output);
    if let Some(dir) = found.get("report-dir") {
        let run_id = opts.map_or_else(
            || found.get("run-id").map_or("unknown", String::as_str),
            |o| o.run_id.as_str(),
        );
        let attempt = opts.map_or_else(
            || {
                found
                    .get("attempt")
                    .and_then(|a| a.parse().ok())
                    .unwrap_or(0)
            },
            |o| o.attempt,
        );
        let check = if show_usage { CHECK_USAGE } else { CHECK_INPUT };
        let report = Report::build(run_id, attempt, command, 2, &output, Some(check), host);
        // The report is best effort here: the exit code is 2 either way.
        if let Err(e) = gold_band_report::write(Path::new(dir), REPORT_FILE, &report) {
            host.print(true, &format!("{NAME}: cannot write the report: {e}\n"));
        }
    }
    2
}

/// Exit 0 or 1: prints the answer, writes the report, returns the code.
fn finish(host: &dyn Host, command: &str, opts: &Options, problems: &[Problem]) -> i32 {
    let (code, output, failed) = match problems.first() {
        None => (0, format!("{NAME}: ok\n"), None),
        Some(first) => {
            let lines: String = problems
                .iter()
                .map(|p| format!("{NAME}: {}: {}\n", p.check, p.text))
                .collect();
            (1, lines, Some(first.check))
        }
    };
    host.print(code != 0, &output);
    let report = Report::build(
        &opts.run_id,
        opts.attempt,
        command,
        code,
        &output,
        failed,
        host,
    );
    match gold_band_report::write(Path::new(&opts.report_dir), REPORT_FILE, &report) {
        Ok(_) => code,
        Err(e) => {
            host.print(true, &format!("{NAME}: cannot write the report: {e}\n"));
            2
        }
    }
}

/// One remote with its repository key (`host/owner/repo`) when its URL has one.
struct Remote {
    name: String,
    url: String,
    key: Option<String>,
}

impl Remote {
    fn owner(&self) -> Option<&str> {
        self.key.as_deref().and_then(|k| k.split('/').nth(1))
    }

    fn is_nedap(&self) -> bool {
        self.owner()
            .is_some_and(|o| o.eq_ignore_ascii_case(WORK_OWNER))
    }

    fn shown(&self) -> String {
        format!("remote {} ({})", self.name, self.url)
    }
}

/// Runs the three checks and returns every problem, in check order. `Err`
/// is unreadable input: exit 2.
fn check(opts: &Options, host: &dyn Host) -> Result<Vec<Problem>, String> {
    let remotes: Vec<Remote> = host
        .remotes(Path::new(&opts.repo))
        .map_err(|e| format!("{}: cannot list the remotes: {e}", opts.repo))?
        .into_iter()
        .map(|(name, url)| Remote {
            key: repo_key(&url),
            name,
            url,
        })
        .collect();
    let workflow = read_workflow(&opts.workflow)?;
    let families = read_families(&opts.families)?;
    let mut problems = repository_problems(opts.instance, &remotes);
    problems.extend(family_problems(opts, host, &remotes, &workflow, &families)?);
    if opts.instance == Instance::Work {
        problems.extend(grant_problems(opts, host, &remotes, &workflow)?);
    }
    Ok(problems)
}

/// The instance's repository rule against every remote.
fn repository_problems(instance: Instance, remotes: &[Remote]) -> Vec<Problem> {
    let mut out = Vec::new();
    if instance == Instance::Work && remotes.is_empty() {
        out.push(problem(
            CHECK_REPOSITORY,
            format!("repository has no remote; {WORK_OWNER} work needs every remote owned by {WORK_OWNER}"),
        ));
    }
    for remote in remotes {
        let text = match (instance, remote.owner()) {
            (_, None) => Some(format!(
                "{} is not a repository URL, so its owner cannot be shown",
                remote.shown()
            )),
            (Instance::Work, Some(owner)) if !remote.is_nedap() => Some(format!(
                "{} is owned by {owner}; the work instance gold-band runs only on {WORK_OWNER} repositories",
                remote.shown()
            )),
            (Instance::Personal, Some(_)) if remote.is_nedap() => Some(format!(
                "{} is owned by {WORK_OWNER}; the personal instance gold-band-personal refuses {WORK_OWNER} repositories",
                remote.shown()
            )),
            _ => None,
        };
        out.extend(text.map(|t| problem(CHECK_REPOSITORY, t)));
    }
    out
}

/// One agent id a node binds.
struct Bound {
    node: String,
    agent: String,
}

/// A workflow as the checks see it: the agent ids its nodes bind, and
/// whether it sends code.
struct Workflow {
    bound: Vec<Bound>,
    sends_code: bool,
}

fn read_text(path: &str) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{path}: cannot read: {e}"))
}

/// Reads a workflow file: the authoring wrapper (`workflow` plus
/// `modelBindings`) or a bare definition.
fn read_workflow(path: &str) -> Result<Workflow, String> {
    let value: Value =
        serde_json::from_str(&read_text(path)?).map_err(|e| format!("{path}: not JSON: {e}"))?;
    let definition = value.get("workflow").unwrap_or(&value);
    let nodes = definition
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{path}: not a Gold Band workflow: nodes must be a list"))?;
    let bindings: Vec<&Value> = value
        .pointer("/modelBindings/bindings")
        .and_then(Value::as_array)
        .map_or_else(Vec::new, |b| b.iter().collect());
    let mut workflow = Workflow {
        bound: Vec::new(),
        sends_code: false,
    };
    for node in nodes {
        let kind = node.get("type").and_then(Value::as_str).unwrap_or("");
        let id = text_of(node, "id").unwrap_or("?").to_string();
        let mut agents: Vec<&str> = Vec::new();
        agents.extend(text_of(node, "provider"));
        if let Some(slot) = text_of(node, "executionSlotId") {
            agents.extend(
                bindings
                    .iter()
                    .filter(|b| text_of(b, "executionSlotId") == Some(slot))
                    .filter_map(|b| text_of(b, "agentId")),
            );
        }
        if let Some(strategy) = node.get("agentStrategy") {
            // Every binding field is read whatever the node kind or mode
            // says, so a strategy cannot hide a binding.
            agents.extend(text_of(strategy, "provider"));
            agents.extend(text_of(strategy, "bootstrapProvider"));
            for available in strategy
                .get("availableAgents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                agents.extend(text_of(available, "provider"));
            }
        }
        // A worker or ai-dynamic node sends code; so does any node that binds
        // an agent, whatever its kind.
        workflow.sends_code |= kind == "worker" || kind == "ai-dynamic" || !agents.is_empty();
        for agent in agents {
            if !workflow
                .bound
                .iter()
                .any(|b| b.node == id && b.agent == agent)
            {
                workflow.bound.push(Bound {
                    node: id.clone(),
                    agent: agent.to_string(),
                });
            }
        }
    }
    Ok(workflow)
}

/// A trimmed, non-blank string field.
fn text_of<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Reads the families file: `<agent id><TAB><family>` lines; blank and `#`
/// lines are ignored.
fn read_families(path: &str) -> Result<BTreeMap<String, String>, String> {
    let mut families = BTreeMap::new();
    for (index, line) in read_text(path)?.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (agent, family) = line
            .split_once('\t')
            .ok_or_else(|| format!("{path}: line {}: is not <agent id><TAB><family>", index + 1))?;
        families.insert(agent.trim().to_string(), family.trim().to_string());
    }
    Ok(families)
}

/// One allowlist entry. The `history` permission is never read.
struct Entry {
    code: bool,
    families: Vec<String>,
}

/// Reads the allowlist: a missing file means no entries. Keys are matched
/// lowercased.
fn read_allowlist(path: &str) -> Result<BTreeMap<String, Entry>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(format!("{path}: cannot read: {e}")),
    };
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("{path}: not JSON: {e}"))?;
    if value.get("version").and_then(Value::as_i64) != Some(1) {
        return Err(format!("{path}: version must be 1"));
    }
    let repositories = value
        .get("repositories")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{path}: repositories must be an object"))?;
    let mut entries = BTreeMap::new();
    for (key, raw) in repositories {
        let families =
            raw.get("families")
                .and_then(Value::as_array)
                .map_or_else(Vec::new, |list| {
                    list.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                });
        let code = raw.get("code").and_then(Value::as_bool).unwrap_or(false);
        entries.insert(key.to_ascii_lowercase(), Entry { code, families });
    }
    Ok(entries)
}

/// `$XDG_CONFIG_HOME/agent-kit/<file>`, else `~/.config/agent-kit/<file>`.
fn config_path(host: &dyn Host, file: &str) -> Result<String, String> {
    let base = match (host.env("XDG_CONFIG_HOME"), host.env("HOME")) {
        (Some(xdg), _) => xdg,
        (None, Some(home)) => format!("{home}/.config"),
        (None, None) => return Err(format!("neither XDG_CONFIG_HOME nor HOME is set, so the path of {file} is unknown; pass it by flag")),
    };
    Ok(format!("{base}/agent-kit/{file}"))
}

/// The family check: every bound agent against the repository's entry, and
/// the code permission.
fn family_problems(
    opts: &Options,
    host: &dyn Host,
    remotes: &[Remote],
    workflow: &Workflow,
    families: &BTreeMap<String, String>,
) -> Result<Vec<Problem>, String> {
    let mut out = Vec::new();
    let nedap = remotes.iter().any(Remote::is_nedap);
    // The entry file is not consulted on a nedap repository, nor when the
    // workflow binds nothing it could narrow.
    let mut entries: Vec<(String, Entry)> = Vec::new();
    if !nedap {
        let path = match &opts.allowlist {
            Some(path) => path.clone(),
            None => config_path(host, "critique-allowlist.json")?,
        };
        let mut all = read_allowlist(&path)?;
        for key in remotes.iter().filter_map(|r| r.key.as_deref()) {
            if let Some(entry) = all.remove(key) {
                entries.push((key.to_string(), entry));
            }
        }
    }
    for bound in &workflow.bound {
        let Some(family) = families.get(&bound.agent) else {
            out.push(problem(
                CHECK_FAMILY,
                format!(
                    "node {}: agent {} has no family in {}",
                    bound.node, bound.agent, opts.families
                ),
            ));
            continue;
        };
        if nedap && !NEDAP_AGENTS.contains(&bound.agent.as_str()) {
            out.push(problem(
                CHECK_FAMILY,
                format!(
                    "node {}: agent {} is not allowed on a {WORK_OWNER} repository (only claude-work and codex, whatever an entry says)",
                    bound.node, bound.agent
                ),
            ));
        }
        for (key, entry) in &entries {
            if !entry.families.iter().any(|f| f == family) {
                out.push(problem(
                    CHECK_FAMILY,
                    format!(
                        "node {}: family {family} (agent {}) is not allowed by entry {key}",
                        bound.node, bound.agent
                    ),
                ));
            }
        }
    }
    if workflow.sends_code {
        for (key, _) in entries.iter().filter(|(_, e)| !e.code) {
            out.push(problem(
                CHECK_FAMILY,
                format!("workflow: entry {key} does not allow code"),
            ));
        }
    }
    Ok(out)
}

/// The repository key the run is named by: `origin`'s, else the first
/// remote's that has one.
fn run_key(remotes: &[Remote]) -> Option<&str> {
    remotes.iter().find_map(|r| r.key.as_deref())
}

/// The gateway's own grants record, read as the gateway reads it
/// (`parseGrantsFile`): version 1, and every grant with a non-empty
/// `answer`, `client`, `id`, `question` and `workflow`, a positive `cap_usd`
/// and UTC `expires_at` and `written_at` (and `ended_at` when present). A
/// missing file holds no grants.
struct Grant {
    id: String,
    client: String,
    workflow: String,
    expires_at: String,
    ended: bool,
}

fn read_grants(path: &str) -> Result<Vec<Grant>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{path}: cannot read: {e}")),
    };
    let bad = |why: String| format!("{path}: {why}");
    let value: Value = serde_json::from_str(&text).map_err(|e| bad(format!("not JSON: {e}")))?;
    if value.get("version").and_then(Value::as_i64) != Some(1) {
        return Err(bad("version must be 1".to_string()));
    }
    let list = value
        .get("grants")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("grants must be a list".to_string()))?;
    let mut grants = Vec::new();
    for (index, raw) in list.iter().enumerate() {
        let at = |field: &str| format!("grants[{index}].{field}");
        let string = |field: &str| match raw.get(field).and_then(Value::as_str) {
            Some(text) if !text.is_empty() => Ok(text.to_string()),
            _ => Err(bad(format!("{} must be a non-empty string", at(field)))),
        };
        let stamp = |field: &str| {
            let text = string(field)?;
            parse_rfc3339(&text)
                .map(|_| text)
                .ok_or_else(|| bad(format!("{} must be an ISO-8601 UTC timestamp", at(field))))
        };
        if !raw
            .get("cap_usd")
            .and_then(Value::as_f64)
            .is_some_and(|c| c > 0.0)
        {
            return Err(bad(format!("{} must be a number above 0", at("cap_usd"))));
        }
        string("answer")?;
        string("question")?;
        stamp("written_at")?;
        let ended = raw.get("ended_at").is_some();
        if ended {
            stamp("ended_at")?;
        }
        grants.push(Grant {
            id: string("id")?,
            client: string("client")?,
            workflow: string("workflow")?,
            expires_at: stamp("expires_at")?,
            ended,
        });
    }
    Ok(grants)
}

/// The grant check: the work instance needs a live grant for this run when
/// the workflow binds a Codex agent id. Live means the client is `codex`, the
/// workflow is this run's name, the grant has not ended and `now` is before
/// its `expires_at`. A refusal says which grant is missing and how to ask.
fn grant_problems(
    opts: &Options,
    host: &dyn Host,
    remotes: &[Remote],
    workflow: &Workflow,
) -> Result<Vec<Problem>, String> {
    let codex: Vec<String> = workflow
        .bound
        .iter()
        .filter(|b| b.agent.to_ascii_lowercase().contains(CODEX_CLIENT))
        .map(|b| format!("node {} (agent {})", b.node, b.agent))
        .collect();
    let Some(key) = run_key(remotes).filter(|_| !codex.is_empty()) else {
        // No Codex agent, or no repository to name the run by (the
        // repository rule has refused that already).
        return Ok(Vec::new());
    };
    let name = format!("{}@{key}@{}", opts.workflow_name, opts.run_id);
    let path = match &opts.grants {
        Some(path) => path.clone(),
        None => config_path(host, "model-gateway-grants.json")?,
    };
    let now = opts.now.unwrap_or_else(|| host.unix_seconds());
    let mut lapsed = Vec::new();
    for grant in read_grants(&path)? {
        if grant.workflow != name || grant.client != CODEX_CLIENT {
            continue;
        }
        let expired = parse_rfc3339(&grant.expires_at).is_none_or(|at| now >= at);
        if !grant.ended && !expired {
            return Ok(Vec::new());
        }
        let why = if grant.ended {
            "ended".to_string()
        } else {
            format!("expired at {}", grant.expires_at)
        };
        lapsed.push(format!("grant {} {why}", grant.id));
    }
    let ask = format!(
        "{{\"workflow\":{},\"client\":\"{CODEX_CLIENT}\",\"cap_usd\":5,\"question\":\"<what you asked the owner>\",\"answer\":\"<what the owner answered>\"}}",
        Value::String(name.clone())
    );
    // The call is meant to be pasted into a shell inside single quotes, so a
    // quote in the run id must not end them early.
    let ask = ask.replace('\'', "'\\''");
    let lapsed = if lapsed.is_empty() {
        String::new()
    } else {
        format!(" ({})", lapsed.join("; "))
    };
    Ok(vec![problem(
        CHECK_GRANT,
        format!(
            "{}: no live grant covers workflow {name}{lapsed}, so {CODEX_CLIENT} may not run: ask the owner, then add one with model-gateway grant write --json '{ask}' (as of {})",
            codex.join(", "),
            rfc3339(now)
        ),
    )])
}

/// The repository key of a remote URL: `host/owner/repo`, lowercased, a
/// trailing `.git` stripped, for https, ssh and scp forms (a user, a port
/// and a trailing slash are not part of the key). `None` for a local path or
/// anything else that names no `host/owner/repo`.
fn repo_key(url: &str) -> Option<String> {
    let path = scheme_path(url).or_else(|| scp_path(url))?;
    let bare = path.trim_end_matches('/');
    let bare = bare.strip_suffix(".git").unwrap_or(bare);
    if bare.split('/').count() < 3 || bare.split('/').any(str::is_empty) {
        return None;
    }
    Some(bare.to_ascii_lowercase())
}

/// The `host/path` of a scheme URL, with any `user@` and `:port` removed.
fn scheme_path(url: &str) -> Option<String> {
    let rest = ["https://", "http://", "ssh://", "git://", "git+ssh://"]
        .iter()
        .find_map(|prefix| url.strip_prefix(prefix))?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let host = host.split(':').next().unwrap_or(host);
    Some(format!("{host}/{path}"))
}

/// The `host/path` of an scp-form URL, `git@host:owner/repo`.
fn scp_path(url: &str) -> Option<String> {
    let (user_host, path) = url.split_once(':')?;
    if user_host.contains('/') {
        return None;
    }
    let host = user_host.rsplit('@').next().unwrap_or(user_host);
    Some(format!("{host}/{path}"))
}

#[cfg(test)]
mod tests;
