//! The command line: `prove`, `check`, `run` and the hidden `stub-acp` and
//! `pinned-acp`.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::harness::{self, ProveOptions};
use crate::live::{self, LiveInputs};
use crate::pinned;
use crate::record::{self, Outcome};
use crate::refuse;
use crate::stub;

/// The process exit code of a refused run.
pub const REFUSED: i32 = 5;
/// The exit code of a live run killed at its `--timeout` limit.
pub const TIMED_OUT: i32 = 6;

/// The seconds one `gold-band run` may take when `--timeout` is not given.
const DEFAULT_TIMEOUT_SECS: u64 = 600;

/// The usage text: the three commands and the flags each takes.
const USAGE: &str = "\
usage: gold-band-proof prove --workflow FILE [--gold-band BIN] [--gold-band-commit SHA]
                             [--record-dir DIR] [--build-record FILE]
                             [--scenario NAME:VERDICT:EXIT]... [--timeout SECS]
       gold-band-proof check --workflow FILE [--record-dir DIR] [--build-record FILE]
       gold-band-proof run   --workflow FILE [--record-dir DIR] [--build-record FILE]
                             [--gold-band BIN] [--pins FILE [--instance-home DIR]]
                             [--timeout SECS] [-- <gold-band run arguments>]

prove  run the workflow headless against the stub agent and record the proof
       (a scenario verdict is clean, needs-human, fail, gate-fail=NAME or
       grant-lapse=NAME)
check  refuse unless a passing proof of the workflow is recorded
run    check, then start `gold-band run` for the workflow; with --pins, in a
       run home that adds a pinned shim agent for every gate, copier and
       verdict id to the instance's agents (docs/gold-band-proof.md); with --pins,
       --timeout kills the live run and its processes after that many seconds
";

/// Everything the command line reads from outside its arguments.
pub struct Env<'a> {
    /// The environment variable lookup.
    pub get: &'a dyn Fn(&str) -> Option<String>,
    /// The running executable, which answers `stub-acp`.
    pub exe: PathBuf,
    /// A directory for the proof's temporary homes; the caller creates it.
    pub scratch: PathBuf,
    /// The RFC 3339 UTC timestamp to record for a new proof.
    pub now: String,
}

/// The commands that name a workflow.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Subcommand {
    /// `prove`.
    Prove,
    /// `check`.
    Check,
    /// `run`.
    Run,
}

/// The flags of a command that names a workflow, as the command line spelled them.
struct Options {
    /// The subcommand.
    command: Subcommand,
    /// The workflow file.
    workflow: PathBuf,
    /// The Gold Band binary, when one was named.
    gold_band: Option<PathBuf>,
    /// The Gold Band commit, when one was named.
    gold_band_commit: Option<String>,
    /// The proof record directory, when one was named.
    record_dir: Option<PathBuf>,
    /// The build record file, when one was named.
    build_record: Option<PathBuf>,
    /// The `--scenario` texts, in order.
    scenarios: Vec<String>,
    /// The seconds one `gold-band run` may take.
    timeout_secs: u64,
    /// The wall-clock limit a live run was given with `--timeout`, if any.
    run_limit: Option<u64>,
    /// The arguments after `--`, for `run`.
    extra: Vec<OsString>,
    /// The pins file of a live run, for `run`.
    pins: Option<PathBuf>,
    /// The instance's Gold Band home of a live run, when one was named.
    instance_home: Option<PathBuf>,
}

/// What a command line asks for.
enum Invocation {
    /// Print the usage text.
    Usage,
    /// Serve the stub ACP agent on the given streams.
    StubAcp,
    /// Serve the pinned shim agent on the given streams.
    PinnedAcp,
    /// One of the commands that names a workflow.
    Command(Box<Options>),
}

/// Runs one command line against `env` and the given streams, and returns the
/// process exit code.
pub fn run_with(
    args: &[OsString],
    env: &Env,
    stdin: &mut dyn BufRead,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let invocation = match parse(args) {
        Ok(invocation) => invocation,
        Err(problem) => return usage_error(stderr, &problem),
    };
    match invocation {
        Invocation::Usage => {
            let _ = write!(stdout, "{}", USAGE);
            0
        }
        Invocation::StubAcp => stub_acp(env, stdin, stdout),
        Invocation::PinnedAcp => pinned_acp(env, stdin, stdout, stderr),
        Invocation::Command(options) => match options.command {
            Subcommand::Prove => prove(&options, env, stdout, stderr),
            Subcommand::Check => check(&options, env, stdout, stderr),
            Subcommand::Run => run_command(&options, env, stderr),
        },
    }
}

