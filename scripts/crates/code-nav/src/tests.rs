use super::*;
use crate::boundary::{is_secret, Boundary, Refusal};
use crate::search::{search, DEFAULT_LIMIT, MAX_FILE_BYTES, MAX_LIMIT};
use code_nav_eval::Json;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A Rust file: an attribute above a struct, an impl with nested fns, a fn.
const SHAPES_RS: &str = "\
use std::fmt;

#[derive(Debug)]
pub struct Circle {
    radius: f64,
}

impl Circle {
    pub fn area(&self) -> f64 {
        3.14 * self.radius
    }

    fn scale(&mut self, by: f64) {
        self.radius *= by;
    }
}

fn main() {
    let c = Circle { radius: 1.0 };
    println!(\"{}\", c.area());
}
";

/// A Python file: indentation alone delimits the defs and the class.
const APP_PY: &str = "\
import os


def load(path):
    with open(path) as f:
        return f.read()


class Store:
    def put(self, key):
        self.key = key

    def get(self):
        return self.key
";

/// A JavaScript file: modifiers before the keyword.
const APP_JS: &str = "\
export async function fetchAll(urls) {
  return Promise.all(urls.map(fetchOne));
}

function fetchOne(url) {
  return url;
}
";

/// A file that declares nothing.
const NOTES_TXT: &str = "just prose\nno declarations here\n";

/// A file whose first symbol runs past the expand cap, with a second of the
/// same name behind it.
fn long_rs() -> String {
    let mut text = String::from("fn dup() {\n");
    for i in 0..399 {
        text.push_str(&format!("    let a{i} = {i};\n"));
    }
    text.push_str("}\n\nfn dup() {\n    let b = 2;\n}\n");
    text
}

