use super::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn temp_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!(
        "code-nav-eval-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ok(stdout: &str) -> Result<Output, String> {
    Ok(Output { code: 0, stdout: stdout.to_string(), stderr: String::new() })
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn question_json(id: &str, family: &str) -> String {
    format!(
        r#"{{"id":"{id}","family":"{family}","repo":"alpha","question":"Where is X?","gold_answer":"in a.rs",
        "gold_files":[{{"repo":"alpha","path":"src/a.rs"}}],"must_contain":["a.rs"],"extra":1}}"#
    )
}

/// A valid 20-question document; `tweak` can edit the text first.
fn doc(tweak: impl Fn(String) -> String) -> String {
    let mut qs = Vec::new();
    for (fam, n) in FAMILIES {
        for i in 0..n {
            qs.push(question_json(&format!("{fam}-{i}"), fam));
        }
    }
    tweak(format!(
        r#"{{"schema":1,"pins":{{"alpha":"{SHA_A}","beta":"{SHA_B}"}},"questions":[{}]}}"#,
        qs.join(",")
    ))
}

fn q() -> Question {
    Question {
        id: "NK-01".into(),
        family: "name-known".into(),
        repo: "alpha".into(),
        question: "Where is X?".into(),
        gold_files: vec![GoldFile { repo: "alpha".into(), path: "src/a.rs".into() }],
        must_contain: s(&["Foo"]),
        must_not_contain: s(&["bar"]),
        commit: None,
    }
}

// ---- JSON

