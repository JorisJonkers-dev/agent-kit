//! The Docker Sandboxes image registry: which tag a refresh picks (the two-day cooldown), the records file, the refresh
//! a launch makes when its record is stale or missing, and the `sandbox refresh` and `sandbox status` answers.

use super::catalog::{cli, sandbox_runtime};
use super::scenarios::{FixedClock, Sandbox};
use crate::json::{self, Value};
use crate::ports::{FileStamp, Files, FsError, Process, ProcessCommand, ProcessResult, Runtime};
use crate::sbx_images::{self, Record, Tag, MAX_AGE_MS};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// 2026-11-27 12:00 UTC: a tag dated the 25th is two days old, the 26th is not.
const NOW: i64 = 1_795_780_800_000;
const HOUR: i64 = 60 * 60 * 1000;
const SHA: &str = "c279c9da843e51b135a5be3750ffcdf8078485b4";
const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const NEWER_DIGEST: &str = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const RECORDS: &str = "/home/me/.local/state/agent-kit/model-gateway/sbx-images.json";
const PI_REFERENCE: &str = "docker.io/sbx/pi-image@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

const HUB: &str = r#"{"count":4,"results":[
 {"name":"latest","digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","last_updated":"2026-11-27T01:00:00Z"},
 {"name":"20261126-c279c9da843e51b135a5be3750ffcdf8078485b4","digest":"sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210","last_updated":"2026-11-26T09:00:00Z"},
 {"name":"20261125-c279c9da843e51b135a5be3750ffcdf8078485b4","digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","last_updated":"2026-11-25T09:00:00Z"},
 {"name":"20261124-c279c9da843e51b135a5be3750ffcdf8078485b4","digest":"sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210","last_updated":"2026-11-24T09:00:00Z"}
]}"#;

const HUB_TOO_NEW: &str = r#"{"results":[{"name":"20261127-c279c9da843e51b135a5be3750ffcdf8078485b4","digest":"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","last_updated":"2026-11-27T09:00:00Z"}]}"#;

fn tag(name: &str, digest: &str, updated: &str) -> Tag {
    Tag { name: name.to_string(), digest: digest.to_string(), updated: updated.to_string() }
}

fn record(tag: &str, digest: &str, checked_at: i64) -> Record {
    Record { tag: tag.to_string(), digest: digest.to_string(), checked_at }
}

fn pi_record(checked_at: i64) -> Record {
    record(&format!("20261125-{SHA}"), DIGEST, checked_at)
}

#[test]
fn the_cutoff_is_two_days_before_today_in_utc_and_the_newest_old_enough_tag_wins() {
    assert_eq!(sbx_images::cutoff(NOW), "20261125");
    assert_eq!(sbx_images::cutoff(NOW - 11 * HOUR - HOUR / 2), "20261125", "a little after midnight is still the 27th");

    let tags = vec![
        tag("latest", DIGEST, "2026-11-27T01:00:00Z"),
        tag(&format!("20261127-{SHA}"), DIGEST, "2026-11-27T09:00:00Z"),
        tag(&format!("20261126-{SHA}"), NEWER_DIGEST, "2026-11-26T09:00:00Z"),
        tag(&format!("20261125-{SHA}"), DIGEST, "2026-11-25T09:00:00Z"),
        tag(&format!("20261124-{SHA}"), NEWER_DIGEST, "2026-11-24T09:00:00Z"),
        // Malformed names and digests never qualify, however new they look.
        tag(&format!("20261125-{}", SHA.to_uppercase()), DIGEST, "2026-11-26T23:00:00Z"),
        tag(&format!("2026112-{SHA}"), DIGEST, "2026-11-26T23:00:00Z"),
        tag(&format!("20261125-{SHA}x"), DIGEST, "2026-11-26T23:00:00Z"),
        tag(&format!("20261125-{SHA}"), "sha256:short", "2026-11-26T23:00:00Z"),
        tag(&format!("20261125-{SHA}"), &DIGEST.to_uppercase().replace("SHA256", "sha256"), "2026-11-26T23:00:00Z"),
    ];
    assert_eq!(sbx_images::choose_tag(&tags, &sbx_images::cutoff(NOW)), Some((format!("20261125-{SHA}"), DIGEST.to_string())));

    // Two tags of one day: the one the registry updated last is the newest.
    let same_day = vec![tag(&format!("20261125-{SHA}"), DIGEST, "2026-11-25T09:00:00Z"), tag(&format!("20261125-{}", "a".repeat(40)), NEWER_DIGEST, "2026-11-25T18:00:00Z")];
    assert_eq!(sbx_images::choose_tag(&same_day, "20261125"), Some((format!("20261125-{}", "a".repeat(40)), NEWER_DIGEST.to_string())));

    // Nothing old enough, or nothing dated at all, chooses nothing.
    assert_eq!(sbx_images::choose_tag(&tags[..2], "20261125"), None);
    assert_eq!(sbx_images::choose_tag(&tags[..1], "20261125"), None);
    assert_eq!(sbx_images::choose_tag(&[], "20261125"), None);
}

