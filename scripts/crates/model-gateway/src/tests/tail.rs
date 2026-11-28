//! Tests for `tail`: the filters, the follow, and the coalescing that holds a run to one line per window.

use super::machine::{now_ms, Machine, DEFAULTS};
use crate::json::{parse, Value};
use crate::ports::LaunchWatch;
use crate::run_events::{RunEvents, RunIdentity, RunStart};
use crate::tail::{parse_args, run, Coalescer, TailOptions, COALESCE_MS, LINES, POLL_MS};
use std::io::Write;

#[test]
fn a_run_prints_one_line_per_window_and_the_rest_is_counted() {
    let mut coalescer = Coalescer::new(10_000);
    // The first line of a run goes out at once; the rest wait for the window's end.
    assert_eq!(coalescer.offer("r1", "diagnostic", "one".into(), 0), ["one"]);
    assert!(coalescer.offer("r1", "diagnostic", "two".into(), 1_000).is_empty());
    assert!(coalescer.offer("r1", "message", "three".into(), 2_000).is_empty());
    assert!(coalescer.due(9_999).is_empty());
    assert_eq!(coalescer.due(10_000), ["three (+1 earlier events coalesced)"]);
    assert!(coalescer.due(30_000).is_empty());
    // Another run has its own window.
    assert_eq!(coalescer.offer("r2", "diagnostic", "other".into(), 2_000), ["other"]);
    // A held line alone has no count.
    assert!(coalescer.offer("r2", "diagnostic", "held".into(), 3_000).is_empty());
    assert_eq!(coalescer.flush(), ["held"]);
    assert!(coalescer.flush().is_empty());
    // After a window with nothing held, the next line goes out at once.
    assert_eq!(coalescer.offer("r1", "diagnostic", "late".into(), 60_000), ["late"]);
}

#[test]
fn a_stall_a_cancel_and_a_finish_are_never_held_back_and_bring_what_was_held_with_them() {
    let mut coalescer = Coalescer::new(10_000);
    assert_eq!(coalescer.offer("r", "diagnostic", "a".into(), 0), ["a"]);
    for (at, line) in [(1, "b"), (2, "c"), (3, "d")] {
        assert!(coalescer.offer("r", "diagnostic", line.into(), at).is_empty());
    }
    assert_eq!(coalescer.offer("r", "stall", "stalled".into(), 4), ["d (+2 earlier events coalesced)", "stalled"]);
    assert_eq!(coalescer.offer("r", "cancel", "cancelled".into(), 5), ["cancelled"]);
    assert!(coalescer.offer("r", "diagnostic", "e".into(), 6).is_empty());
    assert_eq!(coalescer.offer("r", "run.finished", "done".into(), 7), ["e", "done"]);
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_string()).collect()
}

#[test]
fn the_arguments_set_the_options_and_anything_else_is_refused() {
    let defaults = TailOptions::default();
    assert_eq!((defaults.follow, defaults.lines, defaults.poll_ms, defaults.coalesce_ms), (false, LINES, POLL_MS, COALESCE_MS));
    let options = parse_args(&strings(&["-f", "--types", "stall, run.finished,", "--run", "r1", "--workflow", "w", "-n", "5", "--since", "30", "--json", "--exit-when-done", "--poll-ms", "20", "--coalesce-ms", "50"])).expect("options");
    assert_eq!((options.follow, options.types.clone(), options.run.as_deref(), options.workflow.as_deref()), (true, strings(&["stall", "run.finished"]), Some("r1"), Some("w")));
    assert_eq!((options.lines, options.since_s, options.json, options.exit_when_done, options.poll_ms, options.coalesce_ms), (5, 30, true, true, 20, 50));
    assert!(parse_args(&strings(&["--follow"])).expect("long form").follow);
    for bad in [&["--nope"][..], &["--types"], &["--run"], &["--workflow"], &["-n", "x"], &["-n"], &["--since", "x"], &["--poll-ms", "x"], &["--coalesce-ms", "x"]] {
        assert!(parse_args(&strings(bad)).is_none(), "{bad:?}");
    }
}

fn start<'a>(machine: &'a Machine, id: &str, workflow: &str) -> RunEvents<'a> {
    let dir = format!("{}/state/agent-kit/model-gateway/runs/{id}", machine.sandbox.text());
    std::fs::create_dir_all(&dir).expect("run dir");
    let identity = RunIdentity { run_id: id.into(), workflow: workflow.into(), client: "pi".into(), model: "free/hy4-flash".into(), family: "tencent".into() };
    let index = crate::locations::run_index_path(&machine.runtime.env).expect("index path");
    RunEvents::start(machine.runtime.files.as_ref(), machine.runtime.clock.as_ref(), &dir, identity, &RunStart { mode: "read-only", time_limit_s: 900.0 }).indexed(Some(&index))
}

