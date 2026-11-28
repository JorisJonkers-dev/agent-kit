//! Tests for pinned model addresses, `<harness>:<pricing>/<model>`: which harness and model they come to, what
//! is refused and why, and that a pinned run is the one the gateway never swaps.

use crate::address::{is_address, is_proxy_address, resolve, Addressed};
use crate::json::parse;
use crate::run::read_run_input;

fn addressed(client: &str, model: &str) -> Addressed {
    Addressed { client: client.to_string(), model: model.to_string() }
}

#[test]
fn an_address_names_the_harness_and_leaves_the_proxys_address_as_it_is() {
    assert_eq!(resolve("pi:free/glm-4.7-flash"), Ok(addressed("pi-free", "free/glm-4.7-flash")));
    assert_eq!(resolve("pi-free:free/hy4-flash"), Ok(addressed("pi-free", "free/hy4-flash")));
    assert_eq!(resolve("pi:paid/deepseek-v4.1-flash"), Ok(addressed("pi-paid", "paid/deepseek-v4.1-flash")));
    assert_eq!(resolve("pi:auto/glm-5.3-flash"), Ok(addressed("pi-auto", "auto/glm-5.3-flash")));
    assert_eq!(resolve("opencode:paid/deepseek-v4.1-flash"), Ok(addressed("opencode", "litellm-paid/paid/deepseek-v4.1-flash")));
    assert_eq!(resolve("opencode:free/a/b"), Ok(addressed("opencode", "litellm-free/free/a/b")));
}

#[test]
fn an_unknown_pricing_prefix_is_refused_with_the_ones_there_are() {
    for address in ["pi:cheap/glm", "opencode:glm/x", "pi-free:/x"] {
        let error = resolve(address).expect_err(address);
        assert_eq!(error.code, "unknown-pricing", "{address}");
        assert!(error.message.contains("free, paid, auto"), "{}", error.message);
    }
    let bare = resolve("pi:glm-5.3-flash").expect_err("no pricing prefix");
    assert_eq!(bare.code, "unknown-pricing");
    assert!(bare.message.contains("names no pricing prefix"));
}

#[test]
fn an_unknown_harness_or_one_that_does_not_serve_the_pricing_is_refused() {
    let unknown = resolve("claude:free/x").expect_err("claude takes no proxy address");
    assert_eq!(unknown.code, "unknown-harness");
    assert!(unknown.message.contains("\"claude\"") && unknown.message.contains("pi, pi-<route>"), "{}", unknown.message);
    assert_eq!(resolve("nosuch:free/x").expect_err("unknown").code, "unknown-harness");
    let mismatch = resolve("pi-vibe:free/x").expect_err("pi-vibe serves no free chain");
    assert_eq!(mismatch.code, "unknown-harness");
    assert!(mismatch.message.contains("pin pi:free/x, or pi-free:free/x"), "{}", mismatch.message);
}

#[test]
fn an_address_without_a_model_or_a_colon_is_invalid_input() {
    assert_eq!(resolve("pi:free/").expect_err("no model").code, "invalid-input");
    assert_eq!(resolve("free/x").expect_err("no harness").code, "invalid-input");
}

#[test]
fn only_a_harness_a_colon_and_a_slash_make_an_address() {
    assert!(is_address("pi:free/x"));
    assert!(!is_address("qwen3:32b"));
    assert!(!is_address("free/x:y"));
    assert!(!is_address(":free/x"));
    assert!(!is_address("claude-sonnet-5-5"));
}

fn read(extra: &str) -> Result<crate::run::RunInput, crate::error::GatewayError> {
    read_run_input(&parse(&format!(r#"{{"mode":"read-only","prompt":"p","workflow":"w"{extra}}}"#)).expect("JSON"))
}

#[test]
fn a_run_that_names_an_address_runs_on_that_harness_and_is_pinned() {
    let input = read(r#","model":"pi-free:free/hy4-flash""#).expect("an input");
    assert_eq!((input.client.as_str(), input.model.as_str(), input.client_pinned), ("pi-free", "free/hy4-flash", Some(true)));
    // The caller's own flag wins; a client named beside a model keeps both as written and pins nothing.
    assert_eq!(read(r#","model":"pi:free/x","client_pinned":false"#).expect("an input").client_pinned, Some(false));
    let named = read(r#","client":"pi-free","model":"free/hy4-flash""#).expect("an input");
    assert_eq!((named.client.as_str(), named.model.as_str(), named.client_pinned), ("pi-free", "free/hy4-flash", None));
    let tagged = read(r#","client":"opencode","model":"qwen3:32b/x""#).expect("an input");
    assert_eq!(tagged.model, "qwen3:32b/x");
}

#[test]
fn a_run_with_a_bad_address_is_refused_with_its_reason() {
    assert_eq!(read(r#","model":"nosuch:free/x""#).expect_err("refused").code, "unknown-harness");
    assert_eq!(read(r#","model":"pi:cheap/x""#).expect_err("refused").code, "unknown-pricing");
    // No client and no address: the run still says which clients can run.
    assert_eq!(read(r#","model":"glm""#).expect_err("refused").code, "not-supported-yet");
}

#[test]
fn the_proxys_own_address_is_a_pricing_prefix_and_a_model() {
    assert!(is_proxy_address("free/hy4-flash") && is_proxy_address("paid/a/b") && is_proxy_address("auto/x"));
    assert!(!is_proxy_address("free/") && !is_proxy_address("tier-free") && !is_proxy_address("cheap/x") && !is_proxy_address("ministral-8b"));
}

#[test]
fn a_pinned_paid_address_names_a_provider_opencode_may_use_and_a_free_one_stays_refused() {
    let opencode = resolve("opencode:paid/deepseek-v4.1-flash").expect("an address");
    assert_eq!(crate::run::require_allowed_provider(&opencode.model, None), Ok(()));
    // The free chain is for public work only, which the data policy of a Pi route enforces and OpenCode's provider list does not.
    let free = resolve("opencode:free/hy4-flash").expect("an address");
    assert_eq!(crate::run::require_allowed_provider(&free.model, None).expect_err("refused").code, "provider-not-allowed");
}