/// Runs the command line with the real process environment and streams.
pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<OsString> = args.collect();
    let get = |name: &str| std::env::var(name).ok();
    let scratch = std::env::temp_dir().join(format!(
        "gold-band-proof-{}-{}",
        std::process::id(),
        nanos()
    ));
    let _ = fs::create_dir_all(&scratch);
    let env = Env {
        get: &get,
        exe: std::env::current_exe().unwrap_or(PathBuf::from("gold-band-proof")),
        scratch: scratch.clone(),
        now: record::now_utc(),
    };
    let code = {
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        let stderr = std::io::stderr();
        let mut stderr = stderr.lock();
        run_with(&args, &env, &mut stdin, &mut stdout, &mut stderr)
    };
    let _ = fs::remove_dir_all(&scratch);
    code
}

/// Parses the arguments after the program name.
fn parse(args: &[OsString]) -> Result<Invocation, String> {
    let Some(first) = args.first() else {
        return Ok(Invocation::Usage);
    };
    let name = first.to_string_lossy();
    if name == "-h" || name == "--help" {
        return Ok(Invocation::Usage);
    }
    let command = match &*name {
        "prove" => Subcommand::Prove,
        "check" => Subcommand::Check,
        "run" => Subcommand::Run,
        "stub-acp" => return Ok(Invocation::StubAcp),
        "pinned-acp" => return Ok(Invocation::PinnedAcp),
        other => return Err(format!("unknown subcommand `{}`", other)),
    };
    let mut workflow: Option<PathBuf> = None;
    let mut gold_band: Option<PathBuf> = None;
    let mut gold_band_commit: Option<String> = None;
    let mut record_dir: Option<PathBuf> = None;
    let mut build_record: Option<PathBuf> = None;
    let mut scenarios: Vec<String> = Vec::new();
    let mut timeout_secs = DEFAULT_TIMEOUT_SECS;
    let mut run_limit: Option<u64> = None;
    let mut extra: Vec<OsString> = Vec::new();
    let mut pins: Option<PathBuf> = None;
    let mut instance_home: Option<PathBuf> = None;
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].to_string_lossy().into_owned();
        match flag.as_str() {
            "--" => {
                if command != Subcommand::Run {
                    return Err("`--` is only for `run`".to_string());
                }
                extra = args[index + 1..].to_vec();
                // A second workflow would run unproven; the proof covers only the one named.
                if extra.iter().any(|argument| {
                    let argument = argument.to_string_lossy();
                    argument == "--workflow" || argument.starts_with("--workflow=")
                }) {
                    return Err("`--workflow` cannot be repeated after `--`".to_string());
                }
                index = args.len();
            }
            "--workflow" => workflow = Some(PathBuf::from(value(args, &mut index, &flag)?)),
            "--gold-band" => gold_band = Some(PathBuf::from(value(args, &mut index, &flag)?)),
            "--gold-band-commit" => gold_band_commit = Some(text(args, &mut index, &flag)?),
            "--record-dir" => record_dir = Some(PathBuf::from(value(args, &mut index, &flag)?)),
            "--build-record" => build_record = Some(PathBuf::from(value(args, &mut index, &flag)?)),
            "--scenario" => scenarios.push(text(args, &mut index, &flag)?),
            "--timeout" => {
                timeout_secs = timeout(&text(args, &mut index, &flag)?)?;
                run_limit = Some(timeout_secs);
            }
            "--pins" | "--instance-home" if command != Subcommand::Run => {
                return Err(format!("`{}` is only for `run`", flag));
            }
            "--pins" => pins = Some(PathBuf::from(value(args, &mut index, &flag)?)),
            "--instance-home" => {
                instance_home = Some(PathBuf::from(value(args, &mut index, &flag)?))
            }
            other if other.starts_with('-') => return Err(format!("unknown flag `{}`", other)),
            other => return Err(format!("unexpected argument `{}`", other)),
        }
        index += 1;
    }
    let Some(workflow) = workflow else {
        return Err("`--workflow` is required".to_string());
    };
    if instance_home.is_some() && pins.is_none() {
        return Err("`--instance-home` needs `--pins`".to_string());
    }
    Ok(Invocation::Command(Box::new(Options {
        command,
        workflow,
        gold_band,
        gold_band_commit,
        record_dir,
        build_record,
        scenarios,
        timeout_secs,
        run_limit,
        extra,
        pins,
        instance_home,
    })))
}