#[test]
fn json_scalars_and_nesting() {
    let v = parse_json(r#" {"a":[1,-2.5,1e2,true,false,null,"x"],"b":{"c":{}},"d":[]} "#).unwrap();
    let a = v.get("a").unwrap().as_arr().unwrap();
    assert_eq!(a[0].as_u64(), Some(1));
    assert_eq!(a[1].as_f64(), Some(-2.5));
    assert_eq!(a[1].as_u64(), None);
    assert_eq!(a[2].as_f64(), Some(100.0));
    assert_eq!(a[3], Json::Bool(true));
    assert_eq!(a[4], Json::Bool(false));
    assert_eq!(a[5], Json::Null);
    assert_eq!(a[6].as_str(), Some("x"));
    assert_eq!(v.get("b").unwrap().get("c"), Some(&Json::Obj(vec![])));
    assert_eq!(v.get("d"), Some(&Json::Arr(vec![])));
    assert_eq!(v.get("zzz"), None);
    assert_eq!(Json::Null.get("a"), None);
    assert_eq!(Json::Null.as_str(), None);
    assert_eq!(Json::Null.as_arr(), None);
    assert_eq!(Json::Null.as_f64(), None);
    assert_eq!(Json::Null.as_u64(), None);
    assert_eq!(Json::Num(-1.0).as_u64(), None);
}

#[test]
fn json_string_escapes() {
    let v = parse_json(r#""a\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00z""#).unwrap();
    assert_eq!(v.as_str(), Some("a\"\\/\u{8}\u{c}\n\r\t\u{e9}\u{1f600}z"));
}

#[test]
fn json_nesting_limit() {
    let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
    assert!(parse_json(&deep(64)).is_ok());
    assert!(parse_json(&deep(65)).unwrap_err().contains("nesting deeper than 64 levels"));
}

#[test]
fn json_errors() {
    for bad in [
        "", "nul", "tru", "@", "[1,", "[1 2]", "{\"a\" 1}", "{\"a\":1 \"b\":2}", "{1:2}", "\"abc",
        "\"\\", "\"\\x\"", "\"\\u12\"", "\"\\u12zz\"", "\"\\ud83d\"", "\"\\ud83d\\u0041\"",
        "\"\\udc00\"", "\"a\nb\"", "1 2", "-", "{", "[",
    ] {
        assert!(parse_json(bad).is_err(), "should fail: {bad:?}");
    }
}

#[test]
fn json_string_quoting() {
    assert_eq!(json_string("a\"\\\n\r\t\u{1}é"), "\"a\\\"\\\\\\n\\r\\t\\u0001é\"");
    let round = "q\"\\\n\u{2}x";
    assert_eq!(parse_json(&json_string(round)).unwrap().as_str(), Some(round));
}

// ---- validation

#[test]
fn denied_repos() {
    for r in [
        "nix-config", "fleet-infra", "estate-kit-papers", "homelab-x", "coursework", "a-course-material",
        "msc-thesis", "workspace", "My-Nedap-Tool", "Workspace",
    ] {
        assert!(is_denied(r), "{r}");
    }
    for r in ["agent-kit", "homelab", "course-material-x", "workspaces"] {
        assert!(!is_denied(r), "{r}");
    }
}

#[test]
fn valid_set_parses_and_checks() {
    let set = parse_questions(&doc(|d| d)).unwrap();
    assert_eq!(set.questions.len(), 20);
    assert_eq!(set.pins.len(), 2);
    assert_eq!(set.questions[0].gold_files[0].path, "src/a.rs");
    assert!(set.questions[0].must_not_contain.is_empty());
    assert!(check_set(&set).is_empty());
}

fn errs_of(d: String) -> Vec<String> {
    match parse_questions(&d) {
        Err(e) => e,
        Ok(set) => check_set(&set),
    }
}

fn has(errs: &[String], needle: &str) -> bool {
    errs.iter().any(|e| e.contains(needle))
}

#[test]
fn structural_errors() {
    assert!(has(&errs_of("{".into()), "invalid JSON"));
    let e = errs_of(r#"{"schema":2,"pins":[],"questions":{}}"#.into());
    assert!(has(&e, "'schema'") && has(&e, "'pins'") && has(&e, "'questions'"));
    let e = errs_of(r#"{"schema":1,"pins":{"a":"short","b":5},"questions":[]}"#.into());
    assert!(has(&e, "pin a") && has(&e, "pin b"));
    let e = errs_of(format!(
        r#"{{"schema":1,"pins":{{"a":"{SHA_A}"}},"questions":[{{}},{{"id":"X","family":1}}]}}"#
    ));
    assert!(has(&e, "question #1: missing or non-string 'id'"));
    assert!(has(&e, "question X: missing or non-string 'family'"));
    assert!(has(&e, "'gold_files'") && has(&e, "'must_contain'") && has(&e, "'gold_answer'"));
    let e = errs_of(format!(
        r#"{{"schema":1,"pins":{{"a":"{SHA_A}"}},"questions":[{{"id":"X","family":"concept","repo":"a",
        "question":"q","gold_answer":"g","gold_files":[{{"repo":"a"}}],"must_contain":[1],
        "must_not_contain":"no","commit":5}}]}}"#
    ));
    assert!(has(&e, "missing or non-string 'path'"));
    assert!(has(&e, "'must_contain' must hold only strings"));
    assert!(has(&e, "missing or non-array 'must_not_contain'"));
    assert!(has(&e, "'commit' must be a string"));
}

