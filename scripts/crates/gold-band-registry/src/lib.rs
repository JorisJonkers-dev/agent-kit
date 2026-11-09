//! The Gold Band agent registry's validation half (#332): Python flattens each
//! declared agent into a line protocol (one record per line, fields separated
//! by a single TAB) and passes the file here, and this crate refuses every
//! declaration that breaks an owner's hard rule:
//!
//! 1. Ids are lowercase ASCII letters and digits in dash-separated groups, and
//!    an id is declared once.
//! 2. No Codex anywhere: not in the id, the adapter command, an adapter
//!    argument, the provider or the model.
//! 3. No OpenRouter anywhere: not in the provider, the model, an adapter
//!    argument or an adapter environment value.
//! 4. OpenCode models come only from the `opencode-go/` provider, never from
//!    `opencode/...`.
//! 5. No literal secret in any field, argument or environment value: a
//!    secret-named path holds an environment reference or a keychain-proxy
//!    name, any other value matching one of [`SECRET_SHAPES`] is refused, and
//!    a value whose `/`-separated segments are all short is a path, not a
//!    token.
//! 6. The reach is one of [`REACHES`], and `research`, `write-scoped` and
//!    `full` carry the fields they need.
//!
//! Usage: `gold-band-registry validate <FILE>`. Exit 0 clean, 1 refused or
//! malformed input, 2 bad usage.

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fs;

pub const NAME: &str = "gold-band-registry";

/// Every reach a Gold Band agent may declare.
pub const REACHES: [&str; 7] = [
    "read",
    "read-web",
    "research",
    "write-scoped",
    "write",
    "full",
    "run-one-command",
];

/// The words that make a field path secret-named.
pub const SECRET_MARKERS: [&str; 5] = ["authorization", "key", "password", "secret", "token"];

/// The literal secret shapes this registry refuses: a name and its matcher.
pub const SECRET_SHAPES: [(&str, fn(&str) -> bool); 7] = [
    ("openai-style key", is_openai_key),
    ("github token", is_github_token),
    ("slack token", is_slack_token),
    ("aws access key id", is_aws_key),
    ("bearer token", is_bearer_token),
    ("jwt", is_jwt),
    ("opaque token", is_long_token),
];

/// The fields whose value must be `true` or `false`.
const BOOLEAN_FIELDS: [&str; 4] = [
    "externalSessionSyncEnabled",
    "externalSessionSyncSupported",
    "throwawayOnly",
    "useLocalClaude",
];

/// One declaration: its id, the line its `agent` record sits on, and its
/// fields as (path, value) pairs in declaration order.
#[derive(Debug)]
pub struct Agent {
    pub id: String,
    pub line: usize,
    pub fields: Vec<(String, String)>,
}

impl Agent {
    /// The first value declared for `path`, if any.
    fn value(&self, path: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(declared, _)| declared == path)
            .map(|(_, value)| value.as_str())
    }
}

/// Parses one declarations text, or names the first malformed line.
pub fn parse(text: &str) -> Result<Vec<Agent>, String> {
    let mut agents = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let number = n + 1;
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split('\t').collect();
        match parts.as_slice() {
            ["agent", id] => agents.push(Agent {
                id: id.to_string(),
                line: number,
                fields: Vec::new(),
            }),
            ["field", path, value] => match agents.last_mut() {
                Some(agent) => agent.fields.push((path.to_string(), value.to_string())),
                None => return Err(format!("line {number}: field before any agent declaration")),
            },
            _ => return Err(format!("line {number}: malformed line")),
        }
    }
    Ok(agents)
}

/// Every rule violation in `agents`, in declaration order: one
/// `gold-band-registry: <id>: <what>` line each.
pub fn problems(agents: &[Agent]) -> Vec<String> {
    let mut out = Vec::new();
    let mut declared: BTreeSet<&str> = BTreeSet::new();
    let mut reported: BTreeSet<&str> = BTreeSet::new();
    for agent in agents {
        let id = agent.id.as_str();
        if !is_well_formed_id(id) {
            out.push(problem(id, "malformed agent id"));
        }
        if !declared.insert(id) && reported.insert(id) {
            out.push(problem(id, "duplicate agent id"));
        }
        if contains_ci(id, "codex") {
            out.push(problem(id, "id contains \"codex\""));
        }
        let opencode_command = agent
            .value("adapter.command")
            .is_some_and(|command| file_name(command) == "opencode");
        let opencode = id.starts_with("opencode") || opencode_command;
        for (path, value) in &agent.fields {
            out.extend(field_problems(id, opencode, path, value));
        }
        out.extend(reach_problems(agent, id));
    }
    out
}

