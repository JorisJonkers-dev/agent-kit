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
//! * no OpenRouter: `provider`, `model` and every `adapter.args` and
//!   `adapter.env` value must not contain `openrouter` (case-insensitive);
//! * OpenCode models only from the `opencode-go/` provider: no `model` may
//!   start with `opencode/`, and an agent whose id starts with `opencode` or
//!   whose `adapter.command`'s file name is `opencode` must set a `model` that
//!   starts with `opencode-go/`;
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
//! Usage: `gold-band-registry validate <FILE>`. Exit 0 clean (printing
//! `ok (<n> agents)`), 1 rule problems on stderr, 2 bad usage (a usage line),
//! an unreadable file or malformed input.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs;

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
    for field in ["provider", "model"] {
        if let Some(body) = value(a, field) {
            if names(body, "openrouter") {
                p.push(format!("{id}: {field} must not name openrouter"));
            }
        }
    }
    for (path, body) in &a.fields {
        if matches!(kind(path), Kind::Args | Kind::Env) && names(body, "openrouter") {
            p.push(format!("{id}: {path} must not name openrouter"));
        }
    }
    if opencode_agent(a) && value(a, "model").is_none() {
        p.push(format!(
            "{id}: model must start with opencode-go/ (an opencode agent)"
        ));
    }
    if let Some(model) = value(a, "model") {
        if model.starts_with("opencode/") {
            p.push(format!("{id}: model must not start with opencode/"));
        }
        if opencode_agent(a) && !model.starts_with("opencode-go/") {
            p.push(format!(
                "{id}: model must start with opencode-go/ (an opencode agent)"
            ));
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
    let mut args = args.into_iter();
    let (cmd, file, extra) = (args.next(), args.next(), args.next());
    let file = match (cmd.as_deref(), file, extra) {
        (Some(cmd), Some(file), None) if cmd == OsStr::new("validate") => file,
        _ => {
            eprintln!("{}", usage());
            return 2;
        }
    };
    let shown = file.to_string_lossy();
    match fs::read_to_string(&file) {
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

/// The usage line every command-line problem prints.
fn usage() -> String {
    format!("usage: {NAME} validate <FILE>")
}

/// The agent's `path` field value, if it was ever set.
fn value<'a>(a: &'a Agent, path: &str) -> Option<&'a str> {
    a.fields
        .iter()
        .find(|(known, _)| known.as_str() == path)
        .map(|(_, body)| body.as_str())
}

/// True when this is an OpenCode-served agent: the id or the command's file
/// name says `opencode`.
fn opencode_agent(a: &Agent) -> bool {
    a.id.starts_with("opencode")
        || value(a, "adapter.command").is_some_and(|command| file_name(command) == "opencode")
}

/// The file name of a command: its first word's last `/` segment.
fn file_name(command: &str) -> &str {
    let program = command.split_whitespace().next().unwrap_or("");
    program.rsplit('/').next().unwrap_or("")
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
        "projectPrimaryAgentDir" | "outputDir" | "provider" | "model" | "reach" => Kind::Known,
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
    fn openrouter_is_refused_in_providers_and_values() {
        let text = "\
agent\tmixer
field\treach\tread
field\tprovider\topenrouter
field\tmodel\topenrouter/anthropic/claude-3
field\tadapter.args.0\t--provider openrouter
field\tadapter.env.OPENROUTER_API_URL\thttps://openrouter.ai/api/v1
";
        expect(
            text,
            &[
                "mixer: provider must not name openrouter",
                "mixer: model must not name openrouter",
                "mixer: adapter.args.0 must not name openrouter",
                "mixer: adapter.env.OPENROUTER_API_URL must not name openrouter",
            ],
        );
    }

    #[test]
    fn opencode_models_must_come_from_opencode_go() {
        let text = "agent\tsidekick\nfield\treach\tread\nfield\tmodel\topencode/zeta\n";
        expect(text, &["sidekick: model must not start with opencode/"]);
        let text = "\
agent\tsidekick
field\treach\tread
field\tadapter.command\t/usr/local/bin/opencode
field\tmodel\tzeta-3
";
        expect(
            text,
            &["sidekick: model must start with opencode-go/ (an opencode agent)"],
        );
        let text = "agent\topencode\nfield\treach\tread\nfield\tmodel\tzeta-3\n";
        expect(
            text,
            &["opencode: model must start with opencode-go/ (an opencode agent)"],
        );
        let text = "agent\topencode\nfield\treach\tread\nfield\tmodel\topencode/zeta\n";
        expect(
            text,
            &[
                "opencode: model must not start with opencode/",
                "opencode: model must start with opencode-go/ (an opencode agent)",
            ],
        );
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
        assert_eq!(file_name("opencode serve"), "opencode");
        assert_eq!(file_name("/usr/local/bin/opencode"), "opencode");
        assert_eq!(file_name(""), "");
        assert_eq!(file_name("/usr/bin/my-opencode"), "my-opencode");
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
    fn regression_an_opencode_agent_with_no_model_is_refused() {
        expect(
            "agent\topencode\nfield\treach\tread\n",
            &["opencode: model must start with opencode-go/ (an opencode agent)"],
        );
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
}
