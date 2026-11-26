//! `agent-kit-merge converge mcp|plugins`: removes retired and undeclared entries from each agent surface, the Rust
//! port of `scripts/converge.py mcp` and `plugins` (#810).
//!
//! `mcp` reads the MCP servers of each Claude profile's `.claude.json`, Codex's `config.toml`, the `config.toml` of
//! each Vibe home (the shared one and vibe-private's) and the `mcp.servers` of OpenCode's `opencode.json`. `plugins`
//! reads Claude's marketplaces and user-scope plugins, which every profile shares. A retired entry is removed; any
//! other entry the surface's declared set does not name is removed unless the keep file names it. A `--private` entry
//! (Private Memory) is removed from every surface that does not declare it, whatever the keep file says.
//!
//! `--check` prints each removal and changes nothing. A normal run removes the entries, reads every surface again and
//! fails when a removed name is still there.

use crate::json::{self, Value};
use crate::{Env, Failure, Io};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item};

/// An MCP server whose name starts with one of these comes from a plugin and is never removed here.
const DECLARED_SOURCES: [&str; 1] = ["plugin:"];

/// What a command run printed to stdout, and whether it exited 0.
#[derive(Debug, PartialEq)]
pub struct Ran {
    pub stdout: String,
    pub success: bool,
}

/// Runs the agent CLIs (`claude`, `codex`) a surface is read or changed through.
pub trait Runner {
    /// Runs `program` with `args` in the inherited environment, with each `(name, Some(value))` set and each
    /// `(name, None)` removed.
    fn run(&mut self, program: &str, args: &[String], env: &[(String, Option<String>)]) -> Result<Ran, Failure>;
}

/// The runner that starts real processes.
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&mut self, program: &str, args: &[String], env: &[(String, Option<String>)]) -> Result<Ran, Failure> {
        let mut command = std::process::Command::new(program);
        command.args(args).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        for (name, value) in env {
            match value {
                Some(value) => command.env(name, value),
                None => command.env_remove(name),
            };
        }
        let output = command.output().map_err(|error| Failure(format!("{program}: {error}")))?;
        Ok(Ran { stdout: String::from_utf8_lossy(&output.stdout).into_owned(), success: output.status.success() })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Claude,
    Codex,
    Vibe,
    VibePrivate,
    OpenCode,
    Plugin,
    Marketplace,
}

impl Kind {
    /// The name `--declared KIND=...` uses for the surface.
    fn name(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
            Kind::Vibe => "vibe",
            Kind::VibePrivate => "vibe-private",
            Kind::OpenCode => "opencode",
            Kind::Plugin => "plugin",
            Kind::Marketplace => "marketplace",
        }
    }
}

/// One place entries are converged in.
struct Surface {
    label: String,
    kind: Kind,
    path: PathBuf,
    /// What the surface's CLI runs with on top of the inherited environment.
    env: Vec<(String, Option<String>)>,
}

impl Surface {
    fn new(label: &str, kind: Kind, path: &Path) -> Surface {
        Surface { label: label.to_string(), kind, path: path.to_path_buf(), env: Vec::new() }
    }

    fn names(&self, runner: &mut dyn Runner) -> Result<BTreeSet<String>, Failure> {
        match self.kind {
            Kind::Marketplace => {
                return Ok(claude_json(runner, &["plugin", "marketplace", "list", "--json"])?.iter().map(|entry| field_text(entry, "name")).collect())
            }
            Kind::Plugin => {
                let entries = claude_json(runner, &["plugin", "list", "--json"])?;
                return Ok(entries.iter().filter(|entry| matches!(entry.as_object().and_then(|o| o.get("scope")), Some(Value::Str(scope)) if scope == "user")).map(|entry| field_text(entry, "id")).collect());
            }
            _ => {}
        }
        if !self.path.is_file() {
            return Ok(BTreeSet::new());
        }
        let text = fs::read_to_string(&self.path)?;
        match self.kind {
            Kind::Claude => Ok(object_keys(json::parse(&text)?.as_object().ok_or_else(|| self.malformed())?.get("mcpServers"))),
            Kind::OpenCode => Ok(object_keys(opencode_servers(&json::parse(&text)?))),
            Kind::Codex => {
                let doc = self.toml(&text)?;
                Ok(match doc.get("mcp_servers").and_then(Item::as_table_like) {
                    Some(table) => table.iter().map(|(name, _)| name.to_string()).collect(),
                    None => BTreeSet::new(),
                })
            }
            _ => Ok(vibe_entries(&self.toml(&text)?).into_iter().collect()),
        }
    }