/// One problem line: `gold-band-registry: <id>: <what>`.
fn problem(id: &str, what: &str) -> String {
    format!("{NAME}: {id}: {what}")
}

/// The refusals one declared field earns, in rule order.
fn field_problems(id: &str, opencode: bool, path: &str, value: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !is_known_field(path) {
        out.push(problem(id, &format!("unknown agent field {path}")));
        return out;
    }
    if let Some(what) = value_problem(path, value) {
        out.push(problem(id, &what));
    }
    let arg = path.starts_with("adapter.args.");
    let env = path.starts_with("adapter.env.");
    let codex_scope = arg
        || path == "adapter.command"
        || path == "provider"
        || path == "model";
    if codex_scope && contains_ci(value, "codex") {
        out.push(problem(id, &format!("{path} contains \"codex\"")));
    }
    let router_scope = arg || env || path == "provider" || path == "model";
    if router_scope && contains_ci(value, "openrouter") {
        out.push(problem(id, &format!("{path} contains \"openrouter\"")));
    }
    if path == "model" {
        if value.starts_with("opencode/") {
            out.push(problem(id, "model may not use the \"opencode/\" provider"));
        } else if opencode && !value.starts_with("opencode-go/") {
            out.push(problem(id, "model must come from \"opencode-go/\""));
        }
    }
    if let Some(what) = secret_problem(path, value) {
        out.push(problem(id, &what));
    }
    out
}

/// The refusals a declared reach earns from its follow-up fields.
fn reach_problems(agent: &Agent, id: &str) -> Vec<String> {
    let mut out = Vec::new();
    match agent.value("reach") {
        Some(reach @ ("research" | "write-scoped")) if agent.value("outputDir").is_none() => {
            out.push(problem(id, &format!("reach \"{reach}\" needs an outputDir field")));
        }
        Some("full") if agent.value("throwawayOnly") != Some("true") => {
            out.push(problem(id, "reach \"full\" needs throwawayOnly = true"));
        }
        _ => {}
    }
    out
}

/// The value-grammar refusal a known field earns, if any.
fn value_problem(path: &str, value: &str) -> Option<String> {
    if BOOLEAN_FIELDS.contains(&path) && value != "true" && value != "false" {
        return Some(format!("{path} value must be \"true\" or \"false\""));
    }
    if path == "systemPromptDelivery" && value != "none" && value != "meta-append" {
        return Some(format!("{path} value must be \"none\" or \"meta-append\""));
    }
    if path == "reach" && !REACHES.contains(&value) {
        return Some(format!("reach \"{value}\" is not one of {REACHES:?}"));
    }
    None
}

/// The secret refusal a known field earns, if any — it never names the value.
fn secret_problem(path: &str, value: &str) -> Option<String> {
    if is_secret_named(path) {
        if is_env_ref(value) || is_keychain_proxy(value) {
            return None;
        }
        return Some(format!(
            "{path} is secret-named; it needs an environment or keychain-proxy reference"
        ));
    }
    if is_env_ref(value) {
        return None;
    }
    let shape = secret_shape(value)?;
    Some(format!("{path} looks like a literal secret ({shape})"))
}

/// The name of the first secret shape `value` matches, if any.
pub fn secret_shape(value: &str) -> Option<&'static str> {
    let mut found = None;
    for (name, matches) in SECRET_SHAPES {
        if matches(value) {
            found = Some(name);
            break;
        }
    }
    found
}

