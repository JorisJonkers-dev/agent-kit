//! The Pi critic client's readers, at the edges the goldens do not reach. The expected words are the node runner's,
//! from `adapters/pi`.

use crate::clients::CriticOutcome;
use crate::pi_client::{auth_readiness, classify_pi_run, outside_calls, outside_packet, read_pi_stream, PiToolCall};
use model_gateway::json::parse;
use crate::routing::PiCriticClient;
use model_gateway::ports::ProcessResult;

fn config() -> PiCriticClient {
    PiCriticClient { binary: "pi".to_string(), provider: "litellm-vibe-study".to_string(), model: "glm-5-3".to_string(), family: "zhipu".to_string() }
}

fn result(stdout: &str, timed_out: bool) -> ProcessResult {
    ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out }
}

fn failure(outcome: CriticOutcome) -> (String, String) {
    match outcome {
        CriticOutcome::Failed { failure, detail, .. } => (failure, detail),
        CriticOutcome::Ok { .. } => panic!("not a failure"),
    }
}

#[test]
fn an_auth_check_that_could_not_start_or_did_not_answer_says_why() {
    let spawn = auth_readiness(&Err("spawn pi EACCES".to_string()), &config());
    assert_eq!(spawn.detail.as_deref(), Some("pi: auth check for litellm-vibe-study failed: spawn pi EACCES"));
    let slow = auth_readiness(&Ok(result("", true)), &config());
    assert_eq!(slow.detail.as_deref(), Some("pi: auth check for litellm-vibe-study failed: no answer within 20s"));
}

#[test]
fn a_stream_counts_no_tokens_a_message_does_not_report_and_reads_only_text_parts() {
    let stdout = [
        r#"{"type":"message_end","message":{"role":"assistant","content":"plain","usage":{"input":1}}}"#,
        r#"{"type":"message_end","message":{"role":"user","content":[]}}"#,
    ]
    .join("\n");
    let stream = read_pi_stream(&stdout);
    assert_eq!((stream.answer.as_deref(), stream.cost_usd, stream.tokens), (Some(""), None, None));
}

#[test]
fn a_run_the_timer_killed_printed_nothing_or_ended_in_no_assistant_message_is_no_review() {
    let classify = |stdout: &str, timed_out: bool| failure(classify_pi_run(&result(stdout, timed_out), &read_pi_stream(stdout), "/p", 600.0));
    assert_eq!(classify("", true), ("timeout".to_string(), "pi did not return within 600s, so the outer timer killed it".to_string()));
    assert_eq!(classify(" \n", false), ("empty-output".to_string(), "pi exited 0 with empty stdout".to_string()));
    assert_eq!(classify(r#"{"type":"agent_start"}"#, false), ("unparseable-output".to_string(), "stdout is not a Pi event stream with an assistant message".to_string()));
}

#[test]
fn a_path_is_resolved_the_way_pi_resolves_it_before_it_is_judged() {
    let root = "/runs/c1/packet";
    for inside in ["tree/../tree/a.ts", "./a/./b", "@tree/a.ts", "/runs/c1/packet", "/runs/c1/packet/tree/b.ts"] {
        assert!(!outside_packet(inside, root), "{inside}");
    }
    for outside in ["..", "a/../../b", "@/etc/passwd", "~", "/runs/c1/packet/../secret", "/runs/c1/packetx", "/runs", "/../runs/c1/packet/a"] {
        assert!(outside_packet(outside, root), "{outside}");
    }
}

#[test]
fn every_call_that_left_the_packet_is_named_once() {
    let call = |tool: &str, args: &str| PiToolCall { tool: tool.to_string(), args: parse(args).ok().and_then(|value| value.as_object().cloned()) };
    let calls = [call("read", r#"{"path":"~/.ssh/id"}"#), call("read", r#"{"path":"~/.ssh/id"}"#), call("grep", r#"{"pattern":"../x"}"#), call("ls", "5")];
    assert_eq!(outside_calls(&calls, "/p"), vec!["read ~/.ssh/id".to_string()]);
}
