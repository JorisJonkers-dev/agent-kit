//! `tail`: the events of the runs on this machine as lines, which an orchestrator watches with Monitor at no
//! model cost. `--types stall,run.finished` keeps only those types, `--run` and `--workflow` only those runs.
//! With `-f` it follows: a run's events are printed as they are written, coalesced to at most one line per run
//! every 10 seconds, except that a finish, a stall and a cancel are always printed at once.

use crate::json::Value;
use crate::ports::Runtime;
use crate::run_status::{format_event, indexed_runs, read_events};
use crate::text::join;
use std::collections::HashMap;
use std::io::Write;

/// At most one line per run in this window, in milliseconds.
pub const COALESCE_MS: i64 = 10_000;

/// How often a followed log is looked at, in milliseconds.
pub const POLL_MS: u64 = 500;

/// A followed run that started longer ago than this (and ended) is skipped, not replayed, in seconds.
pub const SINCE_S: i64 = 600;

/// How many events a plain `tail` prints.
pub const LINES: usize = 20;

/// The event types that are never held back.
const PRIORITY: [&str; 3] = ["run.finished", "stall", "cancel"];

/// One run's window: when it last printed, and what it holds back.
struct Window {
    opened_ms: i64,
    held: Option<String>,
    count: usize,
}

/// Keeps a run to one line per window.
pub struct Coalescer {
    window_ms: i64,
    runs: HashMap<String, Window>,
}

impl Coalescer {
    pub fn new(window_ms: i64) -> Coalescer {
        Coalescer { window_ms, runs: HashMap::new() }
    }

    fn held_line(window: &mut Window) -> Option<String> {
        let line = window.held.take()?;
        let more = window.count.saturating_sub(1);
        window.count = 0;
        Some(if more == 0 { line } else { format!("{line} (+{more} earlier events coalesced)") })
    }

    /// What to print for an event of `run` at `now_ms`: the line itself when the run has printed nothing in the
    /// window or the type is a priority one (with what was held back before it), else nothing, the line being
    /// held for the window's end.
    pub fn offer(&mut self, run: &str, kind: &str, line: String, now_ms: i64) -> Vec<String> {
        let priority = PRIORITY.contains(&kind);
        let window = self.runs.entry(run.to_string()).or_insert(Window { opened_ms: now_ms - self.window_ms, held: None, count: 0 });
        if priority || now_ms - window.opened_ms >= self.window_ms {
            let mut lines: Vec<String> = Self::held_line(window).into_iter().collect();
            lines.push(line);
            window.opened_ms = now_ms;
            return lines;
        }
        window.held = Some(line);
        window.count += 1;
        Vec::new()
    }

    /// The lines whose window has ended.
    pub fn due(&mut self, now_ms: i64) -> Vec<String> {
        let mut lines = Vec::new();
        for window in self.runs.values_mut() {
            if window.held.is_some() && now_ms - window.opened_ms >= self.window_ms {
                lines.extend(Self::held_line(window));
                window.opened_ms = now_ms;
            }
        }
        lines
    }

    /// Everything still held, at the end.
    pub fn flush(&mut self) -> Vec<String> {
        self.runs.values_mut().filter_map(Self::held_line).collect()
    }
}

/// What `tail` was asked for.
#[derive(Clone, Debug)]
pub struct TailOptions {
    pub follow: bool,
    pub types: Vec<String>,
    pub run: Option<String>,
    pub workflow: Option<String>,
    pub lines: usize,
    pub since_s: i64,
    pub json: bool,
    pub exit_when_done: bool,
    pub poll_ms: u64,
    pub coalesce_ms: i64,
}

impl Default for TailOptions {
    fn default() -> Self {
        TailOptions { follow: false, types: Vec::new(), run: None, workflow: None, lines: LINES, since_s: SINCE_S, json: false, exit_when_done: false, poll_ms: POLL_MS, coalesce_ms: COALESCE_MS }
    }
}

pub const USAGE: &str = "tail takes [-f] [--types TYPE,TYPE] [--run RUN] [--workflow NAME] [-n LINES] [--since SECONDS] [--exit-when-done] [--json]";

/// The options the arguments name, or `None` when they are not tail's.
pub fn parse_args(args: &[String]) -> Option<TailOptions> {
    let mut options = TailOptions::default();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-f" | "--follow" => options.follow = true,
            "--json" => options.json = true,
            "--exit-when-done" => options.exit_when_done = true,
            "--types" => options.types = iter.next()?.split(',').map(|name| name.trim().to_string()).filter(|name| !name.is_empty()).collect(),
            "--run" => options.run = Some(iter.next()?.clone()),
            "--workflow" => options.workflow = Some(iter.next()?.clone()),
            "-n" => options.lines = iter.next()?.parse().ok()?,
            "--since" => options.since_s = iter.next()?.parse().ok()?,
            "--poll-ms" => options.poll_ms = iter.next()?.parse().ok()?,
            "--coalesce-ms" => options.coalesce_ms = iter.next()?.parse().ok()?,
            _ => return None,
        }
    }
    Some(options)
}