fn tailed(machine: &Machine, args: &[&str]) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run(&strings(args), &machine.runtime, &mut out, &mut err);
    (code, String::from_utf8(out).expect("utf8"), String::from_utf8(err).expect("utf8"))
}

/// Three runs: one finished well, one stalled and cancelled, one of another workflow.
fn three(machine: &Machine) {
    let first = start(machine, "r1", "a");
    first.spawned(5);
    first.diagnostic("stderr", "warn", "something");
    first.finish(parse(r#"{"status":"ok","reason":null,"exit_code":0}"#).expect("JSON"));
    let second = start(machine, "r2", "a");
    second.record("stall", parse(r#"{"silent_s":180,"threshold_s":180,"cancel_at_s":540}"#).expect("JSON"));
    second.record("cancel", parse(r#"{"by":"stall","reason":"quiet"}"#).expect("JSON"));
    second.finish(parse(r#"{"status":"cancelled","reason":"cancelled"}"#).expect("JSON"));
    let third = start(machine, "r3", "b");
    third.finish(parse(r#"{"status":"failed","reason":"x"}"#).expect("JSON"));
}

#[test]
fn a_plain_tail_prints_the_last_events_of_the_recent_runs_and_stops() {
    let machine = Machine::new(DEFAULTS);
    three(&machine);
    let (code, out, err) = tailed(&machine, &[]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out.lines().count(), 10);
    let (_, out, _) = tailed(&machine, &["-n", "1"]);
    assert_eq!(out.lines().count(), 1);
    assert!(out.contains(" r3 run.finished "), "{out}");
    let (_, out, _) = tailed(&machine, &["--types", "stall,run.finished", "--workflow", "a"]);
    let kinds: Vec<&str> = out.lines().map(|line| line.split(' ').nth(2).unwrap_or_default()).collect();
    assert_eq!(kinds, ["run.finished", "stall", "run.finished"]);
    let (_, out, _) = tailed(&machine, &["--run", "r2", "--json", "--types", "cancel"]);
    assert_eq!(out.lines().count(), 1);
    assert_eq!(parse(out.lines().next().expect("a line")).ok().as_ref().and_then(Value::as_object).and_then(|event| event.get("type")), Some(&Value::str("cancel")));
}

#[test]
fn a_followed_tail_with_a_type_filter_prints_only_those_events_and_stops_when_the_runs_are_done() {
    let machine = Machine::new(DEFAULTS);
    three(&machine);
    let (code, out, _) = tailed(&machine, &["-f", "--types", "stall,run.finished", "--exit-when-done", "--poll-ms", "5"]);
    assert_eq!(code, 0);
    let kinds: Vec<&str> = out.lines().map(|line| line.split(' ').nth(2).unwrap_or_default()).collect();
    assert_eq!(kinds, ["run.finished", "stall", "run.finished", "run.finished"]);
    assert!(out.contains("status=cancelled") && out.contains("silent 180s of 180s"), "{out}");
}

#[test]
fn a_run_that_ended_before_the_window_is_not_replayed_and_a_named_run_ends_the_follow() {
    let machine = Machine::new(DEFAULTS);
    three(&machine);
    // Everything started at the machine's clock; look from an hour after, with a window of ten minutes.
    let mut late = Machine::at(DEFAULTS, now_ms() + 3_600_000);
    late.runtime.env = machine.runtime.env.clone();
    late.runtime.files = Box::new(crate::adapters::fs::SystemFiles { cwd: machine.sandbox.text() });
    let (_, out, _) = tailed_with(&late, &["-f", "--exit-when-done", "--poll-ms", "5"]);
    assert_eq!(out, "", "the runs ended an hour ago");
    let (_, out, _) = tailed(&machine, &["-f", "--run", "r2", "--types", "run.finished", "--poll-ms", "5"]);
    assert_eq!(out.lines().count(), 1, "{out}");
}

fn tailed_with(machine: &Machine, args: &[&str]) -> (i32, String, String) {
    // With nothing to follow the loop would never end: stop it from outside once it has had its look.
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| run(&strings(args), &machine.runtime, &mut out, &mut err));
        std::thread::sleep(std::time::Duration::from_millis(100));
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        // The follow has no runs, so it only ends when something finishes: give it one.
        let events = start(machine, "r-late", "z");
        events.finish(parse(r#"{"status":"ok"}"#).expect("JSON"));
        handle.join().expect("the tail")
    });
    (0, String::from_utf8(out).expect("utf8"), String::from_utf8(err).expect("utf8"))
}

#[test]
fn a_busy_run_is_coalesced_in_a_followed_tail_and_its_finish_comes_at_once() {
    let machine = Machine::new(DEFAULTS);
    let events = start(&machine, "r-busy", "w");
    for number in 0..6 {
        events.diagnostic("stderr", "warn", &format!("line {number}"));
    }
    events.finish(parse(r#"{"status":"ok","exit_code":0}"#).expect("JSON"));
    let (_, out, _) = tailed(&machine, &["-f", "--exit-when-done", "--poll-ms", "5", "--coalesce-ms", "60000"]);
    let lines: Vec<&str> = out.lines().collect();
    // run.started goes out at once; the six diagnostics are held, and the finish brings the last of them with it.
    assert_eq!(lines.len(), 3, "{out}");
    assert!(lines[0].contains(" run.started ") && lines[1].contains("line 5 (+5 earlier events coalesced)") && lines[2].contains(" run.finished "), "{out}");
}

#[test]
fn a_log_that_is_cut_short_is_read_again_from_its_start_and_a_half_written_line_waits() {
    let machine = Machine::new(DEFAULTS);
    let events = start(&machine, "r-cut", "w");
    let log = format!("{}/state/agent-kit/model-gateway/runs/r-cut/events.jsonl", machine.sandbox.text());
    let mut args = ["-f", "--exit-when-done", "--poll-ms", "5", "--coalesce-ms", "1", "--types", "diagnostic,run.finished"];
    args[0] = "-f";
    let (mut out, mut err) = (Vec::new(), Vec::new());
    std::thread::scope(|scope| {
        let handle = scope.spawn(|| run(&strings(&args), &machine.runtime, &mut out, &mut err));
        std::thread::sleep(std::time::Duration::from_millis(60));
        events.diagnostic("stderr", "warn", "before");
        std::thread::sleep(std::time::Duration::from_millis(60));
        // The file is replaced by a shorter one, whose last line is not finished yet.
        let kept = std::fs::read_to_string(&log).expect("log");
        let first = kept.lines().next().expect("a line").to_string();
        std::fs::write(&log, format!("{first}\n{{\"half")).expect("cut");
        std::thread::sleep(std::time::Duration::from_millis(60));
        std::fs::write(&log, format!("{first}\n")).expect("cut again");
        std::thread::sleep(std::time::Duration::from_millis(60));
        events.diagnostic("stderr", "warn", "after");
        events.finish(parse(r#"{"status":"ok"}"#).expect("JSON"));
        handle.join().expect("the tail");
    });
    let out = String::from_utf8(out).expect("utf8");
    assert!(out.contains("before") && out.contains("run.finished"), "{out}");
}

/// A writer whose reader has gone.
struct Closed;

impl Write for Closed {
    fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_tail_whose_reader_has_gone_ends_quietly_and_a_bad_call_is_refused() {
    let machine = Machine::new(DEFAULTS);
    three(&machine);
    let mut err = Vec::new();
    assert_eq!(run(&strings(&["-f", "--poll-ms", "5"]), &machine.runtime, &mut Closed, &mut err), 0);
    // The first printed line fails: so does the first of a plain tail, which just ends.
    assert_eq!(run(&strings(&[]), &machine.runtime, &mut Closed, &mut err), 0);
    // A held line due later also finds the reader gone.
    let mut options_err = Vec::new();
    assert_eq!(run(&strings(&["-f", "--coalesce-ms", "1", "--poll-ms", "5", "--types", "diagnostic"]), &machine.runtime, &mut Closed, &mut options_err), 0);
    let (code, _, err) = tailed(&machine, &["--bogus"]);
    assert_eq!(code, 1);
    assert!(err.starts_with("model-gateway tail: tail takes [-f]"), "{err}");
    let mut broken = Machine::new(DEFAULTS);
    broken.runtime.env.clear();
    let (code, _, err) = tailed(&broken, &[]);
    assert_eq!(code, 1);
    assert!(err.contains("required environment variable is missing: HOME"), "{err}");
    // The binary's own entry sends `tail` here, with the streams it was given.
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(crate::run_with(["tail".to_string(), "--bogus".to_string()].into_iter(), &machine.runtime, &mut out, &mut err), 1);
}

/// A writer that takes some lines and then finds its reader gone.
struct GoneAfter(usize);

impl Write for GoneAfter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0 == 0 {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        self.0 -= 1;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_held_line_that_falls_due_for_a_reader_that_has_gone_ends_the_tail() {
    let mut machine = Machine::new(DEFAULTS);
    machine.runtime.clock = Box::new(crate::adapters::clock::SystemClock);
    let events = start(&machine, "r-held", "w");
    events.diagnostic("stderr", "warn", "one");
    events.diagnostic("stderr", "warn", "two");
    // `run.started` is printed; the diagnostics are held and fall due 20 ms later, when the reader is gone.
    let started = std::time::Instant::now();
    let mut err = Vec::new();
    assert_eq!(run(&strings(&["-f", "--coalesce-ms", "20", "--poll-ms", "40"]), &machine.runtime, &mut GoneAfter(2), &mut err), 0);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    drop(events);
}
