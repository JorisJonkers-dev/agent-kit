//! The Gold Band instance registry: declares the two Gold Band instances and
//! refuses a declaration that crosses an instance boundary (ADR-0011).
//!
//! Python flattens the registry's `gold_band_instances` section, and the
//! per-machine overlay when one exists, into the line protocol this crate
//! reads: `source<TAB>registry|overlay` starts a section,
//! `instance<TAB><id>` opens one instance's declaration inside it,
//! `field<TAB><path><TAB><value>` sets one field (nested mappings and lists
//! become dotted paths, list items numbered), and blank or `#` lines are
//! ignored. Each rule problem is one line `<where>: <what>`, never printing a
//! secret value.
//!
//! Registry section, one declaration for each of [`INSTANCES`]:
//! * `agents.N`: agent ids of lowercase letters and digits in dash-separated
//!   groups, each declared once. The work instance `gold-band` holds only
//!   `claude-work` and `codex`; the personal instance `gold-band-personal`
//!   holds any other agent, and never Codex or the work Claude agent;
//! * `repository_rule.remotes` and `repository_rule.no_remote`, both
//!   required: work is `nedap-only` and `refuse`, personal is `not-nedap`
//!   and `allow`. An instance declared without its rule, or with the other
//!   instance's, is refused;
//! * `hosts.N`: `linux` or `mac`; the work instance runs on the work machine
//!   (`mac`) only;
//! * `banks.N`: `shared` or `nedap`; the work instance names `nedap`, the
//!   personal instance never does (Private Memory is reachable only from
//!   vibe-private, so no instance names it);
//! * any other field is refused.
//!
//! Overlay section (`~/.config/agent-kit/overlay.yaml`, beside the keep
//! file), instances limited to [`INSTANCES`]:
//! * `agents.N`, held to the same agent rules as the registry's;
//! * `mcp_servers.<name>.{command|url, args.N, env.NAME}` and
//!   `settings.<path>`, which only the work instance may take;
//! * any other field (hosts, banks, the repository rule) is refused: the
//!   overlay adds, it never relaxes a rule.
//!
//! Everywhere: no literal secret in any value (a secret-named field holds an
//! environment reference, `$NAME`), unknown fields refused.
//!
//! Usage: `gold-band-registry validate <FILE>`. Exit 0 clean (printing
//! `ok (<n> instances)`), 1 rule problems on stderr, 2 bad usage, an
//! unreadable file or malformed input.

mod secrets;

use std::ffi::{OsStr, OsString};
use std::fs;

use secrets::{env_name, secret_problem};

/// The crate's name, which prefixes every message line.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The work instance's id.
pub const WORK: &str = "gold-band";
/// The personal instance's id.
pub const PERSONAL: &str = "gold-band-personal";
/// The two Gold Band instances.
pub const INSTANCES: [&str; 2] = [WORK, PERSONAL];

/// The only agent ids the work instance may hold.
pub const WORK_AGENTS: [&str; 2] = ["claude-work", "codex"];

/// Where a declaration came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Registry,
    Overlay,
}

impl Source {
    fn label(self) -> &'static str {
        match self {
            Source::Registry => "registry",
            Source::Overlay => "overlay",
        }
    }
}

/// One instance's declaration: its source, id, the 1-based line it opened on
/// and its fields in declaration order, each a (path, value) pair.
#[derive(Debug)]
pub struct Instance {
    pub source: Source,
    pub id: String,
    pub line: usize,
    pub fields: Vec<(String, String)>,
}

/// Parses the line protocol, or names the first malformed line.
pub fn parse(text: &str) -> Result<Vec<Instance>, String> {
    let mut instances: Vec<Instance> = Vec::new();
    let mut source = None;
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        if line.trim().is_empty() || line.trim().starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        match (parts.as_slice(), source) {
            (["source", "registry"], _) => source = Some(Source::Registry),
            (["source", "overlay"], _) => source = Some(Source::Overlay),
            (["instance", id], Some(source)) => instances.push(Instance {
                source,
                id: (*id).to_string(),
                line: number,
                fields: Vec::new(),
            }),
            (["field", path, value], _) if !instances.is_empty() => instances
                .last_mut()
                .expect("checked non-empty")
                .fields
                .push(((*path).to_string(), (*value).to_string())),
            _ => return Err(format!("line {number}: not a declaration line")),
        }
    }
    Ok(instances)
}

