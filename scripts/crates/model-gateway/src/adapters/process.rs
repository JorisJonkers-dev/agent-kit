//! The real programs: PATH lookup, and a client's own listing or status command run with a time limit.

use crate::ports::{Process, ProcessResult};
use crate::text::join;
use std::collections::HashMap;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Looks for programs along PATH and runs them in the environment it was given.
pub struct SystemProcess {
    pub env: HashMap<String, String>,
}

/// The exit code Node reports for a command a signal ended (the timeout's kill).
const KILLED: i32 = 124;

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
