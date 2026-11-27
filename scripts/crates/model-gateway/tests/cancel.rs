//! A gateway stopped while a client runs passes the stop on to the client's process group, says so, and starts
//! nothing after it. It runs in its own test binary, since the stop is a real SIGTERM to this process.

use model_gateway::adapters::process::SystemProcess;
use model_gateway::ports::{LaunchWatch, ProcessCommand, Stream};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

#[derive(Default)]
struct Watch {
    pid: AtomicU32,
    notes: Mutex<Vec<String>>,
}

impl LaunchWatch for Watch {
    fn spawned(&self, pid: u32) {
        self.pid.store(pid, Ordering::SeqCst);
    }
    fn output(&self, _stream: Stream, _chunk: &[u8]) {}
    fn note(&self, text: &str) {
        self.notes.lock().expect("notes").push(text.to_string());
    }
}

#[test]
fn a_stopped_gateway_stops_its_client_and_starts_nothing_after() {
    let process = SystemProcess { env: HashMap::from([("PATH".to_string(), std::env::var("PATH").unwrap_or_default())]) };
    let watch = Watch::default();
    let command = ProcessCommand::new("sh", &["-c", "sleep 30 & wait"]);
    let launched = std::thread::scope(|scope| {
        scope.spawn(|| {
            while watch.pid.load(Ordering::SeqCst) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let status = std::process::Command::new("kill").args(["-TERM", &std::process::id().to_string()]).status().expect("kill runs");
            assert!(status.success());
        });
        process.watch_with_grace(&command, 200, &watch).expect("the shell starts")
    });
    assert_eq!(launched.ending.cancelled_by, Some(15));
    assert_eq!(launched.ending.signal, Some(15));
    assert!(!launched.result.timed_out);
    assert!(watch.notes.lock().expect("notes").iter().any(|note| note.contains("stopped by SIGTERM")));
    let refused = process.watch_with_grace(&command, 200, &watch).expect_err("refused");
    assert_eq!(refused, "spawn sh refused: the gateway was stopped by SIGTERM");
}