fn kind_of(event: &Value) -> String {
    event.as_object().and_then(|object| object.get("type")).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn ts_of(event: &Value) -> String {
    event.as_object().and_then(|object| object.get("ts")).and_then(Value::as_str).unwrap_or_default().to_string()
}

fn wanted(options: &TailOptions, event: &Value) -> bool {
    options.types.is_empty() || options.types.contains(&kind_of(event))
}

fn shown(options: &TailOptions, event: &Value) -> String {
    if options.json {
        crate::json::compact(event)
    } else {
        format_event(event)
    }
}

/// How far into a followed log a run was read, and whether it ended.
struct Followed {
    offset: usize,
    finished: bool,
}

/// Runs `tail` over `args`, writing to `out`; the exit code. `err` gets the refusal of a bad argument.
pub fn run(args: &[String], runtime: &Runtime, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let Some(options) = parse_args(args) else {
        let _ = writeln!(err, "model-gateway tail: {USAGE}");
        return 1;
    };
    match tail(&options, runtime, out) {
        Ok(()) => 0,
        Err(message) => {
            let _ = writeln!(err, "model-gateway tail: {message}");
            1
        }
    }
}

fn matching_runs(options: &TailOptions, runtime: &Runtime) -> Result<Vec<crate::run_index::IndexedRun>, String> {
    let runs = indexed_runs(runtime).map_err(|error| error.message)?;
    Ok(runs
        .into_iter()
        .filter(|run| options.run.as_deref().is_none_or(|key| run.run_id.starts_with(key)))
        .filter(|run| options.workflow.as_deref().is_none_or(|workflow| run.workflow == workflow))
        .collect())
}

/// Prints the lines; `false` once the reader is gone.
fn print(out: &mut dyn Write, lines: &[String]) -> bool {
    lines.iter().all(|line| writeln!(out, "{line}").is_ok()) && out.flush().is_ok()
}

fn tail(options: &TailOptions, runtime: &Runtime, out: &mut dyn Write) -> Result<(), String> {
    if !options.follow {
        let from = runtime.clock.now_ms() - 86_400_000;
        let mut events: Vec<Value> = matching_runs(options, runtime)?.iter().filter(|run| run.started_ms >= from).flat_map(|run| read_events(runtime, &run.run_dir)).filter(|event| wanted(options, event)).collect();
        events.sort_by_key(ts_of);
        let skip = events.len().saturating_sub(options.lines);
        print(out, &events[skip..].iter().map(|event| shown(options, event)).collect::<Vec<_>>());
        return Ok(());
    }
    let mut coalescer = Coalescer::new(options.coalesce_ms);
    let mut followed: HashMap<String, Followed> = HashMap::new();
    loop {
        let now = runtime.clock.now_ms();
        let runs = matching_runs(options, runtime)?;
        for run in &runs {
            let log = join(&[&run.run_dir, crate::run_events::EVENTS_FILE]);
            let text = runtime.files.read_text(&log).unwrap_or_default();
            let entry = followed.entry(run.run_id.clone()).or_insert_with(|| {
                // A run that ended before the window is not replayed; a running or recent one is, from its start.
                let old = run.started_ms < now - options.since_s * 1000 && text.contains("\"type\":\"run.finished\"");
                Followed { offset: if old { text.len() } else { 0 }, finished: old }
            });
            let Some(fresh) = text.get(entry.offset..) else {
                entry.offset = 0;
                continue;
            };
            // Only whole lines: the last may still be being written.
            let whole = fresh.rfind('\n').map_or(0, |at| at + 1);
            for line in fresh[..whole].lines() {
                let Ok(event) = crate::json::parse(line) else { continue };
                if kind_of(&event) == "run.finished" {
                    entry.finished = true;
                }
                if wanted(options, &event) && !print(out, &coalescer.offer(&run.run_id, &kind_of(&event), shown(options, &event), now)) {
                    return Ok(());
                }
            }
            entry.offset += whole;
        }
        if !print(out, &coalescer.due(runtime.clock.now_ms())) {
            return Ok(());
        }
        let ended = !followed.is_empty() && followed.values().all(|run| run.finished);
        let named_ended = options.run.is_some() && ended;
        if named_ended || (options.exit_when_done && ended) {
            print(out, &coalescer.flush());
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(options.poll_ms));
    }
}
