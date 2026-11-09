//! The Gold Band agent registry: declares which Gold Band agents exist and
//! refuses a declaration that breaks one of the owner's hard rules (#332).
//!
//! Python flattens each declaration into the line protocol this crate reads:
//! `agent<TAB><id>` opens a declaration, `field<TAB><path><TAB><value>` sets
//! one field, and blank or `#` lines are ignored. The rules, each reported as
//! one problem line `<id>: <what>` (never printing a secret value):
//! * id grammar: lowercase letters and digits in dash-separated groups, and
//!   a duplicate id, which is named only once;
//! * no Codex anywhere: the id, `adapter.command`, any `adapter.args` value,
//!   `provider` or `model` must not contain `codex` (case-insensitive);
//! * every OpenRouter and every OpenCode model (an `opencode/` one included)
//!   is allowed (owner decision, 2026-11-08): the per-key LiteLLM budgets
//!   are the guard, not this registry;
//! * no literal secret in any field, argument or environment value: a
//!   secret-named field (its path's last segment, lowercased, contains
//!   `token`, `secret`, `key`, `password` or `authorization`) must hold an
//!   environment reference (`$NAME`, `${NAME}`) or a keychain-proxy name,
//!   and every other value is refused when it matches one of
//!   [`SECRET_SHAPES`] and is not an environment reference (a value whose
//!   `/`-separated segments are all shorter than 32 characters is a path,
//!   not a long opaque token);
//! * reach: one of `read`, `read-web`, `research`, `write-scoped`, `write`,
//!   `full`, `run-one-command`; `research` and `write-scoped` need an
//!   `outputDir` field, and `full` needs `throwawayOnly` `true`.
//!
//! `gold-band-registry converge --declared <FILE> --keep <FILE> --retired
//! <FILE> --settings <PATH> [--check]` makes the `agents` map of Gold Band's
//! settings.json hold exactly the declared agents plus the kept ones (#333):
//! * the declared agents are validated with the rules above first, and a
//!   refusal (exit 1) writes nothing;
//! * each declared agent is written with only the fields Gold Band knows
//!   (#328), typed the way its settings hold them -- booleans as JSON
//!   booleans, `adapter.args` and `compatibleAgentDirs` as arrays,
//!   `adapter.env` as an object -- and the registry-only fields (reach,
//!   outputDir, provider, model, throwawayOnly) never reach the
//!   file; the settings map REPLACES the app's runtime agent list, it is
//!   not merged with the built-in catalog;
//! * a leading `~/` in `adapter.command`, in each `adapter.args` value and
//!   in each `adapter.env` value (#335) is expanded to the user's home
//!   directory (`$HOME`) as it is written, because Gold Band needs absolute
//!   paths and expands no `~` itself; nothing else expands, and validation
//!   rules on the declared value, so it is unchanged;
//! * a built-in id ([`BUILT_INS`]) is written without `adapter.command` and
//!   `adapter.args`: the app strips those two when it saves and re-injects
//!   them from its catalog when it loads, so writing them would make the
//!   app's own save look like drift to the next run;
//! * an existing agent survives only when the keep file names it -- and
//!   never when its id names codex, which no keep-file entry can keep on
//!   this surface, nor when it is retired, which the keep file cannot spare;
//! * every other existing agent is removed, and every other top-level key
//!   of the document keeps its value and its key order.
//!
//! A document that comes out unchanged is written by no one: the command
//! prints `settings unchanged`. Otherwise the old file is copied to
//! `settings.json.bak`, the new document goes in atomically (a temporary
//! file in the same directory, then a rename) and one line per added,
//! changed and removed agent id is printed; `--check` prints those lines
//! and writes nothing. A missing settings file starts from `{}`.
//!
//! Usage: `gold-band-registry validate <FILE> | converge --declared <FILE>
//! --keep <FILE> --retired <FILE> --settings <PATH> [--check]`. Exit 0 clean
//! (printing `ok (<n> agents)` for a validate), 1 rule problems on stderr,
//! 2 bad usage (a usage line), an unreadable file, malformed input or a
//! settings document that is not a JSON object.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde_json::{Map, Value};

/// The crate's name, which prefixes every message line.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The reach values the registry allows.
pub const REACHES: [&str; 7] = [
    "read",
    "read-web",
    "research",
    "write-scoped",
    "write",
    "full",
    "run-one-command",
];

/// The Gold Band agent ids the app ships in its own catalog (#328): for one of
/// these, Gold Band strips `adapter.command` and `adapter.args` when it saves
/// settings.json and re-injects them from the catalog when it loads, so the
/// converged entry never carries them -- writing them would make the app's own
/// save differ from what the next setup run expects.
pub const BUILT_INS: [&str; 11] = [
    "claude-acp",
    "codex-acp",
    "cursor",
    "gemini",
    "codebuddy-code",
    "goose",
    "qwen-code",
    "opencode",
    "kimi",
    "amp-acp",
    "pi-acp",
];

/// One declared agent: its id, the 1-based line it was declared on, and its
/// fields in declaration order, each one a (path, value) pair.
#[derive(Debug)]
pub struct Agent {
    pub id: String,
    pub line: usize,
    pub fields: Vec<(String, String)>,
}

/// Parses the registry's line protocol into its agents, or names the first
/// malformed line.
pub fn parse(text: &str) -> Result<Vec<Agent>, String> {
    let mut agents: Vec<Agent> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = line.split('\t');
        let head = parts.next().unwrap_or("");
        let rest: Vec<&str> = parts.collect();
        if head == "agent" && rest.len() == 1 {
            agents.push(Agent {
                id: rest[0].to_string(),
                line: number,
                fields: Vec::new(),
            });
        } else if head == "field" && rest.len() == 2 {
            let agent = agents
                .last_mut()
                .ok_or_else(|| format!("line {number}: field before any agent"))?;
            agent
                .fields
                .push((rest[0].to_string(), rest[1].to_string()));
        } else {
            return Err(format!("line {number}: malformed line"));
        }
    }
    Ok(agents)
}

/// Every rule violation in `agents`, in declaration order: one line per
/// problem, each naming the agent id.
pub fn problems(agents: &[Agent]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for agent in agents {
        let count = seen.entry(agent.id.as_str()).or_insert(0);
        *count += 1;
        out.extend(agent_problems(agent, *count == 2));
    }
    out
}

/// One agent's rule violations, in rule order; `duplicate` asks for the
/// once-per-id duplicate line.
fn agent_problems(a: &Agent, duplicate: bool) -> Vec<String> {
    let id = a.id.as_str();
    let grammar = "id must be lowercase letters and digits in dash-separated groups";
    let mut p = Vec::new();
    if duplicate {
        p.push(format!("{id}: duplicate id"));
    }
    if !id_ok(id) {
        p.push(format!("{id}: {grammar}"));
    }
    for (path, body) in &a.fields {
        match kind(path) {
            Kind::Unknown => p.push(format!("{id}: unknown agent field {path}")),
            Kind::Boolean if body != "true" && body != "false" => {
                p.push(format!("{id}: {path} must be \"true\" or \"false\""));
            }
            Kind::Delivery if body != "none" && body != "meta-append" => {
                p.push(format!(
                    "{id}: systemPromptDelivery must be \"none\" or \"meta-append\""
                ));
            }
            _ => {}
        }
    }
    if names(id, "codex") {
        p.push(format!("{id}: id must not name codex"));
    }
    if let Some(command) = value(a, "adapter.command") {
        if names(command, "codex") {
            p.push(format!("{id}: adapter.command must not name codex"));
        }
    }
    for (path, body) in &a.fields {
        let named = matches!(kind(path), Kind::Args | Kind::Env) || path == "adapter.displayName";
        if named && names(body, "codex") {
            p.push(format!("{id}: {path} must not name codex"));
        }
    }
    for field in ["provider", "model"] {
        if let Some(body) = value(a, field) {
            if names(body, "codex") {
                p.push(format!("{id}: {field} must not name codex"));
            }
        }
    }
    for (path, body) in &a.fields {
        if env_ref(body) {
            continue; // an environment reference exposes no literal secret
        }
        match secret_shape(body) {
            Some(shape) => p.push(format!("{id}: {path} holds a literal secret ({shape})")),
            None if secret_named(path) && !keychain_name(body) => p.push(format!(
                "{id}: {path} must hold an environment reference or a keychain-proxy name"
            )),
            None => {}
        }
    }
    if value(a, "reach").is_none() {
        p.push(format!("{id}: reach is required"));
    }
    if let Some(reach) = value(a, "reach") {
        let needs_dir = reach == "research" || reach == "write-scoped";
        let needs_throwaway = reach == "full";
        if !REACHES.contains(&reach) {
            p.push(bad_reach(id, reach));
        } else if needs_dir && value(a, "outputDir").is_none() {
            p.push(format!("{id}: reach \"{reach}\" needs an outputDir field"));
        } else if needs_throwaway && value(a, "throwawayOnly") != Some("true") {
            p.push(format!("{id}: reach \"full\" needs throwawayOnly \"true\""));
        }
    }
    p
}