/// An environment-variable name: `[A-Z_][A-Z0-9_]*`.
pub fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_uppercase())
        && chars.all(|c| c == '_' || c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// An agent id: lowercase ASCII letters and digits in dash-separated groups.
pub fn is_well_formed_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|group| {
            !group.is_empty()
                && group.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

/// `path` is `prefix` plus a tail that passes `ok`.
fn prefixed(path: &str, prefix: &str, ok: fn(&str) -> bool) -> bool {
    if let Some(tail) = path.strip_prefix(prefix) {
        return ok(tail);
    }
    false
}

/// A decimal field index, as in `adapter.args.<N>`.
fn is_index(tail: &str) -> bool {
    !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit())
}

/// A known field path of the line protocol.
pub fn is_known_field(path: &str) -> bool {
    const FIELDS: [&str; 14] = [
        "adapter.command",
        "adapter.displayName",
        "externalSessionSyncEnabled",
        "externalSessionSyncSupported",
        "icon",
        "model",
        "outputDir",
        "primaryAgentDir",
        "projectPrimaryAgentDir",
        "provider",
        "reach",
        "systemPromptDelivery",
        "throwawayOnly",
        "useLocalClaude",
    ];
    FIELDS.contains(&path)
        || prefixed(path, "adapter.args.", is_index)
        || prefixed(path, "compatibleAgentDirs.", is_index)
        || prefixed(path, "adapter.env.", is_env_name)
}

/// A field path whose last segment names a secret.
pub fn is_secret_named(path: &str) -> bool {
    let last = path.rsplit('.').next().unwrap_or("");
    SECRET_MARKERS.contains(&last.to_ascii_lowercase().as_str())
}

/// An environment-variable reference: `$NAME` or `${NAME}`.
pub fn is_env_ref(value: &str) -> bool {
    let rest = value.strip_prefix('$').unwrap_or("");
    let braced = rest
        .strip_prefix('{')
        .and_then(|inner| inner.strip_suffix('}'));
    match braced {
        Some(name) => is_env_name(name),
        None => is_env_name(rest),
    }
}

/// A keychain-proxy field name: letter, then name characters up to 64 total,
/// and never a literal secret shape.
pub fn is_keychain_proxy(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        && value.chars().count() <= 64
        && secret_shape(value).is_none()
}

/// Case-insensitive containment, for the forbidden-name rules.
fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_ascii_lowercase().contains(needle)
}

/// The file name at the end of a command path.
fn file_name(command: &str) -> &str {
    command.rsplit('/').next().unwrap_or("")
}

/// The length of the leading run of chars that pass `class_ok`.
fn class_run(s: &str, class_ok: impl Fn(char) -> bool) -> usize {
    s.chars().take_while(|&c| class_ok(c)).count()
}

/// An OpenAI-style key: `sk-` then 16+ chars of [A-Za-z0-9_-].
fn is_openai_key(value: &str) -> bool {
    match value.strip_prefix("sk-") {
        Some(rest) => class_run(rest, |c| c.is_ascii_alphanumeric() || c == '_' || c == '-') >= 16,
        None => false,
    }
}

/// A GitHub token: `ghp_`-style then 20+ alphanumerics/underscores.
fn is_github_token(value: &str) -> bool {
    match ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"]
        .into_iter()
        .find_map(|prefix| value.strip_prefix(prefix))
    {
        Some(rest) => class_run(rest, |c| c.is_ascii_alphanumeric() || c == '_') >= 20,
        None => false,
    }
}

/// A Slack token: `xoxb-` or `xoxp-`.
fn is_slack_token(value: &str) -> bool {
    value.starts_with("xoxb-") || value.starts_with("xoxp-")
}

/// An AWS access key id: `AKIA` then 16+ uppercase letters/digits.
fn is_aws_key(value: &str) -> bool {
    match value.strip_prefix("AKIA") {
        Some(rest) => class_run(rest, |c| c.is_ascii_uppercase() || c.is_ascii_digit()) >= 16,
        None => false,
    }
}

/// A bearer token: `Bearer ` (case-insensitive) then 16+ non-space chars.
fn is_bearer_token(value: &str) -> bool {
    let head_ok = value
        .get(0..7)
        .is_some_and(|head| head.eq_ignore_ascii_case("bearer "));
    head_ok && class_run(&value[7..], |c| !c.is_whitespace()) >= 16
}

/// A JWT: three dot-separated segments, the first starting with `eyJ`.
fn is_jwt(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3 && parts[0].starts_with("eyJ")
}

/// One char of the opaque-token alphabet.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-')
}

/// One split run is an opaque token: at least 32 token chars, with a digit and
/// a letter among them.
fn is_opaque_run(run: &str) -> bool {
    run.chars().count() >= 32
        && run.chars().any(|c| c.is_ascii_digit())
        && run.chars().any(|c| c.is_ascii_alphabetic())
}

