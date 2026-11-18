//! What every client but Pi is started through (#903): `agent-launcher confine`, with the grants its recipe
//! names (ADR-0016). A confined client can read its working directory (or write it, for a writing run), write
//! its run directory and its own home, and run its own install and the system runtime; nothing else of the
//! machine. This module is the shape of those grants; what each client's recipe holds is in `client_run`, which
//! prepares the homes the grants name, and in `run_home`.

use crate::launchers::{LaunchPaths, LaunchRequest};
use crate::run::RunMode;

/// Why a client cannot run without the launcher.
pub fn no_launcher(client: &str) -> String {
    format!("{client}: agent-launcher is not on PATH, and a delegate only runs confined to its working directory; re-run agent-kit setup, which installs it")
}

/// What one confined launch is granted, beyond the working directory, the run directory and the client's own
/// install, which every launch has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Confinement {
    /// The launcher: `AGENT_KIT_LAUNCHER_BIN`, else `agent-launcher` on `PATH`.
    pub launcher: String,
    /// Paths the client may read (a file or a directory) besides its working directory.
    pub reads: Vec<String>,
    /// Programs it may run besides its own (`--run-from`): the MCP servers a caller attaches to the launch.
    pub run_from: Vec<String>,
    /// Paths the client may read and write (a file, or a directory beneath which it may change anything): its
    /// own home, the one login file or directory it keeps tokens in.
    pub writes: Vec<String>,
    /// The TCP ports it may connect to (the proxy, the provider on 443); none leaves the network alone, which a
    /// client that serves itself on a loopback port (OpenCode, Antigravity) needs: neither backend can name a
    /// port chosen at run time.
    pub ports: Vec<u16>,
    /// macOS: it asks the login keychain for the credential it was signed in with.
    pub keychain: bool,
    /// macOS: it reads the system's and the managed preferences before it starts.
    pub preferences: bool,
    /// Variables the client keeps besides the ones the launch sets (`USER` for a keychain item named after it).
    pub env: Vec<String>,
}

fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

impl Confinement {
    /// The words after `agent-launcher`, up to and including the `--` that the client's own command follows.
    /// `names` are the variables the launch sets, which the launcher passes through by name.
    pub fn args(&self, input: &LaunchRequest, paths: &LaunchPaths, binary: &str, names: &[String]) -> Vec<String> {
        let mut args = strings(&["confine"]);
        if input.mode == Some(RunMode::Write) {
            args.extend(strings(&["--write", &paths.work_dir]));
            for dir in paths.git_writable.as_deref().unwrap_or_default() {
                args.extend(strings(&["--write", dir]));
            }
        } else {
            args.extend(strings(&["--read", &paths.work_dir]));
        }
        args.extend(strings(&["--run-from", binary, "--write", crate::launchers::report_dir(paths)]));
        for program in &self.run_from {
            args.extend(strings(&["--run-from", program]));
        }
        for read in &self.reads {
            args.extend(strings(&["--read", read]));
        }
        for write in &self.writes {
            args.extend(strings(&["--write", write]));
        }
        for port in &self.ports {
            args.extend(strings(&["--net-connect", &port.to_string()]));
        }
        if self.keychain {
            args.push("--keychain".to_string());
        }
        if self.preferences {
            args.push("--preferences".to_string());
        }
        let mut kept: Vec<&String> = names.iter().chain(&self.env).collect();
        kept.sort();
        kept.dedup();
        for name in kept {
            args.extend(strings(&["--env", name]));
        }
        args.push("--".to_string());
        args
    }
}
