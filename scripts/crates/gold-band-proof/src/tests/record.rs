use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::record::{
    now_utc, read_record, record_path, timestamp_utc, write_record, Outcome, Record, ScenarioResult,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let tick = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "gold-band-proof-record-{}-{}-{}",
        std::process::id(),
        tick,
        label
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn scenario(name: &str, expected_exit: i32, exit_code: Option<i32>) -> ScenarioResult {
    ScenarioResult {
        name: name.to_string(),
        expected_exit,
        exit_code,
        run_status: Some("completed".to_string()),
        run_outcome: Some("success".to_string()),
        note: None,
    }
}

fn record(outcome: Outcome, with_optional: bool) -> Record {
    Record {
        workflow_id: "release".to_string(),
        workflow_sha256: "a1b2c3".to_string(),
        gold_band_commit: if with_optional {
            Some("9f8e7d".to_string())
        } else {
            None
        },
        outcome,
        scenarios: vec![scenario("happy path", 0, Some(0))],
        skip_reason: if with_optional {
            Some("gold-band is not installed".to_string())
        } else {
            None
        },
        recorded_at: "2026-11-11T16:53:26Z".to_string(),
    }
}

#[test]
fn outcome_spellings_round_trip() {
    assert_eq!(Outcome::Passed.as_str(), "passed");
    assert_eq!(Outcome::Failed.as_str(), "failed");
    assert_eq!(Outcome::Skipped.as_str(), "skipped");
    assert_eq!(Outcome::parse("passed"), Some(Outcome::Passed));
    assert_eq!(Outcome::parse("failed"), Some(Outcome::Failed));
    assert_eq!(Outcome::parse("skipped"), Some(Outcome::Skipped));
    assert_eq!(Outcome::parse("Passed"), None);
    assert_eq!(Outcome::parse(""), None);
}

#[test]
fn a_scenario_passes_only_on_its_expected_exit() {
    assert!(scenario("happy", 0, Some(0)).passed());
    assert!(!scenario("happy", 0, Some(1)).passed());
    assert!(!scenario("happy", 0, None).passed());
    assert!(scenario("paused", 2, Some(2)).passed());
}

#[test]
fn record_path_replaces_characters_outside_the_safe_set() {
    let dir = temp_dir("path");
    assert_eq!(
        record_path(&dir, "team/release: v1"),
        dir.join("team_release__v1.proof.json")
    );
    assert_eq!(
        record_path(&dir, "a.b_c-d9"),
        dir.join("a.b_c-d9.proof.json")
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn to_json_writes_the_keys_in_order_with_a_trailing_newline() {
    let text = record(Outcome::Passed, true).to_json();
    assert!(text.ends_with("}\n"));
    let keys = [
        "\"workflow_id\"",
        "\"workflow_sha256\"",
        "\"gold_band_commit\"",
        "\"outcome\"",
        "\"scenarios\"",
        "\"skip_reason\"",
        "\"recorded_at\"",
    ];
    let mut previous = 0;
    for key in keys {
        let at = text.find(key).expect("key present");
        assert!(at >= previous, "key {} is out of order", key);
        previous = at;
    }
    assert!(text.contains("\"gold_band_commit\": \"9f8e7d\""));
    assert!(text.contains("\"exit_code\": 0"));
    assert!(text.contains("\"note\": null"));
}

#[test]
fn to_json_writes_an_empty_scenario_list_compactly() {
    let mut empty = record(Outcome::Skipped, false);
    empty.scenarios.clear();
    let text = empty.to_json();
    assert!(text.contains("\"scenarios\": [],"));
    assert!(text.contains("\"gold_band_commit\": null"));
    assert!(text.contains("\"skip_reason\": null"));
}

#[test]
fn every_outcome_round_trips_with_and_without_the_optional_fields() {
    for outcome in [Outcome::Passed, Outcome::Failed, Outcome::Skipped] {
        for with_optional in [true, false] {
            let original = record(outcome, with_optional);
            let text = original.to_json();
            let parsed = Record::from_json(&text).expect("record parses");
            assert_eq!(parsed, original);
        }
    }
}

#[test]
fn a_scenario_without_optional_fields_round_trips() {
    let mut original = record(Outcome::Failed, false);
    original.scenarios = vec![ScenarioResult {
        name: "paused run".to_string(),
        expected_exit: 2,
        exit_code: None,
        run_status: None,
        run_outcome: None,
        note: Some("gold-band was killed by a signal".to_string()),
    }];
    let text = original.to_json();
    assert!(text.contains("\"exit_code\": null"));
    assert!(text.contains("\"run_status\": null"));
    assert!(text.contains("\"run_outcome\": null"));
    assert_eq!(Record::from_json(&text).expect("record parses"), original);
}

#[test]
fn absent_optional_fields_parse_as_none() {
    let text = "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\
                \"scenarios\":[{\"name\":\"happy\",\"expected_exit\":0}],\
                \"recorded_at\":\"2026-11-11T16:53:26Z\"}";
    let parsed = Record::from_json(text).expect("record parses");
    assert_eq!(parsed.gold_band_commit, None);
    assert_eq!(parsed.skip_reason, None);
    assert_eq!(parsed.scenarios[0].exit_code, None);
    assert_eq!(parsed.scenarios[0].run_status, None);
    assert_eq!(parsed.scenarios[0].run_outcome, None);
    assert_eq!(parsed.scenarios[0].note, None);
}

fn expect_error(text: &str, expected: &str) {
    match Record::from_json(text) {
        Ok(parsed) => panic!("expected an error, got {:?}", parsed),
        Err(message) => assert!(
            message.contains(expected),
            "message {:?} does not mention {:?}",
            message,
            expected
        ),
    }
}

#[test]
fn from_json_rejects_text_that_is_not_a_record() {
    expect_error("not json at all", "not valid JSON");
    expect_error("[]", "not a JSON object");
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"maybe\",\
         \"scenarios\":[],\"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "unknown outcome",
    );
}

#[test]
fn from_json_names_the_missing_or_mistyped_field() {
    expect_error(
        "{\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\"scenarios\":[],\
         \"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "`workflow_id`",
    );
    expect_error(
        "{\"workflow_id\":7,\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\"scenarios\":[],\
         \"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "`workflow_id`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"outcome\":\"passed\",\"scenarios\":[],\
         \"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "`workflow_sha256`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":[],\"outcome\":\"passed\",\"scenarios\":[],\
         \"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "`workflow_sha256`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"scenarios\":[],\
         \"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "`outcome`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\
         \"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "`scenarios`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\
         \"scenarios\":{},\"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "`scenarios`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\
         \"scenarios\":[3],\"recorded_at\":\"2026-11-11T16:53:26Z\"}",
        "not an object",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\
         \"scenarios\":[],\"recorded_at\":\"2026-11-11T16:53:26Z\",\"gold_band_commit\":1}",
        "`gold_band_commit`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\
         \"scenarios\":[],\"recorded_at\":\"2026-11-11T16:53:26Z\",\"skip_reason\":[]}",
        "`skip_reason`",
    );
    expect_error(
        "{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\"scenarios\":[]}",
        "`recorded_at`",
    );
}

#[test]
fn from_json_names_the_missing_or_mistyped_scenario_field() {
    let wrap = |scenario: &str| {
        format!(
            "{{\"workflow_id\":\"wf\",\"workflow_sha256\":\"abc\",\"outcome\":\"passed\",\
             \"scenarios\":[{}],\"recorded_at\":\"2026-11-11T16:53:26Z\"}}",
            scenario
        )
    };
    expect_error(&wrap("{\"expected_exit\":0}"), "`name`");
    expect_error(&wrap("{\"name\":1,\"expected_exit\":0}"), "`name`");
    expect_error(&wrap("{\"name\":\"happy\"}"), "`expected_exit`");
    expect_error(
        &wrap("{\"name\":\"happy\",\"expected_exit\":\"0\"}"),
        "`expected_exit`",
    );
    expect_error(
        &wrap("{\"name\":\"happy\",\"expected_exit\":1.5}"),
        "`expected_exit`",
    );
    expect_error(
        &wrap("{\"name\":\"happy\",\"expected_exit\":99999999999}"),
        "out of range",
    );
    expect_error(
        &wrap("{\"name\":\"happy\",\"expected_exit\":0,\"exit_code\":\"zero\"}"),
        "`exit_code`",
    );
    expect_error(
        &wrap("{\"name\":\"happy\",\"expected_exit\":0,\"run_status\":true}"),
        "`run_status`",
    );
    expect_error(
        &wrap("{\"name\":\"happy\",\"expected_exit\":0,\"run_outcome\":7}"),
        "`run_outcome`",
    );
    expect_error(
        &wrap("{\"name\":\"happy\",\"expected_exit\":0,\"note\":[]}"),
        "`note`",
    );
}

#[test]
fn write_then_read_returns_the_record() {
    let dir = temp_dir("roundtrip");
    let original = record(Outcome::Passed, true);
    let path = write_record(&dir, &original).expect("record written");
    assert_eq!(path, record_path(&dir, "release"));
    assert!(path.is_file());
    assert_eq!(
        read_record(&dir, "release").expect("record read"),
        Some(original)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_record_creates_the_directory() {
    let dir = temp_dir("nested");
    let nested = dir.join("proofs").join("release");
    let original = record(Outcome::Passed, false);
    let path = write_record(&nested, &original).expect("record written");
    assert!(path.is_file());
    assert_eq!(
        read_record(&nested, "release").expect("record read"),
        Some(original)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_record_replaces_an_earlier_record() {
    let dir = temp_dir("replace");
    write_record(&dir, &record(Outcome::Failed, false)).expect("first record written");
    let second = record(Outcome::Passed, true);
    write_record(&dir, &second).expect("second record written");
    assert_eq!(
        read_record(&dir, "release").expect("record read"),
        Some(second)
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_record_is_none_when_there_is_no_record() {
    let dir = temp_dir("missing");
    assert_eq!(read_record(&dir, "release").expect("no record"), None);
    assert_eq!(
        read_record(&dir.join("absent"), "release").expect("no record"),
        None
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_record_fails_when_the_directory_is_a_file() {
    let dir = temp_dir("dirfile");
    let blocker = dir.join("blocker");
    fs::write(&blocker, b"not a directory").expect("blocker written");
    let message = write_record(&blocker, &record(Outcome::Passed, false)).expect_err("refused");
    assert!(
        message.contains("cannot create the proof directory"),
        "{}",
        message
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn write_record_fails_when_the_target_is_a_directory() {
    let dir = temp_dir("targetdir");
    let target = record_path(&dir, "release");
    fs::create_dir_all(&target).expect("target directory");
    let message = write_record(&dir, &record(Outcome::Passed, false)).expect_err("refused");
    assert!(
        message.contains("cannot write the proof record"),
        "{}",
        message
    );
    assert!(message.contains("release.proof.json"), "{}", message);
    let entries: Vec<_> = fs::read_dir(&dir)
        .expect("dir read")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(entries.len(), 1, "the temporary file is cleaned up");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_record_fails_when_the_path_is_a_directory() {
    let dir = temp_dir("readdir");
    let target = record_path(&dir, "release");
    fs::create_dir_all(&target).expect("target directory");
    let message = read_record(&dir, "release").expect_err("refused");
    assert!(
        message.contains("cannot read the proof record"),
        "{}",
        message
    );
    assert!(message.contains("release.proof.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_record_fails_when_the_file_is_corrupt() {
    let dir = temp_dir("corrupt");
    let target = record_path(&dir, "release");
    fs::write(&target, b"{\"workflow_id\":").expect("corrupt record written");
    let message = read_record(&dir, "release").expect_err("refused");
    assert!(
        message.contains("cannot parse the proof record"),
        "{}",
        message
    );
    assert!(message.contains("release.proof.json"), "{}", message);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn timestamp_utc_converts_the_epoch_and_leap_days() {
    assert_eq!(timestamp_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(timestamp_utc(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(timestamp_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    assert_eq!(timestamp_utc(4_107_456_000), "2100-02-28T00:00:00Z");
    assert_eq!(timestamp_utc(4_107_542_400), "2100-03-01T00:00:00Z");
}

#[test]
fn timestamp_utc_converts_the_year_end_and_a_known_instant() {
    assert_eq!(timestamp_utc(4_102_444_799), "2099-12-31T23:59:59Z");
    assert_eq!(timestamp_utc(4_102_444_800), "2100-01-01T00:00:00Z");
    assert_eq!(timestamp_utc(1_798_761_599), "2026-12-31T23:59:59Z");
    assert_eq!(timestamp_utc(1_794_412_406), "2026-11-11T15:53:26Z");
    assert_eq!(timestamp_utc(1_767_225_600), "2026-01-01T00:00:00Z");
}

#[test]
fn now_utc_is_an_rfc_3339_utc_timestamp() {
    let now = now_utc();
    assert_eq!(now.len(), 20, "{}", now);
    assert!(now.ends_with('Z'), "{}", now);
    assert_eq!(&now[4..5], "-");
    assert_eq!(&now[7..8], "-");
    assert_eq!(&now[10..11], "T");
    assert_eq!(&now[13..14], ":");
    assert_eq!(&now[16..17], ":");
    assert!(now.as_str() > "2020-01-01T00:00:00Z", "{}", now);
}