/// A 32+ opaque run of token characters containing a digit and a letter. A
/// value with a `/` in it is a path, not a token, while every `/`-separated
/// segment of it stays shorter than 32 chars.
fn is_long_token(value: &str) -> bool {
    if value.contains('/') && value.split('/').all(|seg| seg.chars().count() < 32) {
        return false;
    }
    value.split(|c| !is_token_char(c)).any(is_opaque_run)
}

/// The usage a bad command line prints.
fn usage() -> String {
    format!("usage: {NAME} validate <FILE>")
}

/// Prints a usage failure and returns its exit code.
fn usage_error(reason: &str) -> i32 {
    eprintln!("{NAME}: {reason}");
    eprintln!("{}", usage());
    2
}

/// Processes one declarations file and prints its verdict.
fn validate(file: &OsStr) -> i32 {
    match fs::read_to_string(file) {
        Err(err) => usage_error(&format!("cannot read {}: {err}", file.to_string_lossy())),
        Ok(text) => match parse(&text) {
            Err(message) => {
                eprintln!("{NAME}: {message}");
                1
            }
            Ok(agents) => report(&agents),
        },
    }
}

/// Prints one agent set's problems, and the exit code they earn.
fn report(agents: &[Agent]) -> i32 {
    let found = problems(agents);
    for line in &found {
        eprintln!("{line}");
    }
    if found.is_empty() {
        println!("{NAME}: ok ({} agents)", agents.len());
        return 0;
    }
    1
}

/// Runs one command line and returns its exit code: 0 clean, 1 refused or
/// malformed input, 2 bad usage. Success prints to stdout, the rest stderr.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let (Some(cmd), Some(file), None) = (args.next(), args.next(), args.next()) else {
        eprintln!("{}", usage());
        return 2;
    };
    if cmd.as_os_str() != OsStr::new("validate") {
        eprintln!("{}", usage());
        return 2;
    }
    validate(&file)
}

// The unit tests below execute every line of this file, which is the
// workspace's 100 percent line-coverage standard (docs/RUST.md); the
// executable line in main.rs, the exit wrapper, is the one line they cannot.
#[cfg(test)]
mod tests {
    use super::*;

    /// Every refusal's `<id>: <what>`, the crate prefix stripped off.
    fn whats(agents: &[Agent]) -> Vec<String> {
        let mut out = Vec::new();
        for line in &problems(agents) {
            let line = line.strip_prefix("gold-band-registry: ").unwrap_or("");
            out.push(line.to_string());
        }
        out
    }

    /// Every refusal in one declarations text.
    fn refused(text: &str) -> Vec<String> {
        whats(&parse(text).unwrap())
    }

    /// Runs the command line over one file written to the temp dir.
    fn run_on(args: &[&str]) -> i32 {
        run(args.iter().map(|arg| OsString::from(*arg)))
    }

    /// Runs `validate` over one file written to the temp dir.
    fn run_file(tag: &str, text: &str) -> i32 {
        let path = std::env::temp_dir().join(format!(
            "gold-band-registry-{tag}-{}",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        let shown = path.to_string_lossy();
        run_on(&["validate", &shown])
    }

    #[test]
    fn parse_walks_the_line_protocol() {
        let text = concat!(
            "# the agents\n",
            "agent\tclaude\n",
            "\n",
            "field\tadapter.command\tclaude\n",
            "# tail comment\n",
            "agent\tcodex\n",
            "field\tmodel\tgpt\n",
        );
        let agents = parse(text).unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].id, "claude");
        assert_eq!(agents[0].line, 2);
        let one = ("adapter.command".to_string(), "claude".to_string());
        assert_eq!(agents[0].fields, vec![one]);
        assert_eq!(agents[1].line, 6);
    }

    #[test]
    fn parse_names_malformed_lines() {
        assert_eq!(
            parse("field\ticon\tx\n").unwrap_err(),
            "line 1: field before any agent declaration"
        );
        assert_eq!(parse("plain\n").unwrap_err(), "line 1: malformed line");
        assert_eq!(parse("agent\tclaude\nfield\ticon\n").unwrap_err(), "line 2: malformed line");
        assert_eq!(parse("agent\tclaude\nagent\ta\tb\n").unwrap_err(), "line 2: malformed line");
    }

    #[test]
    fn usage_errors_exit_two() {
        assert_eq!(run_on(&[]), 2);
        assert_eq!(run_on(&["check"]), 2);
        assert_eq!(run_on(&["validate"]), 2);
        assert_eq!(run_on(&["validate", "stable", "extra"]), 2);
    }