/// Every rule problem in a parsed declaration, in a stable order.
pub fn problems(instances: &[Instance]) -> Vec<String> {
    let mut out = Vec::new();
    for source in [Source::Registry, Source::Overlay] {
        let mut seen: Vec<&str> = Vec::new();
        for instance in instances.iter().filter(|i| i.source == source) {
            let place = format!("{} {}", source.label(), shown(&instance.id));
            if !INSTANCES.contains(&instance.id.as_str()) {
                out.push(format!(
                    "{place}: unknown instance (only {WORK} and {PERSONAL})"
                ));
                continue;
            }
            if seen.contains(&instance.id.as_str()) {
                out.push(format!("{place}: declared more than once"));
                continue;
            }
            seen.push(&instance.id);
            match source {
                Source::Registry => registry_problems(instance, &place, &mut out),
                Source::Overlay => overlay_problems(instance, &place, &mut out),
            }
        }
        if source == Source::Registry {
            for id in INSTANCES {
                if !seen.contains(&id) {
                    out.push(format!("registry {id}: instance is not declared"));
                }
            }
        }
    }
    out
}

fn registry_problems(instance: &Instance, place: &str, out: &mut Vec<String>) {
    let id = instance.id.as_str();
    let work = id == WORK;
    let mut agents = Vec::new();
    let mut remotes = None;
    let mut no_remote = None;
    refuse_repeats(instance, place, out);
    let mut hosts = Vec::new();
    let mut banks = Vec::new();
    for (path, value) in &instance.fields {
        secret_check(place, path, value, out);
        match path.split_once('.') {
            Some(("agents", n)) if decimal(n) => agents.push(value.as_str()),
            Some(("hosts", n)) if decimal(n) => hosts.push(value.as_str()),
            Some(("banks", n)) if decimal(n) => banks.push(value.as_str()),
            Some(("repository_rule", "remotes")) => remotes = Some(value.as_str()),
            Some(("repository_rule", "no_remote")) => no_remote = Some(value.as_str()),
            _ => out.push(format!("{place}: unknown field {}", shown(path))),
        }
    }
    if agents.is_empty() {
        out.push(format!("{place}: declares no agents"));
    }
    agent_problems(id, place, &agents, out);

    let (want_remotes, want_no_remote) = if work {
        ("nedap-only", "refuse")
    } else {
        ("not-nedap", "allow")
    };
    for (name, got, want) in [
        ("remotes", remotes, want_remotes),
        ("no_remote", no_remote, want_no_remote),
    ] {
        match got {
            None => out.push(format!(
                "{place}: repository_rule.{name} is missing (must be {want})"
            )),
            Some(got) if got != want => out.push(format!(
                "{place}: repository_rule.{name} is {}, must be {want}",
                shown(got)
            )),
            Some(_) => {}
        }
    }

    if hosts.is_empty() {
        out.push(format!("{place}: declares no hosts"));
    }
    for host in &hosts {
        if !matches!(*host, "linux" | "mac") {
            out.push(format!("{place}: unknown host {} (linux or mac)", shown(host)));
        } else if work && *host == "linux" {
            out.push(format!("{place}: runs on the work machine (mac) only"));
        }
    }

    if banks.is_empty() {
        out.push(format!("{place}: declares no banks"));
    }
    for bank in &banks {
        if !matches!(*bank, "shared" | "nedap") {
            out.push(format!("{place}: unknown bank {} (shared or nedap)", shown(bank)));
        } else if !work && *bank == "nedap" {
            out.push(format!("{place}: the nedap bank is reachable only from {WORK}"));
        }
    }
    if work && !banks.contains(&"nedap") && !banks.is_empty() {
        out.push(format!("{place}: must name the nedap bank"));
    }
}

