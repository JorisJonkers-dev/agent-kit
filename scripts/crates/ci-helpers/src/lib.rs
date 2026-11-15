//! The CI helpers that were Python scripts until #813: one binary, four
//! subcommands.
//!
//! * `agents-md [--require] [REPO_ROOT ...]` checks each repository's AGENTS.md
//!   for dead paths and symbols (was scripts/check_agents_md.py).
//! * `marketplace-bump ...` fails a marketplace pin bump that adds anything able
//!   to run code (was scripts/check_marketplace_bump.py).
//! * `skill-sources ...` checks the estate skills' recorded sources against
//!   GitHub (was scripts/check_skill_sources.py).
//! * `verify-release-asset ...` stamps and checks the release version token
//!   (was scripts/verify_release_asset.py).

use std::io::{Read, Write};

pub mod agents_md;
pub mod json;
pub mod marketplace;
pub mod release_asset;
pub mod skill_sources;

#[cfg(test)]
mod tests;

/// The three streams a subcommand talks to, so a test can hold them.
pub struct Io<'a> {
    pub stdin: &'a mut dyn Read,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

impl Io<'_> {
    pub fn say(&mut self, line: &str) {
        let _ = writeln!(self.out, "{line}");
    }

    pub fn complain(&mut self, line: &str) {
        let _ = writeln!(self.err, "{line}");
    }
}

const USAGE: &str =
    "usage: ci-helpers agents-md|marketplace-bump|skill-sources|verify-release-asset [ARGS]";

/// Run one subcommand against the real streams; the exit status.
pub fn run(args: Vec<String>) -> i32 {
    let (stdin, stdout, stderr) = (std::io::stdin(), std::io::stdout(), std::io::stderr());
    let (mut stdin, mut out, mut err) = (stdin.lock(), stdout.lock(), stderr.lock());
    dispatch(
        &args,
        &mut Io {
            stdin: &mut stdin,
            out: &mut out,
            err: &mut err,
        },
    )
}

pub fn dispatch(args: &[String], io: &mut Io) -> i32 {
    let rest = args.get(1..).unwrap_or(&[]);
    match args.first().map(String::as_str) {
        Some("agents-md") => agents_md::run(rest, io),
        Some("marketplace-bump") => marketplace::run(rest, io),
        Some("skill-sources") => skill_sources::run(rest, io),
        Some("verify-release-asset") => release_asset::run(rest, io),
        _ => {
            io.complain(USAGE);
            2
        }
    }
}

/// Take `--name VALUE` or `--name=VALUE` off the front of `args[i]`: the value
/// and how many arguments it used, or None when `args[i]` is another option.
pub(crate) fn option_value<'a>(
    args: &'a [String],
    i: usize,
    name: &str,
) -> Option<(Option<&'a str>, usize)> {
    let arg = args[i].as_str();
    if arg == name {
        return Some((args.get(i + 1).map(String::as_str), 2));
    }
    arg.strip_prefix(name)
        .and_then(|rest| rest.strip_prefix('='))
        .map(|value| (Some(value), 1))
}
