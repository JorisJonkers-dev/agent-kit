//! Tests for the real entrypoint and for every line of the lease
//! record, including each parse error path and both clocks.

use super::*;
use crate::lease::*;
use std::sync::Mutex;

mod cli;
mod store;

static RUN_LOCK: Mutex<()> = Mutex::new(());

fn scratch_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("claim-{}", std::process::id()))
}

fn sample_lease() -> Lease {
    Lease {
        device: "dev-1".to_string(),
        run: "run-42".to_string(),
        issue: 813,
        expires: 1_795_793_105,
    }
}

#[test]
fn run_reports_bad_usage_and_removes_its_scratch_directory() {
    let _lock = RUN_LOCK.lock().unwrap();
    let dir = scratch_dir();
    assert_eq!(run(vec![]), 2);
    assert!(!dir.exists());
}

#[test]
fn run_reports_a_missing_repository_and_removes_its_scratch_directory() {
    let _lock = RUN_LOCK.lock().unwrap();
    let dir = scratch_dir();
    let missing = std::env::temp_dir().join(format!("claim-missing-repo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);
    assert_eq!(
        run(vec![
            "list".to_string(),
            missing.to_string_lossy().to_string()
        ]),
        1
    );
    assert!(!dir.exists());
}

#[test]
fn format_and_parse_time_round_trip() {
    for (secs, text) in [
        (0, "1970-01-01T00:00:00Z"),
        (951_782_400, "2000-02-29T00:00:00Z"),
        (1_795_793_105, "2026-11-27T15:25:05Z"),
        (1_709_164_800, "2024-02-29T00:00:00Z"),
        (4_107_542_400, "2100-03-01T00:00:00Z"),
    ] {
        assert_eq!(format_time(secs), text, "formatting {secs}");
        assert_eq!(parse_time(text), Some(secs), "parsing {text}");
    }
}

#[test]
fn parse_time_rejects_garbage_and_non_digits() {
    for bad in [
        "garbage",
        "2026-11-27 15:25:05Z",
        "2026-11-27T15:25:05+00:00",
        "2026-11-27T15:25:05z",
        "202a-11-27T15:25:05Z",
        "2026-11-27T15:2x:05Z",
    ] {
        assert_eq!(parse_time(bad), None, "{bad}");
    }
}

#[test]
fn parse_time_rejects_wrong_length_and_bad_month() {
    assert_eq!(parse_time(""), None);
    assert_eq!(parse_time("2026-11-27T15:25:05"), None);
    assert_eq!(parse_time("2026-11-27T15:25:05Zextra"), None);
    assert_eq!(parse_time("2026-00-27T15:25:05Z"), None);
    assert_eq!(parse_time("2026-13-27T15:25:05Z"), None);
}

#[test]
fn parse_time_rejects_impossible_dates_and_times() {
    for bad in [
        "2026-02-29T00:00:00Z",
        "2100-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-11-00T00:00:00Z",
        "2026-11-27T24:00:00Z",
        "2026-11-27T15:60:00Z",
        "2026-11-27T15:25:60Z",
        "1969-12-31T23:59:59Z",
    ] {
        assert_eq!(parse_time(bad), None, "{bad}");
    }
}

#[test]
fn message_and_parse_round_trip() {
    let lease = sample_lease();
    let message = lease.message();
    assert_eq!(
        message,
        "claim lease\n\ndevice: dev-1\nrun: run-42\nissue: 813\nlease-expires: 2026-11-27T15:25:05Z\n"
    );
    assert_eq!(Lease::parse(&message), Ok(lease.clone()));
    assert_eq!(lease, lease.clone());
    assert!(format!("{lease:?}").contains("dev-1"));
}

#[test]
fn parse_accepts_any_field_order_and_ignores_other_lines() {
    let message =
        "lease-expires: 2000-02-29T00:00:00Z\nrandom note\nissue: 7\n\nrun: r\ndevice: d\n";
    assert_eq!(
        Lease::parse(message),
        Ok(Lease {
            device: "d".to_string(),
            run: "r".to_string(),
            issue: 7,
            expires: 951_782_400,
        })
    );
}

#[test]
fn parse_reports_each_missing_field() {
    assert_eq!(
        Lease::parse("run: r\nissue: 1\nlease-expires: 1970-01-01T00:00:00Z\n"),
        Err("missing device".to_string())
    );
    assert_eq!(
        Lease::parse("device: d\nissue: 1\nlease-expires: 1970-01-01T00:00:00Z\n"),
        Err("missing run".to_string())
    );
    assert_eq!(
        Lease::parse("device: d\nrun: r\nlease-expires: 1970-01-01T00:00:00Z\n"),
        Err("missing issue".to_string())
    );
    assert_eq!(
        Lease::parse("device: d\nrun: r\nissue: 1\n"),
        Err("missing lease-expires".to_string())
    );
}

#[test]
fn parse_reports_malformed_issue_and_time() {
    assert_eq!(
        Lease::parse("device: d\nrun: r\nissue: many\nlease-expires: 1970-01-01T00:00:00Z\n"),
        Err("malformed issue: many".to_string())
    );
    assert_eq!(
        Lease::parse("device: d\nrun: r\nissue: 1\nlease-expires: 2026-02-29T00:00:00Z\n"),
        Err("malformed lease-expires: 2026-02-29T00:00:00Z".to_string())
    );
}

#[test]
fn expires_includes_the_deadline_itself() {
    let lease = sample_lease();
    assert!(!lease.expired(lease.expires - 1));
    assert!(lease.expired(lease.expires));
    assert!(lease.expired(lease.expires + 1));
}

#[test]
fn fixed_clock_returns_its_instant() {
    let clock = FixedClock(1_700_000_001);
    assert_eq!(clock.now(), 1_700_000_001);
}

#[test]
fn system_clock_is_after_2023() {
    assert!(SystemClock.now() > 1_700_000_000);
}

#[test]
fn env_var_reads_the_process_environment() {
    assert!(env_var("PATH").is_some());
    assert_eq!(env_var("CLAIM_SURELY_UNSET_VARIABLE"), None);
}