fn overlay_problems(instance: &Instance, place: &str, out: &mut Vec<String>) {
    let id = instance.id.as_str();
    let work = id == WORK;
    refuse_repeats(instance, place, out);
    let mut agents = Vec::new();
    let mut servers: Vec<(&str, bool, bool)> = Vec::new();
    for (path, value) in &instance.fields {
        secret_check(place, path, value, out);
        let (head, rest) = path.split_once('.').unwrap_or((path, ""));
        match head {
            "agents" if decimal(rest) => agents.push(value.as_str()),
            "mcp_servers" | "settings" if !work => out.push(format!(
                "{place}: the overlay may add {head} to the work instance {WORK} only"
            )),
            "mcp_servers" if work => mcp_field(place, path, rest, &mut servers, out),
            "settings" if work => {
                if rest.is_empty() || rest.split('.').any(|s| !setting_segment(s)) {
                    out.push(format!("{place}: bad settings path {}", shown(path)));
                }
            }
            _ => out.push(format!("{place}: unknown field {}", shown(path))),
        }
    }
    agent_problems(id, place, &agents, out);
    for (name, has_command, has_url) in servers {
        if has_command == has_url {
            out.push(format!(
                "{place}: mcp server {} needs exactly one of command or url",
                shown(name)
            ));
        }
    }
}

/// Checks one `mcp_servers.<name>.<field>` path and records the server.
fn mcp_field<'a>(
    place: &str,
    path: &str,
    rest: &'a str,
    servers: &mut Vec<(&'a str, bool, bool)>,
    out: &mut Vec<String>,
) {
    let Some((name, field)) = rest.split_once('.') else {
        out.push(format!("{place}: unknown field {}", shown(path)));
        return;
    };
    if !id_ok(name) {
        out.push(format!("{place}: bad mcp server name {}", shown(name)));
    }
    if !servers.iter().any(|s| s.0 == name) {
        servers.push((name, false, false));
    }
    let entry = servers.iter_mut().find(|s| s.0 == name).expect("just added");
    match field.split_once('.') {
        None if field == "command" => entry.1 = true,
        None if field == "url" => entry.2 = true,
        Some(("args", n)) if decimal(n) => {}
        Some(("env", var)) if env_name(var) => {}
        _ => out.push(format!("{place}: unknown field {}", shown(path))),
    }
}

/// The agent rules shared by the registry and the overlay.
fn agent_problems(instance: &str, place: &str, agents: &[&str], out: &mut Vec<String>) {
    let mut seen: Vec<&str> = Vec::new();
    for agent in agents {
        if !id_ok(agent) {
            out.push(format!("{place}: bad agent id {:?}", shown(agent)));
            continue;
        }
        if seen.contains(agent) {
            out.push(format!("{place}: agent {agent} declared more than once"));
            continue;
        }
        seen.push(agent);
        if instance == WORK {
            if !WORK_AGENTS.contains(agent) {
                out.push(format!(
                    "{place}: agent {agent} is not allowed on the work instance (only claude-work and codex)"
                ));
            }
        } else if agent.contains("codex") || *agent == "claude-work" || agent.starts_with("claude-work-")
        {
            out.push(format!(
                "{place}: agent {agent} is not allowed on the personal instance (Codex and the work Claude agent stay on {WORK})"
            ));
        }
    }
}

/// A path set twice would let the last value win, so a repeat is refused.
fn refuse_repeats(instance: &Instance, place: &str, out: &mut Vec<String>) {
    let mut seen: Vec<&str> = Vec::new();
    for (path, _) in &instance.fields {
        if seen.contains(&path.as_str()) {
            out.push(format!("{place}: field {} is set more than once", shown(path)));
        } else {
            seen.push(path);
        }
    }
}

/// A value as a message may show it: never one that is a literal secret.
fn shown(value: &str) -> &str {
    if secret_problem("value", value).is_some() {
        "<redacted>"
    } else {
        value
    }
}

fn secret_check(place: &str, path: &str, value: &str, out: &mut Vec<String>) {
    for segment in path.split('.') {
        if let Some(shape) = secrets::secret_shape_of(segment) {
            out.push(format!("{place}: a field name holds a literal secret ({shape})"));
        }
    }
    if let Some(problem) = secret_problem(path, value) {
        out.push(format!("{place}: {problem}"));
    }
}

