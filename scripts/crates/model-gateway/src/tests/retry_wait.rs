//! Tests for the wait a run takes when the proxy says a free model is back in N seconds: the settings, the bounds
//! of the waits, and the report a retry carries. The clock is fake, so nothing really waits.

use crate::json::parse;
use crate::ports::Clock;
use crate::retry_wait::{merge, RetryWaitSettings, Waiter};

const NOW: i64 = 1_793_786_400_000;
const BACK_IN_5: &str = "429: free limits [limiter refusal, no provider was called]: every free deployment of x is limited; the first is back in 5s";

fn settings(text: &str) -> Result<RetryWaitSettings, crate::error::GatewayError> {
    merge(&RetryWaitSettings::default(), Some(&parse(text).expect("JSON")))
}

#[test]
fn the_defaults_wait_a_minute_in_three_retries_with_a_second_of_margin() {
    let defaults = RetryWaitSettings::default();
    assert_eq!((defaults.max_retries, defaults.max_total_wait_s, defaults.margin_s), (3, 60.0, 1.0));
    assert_eq!(merge(&defaults, None).expect("settings"), defaults);
}

#[test]
fn a_machine_may_state_any_of_the_settings_and_the_rest_stay() {
    let merged = settings(r#"{"max_retries":5,"margin_s":0.5}"#).expect("settings");
    assert_eq!((merged.max_retries, merged.max_total_wait_s, merged.margin_s), (5, 60.0, 0.5));
    assert_eq!(settings(r#"{"max_total_wait_s":0}"#).expect("settings").max_total_wait_s, 0.0);
}

#[test]
fn a_setting_that_is_not_one_or_not_a_number_in_range_is_refused() {
    for (text, said) in [
        ("[]", "must be an object"),
        (r#"{"waits":1}"#, "retry_wait.waits is not a setting"),
        (r#"{"max_retries":-1}"#, "max_retries must be a number from 0"),
        (r#"{"margin_s":"2"}"#, "margin_s must be a number"),
        (r#"{"max_total_wait_s":86401}"#, "max_total_wait_s must be a number"),
        (r#"{"max_retries":1.5}"#, "max_retries must be a whole number"),
    ] {
        let error = settings(text).expect_err(text);
        assert_eq!(error.code, "invalid-config");
        assert!(error.message.contains(said), "{text}: {}", error.message);
    }
}

#[test]
fn a_waiter_takes_the_named_time_plus_the_margin_until_the_retries_or_the_cap_run_out() {
    let mut waiter = Waiter::new(RetryWaitSettings { max_retries: 2, max_total_wait_s: 20.0, margin_s: 2.0 });
    let first = waiter.next("r1", BACK_IN_5, NOW).expect("a wait");
    assert_eq!(first.sleep_ms, 7000);
    assert_eq!((first.report.retry, first.report.of_run_id.as_str(), first.report.back_in_s, first.report.waited_s, first.report.total_waited_s), (1, "r1", 5, 7.0, 7.0));
    assert_eq!(first.report.reason, BACK_IN_5);
    let second = waiter.next("r2", BACK_IN_5, NOW).expect("a wait");
    assert_eq!(second.report.total_waited_s, 14.0);
    // The third would be a third retry of two.
    assert!(waiter.next("r3", BACK_IN_5, NOW).is_none());
    // A wait that does not fit what is left of the cap is not taken either.
    let mut tight = Waiter::new(RetryWaitSettings { max_retries: 9, max_total_wait_s: 10.0, margin_s: 0.0 });
    assert!(tight.next("r1", "back in 8s", NOW).is_some());
    assert!(tight.next("r2", "back in 8s", NOW).is_none());
    assert!(tight.next("r3", "back in 2s", NOW).is_some());
}

#[test]
fn a_refusal_that_names_no_time_is_not_waited_for() {
    let mut waiter = Waiter::new(RetryWaitSettings::default());
    assert!(waiter.next("r1", "429: slow down", NOW).is_none());
    assert!(waiter.next("r1", "back in a moment", NOW).is_none());
}

#[test]
fn a_retry_reports_what_it_waited_for() {
    let report = Waiter::new(RetryWaitSettings::default()).next("r1", BACK_IN_5, NOW).expect("a wait").report;
    let json = report.to_json();
    let object = json.as_object().expect("an object");
    assert_eq!(object.get("retry"), Some(&crate::json::Value::Num(1.0)));
    assert_eq!(object.get("waited_s"), Some(&crate::json::Value::Num(6.0)));
    assert_eq!(object.get("of_run_id").and_then(crate::json::Value::as_str), Some("r1"));
    assert_eq!(report.notice(), "retry 1 of at most 3 after waiting 6s for the free copies the proxy said were back in 5s");
}

#[test]
fn the_system_clock_really_sleeps() {
    let clock = crate::adapters::clock::SystemClock;
    let before = clock.monotonic_ms();
    clock.sleep_ms(5);
    assert!(clock.monotonic_ms() - before >= 4.0);
}