#[test]
fn the_registry_answer_is_read_entry_by_entry_and_an_entry_without_a_name_or_digest_is_skipped() {
    let tags = sbx_images::parse_tags(r#"{"results":[{"name":"20261125-x","digest":"sha256:a"},{"digest":"sha256:b"},{"name":"n"},{"name":"d","digest":"sha256:c"}, 7]}"#).expect("parsed");
    assert_eq!(tags.len(), 2, "{tags:?}");
    assert_eq!(tags[0].updated, "", "a tag with no last_updated still lists");
    assert!(sbx_images::parse_tags("not json").expect_err("not JSON").starts_with("the Docker Hub answer does not parse"));
    assert_eq!(sbx_images::parse_tags(r#"{"count":0}"#), Err("the Docker Hub answer has no results list".to_string()));
}

#[test]
fn the_records_file_round_trips_and_a_file_that_is_not_one_is_named_by_what_is_wrong() {
    let store = Store::default();
    assert_eq!(sbx_images::read_records(&store, RECORDS), Ok(BTreeMap::new()), "no file holds no records");

    let mut records = BTreeMap::new();
    records.insert("pi".to_string(), pi_record(NOW));
    records.insert("opencode".to_string(), record(&format!("20261124-{SHA}"), NEWER_DIGEST, NOW - HOUR));
    sbx_images::write_records(&store, RECORDS, &records).expect("written");
    let text = store.text(RECORDS);
    assert!(text.starts_with("{\n  \"opencode\": {"), "clients are the keys: {text}");
    assert_eq!(sbx_images::read_records(&store, RECORDS), Ok(records));

    let unreadable = Store { read_code: Some("EACCES"), ..Store::default() };
    assert_eq!(sbx_images::read_records(&unreadable, RECORDS), Err(format!("EACCES: {RECORDS}")));
    let no_mkdir = Store { refuse_mkdir: true, ..Store::default() };
    assert_eq!(sbx_images::write_records(&no_mkdir, RECORDS, &BTreeMap::new()), Err(format!("EACCES: {}", "/home/me/.local/state/agent-kit/model-gateway")));
    for (text, reason) in [
        ("{", "not JSON"),
        (r#"{"pi":{"tag":"t","checked_at":1}}"#, "pi has no tag, digest and checked_at"),
        ("[]", "not a JSON object"),
        (r#"{"pi":5}"#, "pi has no tag, digest and checked_at"),
        (r#"{"pi":{"tag":"t","digest":"d"}}"#, "pi has no tag, digest and checked_at"),
        (r#"{"pi":{"tag":"t","digest":"d","checked_at":"then"}}"#, "pi has no tag, digest and checked_at"),
        (r#"{"pi":{"tag":1,"digest":"d","checked_at":5}}"#, "pi has no tag, digest and checked_at"),
    ] {
        let store = Store::with(RECORDS, text);
        let error = sbx_images::read_records(&store, RECORDS).expect_err("refused");
        assert!(error.starts_with(reason), "{text}: {error}");
    }
    let refusing = Store { refuse_write: true, ..Store::default() };
    assert_eq!(sbx_images::write_records(&refusing, RECORDS, &BTreeMap::new()), Err(format!("EACCES: {RECORDS}")));
}

#[test]
fn a_refresh_asks_docker_hub_for_the_newest_old_enough_tag_and_says_why_it_cannot() {
    let hub = Hub::answering(Answer::Tags(HUB));
    let refreshed = sbx_images::refresh_one(&hub, &FixedClock(NOW), &sbx_images::IMAGES[0]).expect("refreshed");
    assert_eq!(refreshed, pi_record(NOW));
    assert_eq!(refreshed.reference(&sbx_images::IMAGES[0]), PI_REFERENCE);
    assert_eq!(hub.calls(), ["curl -fsS https://hub.docker.com/v2/repositories/sbx/pi-image/tags?page_size=100&ordering=last_updated"]);

    let cases = [
        (Answer::Exit(22), "Docker Hub answered exit 22: curl: (22) 503"),
        (Answer::Timeout, "Docker Hub answered exit 0"),
        (Answer::NotRun, "curl did not run: curl is not installed"),
        (Answer::Tags("<html>"), "the Docker Hub answer does not parse"),
        (Answer::Tags(HUB_TOO_NEW), "no pi-image tag is 2 days old yet"),
    ];
    for (answer, reason) in cases {
        let error = sbx_images::refresh_one(&Hub::answering(answer), &FixedClock(NOW), &sbx_images::IMAGES[0]).expect_err("refused");
        assert!(error.starts_with(reason), "{error}");
    }
}

#[test]
fn a_launch_refreshes_a_missing_or_stale_record_and_only_those() {
    let env = HashMap::from([("HOME".to_string(), "/home/me".to_string())]);

    // A fresh record is used as it is: no registry call, no write.
    let store = Store::default();
    store.put(RECORDS, &records_text(&[("pi", pi_record(NOW - HOUR))]));
    let hub = Hub::answering(Answer::Tags(HUB));
    let (reference, warnings) = sbx_images::ensure(&hub, &store, &FixedClock(NOW), &env, "pi").expect("the record");
    assert_eq!((reference.as_str(), warnings.len()), (PI_REFERENCE, 0));
    assert!(hub.calls().is_empty(), "{:?}", hub.calls());

    // A record a day old or older, or one dated in the future, is refreshed.
    for checked in [NOW - MAX_AGE_MS, NOW - MAX_AGE_MS - HOUR, NOW + HOUR] {
        let store = Store::default();
        store.put(RECORDS, &records_text(&[("pi", record("20261124", NEWER_DIGEST, checked))]));
        let hub = Hub::answering(Answer::Tags(HUB));
        let (reference, warnings) = sbx_images::ensure(&hub, &store, &FixedClock(NOW), &env, "pi").expect("refreshed");
        assert_eq!((reference.as_str(), warnings.len()), (PI_REFERENCE, 0));
        assert_eq!(hub.calls().len(), 2, "the registry, then the store of images");
        assert!(store.text(RECORDS).contains(&format!("\"checked_at\": {NOW}")), "the new record is kept");
    }

    // No record at all is refreshed too, and a refusal to write the file is a warning, not a failed launch.
    let store = Store { refuse_write: true, ..Store::default() };
    let hub = Hub::answering(Answer::Tags(HUB));
    let (reference, warnings) = sbx_images::ensure(&hub, &store, &FixedClock(NOW), &env, "pi").expect("launched");
    assert_eq!(reference, PI_REFERENCE);
    assert_eq!(warnings, [format!("sbx-images.json was not written (EACCES: {RECORDS})")]);
}

#[test]
fn a_failed_refresh_keeps_the_record_it_had_with_a_warning_and_with_no_record_refuses_the_launch() {
    let env = HashMap::from([("HOME".to_string(), "/home/me".to_string())]);
    let store = Store::default();
    store.put(RECORDS, &records_text(&[("pi", pi_record(NOW - 3 * MAX_AGE_MS))]));
    let (reference, warnings) = sbx_images::ensure(&Hub::answering(Answer::Exit(6)), &store, &FixedClock(NOW), &env, "pi").expect("the old record launches");
    assert_eq!(reference, PI_REFERENCE);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("the image refresh failed (Docker Hub answered exit 6") && warnings[0].contains("the 20261125-"), "{warnings:?}");

    let empty = Store::default();
    let homeless = sbx_images::ensure(&Hub::answering(Answer::NotRun), &empty, &FixedClock(NOW), &HashMap::new(), "pi").expect_err("no state home");
    assert_eq!(homeless.code, "unexpected");
    let error = sbx_images::ensure(&Hub::answering(Answer::NotRun), &empty, &FixedClock(NOW), &env, "pi").expect_err("refused");
    assert_eq!(error.code, "sandbox-unavailable");
    assert!(error.message.contains("no sbx image is recorded and the refresh failed") && error.message.contains("run model-gateway sandbox refresh"), "{}", error.message);

    // A client the gateway has no image for is refused before anything is asked.
    let hub = Hub::answering(Answer::Tags(HUB));
    let error = sbx_images::ensure(&hub, &empty, &FixedClock(NOW), &env, "gemini").expect_err("no image");
    assert_eq!((error.code.as_str(), hub.calls().len()), ("sandbox-unsupported", 0));
}

#[test]
fn a_state_file_that_cannot_be_read_is_a_warning_and_the_images_are_refreshed() {
    let env = HashMap::from([("HOME".to_string(), "/home/me".to_string())]);
    let store = Store::with(RECORDS, "{ not json");
    let hub = Hub::answering(Answer::Tags(HUB));
    let (reference, warnings) = sbx_images::ensure(&hub, &store, &FixedClock(NOW), &env, "pi").expect("refreshed");
    assert_eq!(reference, PI_REFERENCE);
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("sbx-images.json cannot be read (not JSON"), "{warnings:?}");
}

fn records_text(records: &[(&str, Record)]) -> String {
    let mut map = BTreeMap::new();
    for (client, record) in records {
        map.insert(client.to_string(), record.clone());
    }
    let store = Store::default();
    sbx_images::write_records(&store, RECORDS, &map).expect("written");
    store.text(RECORDS)
}

#[test]
fn sandbox_refresh_records_what_it_chose_and_sandbox_status_prints_the_records_without_the_network() {
    let sandbox = Sandbox::new();
    let runtime = runtime_with(&sandbox, Hub::answering(Answer::Tags(HUB)));

    let status = cli(&runtime, &["sandbox", "status"]);
    assert_eq!(status.exit_code, 0);
    assert_eq!(status.stdout, "pi: none none checked none\nopencode: none none checked none\nclaude: none none checked none\ncodex: none none checked none\nvibe: none none checked none\nhermes: none none checked none\n", "{}", status.stdout);

    let refreshed = cli(&runtime, &["sandbox", "refresh", "--json"]);
    assert_eq!((refreshed.exit_code, refreshed.stderr.as_str()), (0, ""));
    let answer = json::parse(&refreshed.stdout).expect("JSON");
    let Some(Value::Arr(clients)) = answer.as_object().and_then(|object| object.get("clients")) else { panic!("clients is a list") };
    let pi = clients[0].as_object().expect("pi");
    assert_eq!(pi.get("client"), Some(&Value::str("pi")));
    assert_eq!(pi.get("image"), Some(&Value::str("docker.io/sbx/pi-image")));
    assert_eq!(pi.get("tag"), Some(&Value::str(&format!("20261125-{SHA}"))));
    assert_eq!(pi.get("digest"), Some(&Value::str(DIGEST)));
    assert_eq!(pi.get("checked_at"), Some(&Value::str("2026-11-27T12:00:00.000Z")));
    assert_eq!(answer.as_object().and_then(|object| object.get("warnings")), Some(&Value::Arr(Vec::new())));

    let table = cli(&runtime, &["sandbox", "refresh"]);
    assert_eq!(table.exit_code, 0);
    assert!(table.stdout.starts_with(&format!("pi: 20261125-{SHA} {DIGEST} checked 2026-11-27T12:00:00.000Z\n")), "{}", table.stdout);

    // Status now reads the file the refresh wrote, and asks Docker Hub about nothing.
    let status = cli(&runtime_with(&sandbox, Hub::answering(Answer::NotRun)), &["sandbox", "status", "--json"]);
    assert_eq!(status.exit_code, 0);
    assert!(status.stdout.contains(&format!("\"digest\": \"{DIGEST}\"")), "{}", status.stdout);
}

#[test]
fn a_refresh_that_fails_keeps_the_record_and_exits_zero_and_exits_one_only_when_nothing_is_recorded() {
    let sandbox = Sandbox::new();
    let failing = runtime_with(&sandbox, Hub::answering(Answer::Exit(28)));

    let nothing = cli(&failing, &["sandbox", "refresh"]);
    assert_eq!(nothing.exit_code, 1, "every client failed with no record");
    assert!(nothing.stdout.contains("warning: pi: refresh failed (Docker Hub answered exit 28") && nothing.stdout.contains("no record is kept"), "{}", nothing.stdout);

    assert_eq!(cli(&runtime_with(&sandbox, Hub::answering(Answer::Tags(HUB))), &["sandbox", "refresh"]).exit_code, 0);
    let kept = cli(&failing, &["sandbox", "refresh", "--json"]);
    assert_eq!(kept.exit_code, 0, "the records are kept");
    assert!(kept.stdout.contains("\"tag\": \"20261125-") && kept.stdout.contains("the 20261125-"), "{}", kept.stdout);
}

#[test]
fn the_sandbox_commands_take_only_json_and_name_a_state_home_they_cannot_find() {
    let sandbox = Sandbox::new();
    let runtime = runtime_with(&sandbox, Hub::answering(Answer::NotRun));
    let refused = cli(&runtime, &["sandbox", "refresh", "--force"]);
    assert_eq!(refused.exit_code, 1);
    assert!(refused.stderr.contains("sandbox refresh takes [--json]"), "{}", refused.stderr);

    let homeless = Runtime { env: HashMap::new(), ..runtime_with(&sandbox, Hub::answering(Answer::NotRun)) };
    let failed = cli(&homeless, &["sandbox", "status"]);
    assert_eq!(failed.exit_code, 1);
    assert!(failed.stdout.contains("\"code\":\"unexpected\""), "{}", failed.stdout);
    let failed = cli(&homeless, &["sandbox", "refresh"]);
    assert_eq!(failed.exit_code, 1);

    // A refresh whose records file cannot be written, or cannot be read, still answers with what it has and says so.
    let unwritable = Runtime { files: Box::new(Store { refuse_write: true, ..Store::default() }), ..runtime_with(&sandbox, Hub::answering(Answer::Tags(HUB))) };
    let answer = cli(&unwritable, &["sandbox", "refresh"]);
    assert_eq!(answer.exit_code, 0);
    assert!(answer.stdout.contains("warning: sbx-images.json was not written (EACCES:"), "{}", answer.stdout);
    let unreadable = Runtime { files: Box::new(Store { read_code: Some("EACCES"), ..Store::default() }), ..runtime_with(&sandbox, Hub::answering(Answer::Tags(HUB))) };
    let answer = cli(&unreadable, &["sandbox", "refresh"]);
    assert!(answer.stdout.contains("warning: sbx-images.json cannot be read (EACCES:"), "{}", answer.stdout);
}

#[test]
fn a_status_with_a_file_it_cannot_read_says_so_and_all_failed_reads_the_answer() {
    let sandbox = Sandbox::new();
    let runtime = runtime_with(&sandbox, Hub::answering(Answer::NotRun));
    std::fs::create_dir_all(format!("{}/state/agent-kit/model-gateway", sandbox.text())).expect("state dir");
    std::fs::write(format!("{}/state/agent-kit/model-gateway/sbx-images.json", sandbox.text()), "[]").expect("records");
    let status = cli(&runtime, &["sandbox", "status"]);
    assert_eq!(status.exit_code, 0);
    assert!(status.stdout.contains("warning: sbx-images.json cannot be read (not a JSON object)"), "{}", status.stdout);

    assert!(sbx_images::all_failed(&Value::Null), "an answer with no clients failed");
    let mixed = json::parse(r#"{"clients":[{"client":"pi","tag":null},{"client":"opencode","tag":"t"}],"warnings":[]}"#).expect("JSON");
    assert!(!sbx_images::all_failed(&mixed));
}

fn runtime_with(sandbox: &Sandbox, hub: Hub) -> Runtime {
    Runtime { clock: Box::new(FixedClock(NOW)), process: Box::new(hub), ..sandbox_runtime(sandbox, None) }
}

/// What the registry answers a `curl`: a listing, an HTTP failure, a timeout, or no curl at all.
enum Answer {
    Tags(&'static str),
    Exit(i32),
    Timeout,
    NotRun,
}

/// A process that answers every `curl` the way its answer says, and records each call.
struct Hub {
    answer: Answer,
    calls: Mutex<Vec<String>>,
}

impl Hub {
    fn answering(answer: Answer) -> Hub {
        Hub { answer, calls: Mutex::new(Vec::new()) }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

impl Process for Hub {
    fn resolve(&self, _binary: &str) -> Option<String> {
        None
    }
    fn exec(&self, command: &str, args: &[&str], _timeout_ms: Option<u64>) -> Result<ProcessResult, String> {
        self.calls.lock().expect("calls").push(format!("{command} {}", args.join(" ")));
        match self.answer {
            Answer::Tags(text) => Ok(ProcessResult { stdout: text.to_string(), stderr: String::new(), exit_code: 0, timed_out: false }),
            Answer::Exit(code) => Ok(ProcessResult { stdout: String::new(), stderr: format!("curl: ({code}) 503\n"), exit_code: code, timed_out: false }),
            Answer::Timeout => Ok(ProcessResult { stdout: String::new(), stderr: String::new(), exit_code: 0, timed_out: true }),
            Answer::NotRun => Err("curl is not installed".to_string()),
        }
    }
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessResult, String> {
        Err("the registry is not asked for a client run".to_string())
    }
}

/// An in-memory file system: what the gateway writes, what it removes, and what a read refuses.
#[derive(Default)]
struct Store {
    files: Mutex<HashMap<String, String>>,
    read_code: Option<&'static str>,
    refuse_write: bool,
    refuse_mkdir: bool,
}

impl Store {
    fn with(path: &str, text: &str) -> Store {
        let store = Store::default();
        store.put(path, text);
        store
    }

    fn put(&self, path: &str, text: &str) {
        self.files.lock().expect("files").insert(path.to_string(), text.to_string());
    }

    fn text(&self, path: &str) -> String {
        self.files.lock().expect("files").get(path).cloned().expect("a written file")
    }
}

impl Files for Store {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        if let Some(code) = self.read_code {
            return Err(FsError { code: code.to_string(), message: format!("{code}: {path}") });
        }
        self.files.lock().expect("files").get(path).cloned().ok_or(FsError { code: "ENOENT".to_string(), message: format!("ENOENT: {path}") })
    }
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.write_private(path, content)
    }
    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        if self.refuse_mkdir {
            return Err(FsError { code: "EACCES".to_string(), message: format!("EACCES: {path}") });
        }
        Ok(())
    }
    fn create_exclusive(&self, _path: &str) -> Result<bool, FsError> {
        Ok(true)
    }
    fn remove_file(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn modified_ms(&self, _path: &str) -> Option<i64> {
        None
    }
    fn exists(&self, path: &str) -> Result<bool, FsError> {
        Ok(self.files.lock().expect("files").contains_key(path))
    }
    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
    fn stamp(&self, _path: &str) -> Option<FileStamp> {
        None
    }
    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        if self.refuse_write {
            return Err(FsError { code: "EACCES".to_string(), message: format!("EACCES: {path}") });
        }
        self.put(path, content);
        Ok(())
    }
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        self.write_private(path, content)
    }
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        Ok(format!("{parent}/{prefix}-tmp"))
    }
    fn remove(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }
}