/// The value of the flag at `index`, advancing past it.
fn value(args: &[OsString], index: &mut usize, flag: &str) -> Result<OsString, String> {
    *index += 1;
    match args.get(*index) {
        Some(value) => Ok(value.clone()),
        None => Err(format!("`{}` needs a value", flag)),
    }
}

/// The value of the flag at `index` as text, advancing past it.
fn text(args: &[OsString], index: &mut usize, flag: &str) -> Result<String, String> {
    value(args, index, flag).map(|value| value.to_string_lossy().into_owned())
}

/// A positive number of seconds from a `--timeout` value.
fn timeout(text: &str) -> Result<u64, String> {
    match text.parse::<u64>() {
        Ok(seconds) if seconds > 0 => Ok(seconds),
        _ => Err(format!(
            "`--timeout` needs a positive number of seconds, not `{}`",
            text
        )),
    }
}

/// `prove`: run the proof and write its record.
fn prove(options: &Options, env: &Env, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let dir = match record_dir(options, env) {
        Ok(dir) => dir,
        Err(problem) => return failure(stderr, &problem),
    };
    let commit = match &options.gold_band_commit {
        Some(commit) => Some(commit.clone()),
        None => installed_commit(build_record(options, env).as_deref()),
    };
    let mut extra_scenarios = Vec::new();
    for text in &options.scenarios {
        match harness::parse_scenario(text) {
            Ok(scenario) => extra_scenarios.push(scenario),
            Err(problem) => return usage_error(stderr, &problem),
        }
    }
    let prove_options = ProveOptions {
        workflow: options.workflow.clone(),
        gold_band: named_gold_band(options, env),
        gold_band_commit: commit,
        stub_exe: env.exe.clone(),
        scratch: env.scratch.clone(),
        extra_scenarios,
        timeout_secs: options.timeout_secs,
        recorded_at: env.now.clone(),
    };
    let record = match harness::prove(&prove_options) {
        Ok(record) => record,
        Err(problem) => return failure(stderr, &problem),
    };
    let path = match record::write_record(&dir, &record) {
        Ok(path) => path,
        Err(problem) => return failure(stderr, &problem),
    };
    for scenario in &record.scenarios {
        let _ = writeln!(
            stdout,
            "  {}: expected exit {}, got {} ({}/{})",
            scenario.name,
            scenario.expected_exit,
            exit_text(scenario.exit_code),
            status_text(&scenario.run_status),
            status_text(&scenario.run_outcome)
        );
    }
    if record.outcome == Outcome::Skipped {
        let reason = record
            .skip_reason
            .as_deref()
            .unwrap_or("no reason recorded");
        let _ = writeln!(stdout, "gold-band-proof: skipped: {}", reason);
    }
    let _ = writeln!(
        stdout,
        "gold-band-proof: {} {}",
        record.outcome.as_str(),
        path.display()
    );
    if record.outcome != Outcome::Failed {
        return 0;
    }
    for scenario in record
        .scenarios
        .iter()
        .filter(|scenario| !scenario.passed())
    {
        let note = scenario.note.as_deref().unwrap_or("no note recorded");
        let _ = writeln!(stderr, "gold-band-proof: {}: {}", scenario.name, note);
    }
    1
}

/// `check`: refuse unless a passing proof of the workflow is recorded.
fn check(options: &Options, env: &Env, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let dir = match record_dir(options, env) {
        Ok(dir) => dir,
        Err(problem) => return failure(stderr, &problem),
    };
    let commit = installed_commit(build_record(options, env).as_deref());
    match refuse::check(&dir, &options.workflow, commit.as_deref()) {
        Ok(()) => {
            let _ = writeln!(stdout, "gold-band-proof: ok");
            0
        }
        Err(message) => {
            let _ = writeln!(stderr, "{}", message);
            REFUSED
        }
    }
}

