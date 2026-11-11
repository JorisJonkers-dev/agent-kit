use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn fixtures() -> String {
    format!("{}/fixtures", env!("CARGO_MANIFEST_DIR"))
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn run_cli(args: &[&str]) -> (i32, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli(&s(args), &mut out, &mut err);
    (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
}

fn temp_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("transcript-mix-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn prints_the_fixture_values_for_the_window() {
    let (code, out, err) = run_cli(&["--root", &fixtures(), "--since", "2026-11-01", "--until", "2026-12-01"]);
    assert_eq!(code, 0, "{err}");
    let expected = "\
agent claude
  tasks 2 (edited 1)
  tool Bash (search) 1
  tool Glob (search) 1
  tool Grep (search) 1
  tool Read (read) 2
  tool mcp__hindsight__search_knowledge_pages (knowledge) 1
  tool mcp__lsp__find_references (lsp) 1
  exploration_tokens 440
agent opencode
  tasks 1 (edited 1)
  tool grep (search) 1
  tool read (read) 1
  exploration_tokens 60
";
    assert_eq!(out, expected);
}

#[test]
fn the_window_decides_which_tasks_count() {
    let (_, out, _) = run_cli(&["--root", &fixtures(), "--since", "2026-10-01", "--until", "2026-10-02"]);
    assert_eq!(out, "agent claude\n  tasks 1 (edited 1)\n  tool Grep (search) 1\n  exploration_tokens 5000\n");
    let (_, out, _) = run_cli(&["--root", &fixtures(), "--since", "2027-01-01"]);
    assert_eq!(out, "no transcripts in the window\n");
    // --until is exclusive; the opencode task starts at 2026-11-05T12:00:00Z.
    let (_, out, _) = run_cli(&["--root", &fixtures(), "--since", "2026-11-05T00:00:00Z", "--until", "2026-11-05T12:00:00Z"]);
    assert_eq!(out, "no transcripts in the window\n");
    // An offset shifts the instant: 13:00+01:00 is 12:00Z, so the task is inside.
    let (_, out, _) = run_cli(&["--root", &fixtures(), "--since", "2026-11-05T13:00:00+01:00", "--until", "2026-11-05T12:00:01.5Z"]);
    assert!(out.starts_with("agent opencode"));
}

#[test]
fn help_and_argument_errors() {
    let (code, out, _) = run_cli(&["--help"]);
    assert_eq!((code, out.as_str()), (0, USAGE));
    for args in [
        &["--since", "2026-11-01"][..],
        &["--root", "x"],
        &["--root", "x", "--since"],
        &["--root", "x", "--since", "nope"],
        &["--root", "x", "--since", "2026-11-01", "--until", "nope"],
        &["--root", "x", "--since", "2026-11-01", "--bogus"],
        &["--root", "/definitely/missing", "--since", "2026-11-01"],
    ] {
        let (code, out, err) = run_cli(args);
        assert_eq!(code, 2, "{args:?}");
        assert!(out.is_empty() && err.starts_with("error: "), "{args:?}: {err}");
    }
}

#[test]
fn parse_time_accepts_dates_and_rfc3339_and_rejects_the_rest() {
    assert_eq!(parse_time("1970-01-01"), Ok(0));
    assert_eq!(parse_time("1970-01-02T00:00:01Z"), Ok(86401));
    assert_eq!(parse_time("1970-01-01T01:00:00+01:00"), Ok(0));
    assert_eq!(parse_time("1969-12-31T19:00:00-05:00"), Ok(0));
    assert_eq!(parse_time("1970-01-01T00:00:00.123Z"), Ok(0));
    assert_eq!(parse_time("2000-03-01"), Ok(951_868_800));
    assert!(parse_time("2000-02-29").is_ok() && parse_time("2024-02-29").is_ok());
    for bad in [
        "", "2026", "2026-1-01", "2026-13-01", "2026-00-10", "2026-01-32", "2026-02-29", "2026-04-31", "1900-02-29", "2026-aa-01", "+026-01-01",
        "2026-01-01T10:00:00", "2026-01-01T10:00Z", "2026-01-01T24:00:00Z", "2026-01-01T10:60:00Z",
        "2026-01-01T10:00:00+0100", "2026-01-01T10:00:00+aa:00", "2026-01-01T10:00:00+01:aa", "2026-01-01TxxZ",
    ] {
        assert!(parse_time(bad).unwrap_err().contains("invalid time"), "{bad}");
    }
}

#[test]
fn classify_names_the_code_finding_tools() {
    let rg = parse_json(r#"{"command":"rg foo"}"#).unwrap();
    let ls = parse_json(r#"{"command":"ls"}"#).unwrap();
    assert_eq!(classify("Bash", Some(&rg)), Some(Kind::Search));
    assert_eq!(classify("Bash", Some(&ls)), None);
    assert_eq!(classify("Bash", None), None);
    assert_eq!(classify("shell", Some(&parse_json(r#"{"command":""}"#).unwrap())), None);
    assert_eq!(classify("LSP", None), Some(Kind::Lsp));
    assert_eq!(classify("mcp__serena_lsp__x", None), Some(Kind::Lsp));
    assert_eq!(classify("mcp__memory-api__recall", None), Some(Kind::Knowledge));
    assert_eq!(classify("mcp__playwright__click", None), None);
    assert_eq!(classify("Edit", None), None);
    for (name, kind) in [("Grep", Kind::Search), ("View", Kind::Read)] {
        assert_eq!(classify(name, None), Some(kind));
    }
    assert_eq!(Kind::Search.label(), "search");
}

#[test]
fn analyse_transcript_edge_cases() {
    // Unreadable timestamp, no usage, no tool calls, malformed blocks.
    let text = [
        "",
        r#"{"type":"user","timestamp":"garbage"}"#,
        r#"{"type":"assistant"}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use"},{"type":"tool_use","name":7}]}}"#,
        r#"{"type":"assistant","timestamp":"2026-01-01","message":{"content":[{"type":"tool_use","name":"Grep","input":{}}]}}"#,
        r#"{"type":"assistant","message":{"usage":{"output_tokens":9},"content":[{"type":"tool_use","name":"EDIT"}]}}"#,
        r#"{"type":"assistant","message":{"usage":{"output_tokens":9},"content":[{"type":"tool_use","name":"Grep"}]}}"#,
    ]
    .join("\n");
    let task = analyse_transcript(&text);
    assert_eq!(task.started, Some(1_767_225_600)); // the first readable timestamp
    assert!(task.edited);
    assert_eq!(task.exploration_tokens, 0);
    assert_eq!(task.tools["Grep"], (Kind::Search, 2));
}

#[test]
fn walks_nested_directories_and_reports_io_errors() {
    let root = temp_dir();
    let nested = root.join("agent").join("deep");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(root.join("stray.txt"), "x").unwrap();
    std::fs::write(root.join("agent").join("notes.txt"), "x").unwrap();
    std::fs::write(
        nested.join("t.jsonl"),
        "{\"type\":\"assistant\",\"timestamp\":\"2026-02-01T00:00:00Z\",\"message\":{\"usage\":{\"output_tokens\":4},\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\"}]}}\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("empty")).unwrap();
    let mixes = analyse_root(&root, 0, i64::MAX).unwrap();
    assert_eq!(mixes.keys().collect::<Vec<_>>(), ["agent"]);
    assert_eq!(mixes["agent"].exploration_tokens, 4);

    // A directory named like a transcript cannot be read as one.
    let bad = temp_dir();
    std::fs::create_dir_all(bad.join("agent")).unwrap();
    std::os::unix::fs::symlink(bad.join("missing"), bad.join("agent").join("dangling.jsonl")).unwrap();
    assert!(analyse_root(&bad, 0, i64::MAX).unwrap_err().contains("cannot read"));

    // A non-UTF-8 transcript is an error, not a silent skip.
    let bin = temp_dir();
    std::fs::create_dir_all(bin.join("agent")).unwrap();
    std::fs::write(bin.join("agent").join("b.jsonl"), [0xff, 0xfe]).unwrap();
    assert!(analyse_root(&bin, 0, i64::MAX).unwrap_err().contains("cannot read"));

    // An unreadable agent directory is an error.
    let locked = temp_dir();
    std::fs::create_dir_all(locked.join("agent")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(locked.join("agent"), std::fs::Permissions::from_mode(0o000)).unwrap();
    let denied = analyse_root(&locked, 0, i64::MAX);
    std::fs::set_permissions(locked.join("agent"), std::fs::Permissions::from_mode(0o755)).unwrap();
    // Root bypasses permissions; elsewhere the read fails.
    assert!(denied.is_err() || denied.unwrap().is_empty());
}

#[test]
fn run_reads_the_real_process_arguments() {
    assert_eq!(run([OsString::from("--help")]), 0);
}