    #[test]
    fn an_unreadable_file_exits_two() {
        let missing = std::env::temp_dir().join(format!(
            "gold-band-registry-absent-{}",
            std::process::id()
        ));
        let shown = missing.to_string_lossy();
        assert_eq!(run_on(&["validate", &shown]), 2);
    }

    #[test]
    fn run_accepts_a_clean_file_and_refuses_a_broken_one() {
        assert_eq!(run_file("clean", "# nothing yet\n\n"), 0);
        let broken = "agent\tclaude\nfield\treach\tresearch\n";
        assert_eq!(run_file("broken", broken), 1);
        assert_eq!(run_file("malformed", "orphan\n"), 1);
    }

    #[test]
    fn accepts_directory_paths_and_environment_references() {
        let text = concat!(
            "agent\tclaude\n",
            "field\tprimaryAgentDir\t/Users/me/.claude\n",
            "field\tcompatibleAgentDirs.0\t/Users/me/projects/agent-kit\n",
            "field\tadapter.env.ANTHROPIC_API_KEY\t$ANTHROPIC_API_KEY\n",
            "field\tprovider\tanthropic\n",
            "field\tmodel\tclaude-sonnet-4-5\n",
        );
        let agents = parse(text).unwrap();
        assert_eq!(agents.len(), 1);
        assert!(whats(&agents).is_empty());
        assert_eq!(run_file("accepts", text), 0);
    }

    #[test]
    fn accepts_environment_references_and_keychain_proxy_fields() {
        let text = concat!(
            "agent\tclaude\n",
            "field\tadapter.env.GITHUB_TOKEN\t${GITHUB_TOKEN}\n",
            "field\tadapter.env.SLACK_BEARER_TOKEN\tslack-auth-keychain\n",
            "field\tadapter.args.0\t$LONG_ENVELOPE_0123456789ABCDEFGHIJ\n",
            "agent\tbuilder\n",
            "field\tadapter.command\t/usr/bin/env\n",
        );
        assert!(whats(&parse(text).unwrap()).is_empty());
    }

    #[test]
    fn refuses_duplicate_and_malformed_ids() {
        let text = concat!(
            "agent\tclaude\n",
            "agent\tclaude\n",
            "agent\tclaude\n",
            "agent\tClaude\n",
        );
        assert_eq!(refused(text), ["claude: duplicate agent id", "Claude: malformed agent id"]);
    }

    #[test]
    fn refuses_codex_anywhere() {
        let text = concat!(
            "agent\tcodex-mini\n",
            "agent\tgb1\n",
            "field\tadapter.command\t/usr/local/bin/codex\n",
            "field\tprovider\tCodex\n",
            "field\tmodel\tgpt-5-codex\n",
            "field\tadapter.args.0\t--model codex-mini\n",
        );
        assert_eq!(
            refused(text),
            [
                "codex-mini: id contains \"codex\"",
                "gb1: adapter.command contains \"codex\"",
                "gb1: provider contains \"codex\"",
                "gb1: model contains \"codex\"",
                "gb1: adapter.args.0 contains \"codex\"",
            ]
        );
    }

    #[test]
    fn refuses_openrouter_anywhere() {
        let text = concat!(
            "agent\tgb3\n",
            "field\tprovider\topenrouter/anthropic\n",
            "field\tmodel\topenrouter/auto\n",
            "field\tadapter.args.0\t--via openrouter\n",
            "field\tadapter.env.REMOTE_TOKEN\t${OPENROUTER_TOKEN}\n",
        );
        assert_eq!(
            refused(text),
            [
                "gb3: provider contains \"openrouter\"",
                "gb3: model contains \"openrouter\"",
                "gb3: adapter.args.0 contains \"openrouter\"",
                "gb3: adapter.env.REMOTE_TOKEN contains \"openrouter\"",
            ]
        );
    }

    #[test]
    fn refuses_opencode_models_from_the_wrong_provider() {
        let text = concat!(
            "agent\tgb4\n",
            "field\tmodel\topencode/grok-code\n",
            "agent\topencode\n",
            "field\tmodel\tgrok-code\n",
            "agent\tgb5\n",
            "field\tadapter.command\t/opt/bin/opencode\n",
            "field\tmodel\tgrok-code\n",
            "agent\topencode-go\n",
            "field\tmodel\topencode-go/grok-code\n",
        );
        assert_eq!(
            refused(text),
            [
                "gb4: model may not use the \"opencode/\" provider",
                "opencode: model must come from \"opencode-go/\"",
                "gb5: model must come from \"opencode-go/\"",
            ]
        );
    }

