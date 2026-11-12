//! Seam 3 for the code-nav server (spec #392, story 158; ticket B7-10).
//!
//! An agent id at `read` reach, started by the launcher under the operating
//! system's write limit (Landlock on Linux, `sandbox-exec` on macOS), can call
//! every tool of the server; the server's boundary still refuses a path
//! outside its root, a symlink out, a secret file and an employer's tree; and
//! the server writes no file.
//!
//! The test has no libtest harness: it re-executes itself as the launcher
//! (`--launcher ARGS`), so the server runs as the launcher's real child with
//! real stdin and stdout pipes. A control launch of a shell at the same reach
//! level proves the write limit is on for the same record, level and context.

#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::ffi::OsString;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod proof {
    use code_nav_eval::{parse_json, Json};
    use std::collections::BTreeMap;
    use std::ffi::OsString;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    /// The launcher entry: runs `args` (`run RECORD ID -- ...`) and exits with its code.
    pub fn launcher(args: Vec<OsString>) -> ! {
        std::process::exit(agent_launcher::run(args))
    }

    /// Everything under `dir`: kind, size, modification time and content, by path.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, String> {
        fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, String>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let meta = fs::symlink_metadata(&path).unwrap();
                let mut line = format!("{:?} {} {:?}", meta.file_type(), meta.len(), meta.modified().unwrap());
                if meta.file_type().is_symlink() {
                    line += &format!(" -> {:?}", fs::read_link(&path).unwrap());
                } else if meta.is_dir() {
                    walk(&path, out);
                } else {
                    line += &format!(" {:?}", fs::read(&path).unwrap());
                }
                out.insert(path, line);
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, &mut out);
        out
    }

    /// A fixture: the server's root `base/ws`, holding the repositories `repo` (code, a README, a secret, a
    /// symlink that leaves the root, an employer's tree, a `.git` file) and a
    /// directory beside it the root must not reveal.
    fn fixture() -> (PathBuf, PathBuf) {
        let base = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!("code-nav-reach-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("ws/repo");
        let ws = base.join("ws");
        let write = |rel: &str, text: &str| {
            let path = base.join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write("ws/repo/src/lib.rs", "pub fn reachable_marker() -> u32 {\n    1\n}\n\npub fn caller() -> u32 {\n    reachable_marker()\n}\n");
        write("ws/repo/README.md", "reachable_marker is the entry point\n");
        write("ws/repo/.env", "TOKEN=secretwordenv\n");
        write("ws/repo/nedap-app/src/a.rs", "pub fn withheldword() {}\n");
        write("outside/loot.txt", "outsideword reachable_marker\n");
        write("outside/loot.rs", "pub fn outsideword() {}\n");
        symlink(base.join("outside/loot.txt"), repo.join("escape.txt")).unwrap();
        symlink(base.join("outside"), repo.join("escape-dir")).unwrap();
        write("ws/repo/Cargo.toml", "[package]\nname = \"repo\"\n");
        write("ws/other/Cargo.toml", "[package]\nname = \"other\"\n\n[dependencies]\nrepo = \"1\"\n");
        fs::create_dir_all(base.join("meta")).unwrap();
        (base, ws)
    }

    /// The support record: one agent, `code-nav`, and one `sh`, both at `read`
    /// verified on this OS, and the id for each.
    fn record(base: &Path) -> PathBuf {
        let server = env!("CARGO_BIN_EXE_code-nav");
        let host = agent_launcher::this_os();
        let text = format!(
            "agents:\n  code-nav:\n    command: [{server}]\n    levels:\n      read: {{support: {{linux: verified, mac: verified}}}}\n  sh:\n    command: [/bin/sh]\n    levels:\n      read: {{support: {{linux: verified, mac: verified}}}}\nids:\n  code-nav-read:\n    agent: code-nav\n    reach: read\n    hosts: [{host}]\n  sh-read:\n    agent: sh\n    reach: read\n    hosts: [{host}]\n"
        );
        let file = base.join("meta/reach.yaml");
        fs::write(&file, text).unwrap();
        file
    }

    fn spawn_launcher(record: &Path, id: &str, extra: &[&OsString]) -> std::process::Child {
        Command::new(std::env::current_exe().unwrap())
            .arg("--launcher")
            .arg("run")
            .arg(record)
            .arg(id)
            .arg("--")
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }

    fn call(id: u32, tool: &str, args: &str) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{tool}","arguments":{args}}}}}"#)
    }

    fn quoted(path: &Path) -> String {
        format!("{:?}", path.to_str().unwrap())
    }

    /// The text of one reply and whether it is an error result.
    fn text_of(reply: &Json) -> (String, bool) {
        let result = reply.get("result").expect("a result");
        let text = result.get("content").and_then(|c| c.as_arr()).and_then(|a| a.first()).and_then(|b| b.get("text")).and_then(Json::as_str).unwrap_or("").to_string();
        (text, result.get("isError").map(|v| matches!(v, Json::Bool(true))).unwrap_or(false))
    }

    pub fn main() {
        let (base, ws) = fixture();
        let repo = ws.join("repo");
        let record = record(&base);
        let before = snapshot(&base);

        let root_arg = ws.clone().into_os_string();
        let flag = OsString::from("--root");
        let mut child = spawn_launcher(&record, "code-nav-read", &[&flag, &root_arg]);

        let requests = [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}"#.to_string(),
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string(),
            call(3, "search", r#"{"query":"reachable_marker"}"#),
            call(4, "outline", &format!(r#"{{"path":{}}}"#, quoted(&repo.join("src/lib.rs")))),
            call(5, "expand", &format!(r#"{{"path":{},"symbol":"caller"}}"#, quoted(&repo.join("src/lib.rs")))),
            call(6, "map", "{}"),
            call(7, "symbols", r#"{"name":"reachable_marker"}"#),
            call(8, "related_repos", r#"{"repo":"repo"}"#),
            // The boundary, in the same launch.
            call(9, "search", r#"{"query":"secretwordenv"}"#),
            call(10, "search", r#"{"query":"outsideword"}"#),
            call(11, "search", r#"{"query":"withheldword"}"#),
            call(12, "outline", &format!(r#"{{"path":{}}}"#, quoted(&base.join("outside/loot.rs")))),
            call(13, "outline", &format!(r#"{{"path":{}}}"#, quoted(&repo.join(".env")))),
            call(14, "outline", &format!(r#"{{"path":{}}}"#, quoted(&repo.join("escape-dir/loot.rs")))),
            call(15, "search", &format!(r#"{{"query":"outsideword","path":{}}}"#, quoted(&repo.join("escape.txt")))),
            call(16, "outline", &format!(r#"{{"path":{}}}"#, quoted(&repo.join("nedap-app/src/a.rs")))),
            call(17, "search", r#"{"query":"reachable_marker","path":"../outside"}"#),
        ];
        let mut stdin = child.stdin.take().unwrap();
        for request in &requests {
            writeln!(stdin, "{request}").unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "the launcher exited {:?}: {stderr}", output.status);

        let stdout = String::from_utf8(output.stdout).unwrap();
        let replies: Vec<Json> = stdout.lines().map(|l| parse_json(l).unwrap_or_else(|e| panic!("not JSON ({e}): {l}"))).collect();
        assert_eq!(replies.len(), requests.len(), "one reply per request: {stdout}");
        let at = |n: usize| text_of(&replies[n - 1]);

        // The server answers at read reach.
        assert!(replies[0].get("result").and_then(|r| r.get("serverInfo")).is_some(), "initialize: {stdout}");
        let tools = replies[1].get("result").and_then(|r| r.get("tools")).and_then(|t| t.as_arr()).expect("tools");
        assert_eq!(tools.len(), 6, "all six tools are listed");
        let (found, error) = at(3);
        assert!(!error && found.contains("src/lib.rs:1:") && found.contains("README.md:1:"), "search: {found}");
        let (outline, error) = at(4);
        assert!(!error && outline.contains("reachable_marker") && outline.contains("caller"), "outline: {outline}");
        let (expanded, error) = at(5);
        assert!(!error && expanded.contains("reachable_marker()"), "expand: {expanded}");
        let (map, error) = at(6);
        assert!(!error && map.contains("reachable_marker"), "map: {map}");
        let (symbols, error) = at(7);
        assert!(!error && symbols.contains("reachable_marker") && symbols.contains("src/lib.rs"), "symbols: {symbols}");
        let (related, error) = at(8);
        assert!(!error && related.contains("other"), "related_repos: {related}");

        // The boundary holds under the launcher.
        for n in [9, 10, 11] {
            let (text, error) = at(n);
            assert!(!error && text.contains("no matches"), "request {n} must find nothing: {text}");
        }
        for (n, word) in [(12, "outside the allowed roots"), (13, "secret file"), (14, "leaves the allowed roots"), (15, "leaves the allowed roots"), (16, "withheld"), (17, "outside the allowed roots")] {
            let (text, error) = at(n);
            assert!(error && text.contains(word), "request {n} must be refused with {word:?}: {text}");
        }

        // The server wrote nothing: not in its root, not beside it.
        assert_eq!(snapshot(&base), before, "the tree under the root and beside it is unchanged");

        // The control: a shell at the same reach level, same record and context,
        // cannot create a file in the root, the directory beside it, or the
        // working directory, so the limit is on for this launch shape.
        let cwd_probe = std::env::current_dir().unwrap().join("code-nav-reach-probe");
        let probes = [ws.join("probe"), repo.join("probe"), base.join("outside/probe"), base.join("probe"), cwd_probe.clone()];
        for probe in &probes {
            let script = OsString::from(r#"( : > "$1" ) 2>&1; exit $?"#);
            let (c, s, p) = (OsString::from("-c"), OsString::from("sh"), probe.clone().into_os_string());
            let child = spawn_launcher(&record, "sh-read", &[&c, &script, &s, &p]);
            let out = child.wait_with_output().unwrap();
            let said = String::from_utf8_lossy(&out.stdout).to_string();
            assert!(!out.status.success(), "a write to {} must fail at read reach", probe.display());
            assert!(said.contains("Permission denied") || said.contains("Operation not permitted"), "the operating system refused it: {said}");
            assert!(!probe.exists(), "{} was created", probe.display());
        }
        assert_eq!(snapshot(&base), before, "the control wrote nothing either");
        let _ = fs::remove_dir_all(&base);
        println!("read-reach proof: ok ({} requests, {} write probes refused)", requests.len(), probes.len());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn main() {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().map(|a| a == "--launcher").unwrap_or(false) {
        proof::launcher(args[1..].to_vec());
    }
    proof::main();
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
    println!("read-reach proof: skipped, the launcher supports only Linux and macOS");
}