/// The id grammar: lowercase letters and digits in dash-separated groups.
fn id_ok(id: &str) -> bool {
    id.split('-').all(|group| {
        !group.is_empty()
            && group
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}

fn decimal(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// One segment of a settings path.
fn setting_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn usage() -> String {
    format!("usage: {NAME} validate <FILE>")
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    match args.as_slice() {
        [cmd, file] if cmd == OsStr::new("validate") => validate_file(file),
        _ => {
            eprintln!("{}", usage());
            2
        }
    }
}

fn validate_file(file: &OsStr) -> i32 {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("{NAME}: cannot read {}: {error}", file.to_string_lossy());
            return 2;
        }
    };
    let instances = match parse(&text) {
        Ok(instances) => instances,
        Err(error) => {
            eprintln!("{NAME}: {error}");
            return 2;
        }
    };
    let found = problems(&instances);
    if found.is_empty() {
        let declared = instances
            .iter()
            .filter(|i| i.source == Source::Registry)
            .count();
        println!("ok ({declared} instances)");
        0
    } else {
        for problem in &found {
            eprintln!("{NAME}: {problem}");
        }
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const WORK_OK: &str = "instance\tgold-band\n\
        field\tagents.0\tclaude-work\nfield\tagents.1\tcodex\n\
        field\trepository_rule.remotes\tnedap-only\nfield\trepository_rule.no_remote\trefuse\n\
        field\thosts.0\tmac\nfield\tbanks.0\tnedap\nfield\tbanks.1\tshared\n";
    const PERSONAL_OK: &str = "instance\tgold-band-personal\n\
        field\tagents.0\tvibe\nfield\tagents.1\topencode\n\
        field\trepository_rule.remotes\tnot-nedap\nfield\trepository_rule.no_remote\tallow\n\
        field\thosts.0\tlinux\nfield\thosts.1\tmac\nfield\tbanks.0\tshared\n";

    fn registry() -> String {
        format!("source\tregistry\n{WORK_OK}{PERSONAL_OK}")
    }

    fn found(text: &str) -> Vec<String> {
        problems(&parse(text).expect("parses"))
    }

    fn has(text: &str, needle: &str) {
        let all = found(text);
        assert!(all.iter().any(|p| p.contains(needle)), "{needle} in {all:?}");
    }

    fn overlay(body: &str) -> String {
        format!("{}source\toverlay\n{body}", registry())
    }

    #[test]
    fn a_correct_registry_is_clean() {
        assert_eq!(found(&registry()), Vec::<String>::new());
    }

    #[test]
    fn missing_and_duplicate_and_unknown_instances_are_refused() {
        has("source\tregistry\n", "registry gold-band: instance is not declared");
        has("source\tregistry\n", "registry gold-band-personal: instance is not declared");
        has(&format!("{}{WORK_OK}", registry()), "declared more than once");
        has(&format!("{}instance\tother\n", registry()), "unknown instance");
    }

    #[test]
    fn work_instance_agents_are_exactly_claude_work_and_codex() {
        let text = registry().replace("claude-work", "vibe");
        has(&text, "agent vibe is not allowed on the work instance");
        has(&registry().replace("claude-work", "Bad_Id"), "bad agent id");
        has(&format!("{}field\tagents.2\tcodex\n", registry().replace(PERSONAL_OK, "")), "declared more than once");
    }

    #[test]
    fn personal_instance_refuses_codex_and_the_work_claude_agent() {
        for bad in ["codex", "codex-review", "claude-work", "claude-work-2"] {
            let text = registry().replace("agents.1\topencode", &format!("agents.1\t{bad}"));
            has(&text, &format!("agent {bad} is not allowed on the personal instance"));
        }
        assert!(found(&registry().replace("opencode", "claude-personal")).is_empty());
    }

    #[test]
    fn repository_rules_are_required_and_exact() {
        let no_rule = registry().replace("field\trepository_rule.remotes\tnedap-only\n", "");
        has(&no_rule, "repository_rule.remotes is missing (must be nedap-only)");
        let no_remote = registry().replace("field\trepository_rule.no_remote\tallow\n", "");
        has(&no_remote, "repository_rule.no_remote is missing (must be allow)");
        has(&registry().replace("nedap-only", "any"), "remotes is any, must be nedap-only");
        has(&registry().replace("not-nedap", "nedap-only"), "remotes is nedap-only, must be not-nedap");
        has(&registry().replace("\trefuse", "\tallow"), "no_remote is allow, must be refuse");
    }

    #[test]
    fn hosts_and_banks_are_checked() {
        has(&registry().replace("hosts.0\tmac", "hosts.0\tlinux"), "work machine (mac) only");
        has(&registry().replace("hosts.0\tlinux", "hosts.0\twindows"), "unknown host windows");
        has(&registry().replace("banks.0\tnedap\n", "banks.0\tshared\n"), "must name the nedap bank");
        has(&registry().replace("banks.0\tshared\n", "banks.0\tnedap\n"), "reachable only from gold-band");
        has(&registry().replace("banks.0\tshared\n", "banks.0\tprivate\n"), "unknown bank private");
    }

    #[test]
    fn repeated_fields_are_refused() {
        let text = format!("{}field\trepository_rule.remotes\tnedap-only\n", registry());
        has(&text, "field repository_rule.remotes is set more than once");
        has(&overlay("instance\tgold-band\nfield\tsettings.a\tb\nfield\tsettings.a\tc\n"), "settings.a is set more than once");
    }

    #[test]
    fn secrets_in_names_and_ids_are_refused_and_never_printed() {
        let secret = "ghp_aaaaaaaaaaaaaaaaaaaaaaaa";
        let body = format!("instance\t{secret}\ninstance\tgold-band\nfield\tsettings.{secret}\tx\nfield\tmcp_servers.{secret}.url\tx\n");
        let all = found(&overlay(&body));
        assert!(all.iter().any(|p| p.contains("a field name holds a literal secret (github token)")), "{all:?}");
        assert!(all.iter().any(|p| p.contains("unknown instance")));
        assert!(all.iter().all(|p| !p.contains(secret)), "{all:?}");
        let upper = "AKIAAAAAAAAAAAAAAAAA";
        let body = format!("instance\tgold-band\nfield\tmcp_servers.{upper}.command\tx\nfield\tmcp_servers.a.env.{upper}\tx\nfield\tmcp_servers.b.url\tx\nfield\tmcp_servers.b.command\tx\n");
        let all = found(&overlay(&body));
        assert!(all.iter().all(|p| !p.contains(upper)), "{all:?}");
    }

    #[test]
    fn empty_lists_are_refused() {
        let bare = "source\tregistry\ninstance\tgold-band\ninstance\tgold-band-personal\n";
        for what in ["declares no agents", "declares no hosts", "declares no banks"] {
            has(bare, what);
        }
    }

    #[test]
    fn unknown_fields_and_literal_secrets_are_refused() {
        has(&format!("{}field\tmodel\tx\n", registry()), "unknown field model");
        has(&format!("{}field\tagents\tx\n", registry()), "unknown field agents");
        has(&format!("{}field\tagents.x\tx\n", registry()), "unknown field agents.x");
        has(&format!("{}field\tbanks.0.x\tshared\n", registry()), "unknown field banks.0.x");
        let text = registry().replace("hosts.0\tmac", "hosts.0\tsk-aaaaaaaaaaaaaaaaaaaa");
        let all = found(&text);
        assert!(all.iter().any(|p| p.contains("literal secret (openai key)")));
        assert!(all.iter().all(|p| !p.contains("sk-aaaa")));
    }

    #[test]
    fn overlay_adds_servers_and_settings_to_the_work_instance() {
        let body = "instance\tgold-band\n\
            field\tmcp_servers.jira.command\tnpx\nfield\tmcp_servers.jira.args.0\t-y\n\
            field\tmcp_servers.jira.env.JIRA_TOKEN\t$JIRA_TOKEN\n\
            field\tmcp_servers.wiki.url\thttps://wiki.example/mcp\n\
            field\tsettings.editor.theme\tdark\nfield\tagents.0\tcodex\n";
        assert_eq!(found(&overlay(body)), Vec::<String>::new());
    }

    #[test]
    fn overlay_may_not_touch_the_personal_instance_except_for_agents() {
        let body = "instance\tgold-band-personal\nfield\tmcp_servers.x.command\tnpx\nfield\tsettings.a\tb\n";
        has(&overlay(body), "overlay may add mcp_servers to the work instance gold-band only");
        has(&overlay(body), "overlay may add settings to the work instance gold-band only");
        let ok = "instance\tgold-band-personal\nfield\tagents.0\tvibe\n";
        assert!(found(&overlay(ok)).is_empty());
    }

    #[test]
    fn overlay_refuses_cross_instance_agents() {
        for bad in ["codex", "claude-work"] {
            let body = format!("instance\tgold-band-personal\nfield\tagents.0\t{bad}\n");
            has(&overlay(&body), &format!("overlay gold-band-personal: agent {bad} is not allowed"));
        }
        let body = "instance\tgold-band\nfield\tagents.0\tvibe\n";
        has(&overlay(body), "overlay gold-band: agent vibe is not allowed on the work instance");
    }

    #[test]
    fn overlay_shape_is_checked() {
        for (body, needle) in [
            ("field\tmcp_servers.jira\tx\n", "unknown field mcp_servers.jira"),
            ("field\tmcp_servers.Bad.command\tx\n", "bad mcp server name Bad"),
            ("field\tmcp_servers.a.command\tx\nfield\tmcp_servers.a.url\ty\n", "exactly one of command or url"),
            ("field\tmcp_servers.a.args.0\tx\n", "exactly one of command or url"),
            ("field\tmcp_servers.a.what\tx\n", "unknown field mcp_servers.a.what"),
            ("field\tmcp_servers.a.args.x\tx\n", "unknown field mcp_servers.a.args.x"),
            ("field\tmcp_servers.a.env.lower\tx\n", "unknown field mcp_servers.a.env.lower"),
            ("field\tsettings\tx\n", "bad settings path settings"),
            ("field\tsettings.a b\tx\n", "bad settings path settings.a b"),
            ("field\thosts.0\tlinux\n", "unknown field hosts.0"),
            ("field\trepository_rule.remotes\tx\n", "unknown field repository_rule.remotes"),
            ("field\tagents\tx\n", "unknown field agents"),
        ] {
            has(&overlay(&format!("instance\tgold-band\n{body}")), needle);
        }
        has(&overlay("instance\tgold-band\ninstance\tgold-band\n"), "overlay gold-band: declared more than once");
        has(&overlay("instance\tnope\n"), "overlay nope: unknown instance");
    }

    #[test]
    fn overlay_literal_secrets_are_refused() {
        let body = "instance\tgold-band\nfield\tmcp_servers.a.command\tx\nfield\tmcp_servers.a.env.API_TOKEN\thunter2\n\
            field\tsettings.endpoint\tBearer abcdefghijklmnopqrstuv\n";
        has(&overlay(body), "secret-named");
        has(&overlay(body), "literal secret (bearer token)");
    }

    #[test]
    fn parse_rejects_malformed_lines() {
        for bad in [
            "field\ta\tb\n",
            "source\tregistry\nfield\ta\tb\n",
            "instance\tgold-band\n",
            "source\tother\n",
            "source\tregistry\ninstance\ta\nfield\tonly-path\n",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse("# note\n\n").expect("empty").len(), 0);
    }

    fn file(name: &str, text: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("gbr-{}-{name}", std::process::id()));
        fs::write(&path, text).expect("writes");
        path
    }

    #[test]
    fn the_command_line_answers_with_the_documented_exit_codes() {
        let run_with = |args: &[&OsStr]| run(args.iter().map(|a| (*a).to_os_string()));
        let ok = file("ok", &registry());
        let bad = file("bad", "source\tregistry\n");
        let malformed = file("malformed", "nonsense\n");
        let validate = OsStr::new("validate");
        assert_eq!(run_with(&[validate, ok.as_os_str()]), 0);
        assert_eq!(run_with(&[validate, bad.as_os_str()]), 1);
        assert_eq!(run_with(&[validate, malformed.as_os_str()]), 2);
        assert_eq!(run_with(&[validate, OsStr::new("/nonexistent/gbr")]), 2);
        assert_eq!(run_with(&[]), 2);
        assert_eq!(run_with(&[OsStr::new("launch"), ok.as_os_str()]), 2);
        assert!(usage().contains(NAME));
    }
}
