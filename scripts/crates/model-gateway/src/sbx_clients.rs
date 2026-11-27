//! What the sbx backend reads of the Claude Code, Codex and Hermes routes. Claude Code and Codex reach their own
//! provider, so the hosts they need are fixed and no route file is read; Hermes on a LiteLLM route names its proxy in
//! the `config.yaml` of its run home, whose loopback base URL is rewritten to the name the sandbox sees.

use crate::error::GatewayError;
use crate::run::RunClient;
use crate::sbx_launch::{host_rules, HOST_FROM_SANDBOX};

/// Claude Code's API. It is never routed through LiteLLM; telemetry and error-reporting hosts stay blocked.
const CLAUDE_HOSTS: [&str; 1] = ["api.anthropic.com:443"];

/// Codex's ChatGPT backend, the OpenAI API and the host a login's tokens refresh at.
const CODEX_HOSTS: [&str; 3] = ["chatgpt.com:443", "api.openai.com:443", "auth.openai.com:443"];

/// The hosts of a client that has no route file, as `host:port` rules; `None` for a client whose route names its own.
pub fn fixed_hosts(client: RunClient) -> Option<Vec<String>> {
    let hosts: &[&str] = match client {
        RunClient::Claude => &CLAUDE_HOSTS,
        RunClient::Codex => &CODEX_HOSTS,
        _ => return None,
    };
    Some(hosts.iter().map(|host| (*host).to_string()).collect())
}

/// The `base_url` of the first provider a Hermes `config.yaml` names, unquoted.
fn base_of(config: &str) -> Option<String> {
    let value = config.lines().find_map(|line| line.trim().strip_prefix("base_url:"))?.trim().trim_matches(['"', '\'']);
    (!value.is_empty()).then(|| value.to_string())
}

/// The rules a Hermes launch needs: its provider's base URL. A config that names none (a provider of the owner's own,
/// whose key sits in the owner's home) is refused, since sbx allows only the proxy the route names.
pub fn hermes_allowed(config: &str) -> Result<Vec<String>, GatewayError> {
    let base = base_of(config).ok_or_else(|| GatewayError::new("sandbox-unsupported", "hermes: the run home's config.yaml names no base_url; the sbx backend runs only a LiteLLM route"))?;
    host_rules(&base)
}

/// The config with a loopback `base_url` rewritten to the host name the sandbox sees; `None` when it needs none.
pub fn hermes_rewritten(config: &str) -> Option<String> {
    let base = base_of(config)?;
    let host = base.split_once("://").and_then(|(_, rest)| rest.split(['/', ':']).next()).unwrap_or_default();
    (host == "localhost" || host == "127.0.0.1").then(|| config.replacen(&base, &base.replacen(host, HOST_FROM_SANDBOX, 1), 1))
}

/// Hermes' install state (`installs`, `tools`) is kept in the image's own Hermes home, and a launch under another
/// `HERMES_HOME` finds none ("no dependency environment is committed"). Its run home borrows both by a link made inside
/// the sandbox, where the image's path exists, before Hermes starts.
const HERMES_LINKS: &str = r#"for d in installs tools; do [ -e "$HERMES_HOME/$d" ] || ln -s "/home/agent/.hermes/$d" "$HERMES_HOME/$d"; done; exec hermes "$@""#;

/// The program and arguments `sbx exec` runs for a client: the client itself, or for Hermes the shell that links its
/// install state into the run home first and then execs it.
pub fn words_of(program: &str, args: &[String]) -> Vec<String> {
    if program == "hermes" {
        return ["sh", "-c", HERMES_LINKS, "hermes"].iter().map(|word| (*word).to_string()).chain(args.iter().cloned()).collect();
    }
    std::iter::once(program.to_string()).chain(args.iter().cloned()).collect()
}

/// The word after a flag in a client's arguments.
fn after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter().position(|word| word == flag).and_then(|at| args.get(at + 1)).map(String::as_str)
}

/// A Hermes run home's config with the `model:` block the image's newer Hermes reads its provider from; the confined
/// recipe names the provider and model on the command line alone. A config of any other client, or one that has the
/// block already, or a launch that names no provider and model, comes back as it is.
pub fn with_model(client: RunClient, config: &str, args: &[String]) -> String {
    let named = after(args, "--provider").zip(after(args, "-m"));
    match named {
        Some((provider, model)) if client == RunClient::Hermes && !config.lines().any(|line| line.starts_with("model:")) => format!("{}{}model:\n  provider: {provider}\n  default: {model}\n", config, if config.ends_with('\n') { "" } else { "\n" }),
        _ => config.to_string(),
    }
}