/// The refusal line for a reach the registry does not allow.
fn bad_reach(id: &str, reach: &str) -> String {
    format!(
        "{id}: reach \"{reach}\" is not one of {}",
        REACHES.join(", ")
    )
}

/// Runs one command line and returns its exit code: 0 clean, 1 rule problems,
/// 2 bad usage, an unreadable file or malformed input. Success prints to
/// stdout, problems to stderr.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    match args.first().map(|cmd| cmd.as_os_str()) {
        Some(cmd) if cmd == OsStr::new("validate") => {
            let Some(file) = args.get(1).filter(|_| args.len() == 2) else {
                eprintln!("{}", usage());
                return 2;
            };
            validate_file(file)
        }
        Some(cmd) if cmd == OsStr::new("converge") => converge_cli(&args[1..]),
        _ => {
            eprintln!("{}", usage());
            2
        }
    }
}

/// One `validate` command line over its file.
fn validate_file(file: &OsString) -> i32 {
    let shown = file.to_string_lossy();
    match fs::read_to_string(file) {
        Ok(text) => match parse(&text) {
            Ok(agents) => report(&agents),
            Err(e) => {
                eprintln!("{NAME}: {shown}: {e}");
                2
            }
        },
        Err(e) => {
            eprintln!("{NAME}: cannot read {shown}: {e}");
            eprintln!("{}", usage());
            2
        }
    }
}

/// One `converge` command line: 0 converged, unchanged or checked; 1 a
/// refused declaration; 2 bad usage, an unreadable file or a settings
/// document that is not a JSON object.
fn converge_cli(args: &[OsString]) -> i32 {
    let (mut declared, mut keep, mut retired, mut settings) = (None, None, None, None);
    let mut check = false;
    let mut index = 0;
    while let Some(flag) = args.get(index) {
        let slot = match flag.to_string_lossy().as_ref() {
            "--declared" => 0,
            "--keep" => 1,
            "--retired" => 2,
            "--settings" => 3,
            "--check" => {
                check = true;
                index += 1;
                continue;
            }
            _ => {
                eprintln!("{}", usage());
                return 2;
            }
        };
        let Some(body) = args.get(index + 1) else {
            eprintln!("{}", usage());
            return 2;
        };
        match slot {
            0 => declared = Some(body.clone()),
            1 => keep = Some(body.clone()),
            2 => retired = Some(body.clone()),
            _ => settings = Some(body.clone()),
        }
        index += 2;
    }
    let (Some(declared), Some(keep), Some(retired), Some(settings)) =
        (declared, keep, retired, settings)
    else {
        eprintln!("{}", usage());
        return 2;
    };
    let agents = match declared_agents(declared.as_os_str()) {
        Ok(agents) => agents,
        Err(code) => return code,
    };
    let keep = match ids(keep.as_os_str()) {
        Ok(ids) => ids,
        Err(code) => return code,
    };
    let retired = match ids(retired.as_os_str()) {
        Ok(ids) => ids,
        Err(code) => return code,
    };
    converge(&agents, &keep, &retired, Path::new(&settings), check)
}

/// The declared agents of the line-protocol file, validated with the rules
/// first; Err carries the exit code with every line already printed: 1 a
/// refusal, 2 an unreadable or malformed file.
fn declared_agents(file: &OsStr) -> Result<Vec<Agent>, i32> {
    let shown = file.to_string_lossy();
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("{NAME}: cannot read {shown}: {e}");
            eprintln!("{}", usage());
            return Err(2);
        }
    };
    let agents = match parse(&text) {
        Ok(agents) => agents,
        Err(e) => {
            eprintln!("{NAME}: {shown}: {e}");
            return Err(2);
        }
    };
    let found = problems(&agents);
    if found.is_empty() {
        return Ok(agents);
    }
    for problem in &found {
        eprintln!("{NAME}: {problem}");
    }
    Err(1)
}

/// The ids one keep or retired file holds: one per line, blank lines ignored.
fn ids(file: &OsStr) -> Result<Vec<String>, i32> {
    match fs::read_to_string(file) {
        Ok(text) => Ok(text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()),
        Err(e) => {
            eprintln!("{NAME}: cannot read {}: {e}", file.to_string_lossy());
            Err(2)
        }
    }
}

/// Converges the `agents` map of `settings` onto the declared agents plus the
/// kept others: 0 converged, unchanged or checked, 2 when the document could
/// not be read or written. One line is printed per added, changed and removed
/// agent id; a check prints them without writing.
fn converge(
    agents: &[Agent],
    keep: &[String],
    retired: &[String],
    settings: &Path,
    check: bool,
) -> i32 {
    let (mut document, existed) = match read_document(settings) {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let before = document.clone();
    let current = agents_of(&document);
    let mut fresh = Map::new();
    for agent in agents {
        fresh.insert(agent.id.clone(), agent_value(agent));
    }
    for (id, entry) in &current {
        if fresh.contains_key(id) || !kept(id, keep, retired) {
            continue;
        }
        fresh.insert(id.clone(), entry.clone());
    }
    let after = fresh.clone();
    document.insert("agents".to_string(), Value::Object(fresh));
    if document == before {
        println!("{NAME}: settings unchanged");
        return 0;
    }
    report_ids(&current, &after);
    if check {
        return 0;
    }
    write(settings, &Value::Object(document), existed)
}

/// Whether an existing agent id survives: the keep file names it, it is no
/// codex id -- which no keep-file entry can keep on this surface -- and it is
/// not a retired one, which the keep file cannot spare.
fn kept(id: &str, keep: &[String], retired: &[String]) -> bool {
    keep.iter().any(|name| name.as_str() == id)
        && !names(id, "codex")
        && !retired.iter().any(|gone| gone.as_str() == id)
}

/// One line per added, changed and removed agent id, in that order.
fn report_ids(old: &Map<String, Value>, fresh: &Map<String, Value>) {
    for (id, entry) in fresh {
        if !old.contains_key(id) {
            println!("{NAME}: added {id}");
        } else if old.get(id) != Some(entry) {
            println!("{NAME}: changed {id}");
        }
    }
    for id in old.keys() {
        if !fresh.contains_key(id) {
            println!("{NAME}: removed {id}");
        }
    }
}

/// The settings document and whether the file existed: a missing file starts
/// from the empty document; an unreadable file, or one that is not a JSON
/// object, is Err(2) with the reason printed and nothing written.
fn read_document(path: &Path) -> Result<(Map<String, Value>, bool), i32> {
    let (text, existed) = match fs::read_to_string(path) {
        Ok(text) => (text, true),
        Err(e) if e.kind() == ErrorKind::NotFound => ("{}".to_string(), false),
        Err(e) => {
            eprintln!("{NAME}: cannot read {}: {e}", path.display());
            return Err(2);
        }
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(document)) => Ok((document, existed)),
        Ok(_) => {
            eprintln!("{NAME}: {}: not a JSON object", path.display());
            Err(2)
        }
        Err(e) => {
            eprintln!("{NAME}: {}: {e}", path.display());
            Err(2)
        }
    }
}

/// The document's `agents` map: an absent one, or one that holds anything but
/// a JSON object, is an empty map the convergence then fills.
fn agents_of(document: &Map<String, Value>) -> Map<String, Value> {
    match document.get("agents") {
        Some(Value::Object(agents)) => agents.clone(),
        _ => Map::new(),
    }
}

