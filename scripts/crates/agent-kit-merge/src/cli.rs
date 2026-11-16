//! The `agent-kit-merge` command line: one subcommand per script it replaces.

use crate::proxy::{Options, Validate};
use crate::converge::{McpArgs, PluginArgs, Runner, SystemRunner};
use crate::{block, converge, mcp_json, mcp_toml, mcp_yaml, proxy, Env, Failure, Io};
use std::path::PathBuf;

pub const USAGE: &str = "usage: agent-kit-merge COMMAND ARGS...

commands:
  antigravity-mcp FRAGMENT [--config PATH] [--remove NAME ...]
  opencode-mcp    FRAGMENT [--config PATH] [--remove NAME ...]
  hermes-mcp      FRAGMENT [--config PATH] [--remove NAME ...]
  vibe-mcp        FRAGMENT [--key mcp_servers|hooks] [--config PATH] [--remove NAME ...]
  codex-add-url   CONFIG NAME URL
  instruction-block FILE NAME [--remove]            (body on stdin)
  package-proxy   FILE NAME [--remove] [--validate toml|ini] [--refuse REGEX] [--mode MODE]
                                                    (body on stdin)
  converge mcp    --keep PATH --claude-home DIR [--claude DIR ...] [--codex PATH] [--vibe PATH]
                  [--vibe-private PATH] [--opencode PATH] [--declared SURFACE=NAME,... ...]
                  [--retired NAME,...] [--private NAME,...] [--check]
  converge plugins --keep PATH [--declared-markets NAME,...] [--declared-plugins ID,...]
                  [--retired-markets NAME,...] [--retired-plugins ID,...] [--check]";

struct Parsed {
    positional: Vec<String>,
    values: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Parsed {
    fn last(&self, flag: &str) -> Option<&str> {
        self.values.iter().rev().find(|(name, _)| name == flag).map(|(_, value)| value.as_str())
    }

    fn all(&self, flag: &str) -> Vec<String> {
        self.values.iter().filter(|(name, _)| name == flag).map(|(_, value)| value.clone()).collect()
    }
}

/// Splits arguments the way argparse would for these commands: `--flag value`,
/// `--flag=value`, bare switches and positionals.
fn parse(args: &[String], valued: &[&str], switches: &[&str], positionals: usize) -> Result<Parsed, Failure> {
    let mut parsed = Parsed { positional: Vec::new(), values: Vec::new(), switches: Vec::new() };
    let mut at = 0;
    while at < args.len() {
        let arg = &args[at];
        at += 1;
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, inline)) if flag.starts_with("--") => (flag, Some(inline.to_string())),
            _ => (arg.as_str(), None),
        };
        if valued.contains(&flag) {
            let value = match inline {
                Some(inline) => inline,
                None => {
                    at += 1;
                    args.get(at - 1).cloned().ok_or_else(|| Failure(format!("argument {flag}: expected one argument")))?
                }
            };
            parsed.values.push((flag.to_string(), value));
        } else if switches.contains(&flag) && inline.is_none() {
            parsed.switches.push(flag.to_string());
        } else if arg.starts_with("--") {
            return Err(Failure(format!("unrecognized arguments: {arg}")));
        } else {
            parsed.positional.push(arg.clone());
        }
    }
    if parsed.positional.len() != positionals {
        return Err(Failure(format!("expected {positionals} positional arguments, got {}", parsed.positional.len())));
    }
    Ok(parsed)
}

/// Runs one command line (without the program name) and returns the exit status.
pub fn run(args: &[String], io: &mut Io, env: Env) -> i32 {
    run_with(args, io, env, &mut SystemRunner)
}

/// [`run`] with the runner `converge` starts the agent CLIs through.
pub fn run_with(args: &[String], io: &mut Io, env: Env, runner: &mut dyn Runner) -> i32 {
    match dispatch(args, io, env, runner) {
        Ok(status) => status,
        Err(Failure(message)) => {
            let _ = writeln!(io.err, "agent-kit-merge: {message}");
            2
        }
    }
}

/// The value of a flag argparse marks required.
fn required(parsed: &Parsed, flags: &[&str]) -> Result<(), Failure> {
    let missing: Vec<&str> = flags.iter().copied().filter(|flag| parsed.last(flag).is_none()).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(Failure(format!("the following arguments are required: {}", missing.join(", "))))
    }
}

