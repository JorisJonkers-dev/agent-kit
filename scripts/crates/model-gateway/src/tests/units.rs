//! Unit tests for what the goldens cannot reach: the JSON reader and writer,
//! the POSIX path helpers, the timestamp parser and writer, the adapters'
//! failure paths, the lock, and the command line's own edges.

use super::scenarios::{FixedClock, Sandbox, FIXED_NOW};
use crate::adapters::clock::SystemClock;
use crate::adapters::fs::{fail, write_then_rename, SystemFiles};
use crate::cli::run_cli;
use crate::error::GatewayError;
use crate::json::{compact, number_text, numbers, object, parse, pretty, Object, Value};
use crate::lock::FileLock;
use crate::ports::{Clock, Files, FsError, LockTimings, Runtime};
use crate::text::{basename, dirname, join, normalize, resolve};
use crate::timestamps::{parse as parse_time, to_iso};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Mutex;

/// A runtime over a fresh sandbox, with the state home inside it.
fn sandbox_runtime(sandbox: &Sandbox, now: i64) -> Runtime {
    let root = sandbox.text();
    std::fs::create_dir_all(format!("{root}/home")).expect("home");
    std::fs::create_dir_all(format!("{root}/state")).expect("state");
    let env = HashMap::from([("HOME".to_string(), format!("{root}/home")), ("XDG_STATE_HOME".to_string(), format!("{root}/state"))]);
    Runtime {
        clock: Box::new(FixedClock(now)),
        files: Box::new(SystemFiles { cwd: root.clone() }),
        lock: LockTimings::default(),
        process: Box::new(crate::adapters::process::SystemProcess { env: env.clone() }),
        git: Box::new(crate::adapters::git::SystemGit { env: env.clone(), timeout_ms: crate::adapters::git::TIMEOUT_MS }),
        env,
        defaults: crate::SHIPPED_DEFAULTS.to_string(),
        defaults_path: "defaults.json".to_string(),
        routing: crate::SHIPPED_ROUTING.to_string(),
    }
}

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| word.to_string()).collect()
}

#[test]
fn json_numbers_print_as_javascript_prints_them() {
    let cases = [
        ("0", "0"),
        ("-0", "0"),
        ("1", "1"),
        ("1.5", "1.5"),
        ("100", "100"),
        ("1e2", "100"),
        ("1e+2", "100"),
        ("0.1", "0.1"),
        ("-12.25", "-12.25"),
        ("123456789012345680000", "123456789012345680000"),
        ("1e21", "1e+21"),
        ("1.5e21", "1.5e+21"),
        ("1.5e-7", "1.5e-7"),
        ("1e-7", "1e-7"),
        ("0.000001", "0.000001"),
        ("1e999", "null"),
    ];
    for (input, output) in cases {
        let value = parse(input).expect(input);
        assert_eq!(compact(&value), output, "{input}");
    }
    assert_eq!(number_text(f64::NAN), "null");
    assert_eq!(number_text(f64::INFINITY), "null");
}

#[test]
fn json_false_prints_and_reads_back() {
    assert_eq!(compact(&Value::Bool(false)), "false");
    assert_eq!(parse("false"), Ok(Value::Bool(false)));
    assert_eq!(pretty(&object(vec![("live", Value::Bool(false))])), "{\n  \"live\": false\n}");
}

