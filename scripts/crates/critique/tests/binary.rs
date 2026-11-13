//! The binary itself: the real standard input, the exit code and the two streams.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(args: &[&str], stdin: &str) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_critique"));
    command.args(args).env_clear();
    // A coverage run names where its profile goes; without it the binary would leave one in the working directory.
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start critique");
    child.stdin.take().expect("stdin").write_all(stdin.as_bytes()).expect("write stdin");
    child.wait_with_output().expect("finish")
}

#[test]
fn a_command_reads_its_input_from_standard_input() {
    let output = run(&["allow"], "{\"families\":5}");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "critique allow: allow input: families must be a list\n");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "{\"code\":\"invalid-input\",\"message\":\"allow input: families must be a list\",\"status\":\"error\"}\n");
}

#[test]
fn the_usage_exits_2_on_stderr_and_help_exits_0_on_stdout() {
    let bad = run(&["bogus"], "");
    assert_eq!((bad.status.code(), bad.stdout.len() > 0, bad.stderr.starts_with(b"usage: critique ")), (Some(2), false, true));
    let help = run(&["--help"], "");
    assert_eq!((help.status.code(), help.stderr.len(), help.stdout.starts_with(b"usage: critique ")), (Some(0), 0, true));
}
