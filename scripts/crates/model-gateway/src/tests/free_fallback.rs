//! Tests for the free fallback's own pieces: what counts as no free copy, when the free copies are back, which
//! address names a free model, and what the data policy and OpenCode's listing say of the copy to fall back to.

use super::machine::{Machine, DEFAULTS};
use crate::allowlist::AllowlistEntry;
use crate::config_parse::load_config;
use crate::free_fallback::{allows, available_after, free_group, plan, refusal_in};
use crate::json::parse;
use crate::policy::DataScope;
use crate::ports::ProcessResult;
use crate::run::{read_run_input, RunInput};
use crate::timestamps::to_iso;

const NOW: i64 = 1_793_786_400_000; // 2026-11-04T10:00:00Z

const LIMITER: &str = "No deployments available for selected model, Try again in a moment: free limits [limiter refusal, no provider was called]: every free deployment of x is limited; the first is back in 3600s";

fn input(client: &str, model: &str, extra: &str) -> RunInput {
    let text = format!(r#"{{"client":"{client}","mode":"read-only","model":"{model}","prompt":"p","workflow":"w"{extra}}}"#);
    read_run_input(&parse(&text).expect("JSON")).expect("a run input")
}

fn offering(models: &[&str]) -> Machine {
    let machine = Machine::new(DEFAULTS);
    machine.process.answer("opencode", Ok(ProcessResult { stdout: models.join("\n"), stderr: String::new(), exit_code: 0, timed_out: false }));
    machine
}

#[test]
fn the_proxys_limiter_refusal_and_its_kin_are_no_free_copy() {
    assert_eq!(refusal_in(Some("exit-1"), &["", LIMITER]).as_deref(), Some(LIMITER));
    for said in ["every free copy is blocked", "all free deployments are limited", "daily limit reached", "the daily quota is spent", "there is no free copy", "the client reported: 402: Free plan limit: 100 requests per UTC day, all used. It resets at 00:00 UTC.", "429: slow down"] {
        assert_eq!(refusal_in(None, &[said]).as_deref(), Some(said));
    }
    assert_eq!(refusal_in(Some("exit-1"), &["boom", "the client crashed"]), None);
    assert_eq!(refusal_in(None, &[]), None);
}

#[test]
fn a_rate_limited_run_is_no_free_copy_whatever_it_says_and_the_refusal_is_cut() {
    assert_eq!(refusal_in(Some("rate-limited"), &["", "429 too many requests"]).as_deref(), Some("429 too many requests"));
    assert_eq!(refusal_in(Some("rate-limited"), &["  ", ""]).as_deref(), Some("rate-limited"));
    assert_eq!(refusal_in(Some("rate-limited"), &[]).as_deref(), Some("rate-limited"));
    let long = format!("daily limit {}", "x".repeat(1000));
    assert_eq!(refusal_in(None, &[&long]).map(|text| text.chars().count()), Some(400));
}

#[test]
fn the_time_the_free_copies_are_back_is_read_from_the_refusal() {
    let at = |seconds: i64| (Some(seconds), Some(to_iso(NOW + seconds * 1000)));
    assert_eq!(available_after(LIMITER, NOW), at(3600));
    assert_eq!(available_after("back in 1h2m3s", NOW), at(3723));
    assert_eq!(available_after("Back in 45s.", NOW), at(45));
    assert_eq!(available_after("429; Retry-After: 120", NOW), at(120));
    assert_eq!(available_after("retry-after 7", NOW), at(7));
    assert_eq!(available_after("Resets in 5m", NOW), at(300));
    assert_eq!(available_after("quota resets at 12:30 UTC", NOW), at(9000));
    // A time already past today is tomorrow's; the very minute is a day away.
    assert_eq!(available_after("resets at 09:00 UTC", NOW), at(23 * 3600));
    assert_eq!(available_after("resets at 10:00 UTC", NOW), at(86_400));
}

#[test]
fn a_refusal_that_names_no_time_names_none() {
    for text in ["no time here", "back in a moment", "back in 0s", "retry-after soon", "resets at 25:00", "resets at 9:00", "resets at 10:7x", "resets at 1", "back in 99999999999999999999s"] {
        assert_eq!(available_after(text, NOW), (None, None), "{text}");
    }
    // A bare number counts as seconds only where a header says so.
    assert_eq!(available_after("back in 30", NOW), (None, None));
}

#[test]
fn a_free_model_is_named_by_its_group_on_pi_or_through_opencodes_litellm_provider() {
    assert_eq!(free_group("pi-free", "free/deepseek-v4.1-flash"), Some("deepseek-v4.1-flash"));
    assert_eq!(free_group("pi-vibe", "free/hy4-flash"), Some("hy4-flash"));
    assert_eq!(free_group("opencode", "litellm-free/free/hy4-flash"), Some("hy4-flash"));
    assert_eq!(free_group("pi-free", "tier-free"), None);
    assert_eq!(free_group("pi-free", "free/"), None);
    assert_eq!(free_group("opencode", "opencode/hy4-flash-free"), None);
    assert_eq!(free_group("claude", "free/x"), None);
}

#[test]
fn the_policy_lets_a_public_free_model_carry_public_content_only() {
    let config = load_config(&Machine::new(DEFAULTS).runtime).expect("config");
    let run = |extra: &str| input("pi-free", "free/x", extra);
    let target = "opencode/x-free";
    let everything = AllowlistEntry { families: vec!["moonshot".to_string()], code: true, history: true };
    let private = DataScope::Repository { key: "github.com/acme/x".to_string(), entry: Some(everything) };
    assert!(allows(&config, &DataScope::Public { key: "github.com/acme/x".to_string() }, &run(r#","repo":"/r""#), target, "moonshot"));
    assert!(allows(&config, &DataScope::Outside, &run(""), target, "moonshot"));
    assert!(!allows(&config, &DataScope::Outside, &run(r#","carries":{"code":true,"history":false}"#), target, "moonshot"));
    assert!(!allows(&config, &private, &run(r#","repo":"/r""#), target, "moonshot"));
    assert!(!allows(&config, &DataScope::Nedap, &run(r#","repo":"/r""#), target, "moonshot"));
    assert!(!allows(&config, &DataScope::NoRemote, &run(r#","repo":"/r""#), target, "moonshot"));
}

#[test]
fn a_free_model_falls_back_to_the_free_copy_opencode_offers_of_the_same_model() {
    let machine = offering(&["opencode/deepseek-v4.1-flash-free", "opencode/hy4-flash-free"]);
    let asked = input("pi-free", "free/deepseek-v4.1-flash", r#","provider":"x""#);
    let (fallen, report) = plan(&machine.runtime, &asked, "run-1", LIMITER).expect("a plan").expect("a fallback");
    assert_eq!((fallen.client.as_str(), fallen.model.as_str(), fallen.provider.as_deref()), ("opencode", "opencode/deepseek-v4.1-flash-free", None));
    assert_eq!(fallen.prompt, asked.prompt);
    assert_eq!(report.from, "pi-free free/deepseek-v4.1-flash");
    assert_eq!(report.to, "opencode opencode/deepseek-v4.1-flash-free");
    assert_eq!(report.reason, LIMITER);
    assert_eq!(report.available_again_in_s, Some(3600));
    assert_eq!(report.available_at.as_deref(), Some(to_iso(crate::tests::machine::now_ms() + 3_600_000).as_str()));
    assert_eq!(
        report.recommendation,
        format!("free/deepseek-v4.1-flash has no free copy until {}; name opencode:opencode/deepseek-v4.1-flash-free (public data) or tier-free for the next calls", report.available_at.clone().expect("a time"))
    );
}

#[test]
fn the_report_says_so_when_the_refusal_names_no_time() {
    let machine = offering(&["opencode/hy4-flash-free"]);
    let (_, report) = plan(&machine.runtime, &input("pi-free", "free/hy4-flash", ""), "run-1", "rate-limited").expect("a plan").expect("a fallback");
    assert_eq!((report.available_again_in_s, report.available_at.clone()), (None, None));
    assert!(report.recommendation.contains("no free copy until an unknown time"));
    assert!(report.notice().contains("the refusal names no time the free copies are back"));
    let event = crate::json::compact(&report.event());
    assert!(event.contains("recommendation") && event.contains("fallback") && !event.contains("available_at"));
}

#[test]
fn the_notice_says_when_the_free_copies_are_back() {
    let machine = offering(&["opencode/hy4-flash-free"]);
    let (_, report) = plan(&machine.runtime, &input("pi-free", "free/hy4-flash", ""), "run-1", LIMITER).expect("a plan").expect("a fallback");
    let notice = report.notice();
    assert!(notice.starts_with("fell back from pi-free free/hy4-flash to opencode opencode/hy4-flash-free: "));
    assert!(notice.contains("free copies are back in 3600s (at "));
    assert!(notice.ends_with(&report.recommendation));
}

#[test]
fn no_fallback_when_nothing_calls_for_one() {
    let machine = offering(&["opencode/hy4-flash-free"]);
    let said = |extra: &str, model: &str| plan(&machine.runtime, &input("pi-free", model, extra), "run-1", LIMITER).expect("a plan");
    // The caller turned it off, or named its client (ADR-0018: a pinned client is never swapped).
    assert!(said(r#","free_fallback":false"#, "free/hy4-flash").is_none());
    assert!(said(r#","client_pinned":true"#, "free/hy4-flash").is_none());
    // A tier names no one model; a paid or direct model has no free group.
    assert!(said("", "tier-free").is_none());
    // OpenCode offers no free copy of this model.
    assert!(said("", "free/deepseek-v4.1-flash").is_none());
}

#[test]
fn no_fallback_when_opencode_cannot_be_listed_is_disabled_or_unknown() {
    let machine = Machine::new(DEFAULTS);
    assert!(plan(&machine.runtime, &input("pi-free", "free/hy4-flash", ""), "run-1", LIMITER).expect("a plan").is_none());
    let machine = offering(&["opencode/hy4-flash-free"]);
    machine.write_override(r#"{"disabled": ["opencode:opencode/hy4-flash-free"]}"#);
    assert!(plan(&machine.runtime, &input("pi-free", "free/hy4-flash", ""), "run-1", LIMITER).expect("a plan").is_none());
}

#[test]
fn no_fallback_when_the_data_policy_keeps_the_content_from_a_public_free_model() {
    let machine = offering(&["opencode/hy4-flash-free"]);
    let carrying = input("pi-free", "free/hy4-flash", r#","carries":{"code":true,"history":false}"#);
    assert!(plan(&machine.runtime, &carrying, "run-1", LIMITER).expect("a plan").is_none());
}

#[test]
fn a_configuration_that_cannot_be_read_is_an_error() {
    let machine = Machine::new("this is not json");
    assert_eq!(plan(&machine.runtime, &input("pi-free", "free/hy4-flash", ""), "run-1", LIMITER).expect_err("no configuration").code, "invalid-config");
}