    #[test]
    fn refuses_unknown_agent_fields() {
        let text = concat!(
            "agent\tgb6\n",
            "field\tadapter.version\t1\n",
            "field\tadapter.args.round\tx\n",
            "field\tadapter.env.lower\ty\n",
            "field\tcompatibleAgentDirs.two\tz\n",
        );
        assert_eq!(
            refused(text),
            [
                "gb6: unknown agent field adapter.version",
                "gb6: unknown agent field adapter.args.round",
                "gb6: unknown agent field adapter.env.lower",
                "gb6: unknown agent field compatibleAgentDirs.two",
            ]
        );
    }

    #[test]
    fn refuses_values_outside_their_grammars() {
        let text = concat!(
            "agent\tgb7\n",
            "field\tuseLocalClaude\tyes\n",
            "field\tsystemPromptDelivery\tinline\n",
            "field\tthrowawayOnly\tperhaps\n",
            "agent\tgb8\n",
            "field\texternalSessionSyncSupported\tyes\n",
            "agent\tgb9\n",
            "field\texternalSessionSyncEnabled\tmaybe\n",
            "field\tthrowawayOnly\tfalse\n",
            "field\treach\tfull\n",
            "agent\tgb10\n",
            "field\tuseLocalClaude\ttrue\n",
        );
        assert_eq!(
            refused(text),
            [
                "gb7: useLocalClaude value must be \"true\" or \"false\"",
                "gb7: systemPromptDelivery value must be \"none\" or \"meta-append\"",
                "gb7: throwawayOnly value must be \"true\" or \"false\"",
                "gb8: externalSessionSyncSupported value must be \"true\" or \"false\"",
                "gb9: externalSessionSyncEnabled value must be \"true\" or \"false\"",
                "gb9: reach \"full\" needs throwawayOnly = true",
            ]
        );
    }

    #[test]
    fn refuses_an_unknown_reach() {
        let text = "agent\tgb13\nfield\treach\tread-all\n";
        assert_eq!(refused(text), [format!("gb13: reach \"read-all\" is not one of {REACHES:?}")]);
    }

    #[test]
    fn refuses_reaches_that_break_their_own_rules() {
        let text = concat!(
            "agent\tgb14\n",
            "field\treach\tresearch\n",
            "agent\tgb15\n",
            "field\treach\twrite-scoped\n",
            "agent\tgb16\n",
            "field\treach\tresearch\n",
            "field\toutputDir\t/tmp/gold-band\n",
            "agent\tgb17\n",
            "field\treach\tfull\n",
            "field\tthrowawayOnly\ttrue\n",
            "agent\tgb18\n",
            "field\treach\trun-one-command\n",
            "agent\tgb19\n",
            "field\treach\tfull\n",
        );
        assert_eq!(
            refused(text),
            [
                "gb14: reach \"research\" needs an outputDir field",
                "gb15: reach \"write-scoped\" needs an outputDir field",
                "gb19: reach \"full\" needs throwawayOnly = true",
            ]
        );
    }

    #[test]
    fn refuses_literals_in_secret_named_fields() {
        let text = concat!(
            "agent\tgb11\n",
            "field\tadapter.env.OPENAI_API_KEY\tplain-value\n",
            "agent\tgb12\n",
            "field\tadapter.env.SLACK_BEARER_TOKEN\txoxb-123-456\n",
        );
        assert_eq!(
            refused(text),
            [
                "gb11: adapter.env.OPENAI_API_KEY is secret-named; it needs an environment or keychain-proxy reference",
                "gb12: adapter.env.SLACK_BEARER_TOKEN is secret-named; it needs an environment or keychain-proxy reference",
            ]
        );
    }

    #[test]
    fn refuses_every_secret_shape() {
        const LITERAL: [&str; 7] = [
            "sk-ABCDEFGHIJKLMNOPQRSTUVWXYZab",
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZab",
            "xoxb-1234567890-abcdef",
            "AKIAIOSFODNN7EXAMPLE",
            "Bearer abcdefghijklmnop",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.Ktnk-abc",
            "abcdefghij0123456789abcdefghij012345",
        ];
        for (n, literal) in LITERAL.iter().copied().enumerate() {
            let text = format!("agent\tgb\nfield\ticon\t{literal}\n");
            assert_eq!(
                refused(&text),
                [format!("gb: icon looks like a literal secret ({})", SECRET_SHAPES[n].0)]
            );
        }
    }

