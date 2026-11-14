//! Validates and re-indexes the papers directory of the private research
//! repository (#356), and runs Workflow A's two deterministic steps over it:
//! the fetch shim and the verifier (#456).
//!
//! A papers directory holds `catalog.json` (an array of entries with `id`,
//! `title`, `theme`, `tier` A-D, `files`, and optional `year`, `summary`,
//! `url`, `abstract`, `quotes`), a folder per named theme holding a
//! hand-written `README.md` (the explainer) and a generated `INDEX.md`, a
//! generated top-level `INDEX.md`, and `notes/` whose Markdown files cite
//! catalog ids under a front-matter `min_tier` and quote stored pages
//! (`> quote` lines, then `-- <page path>`).
//!
//! * `check <PAPERS_DIR>` fails, each problem one line on stderr as
//!   `research-layout: <path>: <rule>`, for an unlisted file
//!   (`unlisted-file`), a listed file that does not exist
//!   (`file-not-found`), a theme folder without `README.md`
//!   (`missing-theme-readme`), a stale `INDEX.md` (`stale-index`), a
//!   top-level name outside the layout (`not-allowed`), a broken catalog
//!   entry (`missing-field`, `bad-tier`), a note citing an unknown id
//!   (`unknown-cite`) or an entry below its `min_tier` (`tier-below-min`),
//!   a quote not on its stored page (`quote-not-found`), and any path that
//!   leaves the papers directory (`path-escapes`).
//! * `index <PAPERS_DIR>` regenerates every `INDEX.md` deterministically.
//! * `fetch --plan FILE --out DIR [--max-sources N] [--max-bytes N]
//!   [--timeout-secs N] [--curl PROG]` is the fetch shim ([fetch]).
//! * `adopt --plan FILE --fetched DIR --papers DIR` copies every stored page
//!   into the plan's theme folder ([fetch::adopt]).
//! * `verify --papers DIR --fetched DIR [--base REF]` is the verifier
//!   ([verify]); `--base` defaults to `origin/main`.
//!
//! Exit codes: 0 ok, 1 a rule failed, 2 bad usage or an unreadable input.
//! Problems go to stderr, success to stdout.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;

pub mod fetch;
pub mod html;
pub mod layout;
pub mod verify;

pub use layout::{check, write_indexes};

pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The default and the bounds of the fetch limits.
const MAX_SOURCES: (u64, u64, u64) = (8, 1, 32);
const MAX_BYTES: (u64, u64, u64) = (5_000_000, 1, 50_000_000);
const TIMEOUT_SECS: (u64, u64, u64) = (30, 1, 600);

/// The usage text a bad command line prints on stderr.
fn usage() {
    eprintln!("usage: {NAME} check <PAPERS_DIR>");
    eprintln!("       {NAME} index <PAPERS_DIR>");
    eprintln!(
        "       {NAME} fetch --plan FILE --out DIR [--max-sources N] [--max-bytes N] [--timeout-secs N] [--curl PROG]"
    );
    eprintln!("       {NAME} adopt --plan FILE --fetched DIR --papers DIR");
    eprintln!("       {NAME} verify --papers DIR --fetched DIR [--base REF]");
}

/// Prints the problems (each `research-layout: <path>: <rule>`) and returns 1,
/// or prints `ok` and returns 0.
fn report(problems: &[String]) -> i32 {
    if problems.is_empty() {
        println!("{NAME}: ok");
        return 0;
    }
    for problem in problems {
        eprintln!("{NAME}: {problem}");
    }
    1
}

/// Prints an input problem and returns 2.
fn input_error(problem: &str) -> i32 {
    eprintln!("{NAME}: {problem}");
    2
}

/// The `--name value` pairs of a command line, or `None` (after printing
/// the usage) for a flag not in `known`, a repeated flag or a flag without
/// its value.
fn flags(args: Vec<OsString>, known: &[&str]) -> Option<BTreeMap<String, OsString>> {
    let mut found = BTreeMap::new();
    let mut args = args.into_iter();
    while let Some(flag) = args.next() {
        let name = flag.to_string_lossy().into_owned();
        let value = args.next();
        let good = known.contains(&name.as_str()) && value.is_some() && !found.contains_key(&name);
        if !good {
            usage();
            return None;
        }
        found.insert(name, value.unwrap_or_default());
    }
    Some(found)
}

