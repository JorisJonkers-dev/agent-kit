//! The pr-composer skill's two helpers, ported from Python (issue #813):
//!
//! ```text
//! pr-composer check-body -t TEMPLATE -b BODY [--skipped HEADINGS]
//! pr-composer mine-prs [--out PROFILE.json] [--md] DUMP.json
//! ```
//!
//! `check-body` exits 0 when the body matches the template, 1 with the
//! violations printed, 2 for a bad invocation. `mine-prs` writes the
//! house-style profile (and with `--md` a markdown summary) and exits 0, or 1
//! when the dump cannot be read.

mod args;
mod check_body;
mod json;
mod mine_prs;
mod regex;
mod text;

use std::ffi::OsString;
use std::io::Write;

/// What a command wrote, kept apart so the tests can read it.
#[derive(Default)]
pub struct Io {
    pub out: String,
    pub err: String,
}

const USAGE: &str = "usage: pr-composer check-body -t TEMPLATE -b BODY [--skipped HEADINGS]\n       pr-composer mine-prs [--out PROFILE.json] [--md] DUMP.json\n";

/// Runs one command line and returns its exit status; the output is in `io`.
pub fn run_io<I: IntoIterator<Item = OsString>>(args: I, io: &mut Io) -> i32 {
    let args: Vec<String> = args
        .into_iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match args.split_first() {
        Some((cmd, rest)) if cmd == "check-body" => check_body::run(rest, io),
        Some((cmd, rest)) if cmd == "mine-prs" => mine_prs::run(rest, io),
        Some((flag, _)) if flag == "--help" || flag == "-h" => {
            io.out.push_str(USAGE);
            0
        }
        _ => {
            io.err.push_str(USAGE);
            2
        }
    }
}

/// Runs one command line against the real standard streams.
pub fn run<I: IntoIterator<Item = OsString>>(args: I) -> i32 {
    let mut io = Io::default();
    let status = run_io(args, &mut io);
    let _ = std::io::stdout().write_all(io.out.as_bytes());
    let _ = std::io::stderr().write_all(io.err.as_bytes());
    status
}

#[cfg(test)]
mod tests;
