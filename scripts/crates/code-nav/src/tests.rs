use super::*;
use crate::boundary::{is_secret, Boundary, Refusal};
use crate::search::{search, DEFAULT_LIMIT, MAX_FILE_BYTES, MAX_LIMIT};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    let boundary = Boundary::new(&[root.clone()]).unwrap();
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
    assert_eq!(f.boundary.roots(), &[f.root.clone()]);
}

#[test]
fn secret_names_are_recognised() {
    for secret in [
        ".env", ".env.local", ".ENV", "a/.env.production", "id_rsa", "id_ed25519", "id_ecdsa", ".netrc", ".npmrc",
        "x/credentials", "creds/server.key", "a.p12", ".ssh", ".ssh/config", "home/.aws/config", ".gnupg/pubring.kbx",
        "store.kdbx", "Secrets.json",
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
    assert_eq!(search(&[base.clone()], "needle", 0).hits.len(), 1);
    assert_eq!(search(&[base.clone()], "needle", 5000).hits.len(), MAX_LIMIT);
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
    let r = search(&[base.clone()], "needle", 5);
    assert_eq!(r.hits[0].text.chars().count(), 200);
    assert!(r.hits[0].text.starts_with("NEEDLE"));
    assert!(search(&[base.clone()], "absent", 5).hits.is_empty());
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
    let total = search(&[base.clone()], "needle", 5).total;
    fs::set_permissions(base.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
    // Root can read it anyway; anyone else cannot.
    assert!(total <= 1);
}

// ------------------------------------------------------------------ server

fn exchange(f: &Fixture, default_limit: usize, requests: &[&str]) -> Vec<String> {
    let mut input = requests.join("\n").into_bytes();
    input.push(b'\n');
    let mut out = Vec::new();
    server::serve(&f.boundary, default_limit, &mut &input[..], &mut out).unwrap();
    String::from_utf8(out).unwrap().lines().map(String::from).collect()
}

fn tool_call(args: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"search","arguments":{args}}}}}"#)
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
    let replies = exchange(&f, 2, &[&tool_call(r#"{"query":"widget"}"#), &tool_call(r#"{"query":"widget","limit":50}"#), &tool_call(r#"{"query":"zzz"}"#)]);
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
            &tool_call(r#"{"query":"widget","path":"src"}"#),
            &tool_call(&format!(r#"{{"query":"widget","path":"{outside}"}}"#)),
            &tool_call(r#"{"query":"widget","path":"link-out.txt"}"#),
            &tool_call(r#"{"query":"widget","path":".env"}"#),
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
            &tool_call(r#"{}"#),
            &tool_call(r#"{"query":""}"#),
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
fn cli_serves_over_the_given_streams() {
    let f = fixture();
    let root = f.root.display().to_string();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let input = tool_call(r#"{"query":"gadget_factory"}"#) + "\n";
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
