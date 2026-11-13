//! The search server over generated packets and JSON-RPC lines, compared with what the node runner's `search-server`
//! printed and exited with.

use crate::search_server::serve;
use model_gateway::json::{self, Value};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.as_object().and_then(|object| object.get(key)).and_then(Value::as_str).expect("a string field")
}

fn replay(index: usize, case: &Value) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("critique-search-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join("packet");
    std::fs::create_dir_all(dir.join("outside")).expect("outside");
    std::fs::create_dir_all(&root).expect("packet");
    std::fs::write(dir.join("outside/secret.md"), "reach proof: OUTSIDE-THE-PACKET\n").expect("secret");
    let object = case.as_object().expect("case");
    if let Some(Value::Obj(files)) = object.get("files") {
        for (path, body) in files.entries() {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
            std::fs::write(&file, body.as_str().expect("body")).expect("write");
        }
    }
    if let Some(Value::Arr(links)) = object.get("links") {
        for link in links {
            let Value::Arr(pair) = link else { panic!("a link is a pair") };
            let path = root.join(pair[0].as_str().expect("path"));
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::os::unix::fs::symlink(Path::new(pair[1].as_str().expect("target")), &path).expect("symlink");
        }
    }
    let root_text = std::fs::canonicalize(&root).expect("root").to_string_lossy().into_owned();
    let Some(Value::Arr(args)) = object.get("args") else { panic!("args") };
    let argv: Vec<String> = args.iter().map(|arg| arg.as_str().expect("arg").replace("@ROOT@", &root_text)).collect();
    if argv.iter().any(|arg| arg.ends_with("/notes-file")) {
        std::fs::write(root.join("notes-file"), "x\n").expect("file");
    }
    let mut input = std::io::Cursor::new(text(case, "input").as_bytes().to_vec());
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let code = serve(&argv, &mut input, &mut stdout, &mut stderr);
    let _ = std::fs::remove_dir_all(&dir);
    let expected = object.get("expected").expect("expected");
    let mask = |bytes: Vec<u8>| String::from_utf8(bytes).expect("utf-8").replace(&root_text, "@ROOT@");
    let Some(Value::Num(wanted)) = expected.as_object().and_then(|object| object.get("code")) else { panic!("code") };
    assert_eq!(mask(stdout), text(expected, "stdout"), "case {index}: stdout differs\n  input: {}", text(case, "input"));
    assert_eq!(mask(stderr), text(expected, "stderr"), "case {index}: stderr differs");
    assert_eq!(f64::from(code), *wanted, "case {index}: exit code differs");
}

/// Replays the corpus; `CRITIQUE_SEARCH_CORPUS` names a bigger one.
#[test]
fn search_corpus() {
    let source = match std::env::var("CRITIQUE_SEARCH_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/search-server.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}