    fn toml(&self, text: &str) -> Result<DocumentMut, Failure> {
        text.parse::<DocumentMut>().map_err(|error| Failure(format!("{}: {}", self.path.display(), error.to_string().trim_end())))
    }

    fn malformed(&self) -> Failure {
        Failure(format!("{}: not a JSON object", self.path.display()))
    }

    /// The file no longer holds the servers it was read with.
    fn changed(&self) -> Failure {
        Failure(format!("{}: its MCP servers changed while they were converged", self.path.display()))
    }

    fn remove(&self, name: &str, runner: &mut dyn Runner) -> Result<(), Failure> {
        let command: Option<(&str, Vec<&str>)> = match self.kind {
            Kind::Claude => Some(("claude", vec!["mcp", "remove", "--scope", "user", name])),
            Kind::Codex => Some(("codex", vec!["mcp", "remove", name])),
            Kind::Plugin => Some(("claude", vec!["plugin", "uninstall", name, "--scope", "user"])),
            Kind::Marketplace => Some(("claude", vec!["plugin", "marketplace", "remove", name])),
            _ => None,
        };
        if let Some((program, args)) = command {
            let args: Vec<String> = args.into_iter().map(str::to_string).collect();
            // Whether it worked is what the read-back after every removal says.
            runner.run(program, &args, &self.env)?;
            return Ok(());
        }
        let text = fs::read_to_string(&self.path)?;
        if self.kind == Kind::OpenCode {
            let mut config = json::parse(&text)?;
            // The name was read from this table, so it is there unless another writer changed the file since.
            opencode_servers_mut(&mut config).ok_or_else(|| self.changed())?.remove(name);
            let staged = crate::with_suffix(&self.path, ".agent-kit-tmp");
            fs::write(&staged, json::dumps(&config) + "\n")?;
            fs::rename(&staged, &self.path)?;
            return Ok(());
        }
        let mut doc = self.toml(&text)?;
        let item = doc.get_mut("mcp_servers").ok_or_else(|| self.changed())?;
        let mut servers = servers_of(item).ok_or_else(|| self.changed())?;
        servers.retain(|server| server_name(server.get("name")) != name);
        if servers.is_empty() {
            doc.remove("mcp_servers");
        } else {
            // In place: an inline array is written back as `[[mcp_servers]]`, as tomlkit wrote it.
            *item = Item::ArrayOfTables(servers);
        }
        fs::write(&self.path, doc.to_string())?;
        Ok(())
    }
}

/// The `name` of each entry of a Vibe home's `[[mcp_servers]]`, as Python's `str()` prints it.
fn vibe_entries(doc: &DocumentMut) -> Vec<String> {
    match doc.get("mcp_servers").and_then(servers_of) {
        Some(servers) => servers.iter().map(|server| server_name(server.get("name"))).collect(),
        None => Vec::new(),
    }
}

/// A Vibe home's servers, whether written as `[[mcp_servers]]` or as an inline array of tables.
fn servers_of(item: &Item) -> Option<toml_edit::ArrayOfTables> {
    item.clone().into_array_of_tables().ok()
}

/// A server's `name`: its text, `None` when it has none (as Python's `str(None)`), else the value as written.
fn server_name(item: Option<&Item>) -> String {
    match item {
        None => "None".to_string(),
        Some(item) => match item.as_str() {
            Some(text) => text.to_string(),
            None => item.to_string().trim().to_string(),
        },
    }
}

/// A JSON entry's field as Python's `str()` prints a string; any other value as JSON.
fn field_text(entry: &Value, key: &str) -> String {
    match entry.as_object().and_then(|object| object.get(key)) {
        Some(Value::Str(text)) => text.clone(),
        Some(other) => json::dumps(other),
        None => "None".to_string(),
    }
}

/// The keys of an object value; nothing for any other value or none.
fn object_keys(value: Option<&Value>) -> BTreeSet<String> {
    value.and_then(Value::as_object).map(|object| object.keys().into_iter().collect()).unwrap_or_default()
}

/// OpenCode 2's `mcp.servers` table, or none.
fn opencode_servers(config: &Value) -> Option<&Value> {
    config.as_object()?.get("mcp")?.as_object()?.get("servers").filter(|servers| servers.as_object().is_some())
}

fn opencode_servers_mut(config: &mut Value) -> Option<&mut json::Object> {
    config.as_object_mut()?.get_mut("mcp")?.as_object_mut()?.get_mut("servers")?.as_object_mut()
}