fn temp_dir() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("code-nav-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    fs::create_dir_all(&d).unwrap();
    d.canonicalize().unwrap()
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A repo root with a fixture, and a sibling directory outside the allowlist.
struct Fixture {
    root: PathBuf,
    outside: PathBuf,
    boundary: Boundary,
}

fn fixture() -> Fixture {
    let base = temp_dir();
    let root = base.join("repo");
    let outside = base.join("elsewhere");
    write(&root.join("src/widget.rs"), "pub fn make_widget() {}\n// widgets are made here\nlet gadget = 1;\n");
    write(&root.join("src/gadget.rs"), "fn gadget_factory() { widget(); }\n");
    write(&root.join("src/shapes.rs"), SHAPES_RS);
    write(&root.join("src/app.py"), APP_PY);
    write(&root.join("src/app.js"), APP_JS);
    write(&root.join("src/notes.txt"), NOTES_TXT);
    write(&root.join("src/long.rs"), &long_rs());
    write(&root.join("README.md"), "A widget library.\n");
    write(&root.join("deep/er/than/widget_notes.txt"), "widget\n");
    write(&root.join(".env"), "TOKEN=widget\n");
    write(&root.join("keys/server.pem"), "widget\n");
    write(&root.join(".git/config"), "widget\n");
    write(&root.join("target/debug/out.txt"), "widget\n");
    write(&outside.join("loot.txt"), "widget loot\n");
    write(&outside.join("loot.env"), "widget\n");
    fs::write(root.join("blob.bin"), b"widget\0binary").unwrap();
    fs::write(root.join("latin1.txt"), [0x77, 0x69, 0x64, 0x67, 0x65, 0x74, 0xff]).unwrap();
    write(&root.join("big.txt"), &"widget\n".repeat((MAX_FILE_BYTES as usize / 7) + 10));
    symlink(outside.join("loot.txt"), root.join("link-out.txt")).unwrap();
    symlink(&outside, root.join("dir-out")).unwrap();
    symlink(root.join("README.md"), root.join("link-in.md")).unwrap();
    symlink(root.join("nowhere"), root.join("dangling")).unwrap();
    symlink(root.join(".env"), root.join("innocent.txt")).unwrap();
    let boundary = Boundary::new(std::slice::from_ref(&root)).unwrap();
    Fixture { root, outside, boundary }
}

// ---------------------------------------------------------------- boundary

#[test]
fn a_root_outside_the_allowlist_is_refused() {
    let f = fixture();
    let outside = f.outside.join("loot.txt");
    assert_eq!(f.boundary.admit(&outside), Err(Refusal::OutsideAllowlist(outside.clone())));
    // A missing path outside is also "outside", not "missing".
    let ghost = f.outside.join("ghost");
    assert_eq!(f.boundary.admit(&ghost), Err(Refusal::OutsideAllowlist(ghost)));
    // Dot-dot out of the root is outside too.
    let escape = f.root.join("../elsewhere/loot.txt");
    assert!(matches!(f.boundary.admit(&escape), Err(Refusal::OutsideAllowlist(_))));
    assert!(f.boundary.admit(&outside).unwrap_err().to_string().contains("outside the allowed roots"));
}

#[test]
fn a_symlink_out_of_the_roots_is_refused() {
    let f = fixture();
    for link in ["link-out.txt", "dir-out", "dir-out/loot.txt"] {
        let refusal = f.boundary.admit(&f.root.join(link)).unwrap_err();
        assert!(matches!(refusal, Refusal::SymlinkOut(_)), "{link}: {refusal:?}");
        assert!(refusal.to_string().contains("symlink"));
    }
}

#[test]
fn a_symlink_that_stays_inside_is_admitted_as_its_target() {
    let f = fixture();
    assert_eq!(f.boundary.admit(&f.root.join("link-in.md")).unwrap(), f.root.join("README.md"));
}

#[test]
fn a_secret_file_is_refused() {
    let f = fixture();
    for secret in [".env", "keys/server.pem", "innocent.txt"] {
        let refusal = f.boundary.admit(&f.root.join(secret)).unwrap_err();
        assert!(matches!(refusal, Refusal::SecretFile(_)), "{secret}: {refusal:?}");
        assert!(refusal.to_string().contains("secret"));
    }
}

#[test]
fn relative_paths_are_taken_against_the_first_root() {
    let f = fixture();
    assert_eq!(f.boundary.admit(Path::new("src/./widget.rs")).unwrap(), f.root.join("src/widget.rs"));
    assert_eq!(f.boundary.admit(Path::new("")).unwrap(), f.root);
}

#[test]
fn a_missing_path_under_a_root_is_missing() {
    let f = fixture();
    let refusal = f.boundary.admit(&f.root.join("nope.rs")).unwrap_err();
    assert!(matches!(refusal, Refusal::Missing(_)));
    assert!(refusal.to_string().contains("does not exist"));
}

#[test]
fn roots_must_be_existing_directories() {
    let f = fixture();
    assert!(Boundary::new(&[]).unwrap_err().contains("no roots"));
    assert!(Boundary::new(&[f.root.join("nope")]).unwrap_err().contains("nope"));
    assert!(Boundary::new(&[f.root.join("README.md")]).unwrap_err().contains("not a directory"));
    assert_eq!(f.boundary.roots(), std::slice::from_ref(&f.root));
}

#[test]
fn overlapping_roots_collapse_and_secret_roots_are_refused() {
    let f = fixture();
    let b = Boundary::new(&[f.root.join("src"), f.root.clone(), f.root.join("src"), f.root.join("deep")]).unwrap();
    assert_eq!(b.roots(), std::slice::from_ref(&f.root));
    assert!(Boundary::new(&[f.root.join("keys")]).is_ok());
    fs::create_dir_all(f.root.join(".ssh")).unwrap();
    assert!(Boundary::new(&[f.root.join(".ssh")]).unwrap_err().contains("secret"));
    assert!(matches!(f.boundary.admit(Path::new(".git/config")), Err(Refusal::SecretFile(_))));
}

#[test]
fn secret_names_are_recognised() {
    for secret in [
        ".env", ".env.local", ".ENV", "a/.env.production", "id_rsa", "id_ed25519", "id_ecdsa", ".netrc", ".npmrc",
        "x/credentials", "creds/server.key", "a.p12", ".ssh", ".ssh/config", "home/.aws/config", ".gnupg/pubring.kbx",
        "store.kdbx", "Secrets.json", ".git/config", ".envrc", "prod.env", ".kube/config", "kubeconfig", "a.tfvars",
    ] {
        assert!(is_secret(Path::new(secret)), "{secret}");
    }
    for fine in [".env.example", ".env.sample", ".env.template", "src/lib.rs", "environment.rs", "keyboard.rs", "", "a/b"] {
        assert!(!is_secret(Path::new(fine)), "{fine}");
    }
}

// ------------------------------------------------------------------ search

fn files_of(results: &search::Results, root: &Path) -> Vec<String> {
    results
        .hits
        .iter()
        .map(|h| format!("{}:{}", h.path.strip_prefix(root).unwrap().display(), h.line))
        .collect()
}

#[test]
fn search_ranks_by_the_documented_rule() {
    let f = fixture();
    let r = search(f.boundary.roots(), "widget", 50);
    // widget_notes.txt: the bare word (+4) in a matching file name (+2): 7,
    // the top hit despite its depth. README.md and gadget.rs hold the bare
    // word: 5, shallower path first. widget.rs holds only "make_widget" and
    // "widgets", not whole identifiers, but its name matches: 3, by line.
    let order = files_of(&r, &f.root);
    assert_eq!(
        order,
        [
            "deep/er/than/widget_notes.txt:1",
            "README.md:1",
            "src/gadget.rs:1",
            "src/widget.rs:1",
            "src/widget.rs:2",
        ]
    );
    let scores: Vec<u32> = r.hits.iter().map(|h| h.score).collect();
    assert_eq!(scores, [7, 5, 5, 3, 3]);
}

#[test]
fn search_caps_the_result_and_reports_the_total() {
    let f = fixture();
    let r = search(f.boundary.roots(), "widget", 2);
    assert_eq!(r.hits.len(), 2);
    assert_eq!(r.total, 5);
    assert!(r.truncated());
    // The cap keeps the best hits, not the first found.
    assert_eq!(files_of(&r, &f.root), ["deep/er/than/widget_notes.txt:1", "README.md:1"]);
    let all = search(f.boundary.roots(), "widget", 50);
    assert!(!all.truncated());
}

#[test]
fn the_default_cap_is_documented_and_limits_are_clamped() {
    assert_eq!(DEFAULT_LIMIT, 20);
    let base = temp_dir();
    for i in 0..(MAX_LIMIT + 30) {
        write(&base.join(format!("f{i:04}.txt")), "needle\n");
    }
    assert_eq!(search(std::slice::from_ref(&base), "needle", 0).hits.len(), 1);
    assert_eq!(search(std::slice::from_ref(&base), "needle", 5000).hits.len(), MAX_LIMIT);
    let r = search(&[base], "needle", DEFAULT_LIMIT);
    assert_eq!(r.hits.len(), DEFAULT_LIMIT);
    assert_eq!(r.total, MAX_LIMIT + 30);
}

#[test]
fn search_never_reads_what_the_boundary_refuses() {
    let f = fixture();
    let r = search(f.boundary.roots(), "widget", 200);
    let seen = files_of(&r, &f.root).join(" ");
    for hidden in [".env", "server.pem", ".git", "target", "loot", "link-out", "dir-out", "dangling", "blob.bin", "latin1", "big.txt", "innocent", "link-in"] {
        assert!(!seen.contains(hidden), "{hidden} in {seen}");
    }
}

#[test]
fn search_matches_ignoring_case_and_clips_long_lines() {
    let base = temp_dir();
    write(&base.join("a.txt"), &format!("  NEEDLE {}\n", "x".repeat(500)));
    let r = search(std::slice::from_ref(&base), "needle", 5);
    assert_eq!(r.hits[0].text.chars().count(), 200);
    assert!(r.hits[0].text.starts_with("NEEDLE"));
    assert!(search(std::slice::from_ref(&base), "absent", 5).hits.is_empty());
    // A single file as scope, and a scope that does not exist.
    assert_eq!(search(&[base.join("a.txt")], "needle", 5).total, 1);
    assert_eq!(search(&[base.join("gone")], "needle", 5).total, 0);
}

#[test]
fn identifier_boundaries_decide_the_whole_word_bonus() {
    let base = temp_dir();
    write(&base.join("a.txt"), "needle\nneedles\nx_needle\n(needle)\nneedleé\n");
    let r = search(&[base], "needle", 10);
    let scored: Vec<(usize, u32)> = r.hits.iter().map(|h| (h.line, h.score)).collect();
    assert_eq!(scored, [(1, 5), (4, 5), (2, 1), (3, 1), (5, 1)]);
}

#[test]
fn a_flood_of_matches_in_one_file_stays_bounded_and_exact() {
    let base = temp_dir();
    write(&base.join("a.txt"), &"hit\n".repeat(1000));
    let r = search(&[base], "hit", 3);
    assert_eq!(r.total, 1000);
    let lines: Vec<usize> = r.hits.iter().map(|h| h.line).collect();
    assert_eq!(lines, [1, 2, 3]);
}

#[cfg(unix)]
#[test]
fn an_unreadable_directory_is_skipped() {
    use std::os::unix::fs::PermissionsExt;
    let base = temp_dir();
    write(&base.join("locked/a.txt"), "needle\n");
    fs::set_permissions(base.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
    let total = search(std::slice::from_ref(&base), "needle", 5).total;
    fs::set_permissions(base.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    // Root can read it anyway; anyone else cannot.
    assert!(total <= 1);
}

// ----------------------------------------------------------------- outline

fn ranges(found: &[outline::Symbol]) -> Vec<(usize, &str, &str, usize, usize)> {
    found.iter().map(|s| (s.line, s.kind.as_str(), s.name.as_str(), s.start, s.end)).collect()
}

#[test]
fn symbols_read_rust_attributes_impls_and_nested_fns() {
    // The struct's range starts at its attribute; the impl's runs to its
    // closing brace; the nested fns are symbols of their own.
    assert_eq!(
        ranges(&outline::symbols(SHAPES_RS)),
        [
            (4, "struct", "Circle", 3, 6),
            (8, "impl", "Circle", 8, 16),
            (9, "fn", "area", 9, 11),
            (13, "fn", "scale", 13, 15),
            (18, "fn", "main", 18, 21),
        ]
    );
}

#[test]
fn symbols_follow_python_indentation() {
    // Blank lines do not end a block; the next def at the same indent does.
    assert_eq!(
        ranges(&outline::symbols(APP_PY)),
        [(4, "def", "load", 4, 6), (9, "class", "Store", 9, 14), (10, "def", "put", 10, 11), (13, "def", "get", 13, 14)]
    );
}

#[test]
fn symbols_read_modifiers_before_the_keyword() {
    assert_eq!(
        ranges(&outline::symbols(APP_JS)),
        [(1, "function", "fetchAll", 1, 3), (5, "function", "fetchOne", 5, 7)]
    );
}

#[test]
fn every_modifier_and_keyword_is_recognised() {
    let text = "\
#![allow(dead_code)]
pub(crate) fn one() {}
const fn two() {}
const MAX: usize = 3;
unsafe fn three() {}
static mut FOUR: u8 = 0;
public final class Six {}
private abstract class Seven {}
protected default interface Eight {}
type Alias = u8;
mod inner;
trait Shape {}
enum Kind { A }
func nine() {}
@decorator
def ten():
    pass
function eleven()
  return 1
end
const handlers = [
  function twelve() {},
];
fn thirteen(
    a: u8,
) {}
";
    let found = outline::symbols(text);
    let names: Vec<&str> = found.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["one", "two", "three", "Six", "Seven", "Eight", "Alias", "inner", "Shape", "Kind", "nine", "ten", "eleven", "twelve", "thirteen"]
    );
    // `#![` is no attribute, so `one` starts on its own line; `@decorator`
    // is, so `ten` starts above its def.
    assert_eq!(found[0].start, 2);
    assert_eq!(found[11].start, 15);
    // `)` and `end` close a block at their own indentation; the `];` below is the parent's.
    assert_eq!((found[12].start, found[12].end), (18, 20));
    assert_eq!((found[13].start, found[13].end), (22, 22));
    assert_eq!((found[14].start, found[14].end), (24, 26));
}

#[test]
fn a_truncated_line_declares_nothing() {
    for line in ["fn", "impl", "pub(crate fn", "impl<T Foo {", "pub use std::fmt;", "let x = 1;", ""] {
        assert!(outline::symbols(line).is_empty(), "{line}");
    }
}

#[test]
fn an_impl_is_named_for_its_type_or_trait() {
    let text = "impl<T> Foo for Bar {\n    fn get(&self) {}\n}\n";
    assert_eq!(ranges(&outline::symbols(text)), [(1, "impl", "Foo for Bar", 1, 3), (2, "fn", "get", 2, 2)]);
    // A body that opens on a later line still names the impl.
    assert_eq!(ranges(&outline::symbols("impl<T> Foo for Bar")), [(1, "impl", "Foo for Bar", 1, 1)]);
}

#[test]
fn a_one_line_declaration_ends_on_its_own_line() {
    let found = outline::symbols("fn x() {}\n\nfn y() {\n    x();\n}\n\n\n");
    assert_eq!(ranges(&found), [(1, "fn", "x", 1, 1), (3, "fn", "y", 3, 5)]);
}

#[test]
fn expand_returns_one_symbol_without_the_rest_of_the_file() {
    let found = outline::expand(SHAPES_RS, "area");
    assert_eq!(found.len(), 1);
    let (symbol, source) = &found[0];
    assert_eq!((symbol.line, symbol.start, symbol.end), (9, 9, 11));
    assert_eq!(source, "    pub fn area(&self) -> f64 {\n        3.14 * self.radius\n    }");
    for absent in ["scale", "main", "struct", "impl"] {
        assert!(!source.contains(absent), "{absent} in {source}");
    }
    // An ambiguous name returns every match, in file order.
    let both = outline::expand(SHAPES_RS, "Circle");
    assert_eq!(both.len(), 2);
    assert!(both[0].1.starts_with("#[derive(Debug)]\npub struct Circle {"));
    assert!(both[1].1.starts_with("impl Circle {"));
    assert!(outline::expand(SHAPES_RS, "absent").is_empty());
}

// ------------------------------------------------------------------ server

fn exchange(f: &Fixture, default_limit: usize, requests: &[&str]) -> Vec<String> {
    let mut input = requests.join("\n").into_bytes();
    input.push(b'\n');
    let mut out = Vec::new();
    server::serve(&f.boundary, default_limit, &mut &input[..], &mut out).unwrap();
    String::from_utf8(out).unwrap().lines().map(String::from).collect()
}

fn tool_call(tool: &str, args: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#)
}

/// The text block of a tool result.
fn text_of(reply: &str) -> String {
    let json = code_nav_eval::parse_json(reply).unwrap();
    let content = json.get("result").and_then(|r| r.get("content")).and_then(Json::as_arr).unwrap();
    content[0].get("text").and_then(Json::as_str).unwrap().to_string()
}

#[test]
fn the_server_speaks_the_handshake() {
    let f = fixture();
    let replies = exchange(
        &f,
        20,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "",
            r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize"}"#,
            r#"{"jsonrpc":"2.0","id":2.5,"method":"tools/list"}"#,
        ],
    );
    assert_eq!(replies.len(), 4);
    assert!(replies[0].contains(r#""id":1"#) && replies[0].contains("2025-03-26") && replies[0].contains(r#""name":"code-nav""#));
    assert_eq!(replies[1], r#"{"jsonrpc":"2.0","id":"abc","result":{}}"#);
    assert!(replies[2].contains("2024-11-05"));
    assert!(replies[3].contains(r#""id":2.5"#) && replies[3].contains(r#""name":"search""#));
    assert!(parse_json_ok(&replies[3]));
}

fn parse_json_ok(s: &str) -> bool {
    code_nav_eval::parse_json(s).is_ok()
}

#[test]
fn malformed_and_unknown_requests_get_json_rpc_errors() {
    let f = fixture();
    let replies = exchange(
        &f,
        20,
        &["not json", "[1]", r#"{"jsonrpc":"2.0","id":3}"#, r#"{"jsonrpc":"2.0","id":4,"method":"nope"}"#, r#"{"id":true,"method":"nope"}"#],
    );
    assert!(replies[0].contains("-32700") && replies[0].contains(r#""id":null"#));
    assert!(replies[1].contains("-32600"));
    assert!(replies[2].contains("-32600") && replies[2].contains(r#""id":3"#));
    assert!(replies[3].contains("-32601") && replies[3].contains("nope"));
    assert!(replies[4].contains(r#""id":null"#));
    assert!(replies.iter().all(|r| parse_json_ok(r)));
}

#[test]
fn the_search_tool_returns_ranked_capped_text() {
    let f = fixture();
    let replies = exchange(&f, 2, &[&tool_call("search", r#"{"query":"widget"}"#), &tool_call("search", r#"{"query":"widget","limit":50}"#), &tool_call("search", r#"{"query":"zzz"}"#)]);
    let capped = &replies[0];
    assert!(capped.contains(r#""isError":false"#));
    assert!(capped.contains("widget_notes.txt:1: widget"));
    assert!(capped.contains("showing 2 of 5 matches"));
    assert!(!capped.contains("src/widget.rs"));
    assert!(!replies[1].contains("showing"));
    assert!(replies[1].contains("src/widget.rs:1"));
    assert!(replies[2].contains("no matches"));
}

#[test]
fn the_search_tool_scopes_to_a_path_and_refuses_what_the_boundary_refuses() {
    let f = fixture();
    let outside = f.outside.join("loot.txt").display().to_string();
    let replies = exchange(
        &f,
        20,
        &[
            &tool_call("search", r#"{"query":"widget","path":"src"}"#),
            &tool_call("search", &format!(r#"{{"query":"widget","path":"{outside}"}}"#)),
            &tool_call("search", r#"{"query":"widget","path":"link-out.txt"}"#),
            &tool_call("search", r#"{"query":"widget","path":".env"}"#),
        ],
    );
    assert!(replies[0].contains("src/widget.rs") && !replies[0].contains("README"));
    for (reply, why) in [(&replies[1], "outside the allowed roots"), (&replies[2], "symlink"), (&replies[3], "secret file")] {
        assert!(reply.contains(r#""isError":true"#) && reply.contains(why), "{reply}");
        assert!(!reply.contains("loot widget") && !reply.contains("TOKEN"));
    }
}

#[test]
fn bad_tool_calls_are_tool_errors() {
    let f = fixture();
    let replies = exchange(
        &f,
        20,
        &[
            &tool_call("search", r#"{}"#),
            &tool_call("search", r#"{"query":""}"#),
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#,
        ],
    );
    assert!(replies[0].contains("query is required") && replies[1].contains("query is required"));
    assert!(replies[2].contains("unknown tool: write"));
    assert!(replies[3].contains("unknown tool"));
    assert!(replies.iter().all(|r| r.contains(r#""isError":true"#)));
}

#[test]
fn tools_list_names_all_three_tools() {
    let f = fixture();
    let replies = exchange(&f, 20, &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#]);
    assert!(parse_json_ok(&replies[0]));
    let json = code_nav_eval::parse_json(&replies[0]).unwrap();
    let tools = json.get("result").and_then(|r| r.get("tools")).and_then(Json::as_arr).unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|t| t.get("name").and_then(Json::as_str)).collect();
    assert_eq!(names, ["search", "outline", "expand"]);
    for tool in tools {
        let description = tool.get("description").and_then(Json::as_str).unwrap();
        assert!(description.contains("never read"), "{description}");
        assert!(tool.get("inputSchema").and_then(|s| s.get("required")).and_then(Json::as_arr).is_some());
    }
    let required = |tool: &Json| -> Vec<String> {
        tool.get("inputSchema")
            .and_then(|s| s.get("required"))
            .and_then(Json::as_arr)
            .unwrap()
            .iter()
            .filter_map(Json::as_str)
            .map(String::from)
            .collect()
    };
    assert_eq!(required(&tools[0]), ["query"]);
    assert_eq!(required(&tools[1]), ["path"]);
    assert_eq!(required(&tools[2]), ["path", "symbol"]);
}

#[test]
fn the_outline_tool_lists_a_files_symbols() {
    let f = fixture();
    let replies = exchange(
        &f,
        20,
        &[
            &tool_call("outline", r#"{"path":"src/shapes.rs"}"#),
            &tool_call("outline", r#"{"path":"src/app.py"}"#),
            &tool_call("outline", r#"{"path":"src/notes.txt"}"#),
        ],
    );
    assert_eq!(
        text_of(&replies[0]),
        "4 struct Circle (3-6)\n8 impl Circle (8-16)\n9 fn area (9-11)\n13 fn scale (13-15)\n18 fn main (18-21)"
    );
    assert!(text_of(&replies[1]).starts_with("4 def load (4-6)\n9 class Store (9-14)"));
    assert!(replies[2].contains(r#""isError":false"#));
    assert_eq!(text_of(&replies[2]), "no symbols");
}

#[test]
fn the_expand_tool_returns_one_symbol_without_the_rest_of_the_file() {
    let f = fixture();
    let replies = exchange(
        &f,
        20,
        &[
            &tool_call("expand", r#"{"path":"src/shapes.rs","symbol":"area"}"#),
            &tool_call("expand", r#"{"path":"src/shapes.rs","symbol":"Circle"}"#),
            &tool_call("expand", r#"{"path":"src/shapes.rs","symbol":"absent"}"#),
        ],
    );
    let area = text_of(&replies[0]);
    let path = f.root.join("src/shapes.rs").display().to_string();
    assert_eq!(
        area,
        format!("{path}:9-11 fn area\n9\t    pub fn area(&self) -> f64 {{\n10\t        3.14 * self.radius\n11\t    }}")
    );
    for absent in ["scale", "main", "struct", "impl"] {
        assert!(!area.contains(absent), "{absent} in {area}");
    }
    // An ambiguous name returns every match, one block each.
    let both = text_of(&replies[1]);
    assert!(both.starts_with(&format!("{path}:3-6 struct Circle\n3\t#[derive(Debug)]\n")));
    assert!(both.contains(&format!("\n\n{path}:8-16 impl Circle\n8\timpl Circle {{")));
    assert!(replies[2].contains(r#""isError":true"#));
    assert_eq!(text_of(&replies[2]), format!("no symbol named absent in {path}"));
}

#[test]
fn outline_and_expand_refuse_what_search_refuses() {
    let f = fixture();
    let outside = f.outside.join("loot.txt").display().to_string();
    for (path, why) in [(outside.as_str(), "outside the allowed roots"), ("link-out.txt", "symlink"), (".env", "secret file")] {
        let replies = exchange(
            &f,
            20,
            &[
                &tool_call("search", &format!(r#"{{"query":"widget","path":"{path}"}}"#)),
                &tool_call("outline", &format!(r#"{{"path":"{path}"}}"#)),
                &tool_call("expand", &format!(r#"{{"path":"{path}","symbol":"widget"}}"#)),
            ],
        );
        assert!(replies[0].contains(why), "{path}: {}", replies[0]);
        assert_eq!(replies[1], replies[0]);
        assert_eq!(replies[2], replies[0]);
        assert!(replies[0].contains(r#""isError":true"#));
    }
}

#[test]
fn outline_and_expand_report_bad_files_and_missing_arguments() {
    let f = fixture();
    let replies = exchange(
        &f,
        20,
        &[
            &tool_call("outline", r#"{"path":"src"}"#),
            &tool_call("outline", r#"{"path":"big.txt"}"#),
            &tool_call("outline", r#"{"path":"blob.bin"}"#),
            &tool_call("outline", r#"{"path":"latin1.txt"}"#),
            &tool_call("outline", r#"{"path":"nope.rs"}"#),
            &tool_call("outline", r#"{}"#),
            &tool_call("expand", r#"{"path":"src/shapes.rs"}"#),
            &tool_call("expand", r#"{"symbol":"area"}"#),
        ],
    );
    let expected = [
        "src is not a file",
        "file is too large",
        "not a text file",
        "not a text file",
        "does not exist",
        "path is required",
        "symbol is required",
        "path is required",
    ];
    for (reply, message) in replies.iter().zip(expected) {
        assert!(reply.contains(r#""isError":true"#), "{reply}");
        assert!(text_of(reply).ends_with(message), "{reply}");
    }
}

#[test]
fn expand_caps_its_output_at_four_hundred_lines() {
    let f = fixture();
    let replies = exchange(&f, 20, &[&tool_call("expand", r#"{"path":"src/long.rs","symbol":"dup"}"#)]);
    let text = text_of(&replies[0]);
    let lines: Vec<&str> = text.lines().collect();
    // The header, 400 numbered source lines, then the cap message.
    assert_eq!(lines.len(), 402);
    assert!(lines[0].ends_with(":1-401 fn dup"));
    assert_eq!(lines[1], "1\tfn dup() {");
    assert_eq!(lines[400], "400\t    let a398 = 398;");
    assert_eq!(lines[401], "output capped at 400 lines");
    // The second symbol of the same name is left out entirely.
    assert!(!text.contains("let b = 2"));
}

#[test]
fn serve_reports_stream_failures() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("closed"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let f = fixture();
    let ping = br#"{"jsonrpc":"2.0","id":1,"method":"ping"}
"#;
    assert!(server::serve(&f.boundary, 20, &mut &ping[..], &mut Broken).unwrap_err().contains("write"));
    let bad_utf8: &[u8] = &[0xff, 0xfe, b'\n'];
    assert!(server::serve(&f.boundary, 20, &mut &bad_utf8[..], &mut Vec::new()).unwrap_err().contains("read"));
}

// --------------------------------------------------------------------- cli

fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn arguments_configure_roots_and_the_default_limit() {
    let c = parse_args(&args(&["--root", "/a", "--root", "/b", "--limit", "7"])).unwrap();
    assert_eq!(c, Config { roots: vec![PathBuf::from("/a"), PathBuf::from("/b")], limit: 7 });
    assert_eq!(parse_args(&args(&["--root", "/a"])).unwrap().limit, DEFAULT_LIMIT);
    for bad in [
        vec![],
        vec!["--root"],
        vec!["--root", "/a", "--limit"],
        vec!["--root", "/a", "--limit", "x"],
        vec!["--root", "/a", "--limit", "0"],
        vec!["--root", "/a", "--limit", "201"],
        vec!["--root", "/a", "--bogus"],
    ] {
        assert!(parse_args(&args(&bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn a_leading_tilde_in_a_root_expands_against_home() {
    let home = Some("/home/me");
    let c = parse_args_with_home(&args(&["--root", "~/x"]), home).unwrap();
    assert_eq!(c.roots, [PathBuf::from("/home/me/x")]);
    let c = parse_args_with_home(&args(&["--root", "~"]), home).unwrap();
    assert_eq!(c.roots, [PathBuf::from("/home/me")]);
    // Only a bare `~` or `~/` expands; `~user/x` and a plain path are left alone.
    for unchanged in ["~user/x", "/abs/x", "rel/x", "a~/x"] {
        let c = parse_args_with_home(&args(&["--root", unchanged]), home).unwrap();
        assert_eq!(c.roots, [PathBuf::from(unchanged)], "{unchanged}");
    }
}

#[test]
fn a_tilde_root_without_home_is_an_error() {
    for home in [None, Some("")] {
        for root in ["~", "~/x"] {
            assert_eq!(
                parse_args_with_home(&args(&["--root", root]), home).unwrap_err(),
                "--root ~ needs HOME to be set",
                "{root} with {home:?}"
            );
        }
        // A root that needs no expansion is fine without HOME.
        let c = parse_args_with_home(&args(&["--root", "/abs/x"]), home).unwrap();
        assert_eq!(c.roots, [PathBuf::from("/abs/x")]);
    }
}

#[test]
fn a_missing_root_among_several_is_skipped_with_a_warning() {
    let f = fixture();
    let missing = f.root.join("no-such-clone");
    let a = args(&["--root", missing.to_str().unwrap(), "--root", f.root.to_str().unwrap()]);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli(&a, &mut &b""[..], &mut out, &mut err);
    assert_eq!(code, 0);
    let err = String::from_utf8(err).unwrap();
    assert!(err.contains("warning") && err.contains("no-such-clone") && err.contains("skipped"), "{err}");
}

#[test]
fn an_existing_root_that_is_not_a_directory_stays_fatal() {
    let f = fixture();
    let file = f.root.join("README.md");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli(&args(&["--root", file.to_str().unwrap()]), &mut &b""[..], &mut out, &mut err);
    assert_eq!(code, 2);
    assert!(String::from_utf8(err).unwrap().contains("not a directory"));
}

#[test]
fn all_roots_missing_refuses_to_start() {
    let f = fixture();
    let a = args(&["--root", f.root.join("x").to_str().unwrap(), "--root", f.root.join("y").to_str().unwrap()]);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(cli(&a, &mut &b""[..], &mut out, &mut err), 2);
    let err = String::from_utf8(err).unwrap();
    assert!(err.contains("none of the roots exists")
            && err.contains(&f.root.join("x").display().to_string())
            && err.contains(&f.root.join("y").display().to_string()),
        "{err}");
}

#[test]
fn cli_serves_over_the_given_streams() {
    let f = fixture();
    let root = f.root.display().to_string();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let input = tool_call("search", r#"{"query":"gadget_factory"}"#) + "\n";
    let code = cli(&args(&["--root", &root, "--limit", "3"]), &mut input.as_bytes(), &mut out, &mut err);
    assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
    assert!(String::from_utf8(out).unwrap().contains("src/gadget.rs:1"));

    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(cli(&args(&[]), &mut "".as_bytes(), &mut out, &mut err), 2);
    assert!(String::from_utf8(err).unwrap().contains("usage:"));

    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(cli(&args(&["--root", "/no/such/dir"]), &mut "".as_bytes(), &mut out, &mut err), 2);
    assert!(String::from_utf8(err).unwrap().contains("/no/such/dir"));

    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(cli(&args(&["--root", &root]), &mut &[0xffu8, b'\n'][..], &mut out, &mut err), 1);
    assert!(String::from_utf8(err).unwrap().contains("read"));
}

#[test]
fn run_takes_the_process_arguments() {
    assert_eq!(run(vec![OsString::from("--bogus")]), 2);
}

#[test]
fn a_wrapped_signature_keeps_its_body() {
    let rust = "pub fn foo(\n    a: u32,\n) -> u32 {\n    a\n}\n";
    assert_eq!(ranges(&outline::symbols(rust)), [(1, "fn", "foo", 1, 5)]);
    let python = "def f(\n    a,\n):\n    return a\n\nx = 1\n";
    assert_eq!(ranges(&outline::symbols(python)), [(1, "def", "f", 1, 4)]);
    let wheres = "fn f<T>(\n    a: T,\n) -> T\nwhere\n    T: Copy,\n{\n    a\n}\nfn g() {}\n";
    assert_eq!(ranges(&outline::symbols(wheres)), [(1, "fn", "f", 1, 8), (9, "fn", "g", 9, 9)]);
    let lone_brace = "fn h()\n{\n    1\n}\n";
    assert_eq!(ranges(&outline::symbols(lone_brace)), [(1, "fn", "h", 1, 4)]);
    let unclosed = "fn k(\n    a,\n";
    assert_eq!(ranges(&outline::symbols(unclosed)), [(1, "fn", "k", 1, 2)]);
    let tuple = "struct S(\n    u32,\n);\nfn after() {}\n";
    assert_eq!(ranges(&outline::symbols(tuple)), [(1, "struct", "S", 1, 3), (4, "fn", "after", 4, 4)]);
}

#[test]
fn an_enclosing_blocks_closer_is_not_part_of_the_symbol() {
    let text = "trait T {\n    fn a(&self);\n}\n";
    assert_eq!(ranges(&outline::symbols(text)), [(1, "trait", "T", 1, 3), (2, "fn", "a", 2, 2)]);
}

#[test]
fn impl_names_survive_nested_generics() {
    let text = "impl<T: Into<String>> Foo<T> {\n}\nimpl<T";
    assert_eq!(ranges(&outline::symbols(text)), [(1, "impl", "Foo<T>", 1, 2)]);
}