/// One declared agent as a settings.json entry: only the fields Gold Band
/// knows (#328), typed, in Gold Band's own field order.
fn agent_value(a: &Agent) -> Value {
    let mut agent = Map::new();
    let adapter = adapter_value(a);
    if !adapter.is_empty() {
        agent.insert("adapter".to_string(), Value::Object(adapter));
    }
    for path in [
        "icon",
        "primaryAgentDir",
        "projectPrimaryAgentDir",
        "compatibleAgentDirs",
        "systemPromptDelivery",
        "externalSessionSyncSupported",
        "externalSessionSyncEnabled",
        "useLocalClaude",
    ] {
        if let Some(entry) = entry(a, path) {
            agent.insert(path.to_string(), entry);
        }
    }
    Value::Object(agent)
}

/// The typed settings value of one Gold Band field, when the declaration sets
/// it: booleans as JSON booleans, the directories array, the rest strings.
fn entry(a: &Agent, path: &str) -> Option<Value> {
    match path {
        "compatibleAgentDirs" => {
            let dirs: Vec<Value> = array(a, "compatibleAgentDirs.")
                .into_iter()
                .map(Value::String)
                .collect();
            if dirs.is_empty() {
                None
            } else {
                Some(Value::Array(dirs))
            }
        }
        "externalSessionSyncSupported" | "externalSessionSyncEnabled" | "useLocalClaude" => {
            value(a, path).map(|body| Value::Bool(body == "true"))
        }
        _ => value(a, path).map(|body| Value::String(body.to_string())),
    }
}

/// The agent's `adapter` block, in Gold Band's field order; a built-in id
/// leaves command and args out, which the app re-injects from its catalog.
/// A leading `~/` in the command, in an arg or in an env value is expanded
/// to the user's home directory as it is written, because Gold Band needs
/// absolute paths and expands no `~` itself.
fn adapter_value(a: &Agent) -> Map<String, Value> {
    let home = std::env::var_os("HOME");
    let mut adapter = Map::new();
    if !BUILT_INS.contains(&a.id.as_str()) {
        if let Some(command) = value(a, "adapter.command") {
            adapter.insert(
                "command".to_string(),
                Value::String(expand_home(command, home.as_deref())),
            );
        }
        let args: Vec<Value> = array(a, "adapter.args.")
            .into_iter()
            .map(|arg| Value::String(expand_home(&arg, home.as_deref())))
            .collect();
        if !args.is_empty() {
            adapter.insert("args".to_string(), Value::Array(args));
        }
    }
    if let Some(display) = value(a, "adapter.displayName") {
        adapter.insert(
            "displayName".to_string(),
            Value::String(display.to_string()),
        );
    }
    let mut env = Map::new();
    for (path, body) in &a.fields {
        if let Some(name) = path.strip_prefix("adapter.env.") {
            env.insert(
                name.to_string(),
                Value::String(expand_home(body, home.as_deref())),
            );
        }
    }
    if !env.is_empty() {
        adapter.insert("env".to_string(), Value::Object(env));
    }
    adapter
}

/// The declared strings of one array field (`adapter.args`,
/// `compatibleAgentDirs`) in index order.
fn array(a: &Agent, prefix: &str) -> Vec<String> {
    let mut items: Vec<(usize, String)> = Vec::new();
    for (path, body) in &a.fields {
        let Some(index) = path.strip_prefix(prefix) else {
            continue;
        };
        if !decimal(index) {
            continue;
        }
        items.push((index.parse().unwrap_or_default(), body.clone()));
    }
    items.sort_by_key(|(index, _)| *index);
    items.into_iter().map(|(_, body)| body).collect()
}

/// A declared value with a leading `~/` expanded to the user's home
/// directory (`$HOME`): Gold Band reads settings.json without shell
/// expansion, so the converge writes an absolute path -- a command, an
/// arg (#338) or an env value (#335) -- where the declaration shortens the
/// home. A value with no leading `~/`, or a
/// home that is not set (or empty), is written exactly as declared.
fn expand_home(value: &str, home: Option<&OsStr>) -> String {
    let Some(rest) = value.strip_prefix("~/") else {
        return value.to_string();
    };
    let Some(home) = home.filter(|home| !home.is_empty()) else {
        return value.to_string();
    };
    format!("{}/{}", home.to_string_lossy(), rest)
}

/// Writes `document` to `settings` atomically -- a temporary file in the same
/// directory, then a rename -- after copying the old file to its `.bak`;
/// 2 when that could not be done.
fn write(settings: &Path, document: &Value, existed: bool) -> i32 {
    if let Some(dir) = settings.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if existed {
        let backup = settings.with_file_name(format!("{}.bak", name_of(settings)));
        if let Err(e) = fs::copy(settings, &backup) {
            eprintln!(
                "{NAME}: cannot copy {} to {}: {e}",
                settings.display(),
                backup.display()
            );
            return 2;
        }
    }
    let staged =
        settings.with_file_name(format!("{}.{}.tmp", name_of(settings), std::process::id()));
    let body = serde_json::to_string_pretty(document).expect("a Value serialises") + "\n";
    // Owner-only: Gold Band keeps tokens in this file, so the rewrite never
    // widens it past 0600, whatever the umask.
    let written = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged)
        .and_then(|mut file| file.write_all(body.as_bytes()));
    match written.and_then(|()| fs::rename(&staged, settings)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{NAME}: cannot write {}: {e}", settings.display());
            let _ = fs::remove_file(&staged);
            2
        }
    }
}

/// The settings file's name, for its backup and staged siblings.
fn name_of(settings: &Path) -> String {
    settings.file_name().map_or_else(
        || "settings.json".to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// The usage line every command-line problem prints.
fn usage() -> String {
    format!(
        "usage: {NAME} validate <FILE> | {NAME} converge --declared <FILE> --keep <FILE> \\\n       --retired <FILE> --settings <PATH> [--check]"
    )
}

/// Prints the verdict for parsed agents: either the clean line or every
/// problem; returns the exit code it earns.
fn report(agents: &[Agent]) -> i32 {
    let found = problems(agents);
    if found.is_empty() {
        println!("{NAME}: ok ({} agents)", agents.len());
        0
    } else {
        for problem in &found {
            eprintln!("{NAME}: {problem}");
        }
        1
    }
}

/// The agent's `path` field value, if it was ever set.
fn value<'a>(a: &'a Agent, path: &str) -> Option<&'a str> {
    a.fields
        .iter()
        .find(|(known, _)| known.as_str() == path)
        .map(|(_, body)| body.as_str())
}

/// True when `body` contains `word`, ignoring ASCII case.
fn names(body: &str, word: &str) -> bool {
    body.to_ascii_lowercase().contains(word)
}

/// The id grammar: `^[a-z0-9]+(-[a-z0-9]+)*$`.
fn id_ok(id: &str) -> bool {
    id.split('-')
        .all(|group| !group.is_empty() && group.bytes().all(lower_digit))
}

fn lower_digit(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit()
}

/// What a field path tells the registry about its value's shape.
#[derive(PartialEq)]
enum Kind {
    /// A known field with no constraint of its own here.
    Known,
    /// A path the registry does not know.
    Unknown,
    /// A field that must hold `true` or `false`.
    Boolean,
    /// `systemPromptDelivery`, which must hold `none` or `meta-append`.
    Delivery,
    /// `adapter.args.<N>`.
    Args,
    /// `adapter.env.<NAME>`.
    Env,
}

/// Classifies one field path: the named scalars by name, the dotted families
/// by their shape, anything else as unknown.
fn kind(path: &str) -> Kind {
    match path {
        "externalSessionSyncSupported" | "externalSessionSyncEnabled" => Kind::Boolean,
        "useLocalClaude" | "throwawayOnly" => Kind::Boolean,
        "systemPromptDelivery" => Kind::Delivery,
        "adapter.command" | "adapter.displayName" | "icon" | "primaryAgentDir" => Kind::Known,
        "family" | "projectPrimaryAgentDir" | "outputDir" | "provider" | "model" | "reach" => {
            Kind::Known
        }
        _ => prefixed(path),
    }
}