/// `converge mcp|plugins`.
fn converge_command(rest: &[String], io: &mut Io, env: Env, runner: &mut dyn Runner) -> Result<i32, Failure> {
    let Some((surface, rest)) = rest.split_first() else {
        return Err(Failure("converge: expected mcp or plugins".to_string()));
    };
    match surface.as_str() {
        "mcp" => {
            let valued = ["--keep", "--claude-home", "--claude", "--codex", "--vibe", "--vibe-private", "--opencode", "--declared", "--retired", "--private"];
            let parsed = parse(rest, &valued, &["--check"], 0)?;
            required(&parsed, &["--claude-home", "--keep"])?;
            let path = |flag: &str| parsed.last(flag).map(PathBuf::from);
            let args = McpArgs {
                keep: PathBuf::from(parsed.last("--keep").unwrap_or_default()),
                claude_home: PathBuf::from(parsed.last("--claude-home").unwrap_or_default()),
                claude: parsed.all("--claude").into_iter().map(PathBuf::from).collect(),
                codex: path("--codex"),
                vibe: path("--vibe"),
                vibe_private: path("--vibe-private"),
                opencode: path("--opencode"),
                declared: parsed.all("--declared"),
                retired: parsed.last("--retired").unwrap_or_default().to_string(),
                private: parsed.last("--private").unwrap_or_default().to_string(),
                check: !parsed.switches.is_empty(),
            };
            converge::mcp(&args, runner, io, env)
        }
        "plugins" => {
            let valued = ["--keep", "--declared-markets", "--declared-plugins", "--retired-markets", "--retired-plugins"];
            let parsed = parse(rest, &valued, &["--check"], 0)?;
            required(&parsed, &["--keep"])?;
            let text = |flag: &str| parsed.last(flag).unwrap_or_default().to_string();
            let args = PluginArgs {
                keep: PathBuf::from(text("--keep")),
                declared_markets: text("--declared-markets"),
                declared_plugins: text("--declared-plugins"),
                retired_markets: text("--retired-markets"),
                retired_plugins: text("--retired-plugins"),
                check: !parsed.switches.is_empty(),
            };
            converge::plugins(&args, runner, io)
        }
        other => Err(Failure(format!("converge: unknown surface {other:?}; expected mcp or plugins"))),
    }
}

fn dispatch(args: &[String], io: &mut Io, env: Env, runner: &mut dyn Runner) -> Result<i32, Failure> {
    let Some((command, rest)) = args.split_first() else {
        writeln!(io.err, "{USAGE}")?;
        return Ok(2);
    };
    let remove_config = |rest: &[String]| parse(rest, &["--config", "--remove"], &[], 1);
    match command.as_str() {
        "antigravity-mcp" => {
            let parsed = remove_config(rest)?;
            let config = parsed.last("--config").map(PathBuf::from).unwrap_or_else(|| mcp_json::antigravity_default(env));
            mcp_json::antigravity(&PathBuf::from(&parsed.positional[0]), &config, &parsed.all("--remove"), io)
        }
        "opencode-mcp" => {
            let parsed = remove_config(rest)?;
            let config = parsed.last("--config").map(PathBuf::from).unwrap_or_else(|| mcp_json::opencode_default(env));
            mcp_json::opencode(&PathBuf::from(&parsed.positional[0]), &config, &parsed.all("--remove"), io)
        }
        "hermes-mcp" => {
            let parsed = remove_config(rest)?;
            let config = parsed.last("--config").filter(|c| !c.is_empty()).map(PathBuf::from).unwrap_or_else(|| mcp_yaml::default_config(env));
            mcp_yaml::hermes(&PathBuf::from(&parsed.positional[0]), &config, &parsed.all("--remove"), io)
        }
        "vibe-mcp" => {
            let parsed = parse(rest, &["--config", "--remove", "--key"], &[], 1)?;
            let key = parsed.last("--key").unwrap_or("mcp_servers");
            if key != "mcp_servers" && key != "hooks" {
                return Err(Failure(format!("argument --key: invalid choice: '{key}' (choose from 'mcp_servers', 'hooks')")));
            }
            let config = parsed.last("--config").map(PathBuf::from).unwrap_or_else(|| mcp_toml::vibe_default(env, key));
            mcp_toml::vibe(&PathBuf::from(&parsed.positional[0]), &config, &parsed.all("--remove"), key, io)
        }
        "codex-add-url" => {
            let parsed = parse(rest, &[], &[], 3)?;
            mcp_toml::codex_add_url(&PathBuf::from(&parsed.positional[0]), &parsed.positional[1], &parsed.positional[2])
        }
        "instruction-block" => {
            let parsed = parse(rest, &[], &["--remove"], 2)?;
            block::instruction_block(&PathBuf::from(&parsed.positional[0]), &parsed.positional[1], !parsed.switches.is_empty(), io)
        }
        "package-proxy" => {
            let parsed = parse(rest, &["--validate", "--refuse", "--mode"], &["--remove"], 2)?;
            let validate = match parsed.last("--validate") {
                None => None,
                Some("toml") => Some(Validate::Toml),
                Some("ini") => Some(Validate::Ini),
                Some(other) => return Err(Failure(format!("argument --validate: invalid choice: '{other}' (choose from 'toml', 'ini')"))),
            };
            let options = Options {
                remove: !parsed.switches.is_empty(),
                validate,
                refuse: parsed.last("--refuse").map(str::to_string),
                mode: parsed.last("--mode").map(str::to_string),
            };
            proxy::package_proxy(&PathBuf::from(&parsed.positional[0]), &parsed.positional[1], &options, io)
        }
        "converge" => converge_command(rest, io, env, runner),
        other => Err(Failure(format!("unknown command {other:?}\n{USAGE}"))),
    }
}
