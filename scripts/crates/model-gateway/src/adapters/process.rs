//! The real programs: PATH lookup, and a client's own listing or status command run with a time limit.

use crate::ports::{Ending, LaunchWatch, Launched, Process, ProcessCommand, ProcessResult, Stream, Unwatched};
use crate::text::join;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::process::{ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

/// Looks for programs along PATH and runs them in the environment it was given.
pub struct SystemProcess {
    pub env: HashMap<String, String>,
}

/// The exit code Node reports for a command a signal ended (the timeout's kill).
const KILLED: i32 = 124;

/// How long a command has to exit after SIGTERM before it is killed.
pub const KILL_GRACE_MS: u64 = 5_000;

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;

extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn signal(signal: i32, handler: usize) -> usize;
}

/// Like `access(X_OK)`: a directory with an execute bit counts, as it does for Node.
fn is_executable(path: &str) -> bool {
    std::fs::metadata(path).map(|meta| meta.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

impl Process for SystemProcess {
    fn resolve(&self, binary: &str) -> Option<String> {
        let path = self.env.get("PATH").map_or("", String::as_str);
        path.split(':')
            .filter(|folder| !folder.is_empty())
            .map(|folder| join(&[folder, binary]))
            .find(|candidate| is_executable(candidate))
    }

    fn exec(&self, command: &str, args: &[&str], timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        let mut child = Command::new(command)
            .args(args)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn {command} {}", error_words(&error)))?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let out = std::thread::spawn(move || read_all(&mut stdout));
        let err = std::thread::spawn(move || read_all(&mut stderr));
        let started = Instant::now();
        let mut timed_out = false;
        let status = loop {
            if let Ok(Some(status)) = child.try_wait() {
                break status;
            }
            if let Some(limit) = timeout_ms {
                if !timed_out && started.elapsed() >= Duration::from_millis(limit) {
                    timed_out = true;
                    let _ = child.kill();
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        let stdout = out.join().unwrap_or_default();
        let stderr = err.join().unwrap_or_default();
        let exit_code = status.code().unwrap_or(KILLED);
        Ok(ProcessResult { stdout, stderr, exit_code, timed_out })
    }

    fn run(&self, command: &ProcessCommand) -> Result<ProcessResult, String> {
        self.run_with_grace(command, KILL_GRACE_MS)
    }

    fn run_watched(&self, command: &ProcessCommand, watch: &dyn LaunchWatch) -> Result<Launched, String> {
        self.watch_with_grace(command, KILL_GRACE_MS, watch)
    }

    fn is_alive(&self, pid: u32) -> bool {
        // SAFETY: signal 0 sends nothing; it only asks whether the process is there.
        unsafe { kill(pid as i32, 0) == 0 }
    }

    fn signal_group(&self, pid: u32, signal: i32) -> bool {
        signal_group(pid, signal)
    }
}

impl SystemProcess {
    /// `run` with the time a command has to exit after SIGTERM named, so a test need not wait the full grace.
    pub fn run_with_grace(&self, command: &ProcessCommand, kill_grace_ms: u64) -> Result<ProcessResult, String> {
        self.watch_with_grace(command, kill_grace_ms, &Unwatched).map(|launched| launched.result)
    }

    /// The launch itself. The command starts as the leader of a process group of its own, so the time limit, a
    /// stop of the gateway and the clean-up after it reach every process it started; its output is pumped by
    /// one thread per stream and handed to `watch` as it is read; and how it ended is decoded into an exit code
    /// or a signal.
    pub fn watch_with_grace(&self, command: &ProcessCommand, kill_grace_ms: u64, watch: &dyn LaunchWatch) -> Result<Launched, String> {
        self.launch(command, kill_grace_ms, watch, &|| STOPPED_BY.load(Ordering::SeqCst))
    }

    /// The launch, with where the stop of the gateway is read from named, so a test can stop one launch alone.
    pub(crate) fn launch(&self, command: &ProcessCommand, kill_grace_ms: u64, watch: &dyn LaunchWatch, stop: &(dyn Fn() -> i32 + Sync)) -> Result<Launched, String> {
        let stopped = stop();
        if stopped != 0 {
            return Err(format!("spawn {} refused: the gateway was stopped by {}", command.command, signal_name(stopped)));
        }
        let _stop = StopGuard::hold();
        let mut launch = Command::new(&command.command);
        launch.args(&command.args).env_clear().envs(&self.env).envs(&command.env).process_group(0);
        if let Some(cwd) = &command.cwd {
            launch.current_dir(cwd);
        }
        let mut child = launch
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn {} {}", command.command, error_words(&error)))?;
        let pid = child.id();
        watch.spawned(pid);
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let stdin = Arc::new(Mutex::new(child.stdin.take()));
        // The input is written beside the wait and the pipe closed after it, whether or not the
        // command read it all: a command that exits early must not fail the write. A command the watch drives
        // (ACP) is sent its opening instead, and its pipe stays open for the watch's replies to its output.
        let interactive = watch.interactive();
        let input = if interactive { watch.opening() } else { command.stdin.clone().unwrap_or_default() };
        let writer = Arc::clone(&stdin);
        std::thread::spawn(move || {
            let mut pipe = writer.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = pipe.as_mut().map(|open| open.write_all(input.as_bytes()));
            if !interactive {
                *pipe = None;
            }
        });
        std::thread::scope(|scope| {
            let out = scope.spawn(|| pump(stdout, Stream::Stdout, watch, interactive.then_some(&*stdin)));
            let err = scope.spawn(|| pump(stderr, Stream::Stderr, watch, None));
            let waited = wait_for(&mut child, (command.timeout_ms, kill_grace_ms), watch, stop);
            // What the command left running in its group would hold the pipes open, and outlive the run.
            if signal_group(pid, 0) {
                signal_group(pid, SIGKILL);
                watch.note("killed what the command left running in its process group after it exited");
            }
            let stdout = out.join().unwrap_or_default();
            let stderr = err.join().unwrap_or_default();
            let status = waited.status;
            let result = ProcessResult { stdout, stderr, exit_code: status.code().unwrap_or(KILLED), timed_out: waited.terminated && waited.cancelled_by.is_none() };
            Ok(Launched { result, ending: Ending { signal: status.signal(), cancelled_by: waited.cancelled_by, pid: Some(pid) } })
        })
    }
}

/// How the wait for a launch ended.
struct Waited {
    status: std::process::ExitStatus,
    /// Whether the runner sent SIGTERM: the time limit passed or the gateway was stopped.
    terminated: bool,
    cancelled_by: Option<i32>,
}

/// Waits for the command, stopping its group with SIGTERM when it outlives its time limit or the gateway is
/// stopped, and with SIGKILL when it ignores that for `kill_grace_ms`.
fn wait_for(child: &mut std::process::Child, limits: (Option<u64>, u64), watch: &dyn LaunchWatch, stop: &dyn Fn() -> i32) -> Waited {
    let (timeout_ms, kill_grace_ms) = limits;
    let pid = child.id();
    let started = Instant::now();
    let mut terminated: Option<Instant> = None;
    let mut cancelled_by = None;
    let mut killed = false;
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return Waited { status, terminated: terminated.is_some(), cancelled_by };
        }
        let stopped = stop();
        if terminated.is_none() && stopped != 0 {
            terminated = Some(Instant::now());
            cancelled_by = Some(stopped);
            signal_group(pid, SIGTERM);
            watch.note(&format!("the gateway was stopped by {}; sent SIGTERM to the command's process group", signal_name(stopped)));
        }
        if terminated.is_none() && timeout_ms.is_some_and(|limit| started.elapsed() >= Duration::from_millis(limit)) {
            terminated = Some(Instant::now());
            signal_group(pid, SIGTERM);
            watch.note(&format!("the command outlived its time limit of {} ms; sent SIGTERM to its process group", timeout_ms.unwrap_or_default()));
        }
        if !killed && terminated.is_some_and(|at| at.elapsed() >= Duration::from_millis(kill_grace_ms)) {
            killed = true;
            signal_group(pid, SIGKILL);
            watch.note(&format!("the command's process group ignored SIGTERM for {kill_grace_ms} ms; sent SIGKILL"));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Sends `signal` to every process of the group `pid` leads; whether any was there to get it.
fn signal_group(pid: u32, signal: i32) -> bool {
    // SAFETY: `kill` only sends a signal; a negative id names the process group the command leads.
    unsafe { kill(-(pid as i32), signal) == 0 }
}

/// Reads one stream to its end, handing each chunk to `watch` as it comes; the whole stream as text. A stream
/// given the client's stdin writes what the watch replies to each chunk, and closes the pipe when it says so.
fn pump(mut source: impl Read, stream: Stream, watch: &dyn LaunchWatch, stdin: Option<&Mutex<Option<ChildStdin>>>) -> String {
    let mut all = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        match source.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                watch.output(stream, &buffer[..read]);
                all.extend_from_slice(&buffer[..read]);
                if let Some(stdin) = stdin {
                    let reply = watch.reply(&buffer[..read]);
                    let mut pipe = stdin.lock().unwrap_or_else(PoisonError::into_inner);
                    let _ = pipe.as_mut().map(|open| open.write_all(reply.text.as_bytes()));
                    if reply.close {
                        *pipe = None;
                    }
                }
            }
        }
    }
    String::from_utf8_lossy(&all).into_owned()
}

/// The conventional name of a signal (`SIGKILL`), or `SIG<number>` for one without a name here.
pub fn signal_name(signal: i32) -> String {
    let named = [(1, "SIGHUP"), (2, "SIGINT"), (3, "SIGQUIT"), (6, "SIGABRT"), (9, "SIGKILL"), (11, "SIGSEGV"), (13, "SIGPIPE"), (14, "SIGALRM"), (15, "SIGTERM")];
    named.iter().find(|(number, _)| *number == signal).map_or_else(|| format!("SIG{signal}"), |(_, name)| (*name).to_string())
}

/// The signal the gateway was told to stop with while a launch ran (SIGHUP, SIGINT or SIGTERM); 0 while none.
/// It is never cleared: a gateway told to stop starts no further launch.
static STOPPED_BY: AtomicI32 = AtomicI32::new(0);

/// How many launches are running, and the handlers the first of them replaced.
static STOP_HANDLERS: Mutex<(usize, Vec<(i32, usize)>)> = Mutex::new((0, Vec::new()));

const STOP_SIGNALS: [i32; 3] = [SIGHUP, SIGINT, SIGTERM];

extern "C" fn on_stop(signal: i32) {
    STOPPED_BY.store(signal, Ordering::SeqCst);
}

/// Held while a launch runs: the gateway catches the stop signals, so it can pass them on to the command's
/// group (which no longer shares the terminal's) and still write how the run ended. The last launch to end puts
/// the handlers back.
struct StopGuard;

impl StopGuard {
    fn hold() -> StopGuard {
        let mut held = STOP_HANDLERS.lock().unwrap_or_else(PoisonError::into_inner);
        if held.0 == 0 {
            // SAFETY: `on_stop` only stores to an atomic, which is safe in a signal handler.
            held.1 = STOP_SIGNALS.iter().map(|&number| (number, unsafe { signal(number, on_stop as *const () as usize) })).collect();
        }
        held.0 += 1;
        StopGuard
    }
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        let mut held = STOP_HANDLERS.lock().unwrap_or_else(PoisonError::into_inner);
        held.0 -= 1;
        if held.0 == 0 {
            for (number, previous) in held.1.drain(..) {
                // SAFETY: puts back the handler `hold` replaced.
                unsafe { signal(number, previous) };
            }
        }
    }
}

fn read_all(source: &mut impl Read) -> String {
    let mut bytes = Vec::new();
    let _ = source.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A spawn failure as Node words it (`spawn vibe ENOENT`).
fn error_words(error: &std::io::Error) -> String {
    match error.raw_os_error() {
        Some(2) => "ENOENT".to_string(),
        Some(13) => "EACCES".to_string(),
        _ => error.to_string(),
    }
}