/// The dotted field families: `adapter.args.<N>`, `adapter.env.<NAME>` and
/// `compatibleAgentDirs.<N>`; anything else is unknown.
fn prefixed(path: &str) -> Kind {
    if let Some(index) = path.strip_prefix("adapter.args.") {
        if decimal(index) {
            return Kind::Args;
        }
    }
    if let Some(name) = path.strip_prefix("adapter.env.") {
        if env_name(name) {
            return Kind::Env;
        }
    }
    if let Some(index) = path.strip_prefix("compatibleAgentDirs.") {
        if decimal(index) {
            return Kind::Known;
        }
    }
    Kind::Unknown
}

/// A decimal array index: `0`, `1`, `12`...
fn decimal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// An environment-name body for `$NAME`, `${NAME}` and `adapter.env.<NAME>`:
/// `[A-Z_][A-Z0-9_]*`.
fn env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_uppercase() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// True when `body` is an environment-variable reference: `$NAME` or
/// `${NAME}` with a name of `[A-Z_][A-Z0-9_]*`. Such a value exposes no
/// literal secret, so it is accepted wherever one appears.
fn env_ref(body: &str) -> bool {
    if let Some(rest) = body.strip_prefix("${") {
        match rest.strip_suffix('}') {
            Some(name) => env_name(name),
            None => false,
        }
    } else if let Some(name) = body.strip_prefix('$') {
        env_name(name)
    } else {
        false
    }
}

/// True when `body` is a keychain-proxy field name: one leading letter then
/// letters, digits, `_`, `.` or `-`, 64 characters at most — and no secret
/// shape.
fn keychain_name(body: &str) -> bool {
    let length = body.chars().count();
    if length == 0 || length > 64 {
        return false;
    }
    let mut chars = body.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && secret_shape(body).is_none()
}

/// True when the field path's last segment, lowercased, names a secret.
fn secret_named(path: &str) -> bool {
    const WORDS: [&str; 5] = ["token", "secret", "key", "password", "authorization"];
    let last = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    WORDS.iter().any(|word| last.contains(word))
}

/// One literal-secret shape: the name reported in the refusal line and the
/// matcher that recognises the shape.
pub struct SecretShape {
    pub name: &'static str,
    pub matches: fn(&str) -> bool,
}

impl SecretShape {
    fn recognised(&self, value: &str) -> bool {
        (self.matches)(value)
    }
}

/// The secret shapes the registry refuses, whatever field they appear in:
/// each one is a literal credential, so a match is a refusal, named by shape.
pub const SECRET_SHAPES: [SecretShape; 7] = [
    SecretShape {
        name: "openai key",
        matches: openai_key,
    },
    SecretShape {
        name: "github token",
        matches: github_token,
    },
    SecretShape {
        name: "slack token",
        matches: slack_token,
    },
    SecretShape {
        name: "aws access key",
        matches: aws_access_key,
    },
    SecretShape {
        name: "bearer token",
        matches: bearer_token,
    },
    SecretShape {
        name: "jwt",
        matches: jwt,
    },
    SecretShape {
        name: "long opaque token",
        matches: opaque_token,
    },
];

/// The shape a value literally contains, if any.
fn secret_shape(value: &str) -> Option<&'static str> {
    SECRET_SHAPES
        .iter()
        .find(|shape| shape.recognised(value))
        .map(|shape| shape.name)
}

/// The first place `needle` occurs in `haystack`, if it occurs at all.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// True when somewhere after `prefix` there are `least` bytes of `fits`.
fn after_prefix(value: &str, prefix: &str, fits: impl Fn(u8) -> bool, least: usize) -> bool {
    let bytes = value.as_bytes();
    let mut from = 0;
    while let Some(at) = find(&bytes[from..], prefix.as_bytes()) {
        let start = from + at + prefix.len();
        let mut seen = 0;
        while start + seen < bytes.len() && fits(bytes[start + seen]) {
            seen += 1;
        }
        if seen >= least {
            return true;
        }
        from = from + at + 1;
    }
    false
}

/// `sk-` followed by 16 or more word characters.
fn openai_key(value: &str) -> bool {
    after_prefix(value, "sk-", openai_word, 16)
}

fn openai_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// `ghp_`, `gho_`, `ghs_`, `ghu_` or `github_pat_` followed by 20 or more
/// alphanumerics or underscores.
fn github_token(value: &str) -> bool {
    const PREFIXES: [&str; 5] = ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"];
    PREFIXES
        .iter()
        .any(|prefix| after_prefix(value, prefix, github_word, 20))
}

fn github_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A Slack token: `xoxb-` or `xoxp-` anywhere in the value.
fn slack_token(value: &str) -> bool {
    value.contains("xoxb-") || value.contains("xoxp-")
}

/// An AWS access key: `AKIA` followed by 16 uppercase letters or digits.
fn aws_access_key(value: &str) -> bool {
    after_prefix(value, "AKIA", aws_key, 16)
}

fn aws_key(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}

/// `Bearer ` followed by 16 or more non-whitespace characters,
/// case-insensitively.
fn bearer_token(value: &str) -> bool {
    let needle = b"bearer ";
    let bytes = value.as_bytes();
    if bytes.len() < needle.len() {
        return false;
    }
    let mut at = 0;
    while at + needle.len() <= bytes.len() {
        if bytes[at..at + needle.len()].eq_ignore_ascii_case(needle) {
            let start = at + needle.len();
            let mut seen = 0;
            while start + seen < bytes.len() && !bytes[start + seen].is_ascii_whitespace() {
                seen += 1;
            }
            if seen >= 16 {
                return true;
            }
        }
        at += 1;
    }
    false
}

/// A JWT: three base64url segments between dots, the first starting `eyJ`.
fn jwt(value: &str) -> bool {
    let bytes = value.as_bytes();
    let b64 = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let end = |from: usize| {
        let mut at = from;
        while at < bytes.len() && b64(bytes[at]) {
            at += 1;
        }
        at
    };
    let mut from = 0;
    while let Some(at) = find(&bytes[from..], b"eyJ") {
        let head = from + at;
        let head_end = end(head);
        let dot1 = head_end < bytes.len() && bytes[head_end] == b'.';
        if dot1 {
            let middle_start = head_end + 1;
            let middle_end = end(middle_start);
            let dot2 =
                middle_end > middle_start && middle_end < bytes.len() && bytes[middle_end] == b'.';
            if dot2 {
                let tail_start = middle_end + 1;
                if end(tail_start) > tail_start {
                    return true;
                }
            }
        }
        from = head + 1;
    }
    false
}

/// Any run of 32 or more token characters with both a letter and a digit —
/// except a `/`-separated path whose every segment is shorter than 32.
fn opaque_token(value: &str) -> bool {
    let is_path = value.starts_with('/') || value.starts_with("~/") || value.starts_with("./");
    if is_path
        || (value.contains('/') && value.split('/').all(|segment| segment.chars().count() < 32))
    {
        return false;
    }
    let mut run = 0;
    let mut digit = false;
    let mut letter = false;
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-') {
            run += 1;
            digit |= c.is_ascii_digit();
            letter |= c.is_ascii_alphabetic();
            if run >= 32 && digit && letter {
                return true;
            }
        } else {
            run = 0;
            digit = false;
            letter = false;
        }
    }
    false
}

