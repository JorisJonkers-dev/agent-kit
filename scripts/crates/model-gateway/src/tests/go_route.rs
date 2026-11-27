//! Tests for OpenCode's direct Go models, which run through LiteLLM's copy of the same model.

use super::machine::{Machine, DEFAULTS};
use crate::go_route::{litellm_name, opencode_offers, reroute, Rewrite};
use crate::json::parse;
use crate::ports::ProcessResult;
use crate::run::{read_run_input, RunInput};

fn input(client: &str, model: &str) -> RunInput {
    let text = format!(r#"{{"client":"{client}","mode":"read-only","model":"{model}","prompt":"p","workflow":"w"}}"#);
    read_run_input(&parse(&text).expect("JSON")).expect("a run input")
}

fn offering(listing: Result<&str, &str>) -> Machine {
    let machine = Machine::new(DEFAULTS);
    let answer = match listing {
        Ok(stdout) => Ok(ProcessResult { stdout: stdout.to_string(), stderr: String::new(), exit_code: 0, timed_out: false }),
        Err(message) => Err(message.to_string()),
    };
    machine.process.answer("opencode", answer);
    machine
}

#[test]
fn a_direct_go_model_has_a_litellm_name() {
    assert_eq!(litellm_name("opencode-go/deepseek-v4.1-flash").as_deref(), Some("litellm-opencode-go/deepseek-v4.1-flash"));
    assert_eq!(litellm_name("opencode-go/"), None);
    assert_eq!(litellm_name("litellm-opencode-go/x"), None);
    assert_eq!(litellm_name("opencode/x-free"), None);
}

#[test]
fn opencode_offers_what_its_own_listing_names() {
    let machine = offering(Ok("opencode-go/a\nlitellm-opencode-go/a\n"));
    assert!(opencode_offers(&machine.runtime, "litellm-opencode-go/a"));
    assert!(!opencode_offers(&machine.runtime, "litellm-opencode-go/b"));
    assert!(!opencode_offers(&offering(Err("not installed")).runtime, "litellm-opencode-go/a"));
}

#[test]
fn a_direct_go_model_with_a_litellm_copy_runs_on_the_copy_and_says_so() {
    let machine = offering(Ok("opencode-go/a\nlitellm-opencode-go/a\n"));
    let (routed, rewrite) = reroute(&machine.runtime, &input("opencode", "opencode-go/a")).expect("routed").expect("a rewrite");
    assert_eq!(routed.model, "litellm-opencode-go/a");
    assert_eq!(routed.client, "opencode");
    assert_eq!(rewrite, Rewrite { from: "opencode-go/a".to_string(), to: "litellm-opencode-go/a".to_string() });
    let said = crate::json::compact(&rewrite.to_json());
    assert!(said.contains(r#""from":"opencode-go/a""#) && said.contains(r#""to":"litellm-opencode-go/a""#) && said.contains("names no LiteLLM proxy address"));
}

#[test]
fn a_direct_go_model_with_no_litellm_copy_is_refused_naming_the_litellm_model() {
    let machine = offering(Ok("opencode-go/a\n"));
    let refused = reroute(&machine.runtime, &input("opencode", "opencode-go/a")).expect_err("no copy");
    assert_eq!(refused.code, "provider-not-allowed");
    assert!(refused.message.starts_with("opencode-go/a: OpenCode's own opencode-go provider names no LiteLLM proxy address"));
    assert!(refused.message.contains("lists no litellm-opencode-go/a; name litellm-opencode-go/a once LiteLLM serves it"));
}

#[test]
fn any_other_run_is_left_as_it_is() {
    let machine = offering(Ok("litellm-opencode-go/a\n"));
    for (client, model) in [("opencode", "litellm-opencode-go/a"), ("opencode", "opencode/x-free"), ("claude", "opencode-go/a")] {
        assert_eq!(reroute(&machine.runtime, &input(client, model)), Ok(None));
    }
}