#[test]
fn json_strings_escape_as_stringify_does() {
    let value = parse(r#""a\"b\\c\/d\b\f\n\r\t\u0001\u007f\u00e9\ud83d\ude00\u2028""#).expect("string");
    assert_eq!(compact(&value), "\"a\\\"b\\\\c/d\\b\\f\\n\\r\\t\\u0001\u{7f}é😀\u{2028}\"");
    // A lone surrogate cannot be held; it reads as the replacement character, a pair that is not one too.
    assert_eq!(parse(r#""\ud83d""#), Ok(Value::str("\u{fffd}")));
    assert_eq!(parse(r#""\ud83dx""#), Ok(Value::str("\u{fffd}x")));
    assert_eq!(parse(r#""\ud83d\u0041""#), Ok(Value::str("\u{fffd}A")));
    assert_eq!(parse(r#""\ude00""#), Ok(Value::str("\u{fffd}")));
}

#[test]
fn json_objects_keep_javascript_key_order() {
    let value = parse(r#"{"b":1,"10":2,"a":3,"2":4,"4294967295":5,"01":6,"b":7}"#).expect("object");
    assert_eq!(compact(&value), r#"{"2":4,"10":2,"b":7,"a":3,"4294967295":5,"01":6}"#);
    let mut members = Object::new();
    members.set("z", Value::Null);
    members.set("y", Value::Bool(true));
    members.remove("z");
    assert_eq!(pretty(&Value::Obj(members)), "{\n  \"y\": true\n}");
    assert_eq!(pretty(&object(vec![("a", Value::Arr(vec![])), ("b", Value::Obj(Object::new()))])), "{\n  \"a\": [],\n  \"b\": {}\n}");
    assert_eq!(Value::Null.as_object(), None);
    assert_eq!(Value::Null.as_str(), None);
    assert_eq!(numbers(&[1, 2]), Value::Arr(vec![Value::Num(1.0), Value::Num(2.0)]));
    assert_eq!(compact(&numbers(&[1, 2])), "[1,2]");
    assert_eq!(Value::count(3), Value::Num(3.0));
}

#[test]
fn json_syntax_errors_read_as_v8_words_them() {
    let cases = [
        ("", "Unexpected end of JSON input"),
        ("{", "Expected property name or '}' in JSON at position 1 (line 1 column 2)"),
        ("{\"a\"", "Expected ':' after property name in JSON at position 4 (line 1 column 5)"),
        ("{\"a\":1,", "Expected double-quoted property name in JSON at position 7 (line 1 column 8)"),
        ("[1 2]", "Expected ',' or ']' after array element in JSON at position 3 (line 1 column 4)"),
        ("not json {", "Unexpected token 'o', \"not json {\" is not valid JSON"),
        ("[1,]", "Unexpected token ']', \"[1,]\" is not valid JSON"),
        ("\"abc", "Unterminated string in JSON at position 4 (line 1 column 5)"),
        ("\"\\", "Unexpected end of JSON input"),
        ("\"\\x\"", "Bad escaped character in JSON at position 2 (line 1 column 3)"),
        ("\"\\u12\"", "Bad Unicode escape in JSON at position 5 (line 1 column 6)"),
        ("\"\u{1}\"", "Bad control character in string literal in JSON at position 1 (line 1 column 2)"),
        ("-", "No number after minus sign in JSON at position 1 (line 1 column 2)"),
        ("1.", "Unterminated fractional number in JSON at position 2 (line 1 column 3)"),
        ("1e+", "Exponent part is missing a number in JSON at position 3 (line 1 column 4)"),
        ("01", "Unexpected number in JSON at position 1 (line 1 column 2)"),
        ("[] x", "Unexpected non-whitespace character after JSON at position 3 (line 1 column 4)"),
        ("{\"a\":\n1 x}", "Expected ',' or '}' after property value in JSON at position 8 (line 2 column 3)"),
        ("{\"a\":\r\n1 x}", "Expected ',' or '}' after property value in JSON at position 9 (line 2 column 3)"),
        ("{\"a\":\r1 x}", "Expected ',' or '}' after property value in JSON at position 8 (line 2 column 3)"),
        ("trux", "Unexpected token 'x', \"trux\" is not valid JSON"),
        (&"x".repeat(30), "Unexpected token 'x', \"xxxxxxxxxx\"... is not valid JSON"),
        (&format!("[{}x", "1,".repeat(15)), "Unexpected token 'x', ...\"1,1,1,1,1,x\" is not valid JSON"),
        (&format!("[{}x{}", "1,".repeat(15), ",2".repeat(15)), "Unexpected token 'x', ...\"1,1,1,1,1,x,2,2,2,2,\"... is not valid JSON"),
    ];
    for (input, message) in cases {
        assert_eq!(parse(input), Err(message.to_string()), "{input:?}");
    }
    assert_eq!(parse(" [ true , false , null , {} ] "), Ok(Value::Arr(vec![Value::Bool(true), Value::Bool(false), Value::Null, Value::Obj(Object::new())])));
}

#[test]
fn text_helpers_follow_javascript() {
    assert_eq!(normalize("/a//b/./c/../d/.."), "/a/b");
    assert_eq!(normalize("/.."), "/");
    assert_eq!(join(&["/a/", "b", "../c"]), "/a/c");
    assert_eq!(resolve("/work", "x/../y"), "/work/y");
    assert_eq!(resolve("/work", "/abs//path/"), "/abs/path");
    assert_eq!((basename("/a/b/"), basename("/"), basename("a"), basename("/a/b")), ("b", "", "a", "b"));
    assert_eq!((dirname("/a/b/"), dirname("/a"), dirname("/"), dirname("/a/b")), ("/a", "/", "/", "/a"));
}

#[test]
fn timestamps_read_every_form_date_parse_accepts() {
    // A date alone is midnight UTC.
    assert_eq!(parse_time("2026-11-03"), Some(FIXED_NOW - 12 * 3_600_000));
    // A date-time without a zone is UTC; `T`, `t` and a space all separate.
    assert_eq!(parse_time("2026-11-03T12:00"), Some(FIXED_NOW));
    assert_eq!(parse_time("2026-11-03t12:00"), Some(FIXED_NOW));
    assert_eq!(parse_time("2026-11-03 12:00"), Some(FIXED_NOW));
    assert_eq!(parse_time("2026-11-03T12:00:00"), Some(FIXED_NOW));
    assert_eq!(parse_time("2026-11-03T12:00:00Z"), Some(FIXED_NOW));
    assert_eq!(parse_time("2026-11-03T12:00:00z"), Some(FIXED_NOW));
    // Fractional seconds of any length keep their first three digits.
    assert_eq!(parse_time("2026-11-03T12:00:00.5Z"), Some(FIXED_NOW + 500));
    assert_eq!(parse_time("2026-11-03T12:00:00.1234Z"), Some(FIXED_NOW + 123));
    assert_eq!(parse_time("2026-11-03T12:00:00.000Z"), Some(FIXED_NOW));
    // An offset takes its hours and minutes off the wall clock.
    assert_eq!(parse_time("2026-11-03T14:00:00+02:00"), Some(FIXED_NOW));
    assert_eq!(parse_time("2026-11-03T06:30:00-05:30"), Some(FIXED_NOW));
    // An expanded year is six digits after its sign.
    assert_eq!(parse_time("+010000-01-01"), Some(2_932_897 * 86_400_000));
    assert_eq!(parse_time("-000001-01-01"), Some(-719_893 * 86_400_000));
    // Every month's length, and the leap years that change February's.
    for text in [
        "2026-01-31", "2026-03-31", "2026-05-31", "2026-07-31", "2026-08-31", "2026-10-31", "2026-12-31", "2026-04-30", "2026-06-30", "2026-09-30",
        "2026-11-30", "2026-02-28", "2024-02-29", "2000-02-29",
    ] {
        assert!(parse_time(text).is_some(), "{text}");
    }
}

#[test]
fn timestamps_accept_the_short_and_rolling_forms_date_parse_accepts() {
    assert_eq!(parse_time("2026"), Some(1_767_225_600_000));
    assert_eq!(parse_time("2026-11"), Some(1_793_491_200_000));
    assert_eq!(parse_time("2026-11-31"), Some(1_796_083_200_000));
    assert_eq!(parse_time("2026-02-30"), Some(1_772_409_600_000));
    assert_eq!(parse_time("2026-11-03T24:00"), Some(1_793_750_400_000));
}

#[test]
fn timestamps_refuse_what_date_parse_refuses() {
    for text in [
        "",
        "2026-11-03X12:00",
        "2026-11-03T12:00:00Zx",
        "2026-13-01",
        "2026-00-01",
        "2026-11-00",
        "2026-11-32",
        "2026-11-03T24:00:01",
        "2026-11-03T25:00",
        "2026-11-03T12:60",
        "2026-11-03T12:00:60",
        "2026-1-03",
        "2026/11/03",
        "2026-11-03T12:00:00.",
        "2026-11-03T12:00:00X",
        "2026-11-03T12:00:00+02",
        "2026-11-03T12:00:00+2:00",
        "20261103",
        "26-11-03",
        "+26-11-03",
    ] {
        assert_eq!(parse_time(text), None, "{text:?}");
    }
}

#[test]
fn timestamps_write_the_iso_form_to_iso_string_writes() {
    assert_eq!(to_iso(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(to_iso(FIXED_NOW), "2026-11-03T12:00:00.000Z");
    assert_eq!(to_iso(-1), "1969-12-31T23:59:59.999Z");
    assert_eq!(to_iso(951_782_400_000 + 86_399_999), "2000-02-29T23:59:59.999Z");
    assert_eq!(to_iso(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
    assert_eq!(to_iso(4_102_444_799_000), "2099-12-31T23:59:59.000Z");
    // An expanded year outside 0..9999 carries its sign and six digits.
    assert_eq!(to_iso(2_932_897 * 86_400_000), "+010000-01-01T00:00:00.000Z");
    assert_eq!(to_iso(-719_893 * 86_400_000), "-000001-01-01T00:00:00.000Z");
    // Every form the writer makes reads back.
    for ms in [0, FIXED_NOW, -1, 951_782_400_000, 2_932_897 * 86_400_000, -719_893 * 86_400_000] {
        assert_eq!(parse_time(&to_iso(ms)), Some(ms), "{ms}");
    }
}

#[test]
fn system_files_read_and_write_the_real_file_system() {
    let sandbox = Sandbox::new();
    let dir = sandbox.text();
    let files = SystemFiles { cwd: dir.clone() };

    // A missing file and a directory both fail, in Node's words.
    let error = files.read_text("missing.txt").expect_err("no file");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("ENOENT", "ENOENT: no such file or directory, open 'missing.txt'"));
    let error = files.read_text(".").expect_err("a directory is not text");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("EISDIR", "EISDIR: illegal operation on a directory, read"));

    // mkdir_all makes the whole chain, and refuses a path a file is in the way of.
    files.mkdir_all("a/b/c").expect("mkdir -p");
    assert!(std::path::Path::new(&format!("{dir}/a/b/c")).is_dir());
    std::fs::write(format!("{dir}/a/file"), "x").expect("write");
    let error = files.mkdir_all("a/file/deeper").expect_err("a file is not a directory");
    assert!(error.message.starts_with("EEXIST: file already exists, mkdir") || error.message.starts_with("ENOTDIR: not a directory, mkdir"), "{}", error.message);

    // write_atomic replaces the file whole.
    files.write_atomic("a/b/c/one.txt", "one").expect("write");
    files.write_atomic("a/b/c/one.txt", "two").expect("replace");
    assert_eq!(files.read_text("a/b/c/one.txt"), Ok("two".to_string()));

    // A write that cannot be renamed into place leaves no temporary file behind.
    std::fs::create_dir(format!("{dir}/a/b/c/folder")).expect("folder");
    let error = files.write_atomic("a/b/c/folder", "x").expect_err("a folder is not replaced");
    assert_eq!(error.code, "EISDIR");
    let mut left: Vec<String> = std::fs::read_dir(format!("{dir}/a/b/c")).expect("read").flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, vec!["folder".to_string(), "one.txt".to_string()]);

    // create_exclusive says whether it made the file, and names a path it cannot.
    assert_eq!(files.create_exclusive("a/b/c/lock"), Ok(true));
    assert_eq!(files.create_exclusive("a/b/c/lock"), Ok(false));
    let error = files.create_exclusive("nowhere/lock").expect_err("no folder");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("ENOENT", "ENOENT: no such file or directory, open 'nowhere/lock'"));

    // remove_file says nothing about a file that is not there, and names one it cannot remove.
    assert_eq!(files.remove_file("a/b/c/lock"), Ok(()));
    assert_eq!(files.remove_file("a/b/c/lock"), Ok(()));
    let error = files.remove_file("a/b/c/folder").expect_err("a folder is not a file");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("EISDIR", "EISDIR: illegal operation on a directory, unlink 'a/b/c/folder'"));

    // modified_ms reads the file's time, and gives nothing for a file that is not there.
    let modified = files.modified_ms("a/b/c/one.txt").expect("a modification time");
    assert!(modified > 1_600_000_000_000, "{modified}");
    assert_eq!(files.modified_ms("a/b/c/missing.txt"), None);
}

#[test]
fn a_temporary_file_that_cannot_be_written_names_itself() {
    let sandbox = Sandbox::new();
    let dir = sandbox.text();
    let missing = format!("{dir}/nowhere/tmp");
    assert_eq!(
        write_then_rename(&missing, &format!("{dir}/x"), "x").map_err(|error| error.message),
        Err(format!("ENOENT: no such file or directory, open '{missing}'"))
    );
    // The errno table is Node's, and an errno it does not know is UNKNOWN.
    for (errno, code) in [
        (1, "EPERM"),
        (2, "ENOENT"),
        (13, "EACCES"),
        (17, "EEXIST"),
        (20, "ENOTDIR"),
        (21, "EISDIR"),
        (28, "ENOSPC"),
        (30, "EROFS"),
        (crate::adapters::fs::ENAMETOOLONG, "ENAMETOOLONG"),
        (crate::adapters::fs::ENOTEMPTY, "ENOTEMPTY"),
        (crate::adapters::fs::ELOOP, "ELOOP"),
        (5, "UNKNOWN"),
    ] {
        assert_eq!(fail(&std::io::Error::from_raw_os_error(errno), "open", Some("p")).code, code);
    }
    assert_eq!(fail(&std::io::Error::from_raw_os_error(13), "read", None).message, "EACCES: permission denied, read");
}

#[test]
fn the_system_clock_reads_epoch_milliseconds() {
    let now = SystemClock.now_ms();
    assert!(now > 1_700_000_000_000, "{now}");
}

#[test]
fn the_command_line_runs_against_the_real_system() {
    let runtime = crate::default_runtime();
    assert!(runtime.env.contains_key("PATH"));
    assert_eq!(runtime.lock, LockTimings::default());
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.path.join("probe.txt"), "seen").expect("write");
    assert_eq!(runtime.files.read_text(&format!("{}/probe.txt", sandbox.text())), Ok("seen".to_string()));

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    assert_eq!(crate::run_with(["--help".to_string()].into_iter(), &runtime, &mut stdout, &mut stderr), 0);
    assert!(String::from_utf8_lossy(&stdout).starts_with("usage: model-gateway models"));
    assert!(stderr.is_empty());

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    assert_eq!(crate::run_with(["bogus".to_string()].into_iter(), &runtime, &mut stdout, &mut stderr), 1);
    assert!(String::from_utf8_lossy(&stdout).starts_with("{\"status\":\"error\",\"code\":\"unknown-command\""), "{}", String::from_utf8_lossy(&stdout));
    assert!(String::from_utf8_lossy(&stderr).starts_with("model-gateway: unknown subcommand \"bogus\""), "{}", String::from_utf8_lossy(&stderr));
}

#[test]
fn a_workflow_name_must_start_with_a_letter_or_digit() {
    let sandbox = Sandbox::new();
    let runtime = sandbox_runtime(&sandbox, FIXED_NOW);
    let result = run_cli(&argv(&["workflow", "start", "--json", r#"{"name": "-bad"}"#]), &runtime);
    assert_eq!(result.exit_code, 1);
    let message = r#"workflow \"-bad\" is not a workflow name: a letter or digit, then letters, digits and . _ : / @ - only, at most 80 characters"#;
    assert_eq!(result.stdout, format!("{{\"status\":\"error\",\"code\":\"invalid-input\",\"message\":\"{message}\"}}\n"));
    assert_eq!(result.stderr, format!("model-gateway workflow start: {}\n", message.replace("\\\"", "\"")));
}

/// The lock file held in memory, so a stale one and a contended one can be made on demand.
struct LockFiles {
    present: AtomicBool,
    modified: Mutex<Option<i64>>,
    removed: AtomicUsize,
    create_error: Option<FsError>,
}

impl LockFiles {
    fn new(present: bool, modified: Option<i64>) -> LockFiles {
        LockFiles { present: AtomicBool::new(present), modified: Mutex::new(modified), removed: AtomicUsize::new(0), create_error: None }
    }

    fn present(&self) -> bool {
        self.present.load(Ordering::SeqCst)
    }

    fn removed(&self) -> usize {
        self.removed.load(Ordering::SeqCst)
    }
}

impl Files for LockFiles {
    fn write_text(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        unreachable!("the test does not write text")
    }

    fn temp_dir(&self, _parent: &str, _prefix: &str) -> Result<String, FsError> {
        unreachable!("the test makes no temporary directory")
    }

    fn remove(&self, _path: &str) -> Result<(), FsError> {
        unreachable!("the test removes nothing")
    }

    fn read_text(&self, path: &str) -> Result<String, FsError> {
        Err(FsError { code: "ENOENT".into(), message: format!("ENOENT: no such file or directory, open '{path}'") })
    }

    fn write_atomic(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn mkdir_all(&self, _path: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn create_exclusive(&self, _path: &str) -> Result<bool, FsError> {
        if let Some(error) = &self.create_error {
            return Err(error.clone());
        }
        if self.present.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        Ok(true)
    }

    fn remove_file(&self, _path: &str) -> Result<(), FsError> {
        self.removed.fetch_add(1, Ordering::SeqCst);
        self.present.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn modified_ms(&self, _path: &str) -> Option<i64> {
        *self.modified.lock().expect("the modification time")
    }

    fn exists(&self, _path: &str) -> Result<bool, FsError> {
        Ok(false)
    }

    fn append(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }

    fn stamp(&self, _path: &str) -> Option<crate::ports::FileStamp> {
        None
    }

    fn write_private(&self, _path: &str, _content: &str) -> Result<(), FsError> {
        Ok(())
    }
}

/// A clock that moves on at every reading, so a wait that never ends runs out at once.
struct TickingClock {
    now: AtomicI64,
    step: i64,
}

impl Clock for TickingClock {
    fn monotonic_ms(&self) -> f64 {
        0.0
    }

    fn now_ms(&self) -> i64 {
        self.now.fetch_add(self.step, Ordering::SeqCst)
    }
}

const LOCK: &str = "/state/agent-kit/model-gateway/workflows.json.lock";

#[test]
fn a_lock_left_by_a_dead_writer_is_broken() {
    let files = LockFiles::new(true, Some(0));
    let clock = FixedClock(10_000);
    let lock = FileLock::new("workflows", LockTimings { retry_ms: 0, stale_ms: 5_000, timeout_ms: 10_000 });
    assert_eq!(lock.run(LOCK, &files, &clock, || Ok("done")), Ok("done"));
    // Once to break the stale lock, once to release the one it took.
    assert_eq!(files.removed(), 2);
    assert!(!files.present());
}

#[test]
fn a_lock_that_never_frees_is_given_up_on() {
    let files = LockFiles::new(true, None);
    // A millisecond a reading, so the wait sleeps a few times before it gives up.
    let clock = TickingClock { now: AtomicI64::new(0), step: 1 };
    let lock = FileLock::new("workflows", LockTimings { retry_ms: 0, stale_ms: 5_000, timeout_ms: 10 });
    let error = lock.run(LOCK, &files, &clock, || Ok("never")).expect_err("the lock never came free");
    assert_eq!(error.code, "unexpected");
    assert_eq!(error.message, format!("{LOCK}: another gateway process held the workflows lock for 10 ms"));
    assert_eq!(files.removed(), 0);
}

#[test]
fn the_lock_is_released_when_the_action_fails() {
    let files = LockFiles::new(false, None);
    let clock = FixedClock(0);
    let lock = FileLock::new("workflows", LockTimings::default());
    let error = lock.run(LOCK, &files, &clock, || Err::<&str, GatewayError>(GatewayError::new("invalid-config", "the file is not JSON"))).expect_err("the action failed");
    assert_eq!(error, GatewayError::new("invalid-config", "the file is not JSON"));
    assert_eq!(files.removed(), 1);
    assert!(!files.present());
}

#[test]
fn a_lock_that_cannot_be_created_is_reported() {
    let mut files = LockFiles::new(false, None);
    files.create_error = Some(FsError { code: "EACCES".into(), message: format!("EACCES: permission denied, open '{LOCK}'") });
    let clock = FixedClock(0);
    let lock = FileLock::new("workflows", LockTimings::default());
    let error = lock.run(LOCK, &files, &clock, || Ok("never")).expect_err("the lock cannot be made");
    assert_eq!(error, GatewayError::new("unexpected", format!("EACCES: permission denied, open '{LOCK}'")));
}
