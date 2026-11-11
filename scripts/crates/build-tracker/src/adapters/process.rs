use crate::ports::{Process, ProcessOutput};
use std::process::{Command, Stdio};

/// Runs real programs. `env` is set on every program it starts, on top of the environment it inherits.
pub struct SystemProcess {
    pub env: Vec<(String, String)>,
}

impl Process for SystemProcess {
    fn exec(&self, command: &str, args: &[String]) -> Result<ProcessOutput, String> {
        let mut process = Command::new(command);
        process.args(args).stdin(Stdio::null());
        process.envs(self.env.iter().map(|(key, value)| (key, value)));
        let output = process.output().map_err(|error| {
            let reason = if error.kind() == std::io::ErrorKind::NotFound { "ENOENT".to_string() } else { error.to_string() };
            format!("cannot run {command}: spawn {command} {reason}")
        })?;
        Ok(ProcessOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    }
}
