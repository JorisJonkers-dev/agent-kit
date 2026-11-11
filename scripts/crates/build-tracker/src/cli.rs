//! The command line: one command, one JSON object out.

use crate::commands;
use crate::json::{compact, object, pretty, Value};
use crate::ports::Runtime;
use crate::state::Failure;
use std::collections::HashMap;

pub struct CliResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub const USAGE: &str = "usage: build-tracker init --spec SPEC [--milestone <name>=<ticket> ...]
       build-tracker sync --spec SPEC
       build-tracker path --spec SPEC
       build-tracker refresh --spec SPEC [--landed <ticket> ...]
       build-tracker render --state FILE --out FILE
       build-tracker set --state FILE <ticket>=<status>[:note] ...

SPEC is a GitHub issue (#12, owner/repo#12 or its URL) whose sub-issues are
the tickets, or a local markdown spec file whose tickets are the numbered
files (01-name.md) in the issues/ folder beside it.

init writes a new state for the spec from its tickets: each open ticket todo,
each closed one merged, with its blockers. Each --milestone, in order, names a
milestone and the ticket that closes it. init refuses a spec that already has
a state; sync brings that state up to date instead. sync adds new tickets,
updates titles and blockers, marks newly closed tickets merged, and keeps
every status, note and milestone set on open tickets. path prints where the
spec's state file and page live, under $XDG_STATE_HOME (else
~/.local/state)/agent-kit/build-tracker.

refresh brings the spec's page up to date in one step, as /flow runs it: init
(with no milestones) when the spec has no state, else sync; then each
--landed ticket (12, #12, or a local ticket's 01) turns merged, keeping its
note; then the page is rendered at the path path prints.

render writes the build tracker page for the state file. set changes the
named tickets' statuses in the state file; a note after the status replaces
the ticket's note, an empty one removes it, and none keeps it. The statuses
are todo, running, review, merged and owner.

Each command prints one JSON object. An error prints \"status\": \"error\" with
its \"code\" and exits 1, and leaves every file as it was. A missing state file
is an error that names init; an unreadable one names its path and says to
delete it and run init again. Bad arguments print this usage and exit 2.
";

/// The flags a command takes, each `required` (given once) or repeatable (any number of times, none included).
struct CommandSpec {
    required: &'static [&'static str],
    repeatable: &'static [&'static str],
    positionals: bool,
}

fn command_spec(name: &str) -> Option<CommandSpec> {
    let (required, repeatable, positionals): (&'static [&'static str], &'static [&'static str], bool) = match name {
        "init" => (&["spec"], &["milestone"], false),
        "path" | "sync" => (&["spec"], &[], false),
        "refresh" => (&["spec"], &["landed"], false),
        "render" => (&["out", "state"], &[], false),
        "set" => (&["state"], &[], true),
        _ => return None,
    };
    Some(CommandSpec { positionals, repeatable, required })
}

struct Parsed {
    flags: HashMap<String, Vec<String>>,
    positionals: Vec<String>,
}

impl Parsed {
    fn flag(&self, name: &str) -> &str {
        self.flags.get(name).and_then(|values| values.first()).map_or("", String::as_str)
    }

    fn all(&self, name: &str) -> &[String] {
        self.flags.get(name).map_or(&[], Vec::as_slice)
    }
}

/// `--flag value` pairs and positional arguments, or `None` when they do not fit the command.
fn parse_arguments(args: &[String], spec: &CommandSpec) -> Option<Parsed> {
    let mut flags: HashMap<String, Vec<String>> = HashMap::new();
    let mut positionals = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some(name) = arg.strip_prefix("--") else {
            positionals.push(arg.clone());
            index += 1;
            continue;
        };
        let known = spec.required.contains(&name) || spec.repeatable.contains(&name);
        let value = args.get(index + 1)?;
        if !known || (spec.required.contains(&name) && flags.contains_key(name)) {
            return None;
        }
        flags.entry(name.to_string()).or_default().push(value.clone());
        index += 2;
    }
    let flags_ok = spec.required.iter().all(|name| flags.contains_key(*name));
    let positionals_ok = if spec.positionals { !positionals.is_empty() } else { positionals.is_empty() };
    (flags_ok && positionals_ok).then_some(Parsed { flags, positionals })
}

fn run_command(name: &str, parsed: &Parsed, runtime: &Runtime) -> Result<Value, Failure> {
    match name {
        "init" => commands::init(runtime, parsed.flag("spec"), parsed.all("milestone")),
        "path" => commands::path(runtime, parsed.flag("spec")),
        "refresh" => commands::refresh(runtime, parsed.flag("spec"), parsed.all("landed")),
        "render" => commands::render(runtime, parsed.flag("state"), parsed.flag("out")),
        "set" => commands::set(runtime, parsed.flag("state"), &parsed.positionals),
        _ => commands::sync(runtime, parsed.flag("spec")),
    }
}

/// The build tracker's command line: argv in, exit code, stdout and stderr out.
pub fn run_cli(argv: &[String], runtime: &Runtime) -> CliResult {
    let name = argv.first().map_or("", String::as_str);
    if name == "--help" || name == "-h" {
        return CliResult { exit_code: 0, stderr: String::new(), stdout: USAGE.to_string() };
    }
    let usage = || CliResult { exit_code: 2, stderr: USAGE.to_string(), stdout: String::new() };
    let Some(spec) = command_spec(name) else {
        return usage();
    };
    let Some(parsed) = parse_arguments(&argv[1..], &spec) else {
        return usage();
    };
    match run_command(name, &parsed, runtime) {
        Ok(result) => CliResult { exit_code: 0, stderr: String::new(), stdout: format!("{}\n", pretty(&result)) },
        Err(failure) => {
            let body = object(vec![("code", Value::str(failure.code)), ("message", Value::str(&failure.message)), ("status", Value::str("error"))]);
            CliResult { exit_code: 1, stderr: format!("build-tracker {name}: {}\n", failure.message), stdout: format!("{}\n", compact(&body)) }
        }
    }
}
