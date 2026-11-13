//! The acceptance criterion of B2-07, driven through the real executable: a stub
//! proxy that answers `budget_exceeded` ends the node's process with exit status 1
//! and one clear stderr line; the same stub below the cap lets it pass with 0.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Output};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const BUDGET_BODY: &str = r#"{"error":{"message":"ExceededBudget: Budget has been exceeded! Current cost: 5.1, Max budget: 5.0","type":"budget_exceeded","param":null,"code":"400"}}"#;
const KEY: &str = "sk-stub_KEY.123";

/// A one-shot stub proxy that answers whatever it is asked with `status`/`body`.
fn stub(status: u16, body: &'static str) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(_) => return,
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        // Read the whole request (headers, then the declared body) before replying.
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&request).into_owned();
            if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                let length = head
                    .to_ascii_lowercase()
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: ").map(str::to_string))
                    .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                if rest.len() >= length {
                    break;
                }
            }
        }
        let reply = format!(
            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(reply.as_bytes());
    });
    (url, handle)
}

fn gate(url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_litellm-gate"))
        .args(["check", "--node", "research-fetch", "--url", url])
        .args(["--key-env", "STUB_LITELLM_KEY", "--model", "glm-5.3-flash"])
        .env("STUB_LITELLM_KEY", KEY)
        .output()
        .unwrap()
}

#[test]
fn a_budget_refusal_ends_the_process_with_exit_status_1() {
    let (url, server) = stub(400, BUDGET_BODY);
    let out = gate(&url);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(
        stderr.trim_end(),
        "litellm-gate: node research-fetch failed: budget exhausted for key STUB_LITELLM_KEY"
    );
    assert!(out.stdout.is_empty());
    assert!(!stderr.contains(KEY));
}

#[test]
fn the_same_stub_below_the_cap_lets_the_node_pass_with_exit_status_0() {
    let (url, server) = stub(200, r#"{"choices":[]}"#);
    let out = gate(&url);
    server.join().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(
        stdout.trim_end(),
        "litellm-gate: node research-fetch: key STUB_LITELLM_KEY is within its budget"
    );
    assert!(out.stderr.is_empty());
    assert!(!stdout.contains(KEY));
}