#[test]
fn semantic_errors() {
    let e = errs_of(doc(|d| d.replacen("name-known-1", "name-known-0", 1)));
    assert!(has(&e, "duplicate id"));
    let e = errs_of(doc(|d| d.replacen(r#""family":"concept""#, r#""family":"bogus""#, 1)));
    assert!(has(&e, "unknown family 'bogus'"));
    assert!(has(&e, "family concept: expected 5 questions, found 4"));
    let e = errs_of(doc(|d| d.replacen(r#""repo":"alpha","question""#, r#""repo":"nix-config","question""#, 1)));
    assert!(has(&e, "repo nix-config is on the deny list"));
    let e = errs_of(doc(|d| d.replacen(r#""repo":"alpha","question""#, r#""repo":"gamma","question""#, 1)));
    assert!(has(&e, "repo gamma is not in pins"));
    let e = errs_of(doc(|d| d.replacen(r#"{"repo":"alpha","path""#, r#"{"repo":"gamma","path""#, 1)));
    assert!(has(&e, "repo gamma is not in pins"));
    let e = errs_of(doc(|d| d.replace(&format!(r#""alpha":"{SHA_A}""#), r#""workspace":"cccccccccccccccccccccccccccccccccccccccc""#)));
    assert!(has(&e, "pin workspace: repo is on the deny list"));
    let e = errs_of(doc(|d| d.replace(&format!(r#""alpha":"{SHA_A}""#), &format!(r#""a b":"{SHA_A}""#))));
    assert!(has(&e, "pin a b: repo name must be letters"));
    let e = errs_of(doc(|d| d.replacen(r#""must_contain":["a.rs"]"#, r#""must_contain":[]"#, 1)));
    assert!(has(&e, "'must_contain' must not be empty"));
    let e = errs_of(doc(|d| d.replacen(r#""extra":1"#, &format!(r#""commit":"{SHA_B}""#), 1)));
    assert!(has(&e, "commit does not match the pin"));
    let e = errs_of(doc(|d| d.replacen(r#""extra":1"#, &format!(r#""commit":"{SHA_A}""#), 1)));
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn pin_checks_use_git() {
    let set = parse_questions(&doc(|d| d)).unwrap();
    let seen = RefCell::new(Vec::new());
    let exec = |i: &Invocation| {
        seen.borrow_mut().push(i.clone());
        if i.args[1].ends_with("/beta") {
            ok("").map(|mut o| {
                o.code = 1;
                o
            })
        } else {
            ok("")
        }
    };
    let errs = check_pins(&set, Path::new("/root"), &exec);
    assert_eq!(errs.len(), 1);
    assert!(errs[0].contains("pin beta"));
    let calls = seen.borrow();
    assert_eq!(calls[0].program, "git");
    assert_eq!(calls[0].args, s(&["-C", "/root/alpha", "cat-file", "-e", &format!("{SHA_A}^{{commit}}")]));
    let failing = |_: &Invocation| Err("no git".to_string());
    assert_eq!(check_pins(&set, Path::new("/r"), &failing).len(), 2);
}

// ---- prompt, adapters

#[test]
fn prompt_and_quote() {
    let p = build_prompt(&q());
    assert!(p.starts_with("Where is X?"));
    assert!(p.contains("<repo>/<path>") && p.contains("read-only"));
    assert_eq!(shell_quote("a'b"), "'a'\\''b'");
}

#[test]
fn invocations() {
    let cwd = Path::new("/snap");
    let c = build_invocation(&Agent::Claude, "m1", "P", cwd);
    assert_eq!(c.program, "claude");
    assert_eq!(c.args[..4], s(&["-p", "P", "--model", "m1"]));
    assert!(c.args.ends_with(&s(&["--allowedTools", "Read", "Grep", "Glob"])));
    assert_eq!(c.cwd.as_deref(), Some(cwd));
    let g = build_invocation(&Agent::Generic("run --m {model} \"$CODE_NAV_PROMPT\"".into()), "m'1", "P $(x)", cwd);
    assert_eq!(g.program, "sh");
    assert_eq!(g.args, s(&["-c", "run --m 'm'\\''1' \"$CODE_NAV_PROMPT\""]));
    assert_eq!(g.env, vec![("CODE_NAV_PROMPT".to_string(), "P $(x)".to_string())]);
    assert!(!g.args[1].contains("$(x)"));
    assert_eq!(Agent::Claude.name(), "claude");
    assert_eq!(Agent::Generic(String::new()).name(), "generic");
}

const STREAM: &str = r#"{"type":"system","subtype":"init"}

{"type":"assistant","message":{"content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"Read","input":{"file_path":"/s/alpha/src/a.rs"}},{"type":"tool_use","name":"Grep","input":{"pattern":"x"}},{"type":"tool_use","name":"Read","input":{}}]}}
{"type":"assistant"}
{"type":"user"}
{"type":"result","is_error":false,"result":"See alpha/src/a.rs","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":7}}
"#;

#[test]
fn claude_stream() {
    let o = parse_claude(STREAM).unwrap();
    assert_eq!(o.tool_calls, 3);
    assert_eq!(o.files_opened, s(&["/s/alpha/src/a.rs"]));
    assert_eq!(o.answer, "See alpha/src/a.rs");
    assert_eq!((o.input_tokens, o.output_tokens, o.cache_read_tokens), (10, 5, 7));
    let bare = parse_claude(r#"{"type":"result","result":"x"}"#).unwrap();
    assert_eq!((bare.answer.as_str(), bare.input_tokens), ("x", 0));
    let noresult = parse_claude(r#"{"type":"result"}"#).unwrap();
    assert_eq!(noresult.answer, "");
    assert!(parse_claude("{\"type\":\"system\"}").unwrap_err().contains("no result"));
    assert!(parse_claude("not json").unwrap_err().contains("bad stream-json"));
    assert!(parse_claude(r#"{"type":"result","is_error":true,"result":"x"}"#).is_err());
}

#[test]
fn generic_output() {
    let o = parse_generic(r#" {"answer":"a","input_tokens":3,"output_tokens":4,"cache_read_tokens":1,"tool_calls":2,"files_opened":["f",5]} "#).unwrap();
    assert_eq!(o, AgentOut { answer: "a".into(), input_tokens: 3, output_tokens: 4, cache_read_tokens: 1, tool_calls: 2, files_opened: s(&["f"]) });
    let min = parse_generic(r#"{"answer":"a"}"#).unwrap();
    assert_eq!((min.cache_read_tokens, min.files_opened.len()), (0, 0));
    assert!(parse_generic("garbage").unwrap_err().contains("malformed"));
    assert!(parse_generic("{}").unwrap_err().contains("answer"));
}

// ---- grading

#[test]
fn grading() {
    let mut k = q();
    assert_eq!(grade(&k, "FOO in src/a.rs", &[]), (true, 0.0));
    assert_eq!(grade(&k, "foo in alpha/src/a.rs", &[]), (true, 1.0));
    assert_eq!(grade(&k, "foo", &s(&["/snap/alpha/src/a.rs"])), (true, 1.0));
    assert_eq!(grade(&k, "foo", &s(&["/snap/alpha/other.rs"])), (true, 0.0));
    assert_eq!(grade(&k, "nothing", &[]), (false, 0.0));
    assert!(!grade(&k, "foo BAR", &[]).0);
    k.gold_files.push(GoldFile { repo: "alpha".into(), path: "b.rs".into() });
    assert_eq!(grade(&k, "foo alpha/src/a.rs", &[]).1, 0.5);
    k.gold_files.clear();
    assert_eq!(grade(&k, "foo", &[]), (true, 1.0));
}

// ---- run_question

#[test]
fn run_question_paths() {
    let cwd = Path::new("/snap");
    let g = Agent::Generic("x".into());
    let good = |_: &Invocation| ok(r#"{"answer":"Foo in alpha/src/a.rs","input_tokens":2,"output_tokens":3,"tool_calls":4}"#);
    let r = run_question(&q(), 2, &g, "m", cwd, &good);
    assert!(r.correct && r.error.is_none());
    assert_eq!((r.run, r.input_tokens, r.output_tokens, r.tool_calls, r.gold_recall), (2, 2, 3, 4, 1.0));
    assert_eq!((r.agent.as_str(), r.model.as_str(), r.family.as_str()), ("generic", "m", "name-known"));

    let cl = |_: &Invocation| ok(STREAM);
    let r = run_question(&q(), 1, &Agent::Claude, "m", cwd, &cl);
    assert!(!r.correct && r.error.is_none());
    assert_eq!(r.tool_calls, 3);

    let bad = |_: &Invocation| ok("nope");
    assert!(run_question(&q(), 1, &g, "m", cwd, &bad).error.unwrap().contains("malformed"));
    let nz = |_: &Invocation| Ok(Output { code: 3, stdout: String::new(), stderr: " boom\n".into() });
    let r = run_question(&q(), 1, &g, "m", cwd, &nz);
    assert_eq!(r.error.as_deref(), Some("agent exited 3: boom"));
    assert!(!r.correct);
    let fail = |_: &Invocation| Err("spawn".to_string());
    assert_eq!(run_question(&q(), 1, &g, "m", cwd, &fail).error.as_deref(), Some("spawn"));
}

// ---- results and tables

fn row(id: &str, fam: &str, model: &str, correct: bool, tok: u64, calls: u64, recall: f64) -> ResultRow {
    ResultRow {
        id: id.into(), family: fam.into(), repo: "alpha".into(), agent: "generic".into(), model: model.into(),
        run: 1, correct, gold_recall: recall, input_tokens: tok, output_tokens: 0, cache_read_tokens: 0,
        tool_calls: calls, wall_ms: 5, error: None, answer: "a \"b\"\nc".into(),
    }
}

#[test]
fn result_line_roundtrip() {
    let mut r = row("A", "concept", "m", true, 10, 2, 0.5);
    let line = r.to_json_line();
    assert!(!line.contains('\n'));
    assert!(line.contains("\"error\":null") && line.contains("a \\\"b\\\"\\nc"));
    let back = ResultRow::from_json(&parse_json(&line).unwrap()).unwrap();
    assert_eq!(back, r);
    r.error = Some("bad \"x\"".into());
    let back = ResultRow::from_json(&parse_json(&r.to_json_line()).unwrap()).unwrap();
    assert_eq!(back.error.as_deref(), Some("bad \"x\""));
    assert!(ResultRow::from_json(&parse_json("{}").unwrap()).unwrap_err().contains("'id'"));
}

#[test]
fn stats() {
    assert_eq!(median(&[]), 0.0);
    assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
    assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    assert_eq!(mean(&[]), 0.0);
    assert_eq!(mean(&[1.0, 2.0]), 1.5);
    assert_eq!(fmt_num(2.0), "2");
    assert_eq!(fmt_num(2.5), "2.5");
}

#[test]
fn tables() {
    assert_eq!(render_tables(&[]), "no results\n");
    let mut failed = row("E-1", "concept", "m1", false, 9, 1, 0.0);
    failed.error = Some("exit 2".into());
    let failed_cell = cell(&[&failed]);
    assert!(failed_cell.starts_with("0/0 ok") && failed_cell.ends_with("1 errors"), "{failed_cell}");
    assert!(cell(&[]) == "-");
    let mut other = row("B-1", "refs", "m2", true, 7, 1, 1.0);
    other.agent = "claude".into();
    let rows = vec![
        row("A-1", "concept", "m1", true, 10, 2, 1.0),
        row("A-1", "concept", "m1", false, 20, 3, 0.5),
        row("A-1", "concept", "m1", true, 30, 4, 0.0),
        row("A-1", "concept", "m1", true, 40, 5, 0.5),
        other,
    ];
    let t = render_tables(&rows);
    assert!(t.contains("| question | generic/m1 | claude/m2 |"));
    assert!(t.contains("| A-1 (concept, alpha) | 3/4 ok, 25 tok, 3.5 calls, recall 0.50 | - |"));
    assert!(t.contains("| B-1 (refs, alpha) | - | 1/1 ok, 7 tok, 1 calls, recall 1.00 |"));
    assert!(t.contains("| family | generic/m1 | claude/m2 |"));
    assert!(t.contains("| refs | - | 1/1 ok"));
    assert!(t.is_ascii());
}

#[test]
fn results_file_parsing() {
    let r = row("A", "concept", "m", true, 1, 1, 1.0);
    let text = format!("{}\n\n{}\n", r.to_json_line(), r.to_json_line());
    assert_eq!(parse_results(&text).unwrap().len(), 2);
    assert!(parse_results("{").unwrap_err().contains("line 1"));
    assert!(parse_results("{}").unwrap_err().contains("line 1"));
}

// ---- CLI

fn call(args: &[&str], exec: Exec) -> (i32, String, String) {
    let (mut o, mut e) = (Vec::new(), Vec::new());
    let code = cli(&s(args), &mut o, &mut e, exec);
    (code, String::from_utf8(o).unwrap(), String::from_utf8(e).unwrap())
}

fn fake(i: &Invocation) -> Result<Output, String> {
    match i.program.as_str() {
        "git" => ok(""),
        "sh" if i.args[0] == "-c" && i.args.len() > 2 => {
            std::fs::create_dir_all(&i.args[5]).unwrap();
            ok("")
        }
        _ => ok(r#"{"answer":"a.rs","input_tokens":1,"output_tokens":1}"#),
    }
}

#[test]
fn cli_help_and_unknown() {
    for a in [&[][..], &["help"], &["--help"], &["-h"]] {
        let (c, o, _) = call(a, &fake);
        assert_eq!(c, 0);
        assert!(o.contains("usage:"));
    }
    let (c, _, e) = call(&["bogus"], &fake);
    assert_eq!(c, 2);
    assert!(e.contains("unknown command"));
}

#[test]
fn cli_validate() {
    let dir = temp_dir();
    let good = dir.join("q.json");
    std::fs::write(&good, doc(|d| d)).unwrap();
    let g = good.to_str().unwrap();
    assert_eq!(call(&["validate", "--questions", g], &fake), (0, "ok: 20 questions\n".into(), String::new()));
    assert_eq!(call(&["validate", "--questions", g, "--repos-root", "/r"], &fake).0, 0);
    let nogit = |_: &Invocation| Err("x".to_string());
    let (c, _, e) = call(&["validate", "--questions", g, "--repos-root", "/r"], &nogit);
    assert_eq!(c, 1);
    assert!(e.contains("error: pin alpha"));
    let bad = dir.join("bad.json");
    std::fs::write(&bad, "{").unwrap();
    assert_eq!(call(&["validate", "--questions", bad.to_str().unwrap()], &fake).0, 1);
    assert_eq!(call(&["validate", "--questions", "/no/such/file"], &fake).0, 1);
    assert_eq!(call(&["validate"], &fake).0, 2);
    assert_eq!(call(&["validate", "--nope"], &fake).0, 2);
    assert_eq!(call(&["validate", "--questions"], &fake).0, 2);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_run_end_to_end() {
    let dir = temp_dir();
    let qf = dir.join("q.json");
    std::fs::write(&qf, doc(|d| d)).unwrap();
    let out = dir.join("out");
    let (qs, os) = (qf.to_str().unwrap(), out.to_str().unwrap());
    let base = ["run", "--questions", qs, "--repos-root", "/r", "--agent", "generic", "--command", "x",
        "--model", "m", "--runs", "2", "--out", os];

    let mut dry = base.to_vec();
    dry.extend(["--dry-run", "--only", "concept-0,refs-1"]);
    let (c, o, _) = call(&dry, &fake);
    assert_eq!(c, 0);
    assert_eq!(o.lines().count(), 4);
    assert!(o.contains("plan: concept-0 run 2"));
    assert!(!out.exists());

    let mut real = base.to_vec();
    real.extend(["--only", "concept-0"]);
    let (c, o, e) = call(&real, &fake);
    assert_eq!(c, 0, "{e}");
    assert!(o.contains("| concept-0 (concept, alpha) | 2/2 ok"));
    assert!(out.join("snap/alpha").is_dir() && out.join("snap/beta").is_dir());
    let text = std::fs::read_to_string(out.join("results.jsonl")).unwrap();
    assert_eq!(text.lines().count(), 2);
    // Second run appends and reuses the snapshots.
    assert_eq!(call(&real, &fake).0, 0);
    assert_eq!(std::fs::read_to_string(out.join("results.jsonl")).unwrap().lines().count(), 4);
    // The table command reads the file back.
    let (c, o, _) = call(&["table", "--results", out.join("results.jsonl").to_str().unwrap()], &fake);
    assert_eq!(c, 0);
    assert!(o.contains("Per family") && o.contains("4/4 ok"));

    // Claude agent goes through the same path.
    let claude_fake = |i: &Invocation| if i.program == "claude" { ok(STREAM) } else { fake(i) };
    let out2 = dir.join("out2");
    let c = call(&["run", "--questions", qs, "--repos-root", "/r", "--agent", "claude", "--model", "m",
        "--runs", "1", "--out", out2.to_str().unwrap(), "--only", "refs-0"], &claude_fake);
    assert_eq!(c.0, 0);
    assert!(c.1.contains("claude/m"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_run_errors() {
    let dir = temp_dir();
    let qf = dir.join("q.json");
    std::fs::write(&qf, doc(|d| d)).unwrap();
    let (qs, os) = (qf.to_str().unwrap(), dir.join("o").to_str().unwrap().to_string());
    let ok_args = |extra: &[&str]| {
        let mut v = s(&["run", "--questions", qs, "--repos-root", "/r", "--model", "m", "--out", &os]);
        v.extend(s(extra));
        v
    };
    let run = |a: Vec<String>, e: Exec| {
        let (mut o, mut er) = (Vec::new(), Vec::new());
        (cli(&a, &mut o, &mut er, e), String::from_utf8(er).unwrap())
    };
    assert_eq!(run(ok_args(&["--runs", "1"]), &fake).0, 2); // no agent
    assert_eq!(run(ok_args(&["--runs", "1", "--agent", "other"]), &fake).0, 2);
    assert_eq!(run(ok_args(&["--runs", "1", "--agent", "generic"]), &fake).0, 2); // no command
    assert_eq!(run(ok_args(&["--runs", "0", "--agent", "claude"]), &fake).0, 2);
    assert_eq!(run(ok_args(&["--runs", "x", "--agent", "claude"]), &fake).0, 2);
    assert_eq!(run(ok_args(&["--agent", "claude"]), &fake).0, 2); // no runs
    assert_eq!(run(ok_args(&["--bogus"]), &fake).0, 2);
    let (c, e) = run(ok_args(&["--runs", "1", "--agent", "claude", "--only", "nope"]), &fake);
    assert_eq!(c, 1);
    assert!(e.contains("unknown question 'nope'"));
    // Validation failure (pin missing) stops the run.
    let nogit = |_: &Invocation| Err("x".to_string());
    assert_eq!(run(ok_args(&["--runs", "1", "--agent", "claude"]), &nogit).0, 1);
    // Snapshot failure.
    let badsnap = |i: &Invocation| {
        if i.program == "sh" {
            Ok(Output { code: 1, stdout: String::new(), stderr: "tar".into() })
        } else {
            ok("")
        }
    };
    let (c, e) = run(ok_args(&["--runs", "1", "--agent", "claude"]), &badsnap);
    assert_eq!(c, 1);
    assert!(e.contains("snapshot of"));
    // Unwritable out dir: a file where the directory should be.
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, "").unwrap();
    let mut a = ok_args(&["--runs", "1", "--agent", "claude"]);
    let at = a.iter().position(|x| x == &os).unwrap();
    a[at] = blocker.to_str().unwrap().to_string();
    let (c, e) = run(a, &fake);
    assert_eq!(c, 1);
    assert!(e.contains("cannot create"));
    // results.jsonl cannot be opened: it is a directory.
    let out3 = dir.join("o3");
    std::fs::create_dir_all(out3.join("snap/alpha")).unwrap();
    std::fs::create_dir_all(out3.join("snap/beta")).unwrap();
    std::fs::write(out3.join("snap/alpha.sha"), SHA_A).unwrap();
    std::fs::write(out3.join("snap/beta.sha"), SHA_B).unwrap();
    std::fs::create_dir_all(out3.join("results.jsonl")).unwrap();
    let mut a = ok_args(&["--runs", "1", "--agent", "claude"]);
    let at = a.iter().position(|x| x == &os).unwrap();
    a[at] = out3.to_str().unwrap().to_string();
    let (c, e) = run(a, &fake);
    assert_eq!(c, 1);
    assert!(e.contains("cannot open"));
    // results.jsonl opens but every write fails: /dev/full.
    #[cfg(target_os = "linux")]
    {
        let out4 = dir.join("o4");
        std::fs::create_dir_all(out4.join("snap/alpha")).unwrap();
        std::fs::create_dir_all(out4.join("snap/beta")).unwrap();
        std::fs::write(out4.join("snap/alpha.sha"), SHA_A).unwrap();
        std::fs::write(out4.join("snap/beta.sha"), SHA_B).unwrap();
        std::os::unix::fs::symlink("/dev/full", out4.join("results.jsonl")).unwrap();
        let mut a = ok_args(&["--runs", "1", "--agent", "claude"]);
        let at = a.iter().position(|x| x == &os).unwrap();
        a[at] = out4.to_str().unwrap().to_string();
        let (c, e) = run(a, &fake);
        assert_eq!(c, 1);
        assert!(e.contains("cannot write results"));
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_table_errors() {
    assert_eq!(call(&["table"], &fake).0, 2);
    assert_eq!(call(&["table", "--x"], &fake).0, 2);
    let (c, _, e) = call(&["table", "--results", "/no/such"], &fake);
    assert_eq!(c, 1);
    assert!(e.contains("cannot read"));
    let dir = temp_dir();
    let f = dir.join("r.jsonl");
    std::fs::write(&f, "{").unwrap();
    assert_eq!(call(&["table", "--results", f.to_str().unwrap()], &fake).0, 1);
    std::fs::write(&f, "").unwrap();
    let (c, o, _) = call(&["table", "--results", f.to_str().unwrap()], &fake);
    assert_eq!((c, o.as_str()), (0, "no results\n"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn snapshot_skips_existing_and_reports_spawn_errors() {
    let dir = temp_dir();
    let ok = |_: &Invocation| -> Result<Output, String> {
        Ok(Output { code: 0, stdout: String::new(), stderr: String::new() })
    };
    let dest = dir.join("snap-repo");
    assert!(snapshot(Path::new("/r"), SHA_A, &dest, &ok).is_ok());
    assert_eq!(std::fs::read_to_string(dir.join("snap-repo.sha")).unwrap(), SHA_A);
    let boom = |_: &Invocation| -> Result<Output, String> { panic!("must not run") };
    assert!(snapshot(Path::new("/r"), SHA_A, &dest, &boom).is_ok());
    let other = snapshot(Path::new("/r"), SHA_B, &dest, &boom).unwrap_err();
    assert!(other.contains("use a new --out"));
    std::fs::create_dir_all(dir.join("unmarked")).unwrap();
    let unmarked = snapshot(Path::new("/r"), SHA_A, &dir.join("unmarked"), &boom).unwrap_err();
    assert!(unmarked.contains("no commit marker"));
    let fail = |_: &Invocation| Err("no sh".to_string());
    assert_eq!(snapshot(Path::new("/r"), SHA_A, &dir.join("new"), &fail), Err("no sh".to_string()));
    let bad = |_: &Invocation| Ok(Output { code: 1, stdout: String::new(), stderr: "boom".into() });
    assert!(snapshot(Path::new("/r"), SHA_A, &dir.join("bad"), &bad).unwrap_err().contains("boom"));
    // The commit marker cannot be written: a directory sits where it goes.
    std::fs::create_dir_all(dir.join("unmarkable.sha")).unwrap();
    let e = snapshot(Path::new("/r"), SHA_A, &dir.join("unmarkable"), &ok).unwrap_err();
    assert!(e.contains("cannot write"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn real_exec_runs_processes() {
    let inv = Invocation {
        program: "sh".into(),
        args: s(&["-c", "echo $CNE_X; echo err >&2; pwd; exit 3"]),
        env: vec![("CNE_X".into(), "hi".into())],
        cwd: Some(std::env::temp_dir()),
    };
    let o = real_exec(&inv).unwrap();
    assert_eq!(o.code, 3);
    assert!(o.stdout.starts_with("hi\n") && o.stderr == "err\n");
    let nocwd = Invocation { cwd: None, args: s(&["-c", "kill -9 $$"]), ..inv.clone() };
    assert_eq!(real_exec(&nocwd).unwrap().code, -1);
    let missing = Invocation { program: "/no/such/prog".into(), ..inv };
    assert!(real_exec(&missing).unwrap_err().contains("cannot run"));
}

#[test]
fn run_entry_prints_help() {
    assert_eq!(run(vec![OsString::from("--help")]), 0);
}
