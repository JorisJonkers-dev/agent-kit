use crate::cli::run;
use crate::json::{self, Object, Value};
use crate::{mcp_json, mcp_toml, mcp_yaml, proxy, text, Failure};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A throwaway directory, removed when it goes out of scope.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Scratch {
        let dir = std::env::temp_dir().join(format!("agent-kit-merge-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names_of(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

fn fragment(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..").join(relative)
}

struct Ran {
    status: i32,
    out: String,
    err: String,
}

fn exec(args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Ran {
    exec_bytes(args, stdin.as_bytes(), env)
}

fn exec_bytes(args: &[&str], stdin: &[u8], env: &[(&str, &str)]) -> Ran {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let mut input = stdin;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let lookup = |name: &str| env.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string());
    let status = {
        let mut io = crate::Io { stdin: &mut input, out: &mut out, err: &mut err };
        run(&args, &mut io, &lookup)
    };
    Ran { status, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap() }
}

fn p(path: &Path) -> &str {
    path.to_str().unwrap()
}

// ---------------------------------------------------------------------------
// text and json
// ---------------------------------------------------------------------------

#[test]
fn splitlines_breaks_where_python_does() {
    assert_eq!(text::splitlines("a\nb\r\nc\rd\u{b}e\u{2028}f"), vec!["a\n", "b\r\n", "c\r", "d\u{b}", "e\u{2028}", "f"]);
    assert_eq!(text::splitlines(""), Vec::<String>::new());
    assert_eq!(text::splitlines("x\n"), vec!["x\n"]);
}

#[test]
fn strip_and_universal_newlines_and_repr() {
    assert_eq!(text::strip(" \u{1c}x \n"), "x");
    assert_eq!(text::universal("a\r\nb\rc\n"), "a\nb\nc\n");
    assert_eq!(text::repr("it's"), "\"it's\"");
    assert_eq!(text::repr("a'\"b"), "'a\\'\"b'");
    assert_eq!(text::repr("a\\\n\r\t\u{1}\u{7f}é"), "'a\\\\\\n\\r\\t\\x01\\x7fé'");
    assert_eq!(crate::repr_list(&["a".to_string(), "b".to_string()]), "['a', 'b']");
}

#[test]
fn json_round_trips_the_way_python_writes_it() {
    let source = r#"{"b": [1, 2.50, -0, 1e5, true, false, null], "a": {"é": "\u00e9\ud83d\ude00\n\t\b\f\r\"\\\/", "e": [], "o": {}}, "z": 0.1}"#;
    let value = json::parse(source).unwrap();
    let expected = "{\n  \"b\": [\n    1,\n    2.5,\n    0,\n    100000.0,\n    true,\n    false,\n    null\n  ],\n  \"a\": {\n    \"\\u00e9\": \"\\u00e9\\ud83d\\ude00\\n\\t\\b\\f\\r\\\"\\\\/\",\n    \"e\": [],\n    \"o\": {}\n  },\n  \"z\": 0.1\n}";
    assert_eq!(json::dumps(&value), expected);
    assert_eq!(json::dumps(&json::parse(expected).unwrap()), expected);
}

#[test]
fn json_floats_print_as_python_repr() {
    let value = json::parse("[1e16, 1.5e-7, 1E2, 12345678901234567.0, 1e999, -1e999, 0.0001, 0.00001, 1e22]").unwrap();
    let Value::Arr(items) = value else { panic!() };
    let shown: Vec<String> = items.iter().map(|v| if let Value::Num(n) = v { n.clone() } else { String::new() }).collect();
    assert_eq!(shown, ["1e+16", "1.5e-07", "100.0", "1.2345678901234568e+16", "Infinity", "-Infinity", "0.0001", "1e-05", "1e+22"]);
}

#[test]
fn json_keeps_an_objects_order_and_a_repeated_key_keeps_its_place() {
    let value = json::parse(r#"{"b": 1, "a": 2, "b": 3}"#).unwrap();
    let object = value.as_object().unwrap();
    assert_eq!(object.keys(), ["b", "a"]);
    assert_eq!(object.get("b"), Some(&Value::Num("3".into())));
    assert!(object.contains("a") && !object.contains("c"));
    assert_eq!(Value::Null.as_object(), None);
}

#[test]
fn json_falsy_values() {
    for falsy in ["null", "false", "0", "0.0", "\"\"", "[]", "{}"] {
        assert!(json::parse(falsy).unwrap().is_falsy(), "{falsy}");
    }
    for truthy in ["true", "1", "\"x\"", "[1]", "{\"a\": 1}"] {
        assert!(!json::parse(truthy).unwrap().is_falsy(), "{truthy}");
    }
}

#[test]
fn json_errors_read_as_python_words_them() {
    let cases = [
        ("", "Expecting value: line 1 column 1 (char 0)"),
        ("{} x", "Extra data: line 1 column 4 (char 3)"),
        ("[1 2]", "Expecting ',' delimiter: line 1 column 4 (char 3)"),
        ("[1,", "Expecting value: line 1 column 4 (char 3)"),
        ("{\"a\" 1}", "Expecting ':' delimiter: line 1 column 6 (char 5)"),
        ("{1: 2}", "Expecting property name enclosed in double quotes: line 1 column 2 (char 1)"),
        ("{\"a\": 1 \"b\"}", "Expecting ',' delimiter: line 1 column 9 (char 8)"),
        ("\n\"abc", "Unterminated string starting at: line 2 column 1 (char 1)"),
        ("\"ab\\", "Unterminated string starting at: line 1 column 1 (char 0)"),
        ("\"a\\x\"", "Invalid \\escape: line 1 column 3 (char 2)"),
        ("\"a\u{1}\"", "Invalid control character at: line 1 column 3 (char 2)"),
        ("\"\\u12G4\"", "Invalid \\uXXXX escape: line 1 column 3 (char 2)"),
        ("01", "Expecting value: line 1 column 1 (char 0)"),
        ("-", "Expecting value: line 1 column 1 (char 0)"),
        ("// c\n{}", "Expecting value: line 1 column 1 (char 0)"),
    ];
    for (source, message) in cases {
        assert_eq!(json::parse(source), Err(Failure(message.to_string())), "{source:?}");
    }
    // a lone surrogate cannot live in a Rust string
    assert_eq!(json::parse("\"\\ud800\"").unwrap(), Value::Str("\u{FFFD}".into()));
    assert_eq!(json::parse("\"\\ud800\\u0041\"").unwrap(), Value::Str("\u{FFFD}A".into()));
    assert!(json::parse("0.5").is_ok() && json::parse("-0.5e1").is_ok() && json::parse("0e0").is_ok());
}

#[test]
fn io_errors_become_failures() {
    let failure: Failure = std::io::Error::other("boom").into();
    assert_eq!(failure, Failure("boom".to_string()));
}

// ---------------------------------------------------------------------------
// instruction-block (ported from tests/test_model_gateway_reach.py)
// ---------------------------------------------------------------------------

const BEGIN: &str = "<!-- agent-kit model-gateway begin -->";
const END: &str = "<!-- agent-kit model-gateway end -->";

fn marked(body: &str) -> String {
    format!("{BEGIN}\n{body}\n{END}\n")
}

fn helper(file: &Path, body: &str) -> Ran {
    exec(&["instruction-block", p(file), "model-gateway"], body, &[])
}

#[test]
fn the_helper_appends_its_block_to_a_missing_file() {
    let dir = Scratch::new();
    let file = dir.path("sub/AGENTS.md");
    let ran = helper(&file, "Pointer line.");
    assert_eq!((ran.status, ran.out.trim()), (0, "changed"));
    assert_eq!(fs::read_to_string(&file).unwrap(), marked("Pointer line."));
}

#[test]
fn the_helper_is_unchanged_on_a_second_run_with_the_same_body() {
    let dir = Scratch::new();
    let file = dir.path("AGENTS.md");
    helper(&file, "Pointer line.");
    assert_eq!(helper(&file, "Pointer line.\n\n").out.trim(), "unchanged");
}

#[test]
fn the_helper_replaces_its_block_in_place_and_keeps_the_text_around_it() {
    let dir = Scratch::new();
    let file = dir.write("AGENTS.md", &format!("Before.\n\n{}After.\n", marked("old")));
    let ran = helper(&file, "new");
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert_eq!(fs::read_to_string(&file).unwrap(), format!("Before.\n\n{}After.\n", marked("new")));
}

#[test]
fn the_helper_keeps_the_first_block_and_drops_a_repeat() {
    let dir = Scratch::new();
    let file = dir.write("AGENTS.md", &format!("{}mid\n{}", marked("one"), marked("two")));
    helper(&file, "new");
    assert_eq!(fs::read_to_string(&file).unwrap(), format!("{}mid\n", marked("new")));
}

#[test]
fn the_helper_appends_after_text_with_no_trailing_newline_and_after_a_blank_line() {
    let dir = Scratch::new();
    let file = dir.write("a.md", "Mine.");
    helper(&file, "x");
    assert_eq!(fs::read_to_string(&file).unwrap(), format!("Mine.\n{}", marked("x")));
    let file = dir.write("b.md", "Mine.\n\n");
    helper(&file, "x");
    assert_eq!(fs::read_to_string(&file).unwrap(), format!("Mine.\n\n{}", marked("x")));
}

#[test]
fn the_helper_leaves_a_file_with_an_unclosed_block_alone() {
    let dir = Scratch::new();
    let original = format!("Mine.\n{BEGIN}\nhalf a block\n");
    let file = dir.write("AGENTS.md", &original);
    let ran = helper(&file, "new");
    assert_eq!(ran.status, 2);
    assert!(ran.err.contains("model-gateway") && ran.err.contains("never closed"), "{}", ran.err);
    assert_eq!(fs::read_to_string(&file).unwrap(), original);
}

#[test]
fn the_helper_removes_only_its_block() {
    let dir = Scratch::new();
    let file = dir.write("AGENTS.md", &format!("Before.\n\n{}After.\n", marked("pointer")));
    let ran = exec(&["instruction-block", p(&file), "model-gateway", "--remove"], "", &[]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert_eq!(fs::read_to_string(&file).unwrap(), "Before.\n\nAfter.\n");
}

#[test]
fn the_helper_removing_from_a_file_without_its_block_changes_nothing() {
    let dir = Scratch::new();
    let file = dir.write("AGENTS.md", "Mine.\n");
    let ran = exec(&["instruction-block", p(&file), "model-gateway", "--remove"], "", &[]);
    assert_eq!(ran.out.trim(), "unchanged");
    assert_eq!(fs::read_to_string(&file).unwrap(), "Mine.\n");
}

#[test]
fn the_helper_keeps_other_named_blocks_in_the_same_file() {
    let dir = Scratch::new();
    let other = "<!-- agent-kit other begin -->\nother\n<!-- agent-kit other end -->\n";
    let file = dir.write("AGENTS.md", other);
    helper(&file, "pointer");
    assert!(fs::read_to_string(&file).unwrap().starts_with(other));
}

#[test]
fn the_helper_keeps_a_crlf_file_byte_for_byte_around_its_block() {
    let dir = Scratch::new();
    let crlf = |s: String| s.replace('\n', "\r\n");
    let file = dir.write("AGENTS.md", &crlf(format!("Mine.\n\n{}Tail.\n", marked("old"))));
    let ran = helper(&file, "new");
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert_eq!(fs::read(&file).unwrap(), crlf(format!("Mine.\n\n{}Tail.\n", marked("new"))).into_bytes());
}

#[test]
fn the_helper_reads_a_crlf_body_as_lines() {
    let dir = Scratch::new();
    let file = dir.path("AGENTS.md");
    helper(&file, "one\r\ntwo\rthree\n");
    assert_eq!(fs::read_to_string(&file).unwrap(), marked("one\ntwo\nthree"));
}

#[test]
fn the_helper_refuses_a_body_that_is_not_text() {
    let dir = Scratch::new();
    let file = dir.path("a.md");
    let ran = exec_bytes(&["instruction-block", p(&file), "x"], &[0xff, 0xfe], &[]);
    assert_eq!(ran.status, 2);
    assert!(ran.err.contains("agent-kit-merge:"));
    let ran = exec_bytes(&["package-proxy", p(&file), "x"], &[0xff, 0xfe], &[]);
    assert_eq!(ran.status, 2);
}

// ---------------------------------------------------------------------------
// package-proxy (the marked # block)
// ---------------------------------------------------------------------------

fn proxy_run(file: &Path, extra: &[&str], body: &str) -> Ran {
    let mut args = vec!["package-proxy", p(file), "packages-proxy"];
    args.extend_from_slice(extra);
    exec(&args, body, &[])
}

const PBEGIN: &str = "# agent-kit packages-proxy begin";
const PEND: &str = "# agent-kit packages-proxy end";

#[test]
fn a_fresh_file_gets_the_block_and_the_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new();
    let file = dir.path("deep/.npmrc");
    let ran = proxy_run(&file, &["--mode", "600", "--refuse", "^registry="], "registry=https://p/\n//p/:_authToken=sek\n");
    assert_eq!((ran.status, ran.out.trim()), (0, "changed"));
    assert_eq!(fs::read_to_string(&file).unwrap(), format!("{PBEGIN}\nregistry=https://p/\n//p/:_authToken=sek\n{PEND}\n"));
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
    assert!(!dir.path("deep/.npmrc.bak-agent-kit").exists());
    let again = proxy_run(&file, &["--mode", "600"], "registry=https://p/\n//p/:_authToken=sek\n");
    assert_eq!(again.out.trim(), "unchanged");
}

#[test]
fn lines_outside_the_block_survive_byte_for_byte_and_the_first_backup_stays() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new();
    let original = "# mine\n//npm.pkg.github.com/:_authToken=ghp_secret\nfoo=bar";
    let file = dir.write("npmrc", original);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(proxy_run(&file, &[], "a=1\n").status, 0);
    let first = fs::read_to_string(&file).unwrap();
    assert_eq!(first, format!("{original}\n\n{PBEGIN}\na=1\n{PEND}\n"));
    assert_eq!(fs::read_to_string(dir.path("npmrc.bak-agent-kit")).unwrap(), original);
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o640);
    assert_eq!(proxy_run(&file, &[], "b=2\n").status, 0);
    assert_eq!(fs::read_to_string(&file).unwrap(), format!("{original}\n\n{PBEGIN}\nb=2\n{PEND}\n"));
    assert_eq!(fs::read_to_string(dir.path("npmrc.bak-agent-kit")).unwrap(), original);
}

#[test]
fn an_existing_files_mode_is_pinned_when_one_is_asked_for() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new();
    let file = dir.write("c", "x=1\n");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    proxy_run(&file, &["--mode=600"], "a=1\n");
    assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
}

#[test]
fn a_block_in_place_is_replaced_and_crlf_reads_as_lf() {
    let dir = Scratch::new();
    let file = dir.write("c", &format!("a\r\n{PBEGIN}\r\nold\r\n{PEND}\r\nz\r\n"));
    assert_eq!(proxy_run(&file, &[], "new\r\nlines\r\n").out.trim(), "changed");
    assert_eq!(fs::read_to_string(&file).unwrap(), format!("a\n{PBEGIN}\nnew\nlines\n{PEND}\nz\n"));
}

#[test]
fn a_refused_line_names_the_property_never_the_value() {
    let dir = Scratch::new();
    let file = dir.write("npmrc", "a=1\nregistry=https://secret@host/\n");
    let ran = proxy_run(&file, &["--refuse", "^registry="], "registry=https://p/\n");
    assert_eq!(ran.status, 3);
    assert!(ran.err.contains("line 2 ('registry')"), "{}", ran.err);
    assert!(!ran.err.contains("secret"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "a=1\nregistry=https://secret@host/\n");
    let bare = dir.write("bare", "[global]\n");
    let ran = proxy_run(&bare, &["--refuse", "^\\[global\\]"], "x\n");
    assert_eq!(ran.status, 3);
    assert!(ran.err.contains("line 1 is outside"), "{}", ran.err);
}

#[test]
fn the_conflict_check_ignores_our_own_block() {
    let dir = Scratch::new();
    let file = dir.write("npmrc", &format!("{PBEGIN}\nregistry=https://p/\n{PEND}\n"));
    let ran = proxy_run(&file, &["--refuse", "^registry="], "registry=https://p/\n");
    assert_eq!((ran.status, ran.out.trim()), (0, "unchanged"));
}

#[test]
fn a_probe_with_remove_reports_the_conflict_without_writing() {
    let dir = Scratch::new();
    let file = dir.write("pip.conf", "[global]\nx = 1\n");
    let ran = proxy_run(&file, &["--remove", "--refuse", "^\\[global\\]"], "");
    assert_eq!(ran.status, 3);
    assert_eq!(fs::read_to_string(&file).unwrap(), "[global]\nx = 1\n");
}

#[test]
fn remove_takes_the_block_out_and_a_missing_block_is_unchanged() {
    let dir = Scratch::new();
    let file = dir.write("c", &format!("a\n{PBEGIN}\nx\n{PEND}\nb\n"));
    assert_eq!(proxy_run(&file, &["--remove"], "").out.trim(), "changed");
    assert_eq!(fs::read_to_string(&file).unwrap(), "a\nb\n");
    assert_eq!(proxy_run(&file, &["--remove"], "").out.trim(), "unchanged");
    assert_eq!(proxy_run(&dir.path("none"), &["--remove"], "").out.trim(), "unchanged");
    assert!(!dir.path("none").exists());
}

#[test]
fn an_unclosed_block_is_left_alone_with_status_2() {
    let dir = Scratch::new();
    let original = format!("a\n{PBEGIN}\nhalf\n");
    let file = dir.write("c", &original);
    let ran = proxy_run(&file, &[], "x\n");
    assert_eq!(ran.status, 2);
    assert!(ran.err.contains("opened at line 2 is never closed"), "{}", ran.err);
    assert_eq!(fs::read_to_string(&file).unwrap(), original);
}

#[test]
fn an_empty_body_and_a_bad_regex_and_a_bad_mode_are_status_2() {
    let dir = Scratch::new();
    let file = dir.path("c");
    let ran = proxy_run(&file, &[], "");
    assert_eq!((ran.status, ran.err.trim()), (2, "package-proxy-merge: the block body is empty"));
    let ran = proxy_run(&file, &["--refuse", "("], "x\n");
    assert_eq!(ran.status, 2);
    assert!(ran.err.contains("'(' is not a regex"), "{}", ran.err);
    let ran = proxy_run(&file, &["--mode", "9z"], "x\n");
    assert_eq!(ran.status, 2);
    assert!(ran.err.contains("'9z' is not an octal mode"), "{}", ran.err);
    assert!(!file.exists());
}

#[test]
fn a_merge_that_does_not_parse_is_refused_before_anything_is_written() {
    let dir = Scratch::new();
    let toml = dir.write("uv.toml", "[tool]\nx = 1\n");
    let ran = proxy_run(&toml, &["--validate", "toml"], "this is = = not toml\n");
    assert_eq!(ran.status, 3);
    assert!(ran.err.contains("the merged result does not parse"), "{}", ran.err);
    assert_eq!(fs::read_to_string(&toml).unwrap(), "[tool]\nx = 1\n");
    assert!(!dir.path("uv.toml.bak-agent-kit").exists());
    assert_eq!(proxy_run(&toml, &["--validate", "toml"], "[[index]]\nurl = \"u\"\n").status, 0);
}

fn ini(text: &str) -> Option<String> {
    proxy::parse_problem(proxy::Validate::Ini, text)
}

#[test]
fn ini_validation_follows_configparser_strict() {
    assert_eq!(ini("[global]\nindex-url = https://p\n"), None);
    assert_eq!(ini("# c\n; c\n\n[a]\nk: v\n  continued\n\n  more\nk2 = x\n[b]\nk = y\n"), None);
    assert_eq!(ini("[a] trailing\nk = v\n"), None);
    assert!(ini("k = v\n").unwrap().contains("File contains no section headers"));
    assert!(ini("[a]\nnovalue\n").unwrap().contains("parsing errors"));
    assert!(ini("[a]\n= v\n").unwrap().contains("parsing errors"));
    assert!(ini("[a]\n[a]\n").unwrap().contains("section 'a' already exists"));
    assert!(ini("[a]\nK = 1\nk = 2\n").unwrap().contains("option 'k' in section already exists"));
    assert!(ini("[]\nk=1\n").unwrap().contains("no section headers"));
    // an indented option under no section is still no header
    assert!(ini("  k = 1\n").unwrap().contains("no section headers"));
}

#[test]
fn a_pip_conf_merge_validates_as_ini() {
    let dir = Scratch::new();
    let file = dir.write("pip.conf", "[global]\ntimeout = 5\n");
    let ran = proxy_run(&file, &["--validate", "ini", "--refuse", "^index-url"], "index-url = https://p/simple/\n");
    assert_eq!(ran.status, 0, "{}", ran.err);
    let bad = proxy_run(&file, &["--validate", "ini"], "no delimiter here\n");
    assert_eq!(bad.status, 3);
}

#[test]
fn proxy_helpers_on_their_own() {
    let lines: Vec<String> = ["a\n", "# agent-kit n begin\n", "b\n", "# agent-kit n end\n", "c"].iter().map(|s| s.to_string()).collect();
    let (begin, end) = proxy::markers("n");
    assert_eq!(proxy::block_bounds(&lines, &begin, &end), Ok(Some((1, 3))));
    assert_eq!(proxy::block_bounds(&lines[..1], &begin, &end), Ok(None));
    assert_eq!(proxy::without_block(&lines, &begin, &end), ["a\n", "c"]);
    // a block that ends the file without a newline before it
    let tight: Vec<String> = ["a", "# agent-kit n begin\n", "# agent-kit n end"].iter().map(|s| s.to_string()).collect();
    assert_eq!(proxy::without_block(&tight, &begin, &end), ["a\n"]);
    assert_eq!(proxy::append_block(&["a".to_string()], &["B\n".to_string()]), ["a\n", "\n", "B\n"]);
    assert_eq!(proxy::append_block(&[], &["B\n".to_string()]), ["B\n"]);
}

// ---------------------------------------------------------------------------
// antigravity (ported from tests/test_antigravity_merge.py)
// ---------------------------------------------------------------------------

const AGY: &str = "registry/generated/antigravity/mcp_config.json";

fn antigravity(config: &Path, extra: &[&str]) -> Ran {
    let frag = fragment(AGY);
    let mut args = vec!["antigravity-mcp", p(&frag), "--config", p(config)];
    args.extend_from_slice(extra);
    exec(&args, "", &[])
}

fn servers(config: &Path) -> Object {
    let doc = json::parse(&fs::read_to_string(config).unwrap()).unwrap();
    doc.as_object().unwrap().get("mcpServers").unwrap().as_object().unwrap().clone()
}

fn fragment_servers(relative: &str) -> Object {
    let doc = json::parse(&fs::read_to_string(fragment(relative)).unwrap()).unwrap();
    doc.as_object().unwrap().get("mcpServers").unwrap().as_object().unwrap().clone()
}

#[test]
fn antigravity_keeps_the_users_servers_and_settings() {
    let dir = Scratch::new();
    let config = dir.write("mcp_config.json", r#"{"theme": "dark", "mcpServers": {"mine": {"command": "my-mcp", "args": []}, "memory-api": {"serverUrl": "https://stale.example/mcp/"}}}"#);
    let ran = antigravity(&config, &[]);
    assert_eq!(ran.status, 0, "{}{}", ran.out, ran.err);
    assert!(ran.out.contains("managed servers in"));
    let doc = json::parse(&fs::read_to_string(&config).unwrap()).unwrap();
    assert_eq!(doc.as_object().unwrap().get("theme"), Some(&Value::Str("dark".into())));
    let now = servers(&config);
    assert_eq!(json::dumps(now.get("mine").unwrap()), "{\n  \"command\": \"my-mcp\",\n  \"args\": []\n}");
    assert_eq!(now.get("memory-api"), fragment_servers(AGY).get("memory-api"));
}

#[test]
fn antigravity_second_run_changes_nothing() {
    let dir = Scratch::new();
    let config = dir.write("mcp_config.json", r#"{"mcpServers": {"mine": {"command": "my-mcp"}}}"#);
    assert_eq!(antigravity(&config, &[]).status, 0);
    let first = fs::read(&config).unwrap();
    assert_eq!(antigravity(&config, &[]).status, 0);
    assert_eq!(fs::read(&config).unwrap(), first);
}

#[test]
fn antigravity_first_backup_is_kept() {
    let dir = Scratch::new();
    let original = r#"{"mcpServers": {"mine": {"command": "my-mcp"}}}"#;
    let config = dir.write("mcp_config.json", original);
    assert_eq!(antigravity(&config, &[]).status, 0);
    let backup = dir.path("mcp_config.json.bak");
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);
    fs::write(&config, r#"{"mcpServers": {"mine": {"command": "changed"}}}"#).unwrap();
    assert_eq!(antigravity(&config, &[]).status, 0);
    assert_eq!(fs::read_to_string(&backup).unwrap(), original);
}

#[test]
fn antigravity_remove_drops_a_server() {
    let dir = Scratch::new();
    let config = dir.write("mcp_config.json", r#"{"mcpServers": {"retired": {"command": "old-mcp"}, "mine": {"command": "my-mcp"}}}"#);
    let ran = antigravity(&config, &["--remove", "retired", "--remove", "drawio"]);
    assert_eq!(ran.status, 0, "{}{}", ran.out, ran.err);
    let now = servers(&config);
    assert!(!now.contains("retired") && !now.contains("drawio"));
    assert!(now.contains("mine"));
}

#[test]
fn antigravity_creates_a_missing_config_without_a_backup() {
    let dir = Scratch::new();
    let config = dir.path("gemini/config/mcp_config.json");
    assert_eq!(antigravity(&config, &[]).status, 0);
    assert_eq!(servers(&config), fragment_servers(AGY));
    assert!(!dir.path("gemini/config/mcp_config.json.bak").exists());
}

#[test]
fn antigravity_treats_the_empty_file_a_fresh_agy_leaves_as_no_config() {
    let dir = Scratch::new();
    let config = dir.write("mcp_config.json", "");
    assert_eq!(antigravity(&config, &[]).status, 0);
    assert_eq!(servers(&config), fragment_servers(AGY));
}

#[test]
fn antigravity_keeps_a_server_the_user_disabled_disabled() {
    let dir = Scratch::new();
    let config = dir.write("mcp_config.json", r#"{"mcpServers": {"drawio": {"command": "old-drawio", "disabled": true}}}"#);
    assert_eq!(antigravity(&config, &[]).status, 0);
    let fragment = fragment_servers(AGY);
    let mut want = fragment.get("drawio").unwrap().as_object().unwrap().clone();
    want.set("disabled", Value::Bool(true));
    assert_eq!(servers(&config).get("drawio"), Some(&Value::Obj(want)));
    let first = fs::read(&config).unwrap();
    assert_eq!(antigravity(&config, &[]).status, 0);
    assert_eq!(fs::read(&config).unwrap(), first);
}

#[test]
fn antigravity_writes_non_ascii_the_way_python_does_and_treats_falsy_servers_as_none() {
    let dir = Scratch::new();
    let config = dir.write("c.json", r#"{"mcpServers": null, "n": "é"}"#);
    assert_eq!(antigravity(&config, &[]).status, 0);
    assert!(fs::read_to_string(&config).unwrap().contains("\"n\": \"\\u00e9\""));
}

#[test]
fn antigravity_stops_on_a_config_it_cannot_read() {
    let dir = Scratch::new();
    let frag = fragment(AGY);
    for bad in ["{oops", "[1]", r#"{"mcpServers": 3}"#] {
        let config = dir.write("c.json", bad);
        let ran = exec(&["antigravity-mcp", p(&frag), "--config", p(&config)], "", &[]);
        assert_eq!(ran.status, 2, "{bad}");
        assert_eq!(fs::read_to_string(&config).unwrap(), bad);
    }
    let odd = dir.write("frag.json", r#"{"mcpServers": {"a": "not an object"}}"#);
    let ran = exec(&["antigravity-mcp", p(&odd), "--config", p(&dir.path("fresh.json"))], "", &[]);
    assert_eq!(ran.status, 2);
    assert!(ran.err.contains("not an object"), "{}", ran.err);
    assert_eq!(exec(&["antigravity-mcp", p(&dir.path("missing.json"))], "", &[("HOME", p(&dir.0))]).status, 2);
}

#[test]
fn antigravity_default_config_is_under_home() {
    let dir = Scratch::new();
    let frag = dir.write("f.json", r#"{"mcpServers": {"a": {"command": "x"}}}"#);
    let ran = exec(&["antigravity-mcp", p(&frag)], "", &[("HOME", p(&dir.0))]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert!(dir.path(".gemini/config/mcp_config.json").is_file());
}

#[test]
fn json_verification_names_what_did_not_land() {
    let mut managed = Object::default();
    managed.set("a", Value::Num("1".into()));
    managed.set("b", Value::Num("2".into()));
    let mut back = Object::default();
    back.set("a", Value::Num("1".into()));
    back.set("gone", Value::Null);
    let dropped = ["gone".to_string(), "gone".to_string()];
    assert_eq!(
        mcp_json::verify("t", &managed, &back, &dropped),
        Some("t: FAILED, not as written: ['b'], still present: ['gone']".to_string())
    );
    back.set("b", Value::Num("2".into()));
    assert_eq!(mcp_json::verify("t", &managed, &back, &[]), None);
}

// ---------------------------------------------------------------------------
// opencode (ported from tests/test_opencode.py and test_opencode_review.py)
// ---------------------------------------------------------------------------

const OC: &str = "registry/generated/opencode/opencode.json";

fn opencode(config: &Path, extra: &[&str]) -> Ran {
    let frag = fragment(OC);
    let mut args = vec!["opencode-mcp", p(&frag), "--config", p(config)];
    args.extend_from_slice(extra);
    exec(&args, "", &[])
}

fn oc_servers(text: &str) -> Object {
    let doc = json::parse(text).unwrap();
    doc.as_object().unwrap().get("mcp").unwrap().as_object().unwrap().get("servers").unwrap().as_object().unwrap().clone()
}

#[test]
fn opencode_merge_is_idempotent_and_backs_up_the_first_version_only() {
    let dir = Scratch::new();
    let config = dir.write("opencode.json", r#"{"model": "x"}"#);
    assert_eq!(opencode(&config, &[]).status, 0);
    let first = fs::read_to_string(&config).unwrap();
    assert_eq!(opencode(&config, &[]).status, 0);
    assert_eq!(fs::read_to_string(&config).unwrap(), first);
    assert_eq!(fs::read_to_string(dir.path("opencode.json.bak")).unwrap(), r#"{"model": "x"}"#);
    assert!(first.contains("\"$schema\": \"https://opencode.ai/config.json\""));
    assert!(!dir.path("opencode.json.agent-kit-tmp").exists());
}

#[test]
fn opencode_merge_writes_a_fresh_config_and_a_second_merge_writes_nothing_new() {
    let dir = Scratch::new();
    let home = dir.path("new");
    let config = home.join("opencode.json");
    assert_eq!(opencode(&config, &[]).status, 0);
    let fragment_text = fs::read_to_string(fragment(OC)).unwrap();
    assert_eq!(oc_servers(&fs::read_to_string(&config).unwrap()), oc_servers(&fragment_text));
    let before = names_of(&home);
    assert_eq!(opencode(&config, &[]).status, 0);
    assert_eq!(names_of(&home), before);
}

#[test]
fn opencode_merge_refuses_a_config_with_comments() {
    let dir = Scratch::new();
    let text = "{\n  // mine\n  \"model\": \"x\"\n}\n";
    let config = dir.write("opencode.json", text);
    let ran = opencode(&config, &[]);
    assert_eq!(ran.status, 1);
    assert!(ran.err.contains("is not plain JSON"), "{}", ran.err);
    assert_eq!(fs::read_to_string(&config).unwrap(), text);
}

#[test]
fn opencode_keeps_what_the_user_set_and_the_other_mcp_keys() {
    let dir = Scratch::new();
    let name = oc_servers(&fs::read_to_string(fragment(OC)).unwrap()).keys()[0].clone();
    let existing = format!(r#"{{"mcp": {{"extra": 1, "servers": {{"mine": {{"type": "local"}}, "{name}": {{"enabled": false, "type": "stale"}}}}}}, "$schema": "mine"}}"#);
    let config = dir.write("opencode.json", &existing);
    let ran = opencode(&config, &["--remove", "retired"]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    let text = fs::read_to_string(&config).unwrap();
    let now = oc_servers(&text);
    assert!(now.contains("mine"));
    assert_eq!(now.get(&name).unwrap().as_object().unwrap().get("enabled"), Some(&Value::Bool(false)));
    assert!(text.contains("\"extra\": 1") && text.contains("\"$schema\": \"mine\""));
    // the file already held a managed server, so it is not the owner's original
    assert!(!dir.path("opencode.json.bak").exists());
}

#[test]
fn opencode_default_config_honours_xdg_config_home_and_home() {
    let dir = Scratch::new();
    let frag = fragment(OC);
    let xdg = dir.path("xdg");
    assert_eq!(exec(&["opencode-mcp", p(&frag)], "", &[("XDG_CONFIG_HOME", p(&xdg)), ("HOME", "/nowhere")]).status, 0);
    assert!(xdg.join("opencode/opencode.json").is_file());
    assert_eq!(exec(&["opencode-mcp", p(&frag)], "", &[("XDG_CONFIG_HOME", ""), ("HOME", p(&dir.0))]).status, 0);
    assert!(dir.path(".config/opencode/opencode.json").is_file());
}

#[test]
fn opencode_handles_odd_shapes() {
    let dir = Scratch::new();
    // a fragment with no servers, and a config whose mcp is not an object
    let empty = dir.write("empty.json", "{}");
    let config = dir.write("c.json", r#"{"mcp": 3}"#);
    assert_eq!(exec(&["opencode-mcp", p(&empty), "--config", p(&config)], "", &[]).status, 0);
    assert!(fs::read_to_string(&config).unwrap().contains("\"servers\": {}"));
    let bad = dir.write("bad.json", r#"{"mcp": {"servers": {"a": 5}}}"#);
    let ran = exec(&["opencode-mcp", p(&bad), "--config", p(&dir.path("o.json"))], "", &[]);
    assert_eq!(ran.status, 2);
    let ran = exec(&["opencode-mcp", p(&dir.path("absent.json")), "--config", p(&config)], "", &[]);
    assert_eq!(ran.status, 2);
}

// ---------------------------------------------------------------------------
// vibe (ported from tests/test_vibe.py) and codex
// ---------------------------------------------------------------------------

const VIBE: &str = "registry/generated/vibe/mcp-servers.toml";

fn vibe(config: &Path, extra: &[&str]) -> Ran {
    let frag = fragment(VIBE);
    let mut args = vec!["vibe-mcp", p(&frag), "--config", p(config)];
    args.extend_from_slice(extra);
    exec(&args, "", &[])
}

fn names_in(config: &Path, key: &str) -> Vec<String> {
    let doc: toml_edit::DocumentMut = fs::read_to_string(config).unwrap().parse().unwrap();
    match doc.get(key) {
        Some(toml_edit::Item::ArrayOfTables(tables)) => tables.iter().map(mcp_toml::entry_name).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn vibe_merge_keeps_the_users_settings_and_servers() {
    let dir = Scratch::new();
    let config = dir.write(
        "config.toml",
        "active_model = \"mistral-medium-3.5\"\n\n[[mcp_servers]]\nname = \"mine\"\ntransport = \"stdio\"\ncommand = \"my-mcp\"\n\n[[mcp_servers]]\nname = \"memory-api\"\ntransport = \"http\"\nurl = \"https://stale.example/mcp\"\n",
    );
    let ran = vibe(&config, &[]);
    assert_eq!(ran.status, 0, "{}{}", ran.out, ran.err);
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.starts_with("active_model = \"mistral-medium-3.5\"\n"));
    assert!(text.contains("command = \"my-mcp\""));
    assert!(text.contains("url = \"https://hindsight-api.jorisjonkers.dev/mcp/default/\""));
    let names = names_in(&config, "mcp_servers");
    assert_eq!(names.iter().filter(|n| *n == "memory-api").count(), 1);
    assert!(names.contains(&"mine".to_string()));
    let fragment_names = names_in(&fragment(VIBE), "mcp_servers");
    assert!(fragment_names.iter().all(|n| names.contains(n)));
    assert!(ran.out.contains(&format!("vibe-merge-mcp: {} managed mcp_servers entries in", fragment_names.len())));
}

#[test]
fn vibe_merge_is_idempotent_and_creates_the_config() {
    let dir = Scratch::new();
    let config = dir.path("home/config.toml");
    assert_eq!(vibe(&config, &[]).status, 0);
    let first = fs::read_to_string(&config).unwrap();
    assert_eq!(vibe(&config, &[]).status, 0);
    assert_eq!(fs::read_to_string(&config).unwrap(), first);
}

#[test]
fn vibe_merge_is_idempotent_beside_the_users_own_settings() {
    let dir = Scratch::new();
    let config = dir.write("config.toml", "# my vibe\nactive_model = \"mistral-medium-3.5\"\n\n[tools.bash]\npermission = \"ask\"\n");
    assert_eq!(vibe(&config, &[]).status, 0);
    let first = fs::read_to_string(&config).unwrap();
    assert_eq!(vibe(&config, &[]).status, 0);
    assert_eq!(fs::read_to_string(&config).unwrap(), first);
    assert!(first.starts_with("# my vibe\n"));
}

#[test]
fn vibe_merge_keeps_the_first_backup() {
    let dir = Scratch::new();
    let config = dir.write("config.toml", "active_model = \"original\"\n");
    assert_eq!(vibe(&config, &[]).status, 0);
    assert_eq!(vibe(&config, &[]).status, 0);
    assert_eq!(fs::read_to_string(dir.path("config.toml.bak")).unwrap(), "active_model = \"original\"\n");
}

#[test]
fn vibe_merge_can_remove_a_managed_server() {
    let dir = Scratch::new();
    let config = dir.path("config.toml");
    assert_eq!(vibe(&config, &[]).status, 0);
    let before = names_in(&config, "mcp_servers").len();
    assert_eq!(vibe(&config, &["--remove", "kubernetes"]).status, 0);
    let names = names_in(&config, "mcp_servers");
    assert!(!names.contains(&"kubernetes".to_string()));
    assert_eq!(names.len(), before - 1);
}

#[test]
fn vibe_merge_matches_the_layout_tomlkit_wrote() {
    let dir = Scratch::new();
    let config = dir.write(
        "config.toml",
        "active_model = \"m\"\n\n[[mcp_servers]]\nname = \"mine\"\ntransport = \"stdio\"\ncommand = \"my-mcp\"\n\n[mcp_servers.env]\nA = \"b\"\n\n[tools.bash]\npermission = \"ask\"\n",
    );
    assert_eq!(vibe(&config, &["--remove", "kubernetes"]).status, 0);
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.starts_with("active_model = \"m\"\n\n[tools.bash]\npermission = \"ask\"\n\n[[mcp_servers]]\nname = \"mine\"\ntransport = \"stdio\"\ncommand = \"my-mcp\"\n\n[mcp_servers.env]\nA = \"b\"\n\n[[mcp_servers]]\nname = \"memory-api\"\n"), "{text}");
    assert!(text.contains("\n[mcp_servers.auth]\ntype = \"static\"\napi_key_env = \"HINDSIGHT_API_TOKEN\"\n"));
    assert!(text.contains("tool_timeout_sec = 60.0\n"));
}

#[test]
fn vibe_hooks_with_none_declared_leave_an_empty_file() {
    let dir = Scratch::new();
    let hooks = fragment("installer/vibe-hooks.toml");
    let config = dir.path("hooks.toml");
    let ran = exec(&["vibe-mcp", p(&hooks), "--key", "hooks", "--config", p(&config)], "", &[]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert_eq!(fs::read_to_string(&config).unwrap(), "");
    assert!(ran.out.contains("0 managed hooks entries"));
}

#[test]
fn vibe_hooks_replace_the_named_hook_and_keep_the_rest() {
    let dir = Scratch::new();
    let frag = dir.write("hooks-frag.toml", "[[hooks]]\nname = \"a\"\ncommand = \"new\"\n");
    let config = dir.write("hooks.toml", "[[hooks]]\nname = \"a\"\ncommand = \"old\"\n\n[[hooks]]\nname = \"retired\"\ncommand = \"x\"\n\n[[hooks]]\nname = \"mine\"\ncommand = \"y\"\n\n[[hooks]]\ncommand = \"nameless\"\n");
    let ran = exec(&["vibe-mcp", p(&frag), "--key=hooks", "--config", p(&config), "--remove", "retired"], "", &[]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("command = \"new\"") && !text.contains("old") && !text.contains("retired"));
    assert!(text.contains("name = \"mine\"") && text.contains("nameless"));
    assert_eq!(names_in(&config, "hooks"), ["mine", "None", "a"]);
}

#[test]
fn vibe_rebuilds_entries_from_plain_values() {
    let dir = Scratch::new();
    let frag = dir.write(
        "f.toml",
        "# header\n[[mcp_servers]] # trailing\nname = \"x\"\nint = 7\nflag = true\nwhen = 2024-01-02T03:04:05Z\nargs = [ \"a\" ,\"b\" ]\nenv = { A = \"b\", n = 1 }\ntools = [ { t = 1 } ]\n\n[[mcp_servers.more]]\nk = 1\n",
    );
    let config = dir.path("c.toml");
    let ran = exec(&["vibe-mcp", p(&frag), "--config", p(&config)], "", &[]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("int = 7\nflag = true\nwhen = 2024-01-02T03:04:05Z\nargs = [\"a\", \"b\"]\ntools = [{ t = 1 }]\n"), "{text}");
    assert!(text.contains("[mcp_servers.env]\nA = \"b\"\nn = 1\n"), "{text}");
    assert!(text.contains("[[mcp_servers.more]]\nk = 1\n"), "{text}");
    assert!(!text.contains("header") && !text.contains("trailing"));
    let again = fs::read_to_string(&config).unwrap();
    assert_eq!(exec(&["vibe-mcp", p(&frag), "--config", p(&config)], "", &[]).status, 0);
    assert_eq!(fs::read_to_string(&config).unwrap(), again);
}

#[test]
fn vibe_names_a_non_string_name_the_way_python_prints_it() {
    let dir = Scratch::new();
    let frag = dir.write("f.toml", "[[mcp_servers]]\nname = 7\n");
    let config = dir.write("c.toml", "[[mcp_servers]]\nname = 7\nold = 1\n");
    let ran = exec(&["vibe-mcp", p(&frag), "--config", p(&config)], "", &[]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert_eq!(fs::read_to_string(&config).unwrap(), "[[mcp_servers]]\nname = 7\n");
}

#[test]
fn vibe_ignores_a_servers_key_that_is_not_an_array_of_tables() {
    let dir = Scratch::new();
    let config = dir.write("c.toml", "mcp_servers = 3\n");
    assert_eq!(vibe(&config, &[]).status, 0);
    assert!(names_in(&config, "mcp_servers").len() > 1);
}

#[test]
fn vibe_stops_on_toml_it_cannot_read() {
    let dir = Scratch::new();
    let config = dir.write("c.toml", "= broken\n");
    let ran = vibe(&config, &[]);
    assert_eq!(ran.status, 2);
    assert_eq!(fs::read_to_string(&config).unwrap(), "= broken\n");
}

#[test]
fn vibe_default_config_follows_vibe_home() {
    let dir = Scratch::new();
    let frag = fragment("installer/vibe-hooks.toml");
    let home = dir.path("vh");
    assert_eq!(exec(&["vibe-mcp", p(&frag)], "", &[("VIBE_HOME", p(&home))]).status, 0);
    assert!(home.join("config.toml").is_file());
    assert_eq!(exec(&["vibe-mcp", p(&frag), "--key", "hooks"], "", &[("VIBE_HOME", ""), ("HOME", p(&dir.0))]).status, 0);
    assert!(dir.path(".vibe/hooks.toml").is_file());
}

#[test]
fn vibe_verification_names_what_did_not_land() {
    let wanted: toml_edit::DocumentMut = "[[s]]\nname = \"a\"\nv = 1\n\n[[s]]\nname = \"b\"\n\n[[s]]\nname = \"c\"\n".parse().unwrap();
    let tables: Vec<toml_edit::Table> = match wanted.get("s") {
        Some(toml_edit::Item::ArrayOfTables(t)) => t.iter().cloned().collect(),
        _ => panic!(),
    };
    let back: std::collections::BTreeMap<String, mcp_toml::Plain> = [("a".to_string(), mcp_toml::Plain::Int(0)), ("gone".to_string(), mcp_toml::Plain::Int(0))].into();
    let message = mcp_toml::verify(&tables, &["gone".to_string(), "c".to_string(), "gone".to_string()], &back).unwrap();
    assert_eq!(message, "vibe-merge-mcp: FAILED, not as written: ['a', 'b'], still present: ['gone']");
}

#[test]
fn plain_values_compare_by_content() {
    let doc: toml_edit::DocumentMut = "a = 1\nb = 1.5\nc = \"s\"\nd = true\ne = 1979-05-27\nf = [1, 2]\ng = { h = 1 }\n[t]\ni = 2\n[[u]]\nj = 3\n".parse().unwrap();
    let plain = mcp_toml::plain(&toml_edit::Item::Table(doc.as_table().clone()));
    let mcp_toml::Plain::Map(map) = plain else { panic!() };
    assert_eq!(map["a"], mcp_toml::Plain::Int(1));
    assert_eq!(map["b"], mcp_toml::Plain::Float("1.5".into()));
    assert_eq!(map["c"], mcp_toml::Plain::Text("s".into()));
    assert_eq!(map["d"], mcp_toml::Plain::Flag(true));
    assert_eq!(map["e"], mcp_toml::Plain::Moment("1979-05-27".into()));
    assert_eq!(map["f"], mcp_toml::Plain::List(vec![mcp_toml::Plain::Int(1), mcp_toml::Plain::Int(2)]));
    assert!(matches!(map["g"], mcp_toml::Plain::Map(_)) && matches!(map["t"], mcp_toml::Plain::Map(_)) && matches!(map["u"], mcp_toml::Plain::List(_)));
    assert_eq!(mcp_toml::plain(&toml_edit::Item::None), mcp_toml::Plain::Map(Default::default()));
}

#[test]
fn codex_add_url_registers_by_url_and_keeps_the_rest() {
    let dir = Scratch::new();
    let fresh = dir.path("sub/config.toml");
    assert_eq!(exec(&["codex-add-url", p(&fresh), "n8n", "https://n8n.x/mcp"], "", &[]).status, 0);
    assert_eq!(fs::read_to_string(&fresh).unwrap(), "[mcp_servers.n8n]\nurl = \"https://n8n.x/mcp\"\n");
    let other = dir.write("o.toml", "model = \"a\"\n\n[mcp_servers.other]\ncommand = \"x\"\n");
    assert_eq!(exec(&["codex-add-url", p(&other), "n8n", "https://n8n.x/mcp"], "", &[]).status, 0);
    assert_eq!(fs::read_to_string(&other).unwrap(), "model = \"a\"\n\n[mcp_servers.other]\ncommand = \"x\"\n\n[mcp_servers.n8n]\nurl = \"https://n8n.x/mcp\"\n");
    let plain = dir.write("p.toml", "model = \"a\"\n");
    assert_eq!(exec(&["codex-add-url", p(&plain), "n8n", "https://n8n.x/mcp"], "", &[]).status, 0);
    assert_eq!(fs::read_to_string(&plain).unwrap(), "model = \"a\"\n\n[mcp_servers.n8n]\nurl = \"https://n8n.x/mcp\"\n");
    // a second run replaces the entry rather than adding another
    assert_eq!(exec(&["codex-add-url", p(&plain), "n8n", "https://n8n.x/other"], "", &[]).status, 0);
    assert_eq!(fs::read_to_string(&plain).unwrap(), "model = \"a\"\n\n[mcp_servers.n8n]\nurl = \"https://n8n.x/other\"\n");
    let wrong = dir.write("w.toml", "mcp_servers = 3\n");
    assert_eq!(exec(&["codex-add-url", p(&wrong), "n8n", "u"], "", &[]).status, 2);
}

// ---------------------------------------------------------------------------
// hermes
// ---------------------------------------------------------------------------

const HERMES: &str = "registry/generated/hermes/mcp-servers.local.yaml";

fn hermes(config: &Path, extra: &[&str]) -> Ran {
    let frag = fragment(HERMES);
    let mut args = vec!["hermes-mcp", p(&frag), "--config", p(config)];
    args.extend_from_slice(extra);
    exec(&args, "", &[])
}

fn yaml_servers(config: &Path) -> Vec<String> {
    match mcp_yaml::load_config(config).unwrap().get("mcp_servers") {
        Some(serde_yaml::Value::Mapping(m)) => m.keys().map(|k| k.as_str().unwrap().to_string()).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn hermes_merges_the_fragment_and_keeps_every_other_key() {
    let dir = Scratch::new();
    let config = dir.write("config.yaml", "model:\n  default: x\nmcp_servers:\n  mine:\n    command: m\n  memory-api:\n    url: stale\n");
    let ran = hermes(&config, &[]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert!(ran.out.starts_with("hermes: merged "), "{}", ran.out);
    assert!(mcp_yaml::load_config(&config).unwrap().contains_key("model"));
    let names = yaml_servers(&config);
    assert_eq!(names[0], "mine");
    assert!(names.contains(&"memory-api".to_string()));
    assert_eq!(fs::read_to_string(dir.path("config.yaml.bak")).unwrap().lines().next(), Some("model:"));
    let again = hermes(&config, &[]);
    assert!(again.out.starts_with("hermes: already present; no change 9"), "{}", again.out);
}

#[test]
fn hermes_creates_the_config_and_removes_servers() {
    let dir = Scratch::new();
    let config = dir.path("h/config.yaml");
    assert_eq!(hermes(&config, &[]).status, 0);
    assert!(!dir.path("h/config.yaml.bak").exists());
    let name = yaml_servers(&config)[0].clone();
    let ran = hermes(&config, &["--remove", &name, "--remove", "ghost"]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert!(ran.out.starts_with(&format!("hermes: removed MCP server(s) ['ghost', '{name}']")), "{}", ran.out);
    assert!(!yaml_servers(&config).contains(&name));
    let again = hermes(&config, &["--remove", "ghost"]);
    assert!(again.out.starts_with("hermes: already absent MCP server(s) ['ghost']"), "{}", again.out);
}

#[test]
fn hermes_treats_empty_and_non_mapping_configs_as_empty() {
    let dir = Scratch::new();
    for text in ["", "  \n", "- a\n- b\n"] {
        let config = dir.write("c.yaml", text);
        assert_eq!(hermes(&config, &[]).status, 0);
        assert!(!yaml_servers(&config).is_empty());
    }
    let config = dir.write("r.yaml", "- list\n");
    assert_eq!(hermes(&config, &["--remove", "x"]).status, 0);
    assert_eq!(fs::read_to_string(&config).unwrap(), "{}\n");
    let config = dir.write("n.yaml", "mcp_servers: 3\nother: 1\n");
    assert_eq!(hermes(&config, &["--remove", "x"]).status, 0);
    assert!(fs::read_to_string(&config).unwrap().contains("mcp_servers: 3"));
}

#[test]
fn hermes_refuses_what_it_cannot_merge() {
    let dir = Scratch::new();
    let config = dir.write("c.yaml", "mcp_servers: nope\n");
    let ran = hermes(&config, &[]);
    assert_eq!(ran.status, 2);
    assert!(ran.err.contains("non-mapping 'mcp_servers'"), "{}", ran.err);
    assert_eq!(fs::read_to_string(&config).unwrap(), "mcp_servers: nope\n");
    let ran = exec(&["hermes-mcp", p(&dir.path("missing.yaml")), "--config", p(&config)], "", &[]);
    assert_eq!((ran.status, ran.err.starts_with("fragment not found")), (1, true));
    let flat = dir.write("flat.yaml", "other: 1\n");
    let ran = exec(&["hermes-mcp", p(&flat), "--config", p(&config)], "", &[]);
    assert_eq!((ran.status, ran.err.trim()), (1, "fragment must be a mapping with an 'mcp_servers' key"));
    let broken = dir.write("broken.yaml", "a: [\n");
    assert_eq!(exec(&["hermes-mcp", p(&broken), "--config", p(&config)], "", &[]).status, 2);
    let bad_config = dir.write("bad.yaml", "a: [\n");
    assert_eq!(hermes(&bad_config, &[]).status, 2);
}

#[test]
fn hermes_fragment_with_no_servers_adds_an_empty_mapping() {
    let dir = Scratch::new();
    let frag = dir.write("f.yaml", "mcp_servers:\n");
    let config = dir.path("c.yaml");
    let ran = exec(&["hermes-mcp", p(&frag), "--config", p(&config)], "", &[]);
    assert_eq!(ran.status, 0, "{}", ran.err);
    assert!(ran.out.starts_with("hermes: merged 0 MCP"), "{}", ran.out);
}

#[test]
fn hermes_default_config_follows_the_environment() {
    let dir = Scratch::new();
    let frag = fragment(HERMES);
    let named = dir.path("named.yaml");
    assert_eq!(exec(&["hermes-mcp", p(&frag)], "", &[("HERMES_CONFIG", &format!(" {} ", p(&named)))]).status, 0);
    assert!(named.is_file());
    assert_eq!(exec(&["hermes-mcp", p(&frag), "--config="], "", &[("HERMES_CONFIG", "  "), ("HOME", p(&dir.0))]).status, 0);
    assert!(dir.path(".hermes/config.yaml").is_file());
}

#[test]
fn hermes_verification_names_what_did_not_land() {
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(mcp_yaml::verify(&names(&["a", "b", "a"]), &[], &names(&["a", "b"])), Some("verification failed; still present after remove: ['a', 'b']".to_string()));
    assert_eq!(mcp_yaml::verify(&names(&["a"]), &[], &[]), None);
    assert_eq!(mcp_yaml::verify(&[], &names(&["z", "y"]), &names(&["y"])), Some("verification failed; missing after merge: ['z']".to_string()));
    assert_eq!(mcp_yaml::verify(&[], &names(&["y"]), &names(&["y"])), None);
}

// ---------------------------------------------------------------------------
// the command line
// ---------------------------------------------------------------------------

#[test]
fn the_command_line_reports_usage_errors_with_status_2() {
    let none = exec(&[], "", &[]);
    assert_eq!(none.status, 2);
    assert!(none.err.starts_with("usage: agent-kit-merge COMMAND"));
    let cases: &[&[&str]] = &[
        &["nonsense"],
        &["vibe-mcp"],
        &["vibe-mcp", "f", "--config"],
        &["vibe-mcp", "f", "--key", "other"],
        &["vibe-mcp", "f", "--wat"],
        &["instruction-block", "f", "n", "--remove=1"],
        &["package-proxy", "f", "n", "--validate", "yaml"],
        &["codex-add-url", "a", "b"],
    ];
    for args in cases {
        let ran = exec(args, "", &[]);
        assert_eq!(ran.status, 2, "{args:?}");
        assert!(ran.err.starts_with("agent-kit-merge:"), "{args:?}: {}", ran.err);
    }
}

#[test]
fn a_merge_that_does_not_read_back_is_status_1_on_stderr() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut input: &[u8] = &[];
    let mut io = crate::Io { stdin: &mut input, out: &mut out, err: &mut err };
    assert_eq!(crate::finish(Some("it failed".to_string()), "fine".to_string(), &mut io), Ok(1));
    assert_eq!(crate::finish(None, "fine".to_string(), &mut io), Ok(0));
    assert_eq!((String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap()), ("fine\n".to_string(), "it failed\n".to_string()));
}

#[test]
fn json_leaves_delete_alone_and_toml_rebuild_keeps_nothing_for_none() {
    assert_eq!(json::dumps(&Value::Str("\u{7f}".into())), "\"\u{7f}\"");
    assert!(matches!(mcp_toml::rebuilt_item(&toml_edit::Item::None), toml_edit::Item::None));
}

#[test]
fn a_new_file_without_a_mode_takes_the_umask_default() {
    use std::os::unix::fs::PermissionsExt;
    let dir = Scratch::new();
    let file = dir.path("fresh.conf");
    assert_eq!(proxy_run(&file, &[], "a=1\n").status, 0);
    let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & 0o111, 0);
    assert_eq!(mode & 0o600, 0o600);
}

#[test]
fn the_real_vibe_fragment_starts_its_servers_as_the_registry_declares_them() {
    // Ported from tests/test_n8n_mcp.py, test_oauth_mcp.py and test_playwright_pinned.py.
    let dir = Scratch::new();
    let config = dir.path("config.toml");
    assert_eq!(vibe(&config, &[]).status, 0);
    let doc: toml_edit::DocumentMut = fs::read_to_string(&config).unwrap().parse().unwrap();
    let Some(toml_edit::Item::ArrayOfTables(tables)) = doc.get("mcp_servers") else { panic!("no servers") };
    let find = |name: &str| tables.iter().find(|t| mcp_toml::entry_name(t) == name).unwrap_or_else(|| panic!("no {name}")).clone();
    let text = |table: &toml_edit::Table, key: &str| table.get(key).and_then(|v| v.as_str()).map(str::to_string);

    let n8n_mcp = find("n8n-mcp");
    assert_eq!(text(&n8n_mcp, "command").as_deref(), Some("n8n-mcp"));
    assert_eq!(n8n_mcp.get("args").and_then(|a| a.as_array()).map(|a| a.len()), Some(0));
    assert!(n8n_mcp.get("env").is_none(), "no api key is written");

    let n8n = find("n8n");
    assert_eq!(text(&n8n, "url").as_deref(), Some("https://n8n.jorisjonkers.dev/mcp-server/http"));
    let auth = n8n.get("auth").and_then(|a| a.as_table()).unwrap();
    assert_eq!(text(auth, "type").as_deref(), Some("oauth"));
    assert_eq!(auth.get("scopes").and_then(|a| a.as_array()).map(|a| a.len()), Some(0));
    assert!(auth.get("api_key_env").is_none(), "no token is written");

    let playwright = find("playwright");
    assert_eq!(text(&playwright, "command").as_deref(), Some("playwright-mcp"));
    let args: Vec<&str> = playwright.get("args").and_then(|a| a.as_array()).unwrap().iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(args, ["--headless", "--browser", "chromium"]);
    assert_eq!(playwright.get("startup_timeout_sec").and_then(|v| v.as_float()), Some(60.0));
}
