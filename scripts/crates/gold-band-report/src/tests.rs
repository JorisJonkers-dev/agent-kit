use super::*;

struct Fixed(u64);

impl Clock for Fixed {
    fn unix_seconds(&self) -> u64 {
        self.0
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gb-report-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn sample(failed: Option<&str>) -> Report {
    Report::build(
        "run-7",
        2,
        "gold-band-run-start check --repo .",
        if failed.is_some() { 1 } else { 0 },
        "hello\n",
        failed,
        &Fixed(1_794_389_400),
    )
}

#[test]
fn sha256_matches_the_nist_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // Two blocks of padding: 56 bytes forces the length into a second block.
    assert_eq!(
        sha256_hex(&[b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn rfc3339_renders_known_instants() {
    assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    assert_eq!(rfc3339(1_794_389_400), "2026-11-11T09:30:00Z");
    assert_eq!(rfc3339(4_102_444_799), "2099-12-31T23:59:59Z");
}

#[test]
fn rfc3339_round_trips_and_takes_a_millisecond_fraction() {
    for secs in [0, 59, 86_399, 951_782_400, 1_794_389_400, 4_102_444_799] {
        assert_eq!(parse_rfc3339(&rfc3339(secs)), Some(secs));
    }
    assert_eq!(
        parse_rfc3339("2026-11-11T09:30:00.123Z"),
        Some(1_794_389_400)
    );
}

#[test]
fn rfc3339_refuses_what_is_not_utc_or_not_a_date() {
    for bad in [
        "",
        "2026-11-11",
        "2026-11-11T09:30:00",
        "2026-11-11T09:30:00+01:00",
        "2026-11-11 09:30:00Z",
        "2026-11-11T09:30:00.Z",
        "2026-11-11T09:30:00.1234Z",
        "2026-11-11T09:30:00.12xZ",
        "2026-13-11T09:30:00Z",
        "2026-00-11T09:30:00Z",
        "2026-02-30T09:30:00Z",
        "2025-02-29T09:30:00Z",
        "2026-11-11T24:30:00Z",
        "2026-11-11T09:60:00Z",
        "2026-11-11T09:30:60Z",
        "1969-12-31T23:59:59Z",
        "20x6-11-11T09:30:00Z",
    ] {
        assert_eq!(parse_rfc3339(bad), None, "{bad}");
    }
}

#[test]
fn build_hashes_the_output_and_stamps_the_clock() {
    let report = sample(None);
    assert_eq!(report.output_hash, sha256_hex(b"hello\n"));
    assert_eq!(report.timestamp, "2026-11-11T09:30:00Z");
    assert_eq!(report.failed_check, None);
}

#[test]
fn json_has_a_stable_key_order_and_names_a_failed_check_only_when_refused() {
    let passed = sample(None).to_json();
    assert!(passed.starts_with(
        "{\"run_id\":\"run-7\",\"attempt\":2,\"command\":\"gold-band-run-start check --repo .\",\"exit_code\":0,\"output_hash\":\""
    ));
    assert!(passed.ends_with("\",\"timestamp\":\"2026-11-11T09:30:00Z\"}"));
    assert!(!passed.contains("failed_check"));
    let refused = sample(Some("grant")).to_json();
    assert!(refused.ends_with("\"timestamp\":\"2026-11-11T09:30:00Z\",\"failed_check\":\"grant\"}"));
}

#[test]
fn json_escapes_what_a_command_line_can_hold() {
    let mut report = sample(None);
    report.command = "say \"hi\"\n\\".to_string();
    assert_eq!(Report::from_json(&report.to_json()).unwrap(), report);
}

#[test]
fn write_then_read_returns_the_same_report_and_leaves_no_temp_file() {
    let dir = scratch("roundtrip").join("nested");
    for failed in [None, Some("family-allowlist")] {
        let report = sample(failed);
        let path = write(&dir, "run-start.json", &report).unwrap();
        assert_eq!(path, dir.join("run-start.json"));
        assert_eq!(read(&path).unwrap(), report);
    }
    let names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["run-start.json"]);
}

#[test]
fn write_fails_when_the_directory_is_a_file_or_the_target_is_a_directory() {
    let base = scratch("badwrite");
    fs::create_dir_all(&base).unwrap();
    let file = base.join("file");
    fs::write(&file, "x").unwrap();
    assert!(write(&file, "r.json", &sample(None)).is_err());
    fs::create_dir(base.join("r.json")).unwrap();
    assert!(write(&base, "r.json", &sample(None)).is_err());
    let names: Vec<_> = fs::read_dir(&base)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2, "no temp file left behind: {names:?}");
}

#[test]
fn read_names_the_file_when_it_is_missing_or_invalid() {
    let base = scratch("badread");
    fs::create_dir_all(&base).unwrap();
    let missing = read(&base.join("none.json")).unwrap_err();
    assert!(missing.contains("none.json: cannot read"), "{missing}");
    let bad = base.join("bad.json");
    fs::write(&bad, "[]").unwrap();
    assert!(read(&bad).unwrap_err().contains("bad.json:"));
}

#[test]
fn from_json_refuses_each_bad_field() {
    let good = sample(Some("grant")).to_json();
    assert!(Report::from_json("nope")
        .unwrap_err()
        .starts_with("not JSON"));
    let cases = [
        ("\"run_id\":\"run-7\"", "\"run_id\":7", "run_id"),
        ("\"command\":", "\"cmd\":", "command"),
        ("\"attempt\":2", "\"attempt\":-1", "attempt"),
        (
            "\"attempt\":2",
            "\"attempt\":\"2\"",
            "attempt must be an integer",
        ),
        ("\"exit_code\":1", "\"exit_code\":9999999999", "exit_code"),
        (
            "\"timestamp\":\"2026-11-11T09:30:00Z\"",
            "\"timestamp\":\"yesterday\"",
            "timestamp",
        ),
        (
            "\"failed_check\":\"grant\"",
            "\"failed_check\":3",
            "failed_check",
        ),
    ];
    for (from, to, expect) in cases {
        assert!(good.contains(from), "{from}");
        let err = Report::from_json(&good.replace(from, to)).unwrap_err();
        assert!(err.contains(expect), "{expect}: {err}");
    }
    let short = good.replace(&sample(None).output_hash, "abc");
    assert!(Report::from_json(&short)
        .unwrap_err()
        .contains("output_hash"));
    let upper = good.replace(&sample(None).output_hash, &"A".repeat(64));
    assert!(Report::from_json(&upper)
        .unwrap_err()
        .contains("output_hash"));
}

#[test]
fn the_system_clock_is_after_this_ticket_was_written() {
    assert!(SystemClock.unix_seconds() > 1_794_000_000);
    assert!(parse_rfc3339(&rfc3339(SystemClock.unix_seconds())).is_some());
}
