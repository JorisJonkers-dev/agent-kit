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

/// The search server answers each request while its client keeps standard input open, as Vibe's MCP client does.
#[test]
fn the_search_server_answers_a_line_before_its_input_closes() {
    use std::io::{BufRead, BufReader};
    let root = std::env::temp_dir().join(format!("critique-binary-search-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("packet");
    let mut command = Command::new(env!("CARGO_BIN_EXE_critique"));
    command.args(["search-server", "--root", &root.to_string_lossy()]).env_clear();
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("start the server");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut replies = BufReader::new(child.stdout.take().expect("stdout"));
    stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n").expect("write");
    stdin.flush().expect("flush");
    let mut line = String::new();
    replies.read_line(&mut line).expect("read the reply");
    assert_eq!(line, "{\"id\":1,\"jsonrpc\":\"2.0\",\"result\":{}}\n");
    drop(stdin);
    assert_eq!(child.wait().expect("finish").code(), Some(0));
    let _ = std::fs::remove_dir_all(&root);
}

/// The real binary serves the reach proof against a real packet with real symlinks out of it, and the proof finds no way out.
#[test]
fn the_real_search_server_passes_the_reach_proof() {
    use critique::packet_search::{default_search_limits, reach_problems, reach_proof_calls};
    use critique::search_client::{prove_search_reach, session, SearchCommand, SearchSession};
    let dir = std::env::temp_dir().join(format!("critique-binary-reach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut runtime = critique::runtime::default_runtime();
    let own = SearchCommand { command: env!("CARGO_BIN_EXE_critique").to_string(), args: Vec::new() };
    runtime.search = Some(own.clone());
    let root = dir.to_string_lossy().into_owned();
    assert_eq!(prove_search_reach(&runtime, &root, &default_search_limits()), Ok(None));
    // The same session by hand: the packet's own file is found, and every way out answers nothing.
    let packet = format!("{root}/packet");
    let SearchSession::Started { tools, answers } = session(&runtime, &own, &packet, &default_search_limits(), &reach_proof_calls(&format!("{root}/outside"))) else {
        panic!("the real server starts");
    };
    // Without a REPO-MAP in the packet the server offers no symbol search.
    assert_eq!(tools, [critique::packet_search::SearchToolName::Grep, critique::packet_search::SearchToolName::List]);
    assert!(answers[0].contains("inside.md"), "{answers:?}");
    assert_eq!(reach_problems(&answers), Vec::<String>::new());
    assert!(answers[1..].iter().all(|answer| !answer.contains("OUTSIDE-THE-PACKET")));
    let _ = std::fs::remove_dir_all(&dir);
}