/// What `claude ARGS` prints, read as a JSON list; a failed run or other output is a failure.
fn claude_json(runner: &mut dyn Runner, args: &[&str]) -> Result<Vec<Value>, Failure> {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let ran = runner.run("claude", &args, &[])?;
    if !ran.success {
        return Err(Failure(format!("claude {} failed", args.join(" "))));
    }
    match json::parse(&ran.stdout)? {
        Value::Arr(entries) => Ok(entries),
        _ => Err(Failure(format!("claude {} did not print a JSON list", args.join(" ")))),
    }
}

/// True when `path` is missing or parses as JSON.
fn plain_json(path: &Path) -> Result<bool, Failure> {
    if !path.is_file() {
        return Ok(true);
    }
    Ok(json::parse(&fs::read_to_string(path)?).is_ok())
}

/// `path` under the home as `~/...`, else as given.
fn tilde(path: &Path, env: Env) -> String {
    match path.strip_prefix(crate::home(env)) {
        Ok(rest) if rest.as_os_str().is_empty() => "~/.".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// One Claude profile's surface. The primary profile without `CLAUDE_CONFIG_DIR` keeps `.claude.json` in the home.
fn claude(profile: &Path, claude_home: &Path, env: Env) -> Surface {
    let ambient = env("CLAUDE_CONFIG_DIR").filter(|dir| !dir.is_empty());
    let (path, value) = if same_path(profile, claude_home) && ambient.is_none() {
        (crate::home(env).join(".claude.json"), None)
    } else {
        (profile.join(".claude.json"), Some(profile.display().to_string()))
    };
    let mut surface = Surface::new(&format!("claude {}", tilde(profile, env)), Kind::Claude, &path);
    surface.env.push(("CLAUDE_CONFIG_DIR".to_string(), value));
    surface
}

/// Whether two paths are equal as Python's `Path` compares them: component by component.
fn same_path(left: &Path, right: &Path) -> bool {
    left.components().eq(right.components())
}

/// The keep file's `keep:` names; nothing when the file or the list is missing.
pub(crate) fn keep(path: &Path, io: &mut Io) -> Result<Result<BTreeSet<String>, i32>, Failure> {
    if !path.is_file() {
        return Ok(Ok(BTreeSet::new()));
    }
    let data: serde_yaml::Value = serde_yaml::from_str(&fs::read_to_string(path)?).map_err(|error| Failure(format!("{}: {error}", path.display())))?;
    let listed = match data.as_mapping().and_then(|mapping| mapping.get("keep")) {
        None | Some(serde_yaml::Value::Null) => return Ok(Ok(BTreeSet::new())),
        Some(listed) => listed,
    };
    let names: Option<BTreeSet<String>> = listed.as_sequence().and_then(|items| items.iter().map(|item| item.as_str().map(str::to_string)).collect());
    match names {
        Some(names) => Ok(Ok(names)),
        None => {
            writeln!(io.err, "converge: {} must hold a top-level `keep:` list of names; nothing removed", path.display())?;
            Ok(Err(1))
        }
    }
}

/// The names of a comma-separated list.
fn names(value: &str) -> BTreeSet<String> {
    value.split(',').filter(|name| !name.is_empty()).map(str::to_string).collect()
}

/// Why an entry goes, or `None` when it stays.
type Verdict<'a> = &'a dyn Fn(&Surface, &str) -> Option<&'static str>;

/// Removes each entry the verdict sends away, then reads every surface back; status 1 when a removed entry remains.
fn converge(surfaces: &[Surface], verdict: Verdict, check: bool, runner: &mut dyn Runner, io: &mut Io) -> Result<i32, Failure> {
    let mut removals: Vec<(&Surface, String, &str)> = Vec::new();
    for surface in surfaces {
        for name in surface.names(runner)? {
            if let Some(reason) = verdict(surface, &name) {
                removals.push((surface, name, reason));
            }
        }
    }
    for (surface, name, reason) in &removals {
        if check {
            writeln!(io.out, "would remove {name} ({}, {reason})", surface.label)?;
        } else {
            surface.remove(name, runner)?;
            writeln!(io.out, "removed {name} ({}, {reason})", surface.label)?;
        }
    }
    let mut lingering = false;
    if !check {
        for (surface, name, _) in &removals {
            if surface.names(runner)?.contains(name) {
                writeln!(io.out, "{name} is still present ({})", surface.label)?;
                lingering = true;
            }
        }
    }
    Ok(i32::from(lingering))
}

/// The flags of `converge mcp`.
pub struct McpArgs {
    pub keep: PathBuf,
    pub claude_home: PathBuf,
    pub claude: Vec<PathBuf>,
    pub codex: Option<PathBuf>,
    pub vibe: Option<PathBuf>,
    pub vibe_private: Option<PathBuf>,
    pub opencode: Option<PathBuf>,
    pub declared: Vec<String>,
    pub retired: String,
    pub private: String,
    pub check: bool,
}

/// `converge mcp`.
pub fn mcp(args: &McpArgs, runner: &mut dyn Runner, io: &mut Io, env: Env) -> Result<i32, Failure> {
    let keep = match keep(&args.keep, io)? {
        Ok(keep) => keep,
        Err(status) => return Ok(status),
    };
    let retired = names(&args.retired);
    let declared: Vec<(String, BTreeSet<String>)> = args
        .declared
        .iter()
        .map(|entry| {
            let (kind, listed) = entry.split_once('=').unwrap_or((entry, ""));
            (kind.to_string(), names(listed))
        })
        .collect();
    let private = names(&args.private);
    let mut surfaces: Vec<Surface> = args.claude.iter().map(|profile| claude(profile, &args.claude_home, env)).collect();
    for (path, label, kind) in [(&args.codex, "codex", Kind::Codex), (&args.vibe, "vibe", Kind::Vibe), (&args.vibe_private, "vibe-private", Kind::VibePrivate)] {
        if let Some(path) = path {
            surfaces.push(Surface::new(label, kind, path));
        }
    }
    let mut unreadable = 0;
    if let Some(path) = &args.opencode {
        if plain_json(path)? {
            surfaces.push(Surface::new("opencode", Kind::OpenCode, path));
        } else {
            // OpenCode accepts comments; rewriting would drop them, and one bad file must not stop every other surface.
            writeln!(io.out, "{} is not plain JSON; opencode not converged", tilde(path, env))?;
            unreadable = 1;
        }
    }
    // The last `--declared` for a surface wins, as a dict built from them would keep it.
    let declared_for = |surface: &Surface, name: &str| declared.iter().rev().find(|(kind, _)| kind == surface.kind.name()).is_some_and(|(_, names)| names.contains(name));
    let verdict = |surface: &Surface, name: &str| {
        if private.contains(name) && !declared_for(surface, name) {
            Some("private")
        } else if retired.contains(name) {
            Some("retired")
        } else if declared_for(surface, name) || DECLARED_SOURCES.iter().any(|source| name.starts_with(source)) || keep.contains(name) {
            None
        } else {
            Some("undeclared")
        }
    };
    let status = converge(&surfaces, &verdict, args.check, runner, io)?;
    Ok(if status != 0 { status } else { unreadable })
}

/// The flags of `converge plugins`.
pub struct PluginArgs {
    pub keep: PathBuf,
    pub declared_markets: String,
    pub declared_plugins: String,
    pub retired_markets: String,
    pub retired_plugins: String,
    pub check: bool,
}

/// The marketplace name of the plugins bundled with Claude Code itself.
const BUILTIN_MARKET: &str = "builtin";

/// `converge plugins`.
pub fn plugins(args: &PluginArgs, runner: &mut dyn Runner, io: &mut Io) -> Result<i32, Failure> {
    let keep = match keep(&args.keep, io)? {
        Ok(keep) => keep,
        Err(status) => return Ok(status),
    };
    let retired: BTreeSet<String> = names(&args.retired_plugins).union(&names(&args.retired_markets)).cloned().collect();
    let declared: BTreeSet<String> = names(&args.declared_plugins).union(&names(&args.declared_markets)).cloned().collect();
    let verdict = |_: &Surface, name: &str| {
        let (plugin, market) = name.split_once('@').unwrap_or((name, ""));
        if market == BUILTIN_MARKET {
            // Claude Code ships these itself; `claude plugin uninstall` cannot remove them.
            None
        } else if retired.contains(name) {
            Some("retired")
        } else if declared.contains(name) || [name, plugin, market].iter().any(|part| keep.contains(*part)) {
            None
        } else {
            Some("undeclared")
        }
    };
    // Plugins first: removing a marketplace uninstalls its plugins without saying which.
    let surfaces = [Surface::new("claude plugin", Kind::Plugin, Path::new("")), Surface::new("claude marketplace", Kind::Marketplace, Path::new(""))];
    converge(&surfaces, &verdict, args.check, runner, io)
}

#[cfg(test)]
#[path = "converge/tests.rs"]
mod tests;