/// A numeric flag within its bounds, its default when absent, or `None`
/// (after printing the usage) when it is not a number in bounds.
fn number(found: &BTreeMap<String, OsString>, name: &str, bounds: (u64, u64, u64)) -> Option<u64> {
    let (default, low, high) = bounds;
    let Some(value) = found.get(name) else {
        return Some(default);
    };
    let parsed = value.to_str().and_then(|v| v.parse::<u64>().ok());
    match parsed {
        Some(n) if (low..=high).contains(&n) => Some(n),
        _ => {
            usage();
            None
        }
    }
}

/// The `check` command line.
fn check_command(dir: &Path) -> i32 {
    report(&check(dir))
}

/// The `index` command line.
fn index_command(dir: &Path) -> i32 {
    match write_indexes(dir) {
        Ok(count) => {
            println!("{NAME}: {count} indexes regenerated");
            0
        }
        Err(problems) => report(&problems),
    }
}

/// The `fetch` command line.
fn fetch_command(args: Vec<OsString>) -> i32 {
    let known = [
        "--plan",
        "--out",
        "--max-sources",
        "--max-bytes",
        "--timeout-secs",
        "--curl",
    ];
    let Some(found) = flags(args, &known) else {
        return 2;
    };
    let (Some(plan), Some(out)) = (found.get("--plan"), found.get("--out")) else {
        usage();
        return 2;
    };
    let Some(max_sources) = number(&found, "--max-sources", MAX_SOURCES) else {
        return 2;
    };
    let Some(max_bytes) = number(&found, "--max-bytes", MAX_BYTES) else {
        return 2;
    };
    let Some(timeout_secs) = number(&found, "--timeout-secs", TIMEOUT_SECS) else {
        return 2;
    };
    let limits = fetch::Limits {
        max_sources: max_sources as usize,
        max_bytes,
        timeout_secs,
        curl: found
            .get("--curl")
            .map_or("curl".to_string(), |c| c.to_string_lossy().into_owned()),
    };
    match fetch::fetch(Path::new(plan), Path::new(out), &limits) {
        Ok(count) => {
            println!("{NAME}: {count} pages fetched");
            0
        }
        Err(fetch::Failure::Usage(problem)) => input_error(&problem),
        Err(fetch::Failure::Refused(problems)) => report(&problems),
    }
}

/// The `adopt` command line.
fn adopt_command(args: Vec<OsString>) -> i32 {
    let Some(found) = flags(args, &["--plan", "--fetched", "--papers"]) else {
        return 2;
    };
    let (Some(plan), Some(fetched), Some(papers)) = (
        found.get("--plan"),
        found.get("--fetched"),
        found.get("--papers"),
    ) else {
        usage();
        return 2;
    };
    match fetch::adopt(Path::new(plan), Path::new(fetched), Path::new(papers)) {
        Ok(count) => {
            println!("{NAME}: {count} pages adopted");
            0
        }
        Err(fetch::Failure::Usage(problem)) => input_error(&problem),
        Err(fetch::Failure::Refused(problems)) => report(&problems),
    }
}

/// The `verify` command line.
fn verify_command(args: Vec<OsString>, git: &str) -> i32 {
    let Some(found) = flags(args, &["--papers", "--fetched", "--base"]) else {
        return 2;
    };
    let (Some(papers), Some(fetched)) = (found.get("--papers"), found.get("--fetched")) else {
        usage();
        return 2;
    };
    let base = found.get("--base").map_or("origin/main".to_string(), |b| {
        b.to_string_lossy().into_owned()
    });
    match verify::verify(git, Path::new(papers), Path::new(fetched), &base) {
        Ok(problems) => report(&problems),
        Err(problem) => input_error(&problem),
    }
}

/// Runs one command line with `git` as the git program, and returns its
/// exit code (see the crate documentation).
pub fn run_with_git<I>(args: I, git: &str) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let mut args = args.into_iter();
    let cmd = args.next();
    let rest: Vec<OsString> = args.collect();
    match cmd.as_deref() {
        Some(c) if c == OsStr::new("check") && rest.len() == 1 => {
            check_command(Path::new(&rest[0]))
        }
        Some(c) if c == OsStr::new("index") && rest.len() == 1 => {
            index_command(Path::new(&rest[0]))
        }
        Some(c) if c == OsStr::new("fetch") => fetch_command(rest),
        Some(c) if c == OsStr::new("adopt") => adopt_command(rest),
        Some(c) if c == OsStr::new("verify") => verify_command(rest, git),
        _ => {
            usage();
            2
        }
    }
}

/// Runs one command line and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_with_git(args, "git")
}

#[cfg(test)]
mod tests;