    #[test]
    fn refuses_shapes_inside_args_env_and_plain_fields() {
        let text = concat!(
            "agent\tgb20\n",
            "field\tadapter.args.0\tsk-ABCDEFGHIJKLMNOPQRSTUVWXYZab\n",
            "agent\tgb21\n",
            "field\tadapter.displayName\tghp_ABCDEFGHIJKLMNOPQRSTUVWXYZab\n",
            "agent\tgb22\n",
            "field\tadapter.env.MODE\tBearer abcdefghijklmnop\n",
        );
        assert_eq!(
            refused(text),
            [
                "gb20: adapter.args.0 looks like a literal secret (openai-style key)",
                "gb21: adapter.displayName looks like a literal secret (github token)",
                "gb22: adapter.env.MODE looks like a literal secret (bearer token)",
            ]
        );
    }

    #[test]
    fn refuses_a_long_opaque_path_segment() {
        let text = "agent\tgb23\nfield\ticon\t/mnt/shared/abcdefghij0123456789abcdefghij01\n";
        assert_eq!(refused(text), ["gb23: icon looks like a literal secret (opaque token)"]);
    }

    #[test]
    fn values_that_match_no_secret_shape_pass() {
        const PLAIN: [&str; 11] = [
            "",
            "/Users/me/.claude",
            "plain-key-not-a-secret",
            "sk-tooshort",
            "AKIAIOSFODNN7EXAMPL",
            "Bearer short",
            "no-jwt.here.yet",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefg",
            "0123456789abcdefghijklmnopqrstu",
            "$OPENAI_API_KEY",
            "claude-auth-keychain",
        ];
        for value in PLAIN {
            assert_eq!(secret_shape(value), None);
        }
    }

    #[test]
    fn long_tokens_need_32_chars_and_honor_path_segments() {
        assert!(is_long_token("0123456789abcdefghij0123456789ab"));
        assert!(!is_long_token("0123456789abcdefghij0123456789a"));
        assert!(is_long_token("/mnt/shared/0123456789abcdefghij0123456789ab"));
        assert!(!is_long_token("/Users/me/.claude/projects/tooling"));
    }

    #[test]
    fn ids_and_environment_references_are_judged() {
        for id in ["claude", "opencode-go", "a", "0code"] {
            assert!(is_well_formed_id(id));
        }
        for id in ["", "Claude", "-claude", "claude-", "claude--go", "a_b"] {
            assert!(!is_well_formed_id(id));
        }
        assert!(is_env_name("OPENAI_API_KEY"));
        assert!(is_env_name("_X2"));
        assert!(!is_env_name("lower"));
        assert!(!is_env_name("A-B"));
        assert!(is_env_ref("$HOME"));
        assert!(is_env_ref("${HOME}"));
        assert!(!is_env_ref("HOME"));
        assert!(!is_env_ref("$lower"));
        assert!(!is_env_ref("${}"));
        assert!(!is_env_ref("${HOME"));
    }

    #[test]
    fn keychain_proxy_names_have_a_shape_and_a_length() {
        assert!(is_keychain_proxy("claude-auth-keychain"));
        assert!(is_keychain_proxy("openai.keychain-2"));
        assert!(is_keychain_proxy("A"));
        let fits = format!("a{}", "-".repeat(63));
        let spills = format!("a{}", "-".repeat(64));
        assert!(is_keychain_proxy(&fits));
        assert!(!is_keychain_proxy(&spills));
        assert!(!is_keychain_proxy("!"));
        assert!(!is_keychain_proxy("has space"));
        assert!(!is_keychain_proxy("sk-ABCDEFGHIJKLMNOPQRSTUVWXYZab"));
    }

    #[test]
    fn problem_lines_carry_the_crate_prefix_and_the_id() {
        let text = "agent\tclaude\nfield\treach\tread-all\n";
        let found = problems(&parse(text).unwrap());
        let wanted = format!(
            "gold-band-registry: claude: reach \"read-all\" is not one of {REACHES:?}"
        );
        assert_eq!(found, vec![wanted]);
    }
}