/// `run`: check the proof, then start `gold-band run` and return its exit code.
fn run_command(options: &Options, env: &Env, stderr: &mut dyn Write) -> i32 {
    let dir = match record_dir(options, env) {
        Ok(dir) => dir,
        Err(problem) => return failure(stderr, &problem),
    };
    let commit = installed_commit(build_record(options, env).as_deref());
    if let Err(message) = refuse::check(&dir, &options.workflow, commit.as_deref()) {
        let _ = writeln!(stderr, "{}", message);
        return REFUSED;
    }
    let program = gold_band_program(options, env);
    let mut command = Command::new(&program);
    if let Some(pins) = &options.pins {
        match live_home(options, pins, env) {
            Ok(home) => {
                let _ = writeln!(stderr, "gold-band-proof: live run home {}", home.display());
                command.env("GOLD_BAND_HOME", home);
            }
            Err(problems) => {
                for problem in problems {
                    let _ = writeln!(stderr, "gold-band-proof: {}", problem);
                }
                return REFUSED;
            }
        }
    }
    command
        .arg("run")
        .arg("--workflow")
        .arg(&options.workflow)
        .args(&options.extra)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    // A live run may be bounded by `--timeout`; its own process group is
    // killed at the limit so no agent outlives the run.
    let limit = options.pins.as_ref().and(options.run_limit);
    if limit.is_some() {
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let _ = writeln!(
                stderr,
                "gold-band-proof: cannot start {}: {}",
                program.display(),
                error
            );
            return 3;
        }
    };
    let Some(limit) = limit else {
        return child
            .wait()
            .ok()
            .and_then(|status| status.code())
            .unwrap_or(3);
    };
    let deadline = Instant::now() + Duration::from_secs(limit);
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code().unwrap_or(3);
        }
        if Instant::now() >= deadline {
            let group = format!("-{}", child.id());
            let _ = Command::new("kill").args(["-KILL", "--", &group]).status();
            let _ = child.kill();
            let _ = child.wait();
            let _ = writeln!(
                stderr,
                "gold-band-proof: the live run passed its limit of {} seconds and was killed",
                limit
            );
            return TIMED_OUT;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `stub-acp`: serve the stub agent on the given streams.
fn stub_acp(env: &Env, stdin: &mut dyn BufRead, mut stdout: &mut dyn Write) -> i32 {
    let verdict = (env.get)(stub::VERDICT_ENV).unwrap_or_default();
    let models = (env.get)(stub::MODELS_ENV).unwrap_or_default();
    let role = stub::Role::parse(&(env.get)(stub::ROLE_ENV).unwrap_or_default());
    let report_dir = (env.get)(stub::REPORT_DIR_ENV).unwrap_or_default();
    match stub::serve_with_role(stdin, &mut stdout, &verdict, &models, &role, &report_dir) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// `pinned-acp`: serve the pinned shim agent on the given streams.
fn pinned_acp(
    env: &Env,
    stdin: &mut dyn BufRead,
    mut stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let config = match pinned::Config::from_env(env.get) {
        Ok(config) => config,
        Err(problem) => return failure(stderr, &problem),
    };
    match pinned::serve(stdin, &mut stdout, config) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Builds the run home of a live run and returns it, or every reason the run
/// cannot be bound.
fn live_home(options: &Options, pins: &Path, env: &Env) -> Result<PathBuf, Vec<String>> {
    let text = fs::read_to_string(pins).map_err(|error| {
        vec![format!(
            "cannot read the pins file {}: {}",
            pins.display(),
            error
        )]
    })?;
    let pins = live::parse_pins(&text).map_err(|problem| vec![problem])?;
    let workflow =
        fs::read_to_string(&options.workflow).map_err(|error| vec![error.to_string()])?;
    let info = harness::inspect(&workflow).map_err(|problem| vec![problem])?;
    let instance_home = options
        .instance_home
        .clone()
        .or_else(|| (env.get)("GOLD_BAND_HOME").map(PathBuf::from))
        .or_else(|| (env.get)("HOME").map(PathBuf::from))
        .ok_or_else(|| vec!["no instance home: pass --instance-home or set HOME".to_string()])?;
    let state = state_dir(env).ok_or_else(|| vec!["no state directory: set HOME".to_string()])?;
    let run_home = state
        .join("agent-kit/gold-band/live")
        .join(format!("{}-{}", info.id, nanos()));
    let mut values = BTreeMap::new();
    let tools = env.exe.parent().unwrap_or(Path::new("/"));
    values.insert("tools".to_string(), tools.to_string_lossy().into_owned());
    values.extend(head_commit().map(|base| ("base".to_string(), base)));
    if let Some(requirement) = requirement_file(&options.extra) {
        values.insert(
            "requirement".to_string(),
            requirement.to_string_lossy().into_owned(),
        );
    }
    live::compose(&LiveInputs {
        info: &info,
        pins: &pins,
        instance_home: &instance_home,
        run_home: &run_home,
        exe: &env.exe,
        values: &values,
    })?;
    Ok(run_home)
}

/// The commit the working directory's repository is at when the run starts,
/// the base a gate measures the run's change against.
fn head_commit() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

/// The requirement file the run arguments name, made absolute against the
/// working directory, since the gates run from the node's.
fn requirement_file(extra: &[OsString]) -> Option<PathBuf> {
    let index = extra
        .iter()
        .position(|argument| argument == "--requirement-file")?;
    let path = PathBuf::from(extra.get(index + 1)?);
    if path.is_absolute() {
        return Some(path);
    }
    std::env::current_dir().ok().map(|dir| dir.join(path))
}

/// The proof record directory: the flag, else the environment, else the state
/// directory.
fn record_dir(options: &Options, env: &Env) -> Result<PathBuf, String> {
    if let Some(dir) = &options.record_dir {
        return Ok(dir.clone());
    }
    if let Some(dir) = (env.get)("GOLD_BAND_PROOF_DIR") {
        return Ok(PathBuf::from(dir));
    }
    match state_dir(env) {
        Some(dir) => Ok(dir.join("agent-kit/gold-band/proofs")),
        None => Err(
            "no proof directory: pass --record-dir or set GOLD_BAND_PROOF_DIR or HOME".to_string(),
        ),
    }
}

/// The state directory: `$XDG_STATE_HOME`, else `$HOME/.local/state`.
fn state_dir(env: &Env) -> Option<PathBuf> {
    if let Some(dir) = (env.get)("XDG_STATE_HOME") {
        return Some(PathBuf::from(dir));
    }
    (env.get)("HOME").map(|home| PathBuf::from(home).join(".local/state"))
}

/// The build record file: the flag, else the state directory's.
fn build_record(options: &Options, env: &Env) -> Option<PathBuf> {
    if let Some(path) = &options.build_record {
        return Some(path.clone());
    }
    state_dir(env).map(|dir| dir.join("agent-kit/gold-band/build.json"))
}

/// The commit the build record names, or `None` when there is no readable
/// record or it names no commit.
fn installed_commit(path: Option<&Path>) -> Option<String> {
    let text = fs::read_to_string(path?).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value.get("commit")?.as_str().map(str::to_string)
}

/// The Gold Band binary the flag or `GOLD_BAND_BIN` names; an empty value names none.
fn named_gold_band(options: &Options, env: &Env) -> Option<PathBuf> {
    options
        .gold_band
        .clone()
        .or_else(|| (env.get)("GOLD_BAND_BIN").map(PathBuf::from))
        .filter(|path| !path.as_os_str().is_empty())
}

/// The Gold Band binary to start: the one named, else `gold-band` on PATH.
fn gold_band_program(options: &Options, env: &Env) -> PathBuf {
    named_gold_band(options, env).unwrap_or(PathBuf::from("gold-band"))
}

/// An exit code as the proof summary spells it, `none` when there is none.
fn exit_text(code: Option<i32>) -> String {
    code.map(|code| code.to_string())
        .unwrap_or("none".to_string())
}

/// A run state field as the proof summary spells it, `none` when there is none.
fn status_text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("none")
}

/// Prints a usage problem and the hint, and returns the usage exit code.
fn usage_error(stderr: &mut dyn Write, problem: &str) -> i32 {
    let _ = writeln!(stderr, "gold-band-proof: {}", problem);
    let _ = writeln!(stderr, "hint: run `gold-band-proof --help` for the usage");
    2
}

/// Prints a failure and returns the usage exit code.
fn failure(stderr: &mut dyn Write, problem: &str) -> i32 {
    let _ = writeln!(stderr, "gold-band-proof: {}", problem);
    2
}

/// The nanoseconds since the epoch, or 0 when the clock is before it.
fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0)
}
