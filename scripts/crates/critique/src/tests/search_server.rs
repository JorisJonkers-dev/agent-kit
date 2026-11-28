//! What the corpus does not reach: the search server's command-line numbers, and its start through `run_with`.

use crate::search_server::js_number;

#[test]
fn a_command_line_number_reads_as_javascript_number_reads_it() {
    let cases: [(&str, Option<f64>); 12] = [
        ("", Some(0.0)),
        ("  ", Some(0.0)),
        (" 5 ", Some(5.0)),
        ("0x1F", Some(31.0)),
        ("0b", None),
        ("Infinity", Some(f64::INFINITY)),
        ("-Infinity", Some(f64::NEG_INFINITY)),
        ("1e3", Some(1000.0)),
        (".5", Some(0.5)),
        ("1e", None),
        ("inf", None),
        ("1.2.3", None),
    ];
    for (text, number) in cases {
        assert_eq!(js_number(text), number, "{text:?}");
    }
}

#[test]
fn the_search_server_starts_from_the_command_line() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let runtime = crate::runtime::default_runtime();
    let code = crate::run_with(["search-server".to_string()].into_iter(), &runtime, &mut std::io::empty(), &mut out, &mut err);
    assert_eq!((code, String::from_utf8_lossy(&err).into_owned()), (2, crate::search_server::SEARCH_SERVER_USAGE.to_string()));
}

#[test]
fn a_file_system_failure_reads_as_node_words_it() {
    let missing = std::fs::read("/nonexistent/critique-search");
    assert_eq!(crate::search_server::io(missing, "open", "/x").unwrap_err(), "ENOENT: no such file or directory, open '/x'");
}

/// One request to the HTTP server and the status line and body it answers with.
fn post(port: u16, method: &str, body: &str) -> (String, String) {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(stream, "{method} /mcp HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("write");
    let mut text = String::new();
    // The server keeps the connection open, so read the one answer by its length.
    let mut buffer = [0u8; 4096];
    loop {
        let read = stream.read(&mut buffer).expect("read");
        text.push_str(&String::from_utf8_lossy(&buffer[..read]));
        let Some((head, rest)) = text.split_once("\r\n\r\n") else { continue };
        let length: usize = head.lines().find_map(|line| line.strip_prefix("Content-Length: ")).and_then(|value| value.parse().ok()).unwrap_or(0);
        if rest.len() >= length || read == 0 {
            return (head.lines().next().unwrap_or("").to_string(), rest.to_string());
        }
    }
}

#[test]
fn the_http_search_server_answers_the_same_calls_as_stdio_over_the_packet_alone() {
    let dir = std::env::temp_dir().join(format!("critique-http-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("tree")).unwrap();
    std::fs::write(dir.join("tree/a.txt"), "alpha\nneedle here\n").unwrap();
    let server = crate::search_server::serve_http(&dir.to_string_lossy(), crate::packet_search::default_search_limits()).expect("a server");
    let port = server.port();
    let (status, listed) = post(port, "POST", r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
    assert!(status.contains("200") && listed.contains("grep") && listed.contains("list"), "{status} {listed}");
    let (_, found) = post(port, "POST", r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"grep","arguments":{"pattern":"needle"}}}"#);
    assert!(found.contains("tree/a.txt:2: needle here"), "{found}");
    let (_, outside) = post(port, "POST", r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list","arguments":{"glob":"../*"}}}"#);
    assert!(outside.contains("No paths match") && !outside.contains("a.txt"), "{outside}");
    let (notified, empty) = post(port, "POST", r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert!(notified.contains("202") && empty.is_empty(), "{notified}");
    let (refused, _) = post(port, "GET", "");
    assert!(refused.contains("405"), "{refused}");
    drop(server);
    assert!(crate::search_server::serve_http("/nonexistent/critique-http", crate::packet_search::default_search_limits()).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_listener_that_cannot_open_or_name_itself_says_so() {
    let error = || std::io::Error::other("boom");
    assert_eq!(crate::search_server::listen_error(error()), "the search server cannot listen: boom");
    assert_eq!(crate::search_server::address_error(error()), "the search server has no address: boom");
}