// The unit tests below execute every line of this file, which is the
// workspace's 100 percent line-coverage standard (docs/RUST.md); the
// executable line in main.rs, the exit wrapper, is the one line they cannot.
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    /// The problems a parseable fixture carries.
    fn problems_of(text: &str) -> Vec<String> {
        problems(&parse(text).expect("the fixture parses"))
    }

    /// Asserts the fixture's problems are exactly `want`, in order.
    fn expect(text: &str, want: &[&str]) {
        let found = problems_of(text);
        assert_eq!(
            found,
            want.iter().map(|line| line.to_string()).collect::<Vec<_>>(),
            "for {text:?}"
        );
    }

    /// Writes `text` to a uniquely named file under the temp directory.
    fn temp(tag: &str, text: &str) -> PathBuf {
        let name = format!("gold-band-registry-{tag}-{}", std::process::id());
        let path = std::env::temp_dir().join(name);
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn a_clean_declaration_with_paths_and_env_references_is_accepted() {
        let text = "\
# the registry's own agent
agent\tclaude-code

field\tadapter.command\t/usr/local/bin/claude
field\tadapter.displayName\tClaude Code
field\tadapter.args.0\t--continue
field\tadapter.args.12\t--dangerously-accept
field\tadapter.env.CLAUDE_CONFIG_DIR\t/Users/me/.claude
field\tadapter.env.ANTHROPIC_API_KEY\t${ANTHROPIC_API_KEY}
field\ticon\tsparkles
field\tprimaryAgentDir\t/Users/me/.claude
field\tprojectPrimaryAgentDir\t/Users/me/.claude/projects
field\tcompatibleAgentDirs.3\t/Users/me/.claude/agents
field\tsystemPromptDelivery\tmeta-append
field\texternalSessionSyncSupported\ttrue
field\texternalSessionSyncEnabled\tfalse
field\tuseLocalClaude\ttrue
field\toutputDir\t/Users/me/gold-band/out
field\tprovider\tanthropic
field\tmodel\tclaude-sonnet-4
field\treach\tread
";
        let agents = parse(text).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].line, 2);
        assert_eq!(agents[0].id, "claude-code");
        assert!(problems(&agents).is_empty());
        let file = temp("clean-declaration", text).into_os_string();
        assert_eq!(run(vec!["validate".into(), file]), 0);
    }

    #[test]
    fn the_family_field_is_a_known_plain_string_field() {
        expect(
            "agent\tzeta\nfield\tfamily\tdeepseek\nfield\treach\tread\n",
            &[],
        );
    }

    #[test]
    fn malformed_and_duplicate_ids_are_refused() {
        let text = "\
agent\tClaude
field\treach\tread
agent\tjules
field\treach\tread
agent\ta--b
field\treach\tread
agent\t
field\treach\tread
agent\tjules
field\treach\tread
agent\tjules
field\treach\tread
";
        let agents = parse(text).unwrap();
        assert_eq!(agents[3].line, 7);
        expect(
            text,
            &[
                "Claude: id must be lowercase letters and digits in dash-separated groups",
                "a--b: id must be lowercase letters and digits in dash-separated groups",
                ": id must be lowercase letters and digits in dash-separated groups",
                "jules: duplicate id",
            ],
        );
        let found = problems_of(text);
        assert_eq!(
            found.iter().filter(|l| l.contains("duplicate id")).count(),
            1
        );
    }

    #[test]
    fn codex_is_refused_wherever_it_appears() {
        let text = "\
agent\tmy-codex
field\treach\tread
field\tadapter.command\t/Applications/Codex.app/Contents/MacOS/codex
field\tadapter.args.0\tcodex exec
field\tprovider\tOpenAI Codex CLI
field\tmodel\tcodex-5
";
        expect(
            text,
            &[
                "my-codex: id must not name codex",
                "my-codex: adapter.command must not name codex",
                "my-codex: adapter.args.0 must not name codex",
                "my-codex: provider must not name codex",
                "my-codex: model must not name codex",
            ],
        );
    }

    #[test]
    fn openrouter_providers_and_values_are_accepted() {
        let text = "\
agent\tmixer
field\treach\tread
field\tprovider\topenrouter
field\tmodel\topenrouter/anthropic/claude-3
field\tadapter.args.0\t--provider openrouter
field\tadapter.env.OPENROUTER_API_URL\thttps://openrouter.ai/api/v1
";
        expect(text, &[]);
    }

    #[test]
    fn opencode_models_and_opencode_agents_are_accepted() {
        let text = "agent\tsidekick\nfield\treach\tread\nfield\tmodel\topencode/zeta\n";
        expect(text, &[]);
        let text = "\
agent\tsidekick
field\treach\tread
field\tadapter.command\t/usr/local/bin/opencode
field\tmodel\tzeta-3
";
        expect(text, &[]);
        let text = "agent\topencode\nfield\treach\tread\nfield\tmodel\tzeta-3\n";
        expect(text, &[]);
        let text = "agent\topencode\nfield\treach\tread\nfield\tmodel\topencode/zeta\n";
        expect(text, &[]);
        let text = "\
agent\topencode
field\treach\tread
field\tadapter.command\topencode serve
field\tmodel\topencode-go/zeta
";
        expect(text, &[]);
    }

    #[test]
    fn unknown_fields_are_refused_with_id_and_path() {
        let text = "\
agent\tzeta
field\treach\tread
field\tfavouriteColour\tblue
field\tadapter.args.not-a-number\t1
field\tadapter.env.lower\t2
field\tcompatibleAgentDirs.oops\t3
";
        expect(
            text,
            &[
                "zeta: unknown agent field favouriteColour",
                "zeta: unknown agent field adapter.args.not-a-number",
                "zeta: unknown agent field adapter.env.lower",
                "zeta: unknown agent field compatibleAgentDirs.oops",
            ],
        );
    }

    #[test]
    fn constrained_values_are_refused_when_not_exact() {
        let text = "\
agent\tzeta
field\treach\tread
field\tuseLocalClaude\tyes
field\tthrowawayOnly\t1
field\texternalSessionSyncSupported\tTRUE
field\texternalSessionSyncEnabled\tmaybe
field\tsystemPromptDelivery\tinline
";
        expect(
            text,
            &[
                "zeta: useLocalClaude must be \"true\" or \"false\"",
                "zeta: throwawayOnly must be \"true\" or \"false\"",
                "zeta: externalSessionSyncSupported must be \"true\" or \"false\"",
                "zeta: externalSessionSyncEnabled must be \"true\" or \"false\"",
                "zeta: systemPromptDelivery must be \"none\" or \"meta-append\"",
            ],
        );
    }

    #[test]
    fn reach_values_are_constrained() {
        let text = "agent\tzeta\nfield\treach\tbanana\n";
        expect(
            text,
            &["zeta: reach \"banana\" is not one of \
                 read, read-web, research, write-scoped, write, full, run-one-command"],
        );
        let text = "agent\tzeta\nfield\treach\tresearch\n";
        expect(text, &["zeta: reach \"research\" needs an outputDir field"]);
        let text = "agent\tzeta\nfield\treach\twrite-scoped\n";
        expect(
            text,
            &["zeta: reach \"write-scoped\" needs an outputDir field"],
        );
        let text = "agent\tzeta\nfield\treach\tfull\nfield\tthrowawayOnly\tfalse\n";
        expect(text, &["zeta: reach \"full\" needs throwawayOnly \"true\""]);
        let text = "agent\tzeta\nfield\treach\tread-web\n";
        expect(text, &[]);
        let text = "agent\tzeta\nfield\treach\twrite-scoped\nfield\toutputDir\t/Users/me/out\n";
        expect(text, &[]);
        let text = "agent\tzeta\nfield\treach\tfull\nfield\tthrowawayOnly\ttrue\n";
        expect(text, &[]);
        let text = "agent\tzeta\nfield\treach\trun-one-command\n";
        expect(text, &[]);
    }

    #[test]
    fn literal_secrets_are_refused_by_shape_without_printing_the_value() {
        let text = "\
agent\tzeta
field\treach\tread
field\tadapter.args.1\tsk-AbCdEf1234567890Xy
field\tadapter.env.GITHUB_TOKEN\tghp_AbCdEf12345678901234
field\tprovider\txoxb-1234567890ABCDEF
field\ticon\tAKIAIOSFODNN7EXAMPLE
field\tadapter.displayName\tBearer AbCdEf1234567890
field\tmodel\teyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.SflKxwRJSMeKKF2QT4f
field\tprimaryAgentDir\tAbCdEf1234567890AbCdEf1234567890A1
";
        expect(
            text,
            &[
                "zeta: adapter.args.1 holds a literal secret (openai key)",
                "zeta: adapter.env.GITHUB_TOKEN holds a literal secret (github token)",
                "zeta: provider holds a literal secret (slack token)",
                "zeta: icon holds a literal secret (aws access key)",
                "zeta: adapter.displayName holds a literal secret (bearer token)",
                "zeta: model holds a literal secret (jwt)",
                "zeta: primaryAgentDir holds a literal secret (long opaque token)",
            ],
        );
        let found = problems_of(text);
        assert!(!found.iter().any(|line| line.contains("sk-AbCdEf")));
    }

    #[test]
    fn secret_named_fields_must_hold_references_or_keychain_names() {
        let text = "\
agent\tzeta
field\treach\tread
field\tadapter.env.ANTHROPIC_API_KEY\t$ANTHROPIC_API_KEY
field\tadapter.env.OPENAI_API_KEY\t${OPENAI_API_KEY}
field\tadapter.env.GITHUB_TOKEN\tkeychain-proxy-github-token
field\tadapter.env.SLACK_TOKEN\t1slack-token
";
        expect(
            text,
            &[
                "zeta: adapter.env.SLACK_TOKEN must hold an environment reference or a keychain-proxy name",
            ],
        );
        let found = problems_of(text);
        assert!(!found.iter().any(|line| line.contains("1slack-token")));
    }

    #[test]
    fn every_secret_shape_is_recognised_and_benign_values_are_not() {
        for (shape, sample) in [
            ("openai key", "x sk-AbCdEf1234567890xx"),
            ("github token", "ghp_AbCdEf123456789012345"),
            ("slack token", "xoxp-2345678901abcdef"),
            ("aws access key", "AKIAIOSFODNN7EXAMPLE"),
            ("bearer token", "use bearer AbCdEf123456789Z please"),
            (
                "jwt",
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.SflKxwRJSMeKKF2QT4fwp",
            ),
            ("long opaque token", "AbCdEf1234567890AbCdEf1234567890A1/x"),
        ] {
            assert_eq!(secret_shape(sample), Some(shape), "for {sample}");
        }
        for prefix in ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"] {
            let sample = format!("{prefix}AbCdEf123456789012345");
            assert_eq!(secret_shape(&sample), Some("github token"), "for {sample}");
        }
        for benign in [
            "/Users/me/.claude",
            "token sk-abc here",
            "bearer short",
            "xy",
            "eyJ",
            "eyJ.sig",
            "eyJ.a.",
            "ghp_short",
            "0123456789012345678901234567890123456789",
            "abcdefghijklmnopqrstuvwxyzabcdef",
        ] {
            assert_eq!(secret_shape(benign), None, "for {benign}");
        }
        for shape in SECRET_SHAPES {
            let name = shape.name;
            assert!(!shape.recognised("/Users/me/.claude"), "{name}");
        }
    }

    #[test]
    fn reference_and_keychain_helpers_decide_exactly() {
        for body in ["$A", "$A_1", "${A}", "${_X_9}"] {
            assert!(env_ref(body), "{body}");
        }
        for body in ["a", "$a", "$1A", "${A", "${}", "A", "$A$", "${A}x", ""] {
            assert!(!env_ref(body), "{body}");
        }
        for body in ["a", "z9", "keychain-proxy-token", "A.b_c-d", "openAi"] {
            assert!(keychain_name(body), "{body}");
        }
        let long = "x".repeat(65);
        for body in [
            "",
            "1a",
            "-a",
            "a b",
            "a/b",
            long.as_str(),
            "sk-AbCdEf1234567890x",
        ] {
            assert!(!keychain_name(body), "{body}");
        }
    }

    #[test]
    fn grammar_helpers_decide_exactly() {
        assert!(decimal("0") && decimal("12"));
        assert!(!decimal("") && !decimal("1a") && !decimal("-1"));
        assert!(env_name("A") && env_name("_A_1"));
        assert!(!env_name("a") && !env_name("1A") && !env_name("A-B"));
        assert!(id_ok("a") && id_ok("opencode-2"));
        assert!(!id_ok("") && !id_ok("a-") && !id_ok("A") && !id_ok("a_b"));
        assert!(!id_ok("a--b"));
    }

    #[test]
    fn malformed_lines_are_parse_errors_naming_the_line() {
        assert_eq!(
            parse("field\tf\tv\n").unwrap_err(),
            "line 1: field before any agent"
        );
        assert_eq!(
            parse("\nagent\tz\nhello\n").unwrap_err(),
            "line 3: malformed line"
        );
        assert_eq!(
            parse("agent\tz\nagent\n").unwrap_err(),
            "line 2: malformed line"
        );
        assert_eq!(
            parse("agent\tz\textra\n").unwrap_err(),
            "line 1: malformed line"
        );
        let agents = parse("# c\n\nagent\tzeta\nfield\ticon\tx\n").unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].line, 3);
        assert_eq!(agents[0].id, "zeta");
        assert_eq!(
            agents[0].fields,
            vec![("icon".to_string(), "x".to_string())]
        );
    }

    #[test]
    fn run_answers_clean_dirty_broken_and_bad_usage() {
        let clean = temp(
            "run-clean",
            "agent\tzeta\nfield\treach\tread\nfield\tmodel\tzeta-3\n",
        )
        .into_os_string();
        assert_eq!(run(vec!["validate".into(), clean.clone()]), 0);
        let dirty = temp("run-dirty", "agent\tcodex\nfield\treach\tread\n").into_os_string();
        assert_eq!(run(vec!["validate".into(), dirty]), 1);
        let broken = temp("run-broken", "agent\tzeta\nfield\treach\tread\noops\n").into_os_string();
        assert_eq!(run(vec!["validate".into(), broken]), 2);
        assert_eq!(run(std::iter::empty()), 2);
        assert_eq!(run(vec!["dance".into()]), 2);
        assert_eq!(run(vec!["validate".into()]), 2);
        assert_eq!(run(vec!["validate".into(), clean, "extra".into()]), 2);
        assert_eq!(
            run(vec![
                "validate".into(),
                "/no-such/gold-band-registry-file".into()
            ]),
            2
        );
    }

    #[test]
    fn regression_an_agent_with_no_reach_is_refused() {
        expect(
            "agent\tzeta\nfield\tmodel\tzeta-3\n",
            &["zeta: reach is required"],
        );
    }

    #[test]
    fn regression_an_opencode_agent_with_no_model_is_accepted() {
        expect("agent\topencode\nfield\treach\tread\n", &[]);
    }

    #[test]
    fn regression_codex_is_refused_in_env_values_and_display_name() {
        expect(
            "agent\tzeta\nfield\treach\tread\nfield\tadapter.env.AGENT\tcodex\nfield\tadapter.displayName\tCodex CLI\n",
            &[
                "zeta: adapter.env.AGENT must not name codex",
                "zeta: adapter.displayName must not name codex",
            ],
        );
    }

    #[test]
    fn regression_a_directory_path_with_a_long_segment_is_not_a_secret() {
        assert_eq!(
            secret_shape("/Users/me/gold-band-registry-20261108-backup"),
            None
        );
    }

    /// One `converge` command line over temporary fixture files holding the
    /// declared line protocol, the keep ids, the retired ids and the settings
    /// document; returns its exit code.
    fn converge(
        tag: &str,
        declared: &str,
        keep: &str,
        retired: &str,
        settings: &Path,
        check: bool,
    ) -> i32 {
        let mut args = vec![
            OsString::from("converge"),
            OsString::from("--declared"),
            temp(&format!("{tag}-declared"), declared).into_os_string(),
            OsString::from("--keep"),
            temp(&format!("{tag}-keep"), keep).into_os_string(),
            OsString::from("--retired"),
            temp(&format!("{tag}-retired"), retired).into_os_string(),
            OsString::from("--settings"),
            settings.as_os_str().to_os_string(),
        ];
        if check {
            args.push(OsString::from("--check"));
        }
        run(args)
    }

    /// The `agents` map of a settings document, parsed.
    fn agents_of(path: &std::path::Path) -> Map<String, Value> {
        let written: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        written.get("agents").unwrap().as_object().unwrap().clone()
    }

    #[test]
    fn converge_keeps_the_kept_strips_builtins_and_preserves_the_rest() {
        let declared = "agent\topencode\nfield\tadapter.command\t/usr/local/bin/opencode\nfield\tadapter.args.0\tserve\nfield\tadapter.args.1\t--quiet\nfield\tadapter.displayName\tOpenCode\nfield\tadapter.env.OPENCODE_CONFIG\topencode.json\nfield\ticon\topencode\nfield\tprimaryAgentDir\t.opencode\nfield\tcompatibleAgentDirs.0\t.agents\nfield\tsystemPromptDelivery\tnone\nfield\texternalSessionSyncSupported\tfalse\nfield\texternalSessionSyncEnabled\ttrue\nfield\treach\twrite\nfield\tmodel\topencode-go/kimi-k2.7-code\nagent\tmy-agent\nfield\tadapter.command\t/usr/local/bin/my-agent\nfield\tadapter.args.0\t--continue\nfield\tadapter.displayName\tMy Agent\nfield\ticon\tsparkles\nfield\treach\tread\nfield\tmodel\tzeta-3\n";
        // A stray agent, a kept one, a non-object entry, a retired one kept by
        // the keep file, a codex one the keep file names too, and the app's
        // own injected command inside the built-in's entry.
        let json = "{\"theme\":\"dark\",\"agents\":{\"stray\":{\"icon\":\"stray\"},\"mine\":{\"icon\":\"mine\"},\"weird\":\"huh\",\"old-friend\":{\"icon\":\"old\"},\"codex-acp\":{\"command\":\"codex\"},\"opencode\":{\"adapter\":{\"displayName\":\"Wrong\"}}},\"window\":{\"width\":1280}}";
        let settings = temp("cv-full-settings", json);
        let keep = "mine\nweird\n\nold-friend\ncodex-acp\n\n";
        let retired = "old-friend\n";
        assert_eq!(
            converge("cv-full", declared, keep, retired, &settings, false),
            0
        );
        let written: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        let document = written.as_object().unwrap();
        // Every other key keeps its value and its key order.
        assert_eq!(
            document.keys().map(String::as_str).collect::<Vec<_>>(),
            ["theme", "agents", "window"]
        );
        assert_eq!(document.get("theme"), Some(&Value::from("dark")));
        assert_eq!(
            document.get("window"),
            Some(&serde_json::json!({"width": 1280}))
        );
        let agents = agents_of(&settings);
        // The declared agents first, in declaration order, then the kept ones.
        assert_eq!(
            agents.keys().map(String::as_str).collect::<Vec<_>>(),
            ["opencode", "my-agent", "mine", "weird"]
        );
        let expected: Value = serde_json::json!({
            "opencode": {
                "adapter": {
                    "displayName": "OpenCode",
                    "env": {"OPENCODE_CONFIG": "opencode.json"}
                },
                "icon": "opencode",
                "primaryAgentDir": ".opencode",
                "compatibleAgentDirs": [".agents"],
                "systemPromptDelivery": "none",
                "externalSessionSyncSupported": false,
                "externalSessionSyncEnabled": true
            },
            "my-agent": {
                "adapter": {
                    "command": "/usr/local/bin/my-agent",
                    "args": ["--continue"],
                    "displayName": "My Agent"
                },
                "icon": "sparkles"
            },
            "mine": {"icon": "mine"}
        });
        let want = expected.as_object().unwrap();
        for id in ["opencode", "my-agent", "mine"] {
            assert_eq!(agents.get(id), want.get(id), "for {id}");
        }
        assert_eq!(agents.get("weird"), Some(&Value::from("huh")));
        // Undeclared and unkept, retired though kept, and codex though kept.
        for id in ["stray", "old-friend", "codex-acp"] {
            assert!(!agents.contains_key(id), "{id} should be gone");
        }
        // The old document was backed up before anything was written.
        let name = settings.file_name().unwrap().to_string_lossy().into_owned();
        let backup = settings.with_file_name(format!("{name}.bak"));
        assert_eq!(fs::read_to_string(&backup).unwrap(), json);
        // A second run changes nothing, by no one.
        let bytes = fs::read_to_string(&settings).unwrap();
        assert_eq!(
            converge("cv-full", declared, keep, retired, &settings, false),
            0
        );
        assert_eq!(fs::read_to_string(&settings).unwrap(), bytes);
        assert_eq!(fs::read_to_string(&backup).unwrap(), json);
    }

    #[test]
    fn a_declared_use_local_claude_reaches_the_settings_entry() {
        let declared = "agent\tzeta\nfield\tuseLocalClaude\ttrue\nfield\treach\tread\n";
        let settings = temp("cv-use-local-claude", "{}");
        assert_eq!(converge("cv-ulc", declared, "", "", &settings, false), 0);
        let agents = agents_of(&settings);
        assert_eq!(
            agents
                .get("zeta")
                .and_then(|entry| entry.get("useLocalClaude")),
            Some(&Value::Bool(true)),
            "a declared useLocalClaude is validated and then dropped"
        );
    }

    #[test]
    fn check_reports_without_writing_and_a_missing_file_starts_from_empty() {
        let declared = "agent\topencode\nfield\ticon\topencode\nfield\treach\twrite\nfield\tmodel\topencode-go/kimi-k2.7-code\n";
        let home = std::env::temp_dir().join(format!(
            "gold-band-registry-cv-check-home-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&home);
        let settings = home.join("settings.json");
        // A checked run creates neither the directory nor the file.
        assert_eq!(converge("cv-check", declared, "", "", &settings, true), 0);
        assert!(!settings.exists());
        // A real run creates both, without a backup of what did not exist.
        assert_eq!(converge("cv-check", declared, "", "", &settings, false), 0);
        let bytes = fs::read_to_string(&settings).unwrap();
        assert!(bytes.contains("opencode"));
        let name = settings.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!settings.with_file_name(format!("{name}.bak")).exists());
        // A checked run on the converged document then changes nothing, and
        // neither does the unchanged second run.
        assert_eq!(converge("cv-check", declared, "", "", &settings, true), 0);
        assert_eq!(fs::read_to_string(&settings).unwrap(), bytes);
        assert_eq!(converge("cv-check", declared, "", "", &settings, false), 0);
        assert_eq!(fs::read_to_string(&settings).unwrap(), bytes);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn a_refused_or_malformed_declaration_writes_nothing() {
        let json = "{\"agents\":{\"mine\":{\"icon\":\"mine\"}}}";
        let settings = temp("cv-refused-settings", json);
        assert_eq!(
            converge(
                "cv-refused",
                "agent\tmy-codex\nfield\treach\tread\n",
                "mine\n",
                "",
                &settings,
                false
            ),
            1
        );
        assert_eq!(
            converge(
                "cv-malformed",
                "agent\tzeta\nfield\treach\tread\noops\n",
                "mine\n",
                "",
                &settings,
                false
            ),
            2
        );
        assert_eq!(fs::read_to_string(&settings).unwrap(), json);
    }

    #[test]
    fn a_settings_document_that_is_not_an_object_exits_two() {
        for (tag, text) in [("cv-broken", "not json at all"), ("cv-array", "[1, 2, 3]")] {
            let settings = temp(tag, text);
            assert_eq!(
                converge(
                    tag,
                    "agent\tzeta\nfield\treach\tread\n",
                    "",
                    "",
                    &settings,
                    false
                ),
                2
            );
            assert_eq!(fs::read_to_string(&settings).unwrap(), text);
        }
        // A directory in place of the settings file is unreadable, not missing.
        let dir =
            std::env::temp_dir().join(format!("gold-band-registry-cv-dir-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            converge(
                "cv-dir",
                "agent\tzeta\nfield\treach\tread\n",
                "",
                "",
                &dir,
                false
            ),
            2
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_settings_agents_value_that_is_not_an_object_is_replaced() {
        let settings = temp("cv-agents-list", "{\"agents\":[\"stray\"]}");
        assert_eq!(
            converge(
                "cv-agents-list",
                "agent\tzeta\nfield\treach\tread\n",
                "",
                "",
                &settings,
                false
            ),
            0
        );
        let agents = agents_of(&settings);
        assert!(agents.contains_key("zeta"));
        assert!(!agents.contains_key("stray"));
    }

    #[test]
    fn a_backup_or_staged_write_that_cannot_happen_exits_two() {
        let declared = "agent\tzeta\nfield\ticon\tsparkles\nfield\treach\tread\n";
        let json = "{\"agents\":{\"stray\":{\"icon\":\"stray\"}}}";
        let keep = "mine\n";
        // The old file cannot be backed up: its .bak is a directory already.
        let blocked = temp("cv-blocked-settings", json);
        let name = blocked.file_name().unwrap().to_string_lossy().into_owned();
        let backup = blocked.with_file_name(format!("{name}.bak"));
        fs::create_dir_all(&backup).unwrap();
        assert_eq!(
            converge("cv-blocked", declared, keep, "", &blocked, false),
            2
        );
        assert_eq!(fs::read_to_string(&blocked).unwrap(), json);
        let _ = fs::remove_dir_all(&backup);
        // The staged file cannot be written: it is a directory already.
        let staged_blocked = temp("cv-staged-settings", json);
        let same = staged_blocked
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let staged = staged_blocked.with_file_name(format!("{same}.{}.tmp", std::process::id()));
        fs::create_dir_all(&staged).unwrap();
        assert_eq!(
            converge("cv-staged", declared, keep, "", &staged_blocked, false),
            2
        );
        assert_eq!(fs::read_to_string(&staged_blocked).unwrap(), json);
        let _ = fs::remove_dir_all(&staged);
        let _ = fs::remove_file(staged_blocked.with_file_name(format!("{same}.bak")));
    }

    #[test]
    fn converge_bad_usage_and_unreadable_files_exit_two() {
        let flag = |name: &str| OsString::from(name);
        let declared =
            temp("cv-usage-declared", "agent\tzeta\nfield\treach\tread\n").into_os_string();
        let settings = temp("cv-usage-settings", "{}").into_os_string();
        let missing = OsString::from("/no-such/gold-band-registry-file");
        assert_eq!(run(vec![flag("converge")]), 2);
        assert_eq!(
            run(vec![flag("converge"), flag("--declared"), declared.clone()]),
            2
        );
        assert_eq!(run(vec![flag("converge"), flag("--declared")]), 2);
        assert_eq!(run(vec![flag("converge"), flag("--dance")]), 2);
        assert_eq!(
            run(vec![flag("converge"), flag("--settings"), settings.clone()]),
            2
        );
        // The declared, keep and retired files must each be readable.
        assert_eq!(
            run(vec![
                flag("converge"),
                flag("--declared"),
                missing.clone(),
                flag("--keep"),
                missing.clone(),
                flag("--retired"),
                missing.clone(),
                flag("--settings"),
                settings.clone(),
            ]),
            2
        );
        assert_eq!(
            run(vec![
                flag("converge"),
                flag("--declared"),
                declared.clone(),
                flag("--keep"),
                missing.clone(),
                flag("--retired"),
                missing.clone(),
                flag("--settings"),
                settings.clone(),
            ]),
            2
        );
        // The full command line answers 0 on an empty document.
        assert_eq!(
            run(vec![
                flag("converge"),
                flag("--declared"),
                declared,
                flag("--keep"),
                temp("cv-usage-keep", "mine\n").into_os_string(),
                flag("--retired"),
                temp("cv-usage-retired", "").into_os_string(),
                flag("--settings"),
                settings,
            ]),
            0
        );
    }

    #[test]
    fn regression_the_rewritten_settings_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let declared = "agent\tzeta\nfield\treach\tread\n";
        let settings = temp("cv-mode", "{}");
        fs::set_permissions(&settings, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(converge("cv-mode", declared, "", "", &settings, false), 0);
        let mode = fs::metadata(&settings).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_leading_home_shortcut_expands_and_anything_else_stays_declared() {
        let home = std::ffi::OsStr::new("/Users/example");
        assert_eq!(
            expand_home("~/.local/bin/vibe-acp", Some(home)),
            "/Users/example/.local/bin/vibe-acp"
        );
        // No leading ~/: an absolute command is written as declared.
        assert_eq!(
            expand_home("/opt/homebrew/bin/agy", Some(home)),
            "/opt/homebrew/bin/agy"
        );
        // ~ without the slash is not the home shortcut.
        assert_eq!(expand_home("~other/x", Some(home)), "~other/x");
        // No home set, or an empty one: writing as declared beats an empty
        // path, and validation never sees an expansion.
        assert_eq!(expand_home("~/bin/x", None), "~/bin/x");
        assert_eq!(
            expand_home("~/bin/x", Some(std::ffi::OsStr::new(""))),
            "~/bin/x"
        );
    }

    #[test]
    fn converge_writes_home_expanded_absolute_adapter_paths() {
        let home = std::env::var_os("HOME").expect("a home directory");
        let prefix = home.to_string_lossy();
        let declared = "agent\tvibe\nfield\tadapter.command\t~/.local/bin/vibe-acp\nfield\tadapter.displayName\tVibe\nfield\treach\twrite\nagent\texotic\nfield\tadapter.command\t/opt/homebrew/bin/exotic\nfield\tadapter.args.0\t~/.config/exotic\nfield\tadapter.displayName\tExotic\nfield\treach\twrite\n";
        let settings = temp("cv-expand", "{}");
        assert_eq!(converge("cv-expand", declared, "", "", &settings, false), 0);
        let agents = agents_of(&settings);
        let command = agents
            .get("vibe")
            .and_then(|entry| entry.get("adapter"))
            .and_then(|adapter| adapter.get("command"));
        assert_eq!(
            command,
            Some(&Value::String(format!("{prefix}/.local/bin/vibe-acp")))
        );
        let args = agents
            .get("exotic")
            .and_then(|entry| entry.get("adapter"))
            .and_then(|adapter| adapter.get("args"));
        assert_eq!(
            args,
            Some(&Value::Array(vec![Value::String(format!(
                "{prefix}/.config/exotic"
            ))]))
        );
        let display = agents
            .get("exotic")
            .and_then(|entry| entry.get("adapter"))
            .and_then(|adapter| adapter.get("displayName"));
        assert_eq!(display, Some(&Value::String("Exotic".to_string())));
        // The expansion is deterministic, so a second run changes nothing.
        let bytes = fs::read_to_string(&settings).unwrap();
        assert_eq!(converge("cv-expand", declared, "", "", &settings, false), 0);
        assert_eq!(fs::read_to_string(&settings).unwrap(), bytes);
    }

    #[test]
    fn converge_expands_a_leading_home_shortcut_in_adapter_env_values() {
        let home = std::env::var_os("HOME").expect("a home directory");
        let prefix = home.to_string_lossy();
        // claude-acp is a built-in id, so its env is written with neither
        // command nor args; a custom id shows an env value with no leading
        // `~/` staying exactly as declared.
        let declared = "agent\tclaude-acp\nfield\tadapter.displayName\tClaude (work)\nfield\tadapter.env.CLAUDE_CONFIG_DIR\t~/.claude\nfield\ticon\tclaude\nfield\tprimaryAgentDir\t.claude\nfield\tsystemPromptDelivery\tmeta-append\nfield\treach\twrite\nagent\tclaude-personal\nfield\tadapter.command\tnpx\nfield\tadapter.args.0\t-y\nfield\tadapter.args.1\t@agentclientprotocol/claude-agent-acp@0.87.0\nfield\tadapter.displayName\tClaude (personal)\nfield\tadapter.env.CLAUDE_CONFIG_DIR\t~/.claude-personal\nfield\tadapter.env.PASSTHROUGH\tas-declared\nfield\ticon\tclaude\nfield\tprimaryAgentDir\t.claude-personal\nfield\tsystemPromptDelivery\tmeta-append\nfield\treach\twrite\n";
        let settings = temp("cv-env-expand", "{}");
        assert_eq!(
            converge("cv-env-expand", declared, "", "", &settings, false),
            0
        );
        let agents = agents_of(&settings);
        let env = |id: &str| {
            agents
                .get(id)
                .and_then(|entry| entry.get("adapter"))
                .and_then(|adapter| adapter.get("env"))
                .cloned()
        };
        assert_eq!(
            env("claude-acp"),
            Some(serde_json::json!({
                "CLAUDE_CONFIG_DIR": format!("{prefix}/.claude")
            }))
        );
        assert_eq!(
            env("claude-personal"),
            Some(serde_json::json!({
                "CLAUDE_CONFIG_DIR": format!("{prefix}/.claude-personal"),
                "PASSTHROUGH": "as-declared"
            }))
        );
        // The built-in id carries neither command nor args, the custom one
        // keeps its declared npx launch untouched by the expansion.
        assert_eq!(
            agents
                .get("claude-acp")
                .and_then(|entry| entry.get("adapter"))
                .and_then(|adapter| adapter.get("command")),
            None
        );
        assert_eq!(
            agents
                .get("claude-personal")
                .and_then(|entry| entry.get("adapter"))
                .and_then(|adapter| adapter.get("command")),
            Some(&Value::String("npx".to_string()))
        );
        // The expansion is deterministic, so a second run changes nothing.
        let bytes = fs::read_to_string(&settings).unwrap();
        assert_eq!(
            converge("cv-env-expand", declared, "", "", &settings, false),
            0
        );
        assert_eq!(fs::read_to_string(&settings).unwrap(), bytes);
    }
}
