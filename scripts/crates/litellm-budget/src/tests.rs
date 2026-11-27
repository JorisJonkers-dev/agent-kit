use super::*;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

const KEY: &str = "sk-virtual-key-1234";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Tmp {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("litellm-budget-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
    fn file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }
    fn script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.file(name, &format!("#!/bin/sh\n{body}\n"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn info() -> String {
    r#"{"info": {"max_budget": 255.0, "budget_duration": "30d", "budget_reset_at": "2026-12-01T00:00:00Z", "spend": 1.5, "key_name": "sk-...1234", "token": "hashed-secret-field"}}"#.into()
}

/// A stub proxy: answers each connection with the next reply and yields the raw requests.
fn stub(replies: Vec<(u16, String)>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, body) in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !String::from_utf8_lossy(&request).contains("\r\n\r\n") {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..n]);
            }
            seen.push(String::from_utf8_lossy(&request).into_owned());
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nLocation: http://127.0.0.1:1/\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
        seen
    });
    (url, handle)
}

fn routes(tmp: &Tmp, base: &str) -> PathBuf {
    tmp.file(
        "routes.json",
        &format!(
            r#"{{"base_url": "{base}", "routes": [{{"agent": "vibe", "key_env": "LITELLM_VIBE_KEY"}}]}}"#
        ),
    )
}

fn ctx(tmp: &Tmp, base: &str) -> Ctx {
    let mut env = BTreeMap::new();
    env.insert("LITELLM_VIBE_KEY".to_string(), KEY.to_string());
    Ctx {
        env,
        routes: routes(tmp, base),
        secret: tmp.0.join("no-store"),
        curl: "curl".into(),
        secret_timeout: Duration::from_secs(5),
    }
}

fn main_run(args: &[&str], ctx: Ctx) -> (i32, String, String) {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = main_with(&args, ctx, &mut out, &mut err);
    (
        code,
        String::from_utf8(out).unwrap(),
        String::from_utf8(err).unwrap(),
    )
}

#[test]
fn prints_only_the_budget_fields_and_a_redacted_request() {
    let tmp = Tmp::new();
    let (url, seen) = stub(vec![(200, info())]);
    let (code, out, err) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 0);
    let request = &seen.join().unwrap()[0];
    assert!(request.starts_with("GET /key/info HTTP/1.1"));
    assert!(
        request.contains(&format!("authorization: Bearer {KEY}"))
            || request.contains(&format!("Authorization: Bearer {KEY}"))
    );
    assert!(!request.to_lowercase().contains("x-litellm-api-key"));
    for needle in [
        "\"30d\"",
        "255.0",
        "2026-12-01T00:00:00Z",
        "1.5",
        "GET ",
        "/key/info",
        "redacted",
    ] {
        assert!(out.contains(needle), "{needle} in {out}");
    }
    for secret in [KEY, "hashed-secret-field", "key_name"] {
        assert!(!out.contains(secret) && !err.contains(secret));
    }
}

#[test]
fn a_stored_legacy_edge_token_is_never_sent() {
    let tmp = Tmp::new();
    let (url, seen) = stub(vec![(200, info())]);
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.env
        .insert("LITELLM_EDGE_TOKEN".into(), "smt_old_edge".into());
    let (shown, _) = measure("vibe", Some(&url), &c).unwrap();
    let request = &seen.join().unwrap()[0];
    assert!(request.contains(KEY) && !request.contains("smt_old_edge"));
    assert!(!shown.contains(KEY));
}

#[test]
fn an_unset_budget_prints_null() {
    let tmp = Tmp::new();
    let (url, _seen) = stub(vec![(200, r#"{"info": {"spend": 0}}"#.into())]);
    let (code, out, _) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 0);
    assert!(out.contains("budget_duration: null"));
}

#[test]
fn a_refusal_fails_with_the_status_and_no_secret() {
    let tmp = Tmp::new();
    let (url, _seen) = stub(vec![(401, format!(r#"{{"error": "bad key {KEY}"}}"#))]);
    let (code, _, err) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 1);
    assert!(err.contains("HTTP 401") && !err.contains(KEY));
}

#[test]
fn an_unusable_reply_fails() {
    for body in ["not json", "[]", r#"{"info": 3}"#] {
        let tmp = Tmp::new();
        let (url, _seen) = stub(vec![(200, body.into())]);
        let (code, _, err) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
        assert_eq!(code, 1, "{body}");
        assert!(err.contains("proxy"), "{err}");
    }
}

#[test]
fn an_unreachable_proxy_fails() {
    let tmp = Tmp::new();
    let (code, _, err) = main_run(
        &["--url", "http://127.0.0.1:1"],
        ctx(&tmp, "https://x.example/v1"),
    );
    assert_eq!(code, 1);
    assert!(err.contains("no usable answer"));
}

#[test]
fn a_missing_curl_or_one_that_prints_no_status_is_no_usable_answer() {
    let tmp = Tmp::new();
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.curl = "/no/such/curl".into();
    let (code, _, err) = main_run(&["--url", "http://127.0.0.1:1"], c);
    assert!(code == 1 && err.contains("did not run"));
    for output in ["printf x", "printf 'x\\nabc'"] {
        let mut c = ctx(&tmp, "https://x.example/v1");
        c.curl = tmp
            .script("curl", &format!("cat >/dev/null; {output}"))
            .display()
            .to_string();
        let (code, _, err) = main_run(&["--url", "http://127.0.0.1:1"], c);
        assert_eq!(code, 1);
        assert!(err.contains("HTTP 0"), "{err}");
    }
}

#[test]
fn plain_http_off_loopback_is_refused() {
    let tmp = Tmp::new();
    let (code, _, err) = main_run(
        &["--url", "http://litellm.example.com"],
        ctx(&tmp, "https://x.example/v1"),
    );
    assert_eq!(code, 2);
    assert!(err.contains("https"));
}

#[test]
fn the_default_url_is_the_route_tables() {
    let tmp = Tmp::new();
    let (url, seen) = stub(vec![(200, info())]);
    let (code, _, _) = main_run(&[], ctx(&tmp, &url));
    assert_eq!(code, 0);
    assert!(seen.join().unwrap()[0].starts_with("GET /key/info"));
}

#[test]
fn a_url_with_credentials_is_refused() {
    let tmp = Tmp::new();
    let (code, _, err) = main_run(
        &["--url", "https://u:p@litellm.example.com"],
        ctx(&tmp, "https://x.example/v1"),
    );
    assert_eq!(code, 2);
    assert!(err.contains("credentials"));
}

#[test]
fn a_redirect_is_not_followed() {
    let tmp = Tmp::new();
    let (url, seen) = stub(vec![(302, String::new())]);
    let (code, _, err) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 1);
    assert_eq!(seen.join().unwrap().len(), 1);
    assert!(err.contains("HTTP 302"));
}

#[test]
fn an_unknown_agent_is_a_usage_error() {
    let tmp = Tmp::new();
    let (code, _, err) = main_run(&["--agent", "nope"], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 2);
    assert!(err.contains("known: vibe"));
}

#[test]
fn a_missing_credential_is_named_not_printed() {
    let tmp = Tmp::new();
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.env.clear();
    c.secret = tmp.script("refuse", "exit 1");
    let failure = credential("LITELLM_VIBE_KEY", &c).unwrap_err();
    assert_eq!(failure.code, 2);
    assert!(failure.message.contains("LITELLM_VIBE_KEY"));
    // a store that cannot run at all counts as missing too
    c.secret = tmp.0.join("gone");
    assert_eq!(credential("X", &c).unwrap_err().code, 2);
}

#[test]
fn a_hung_secret_store_counts_as_missing() {
    let tmp = Tmp::new();
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.env.clear();
    c.secret = tmp.script("hang", "sleep 30");
    c.secret_timeout = Duration::from_millis(100);
    assert_eq!(credential("NAME", &c).unwrap_err().code, 2);
}

#[test]
fn the_secret_store_is_the_fallback() {
    let tmp = Tmp::new();
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.env.clear();
    c.secret = tmp.script("store", "echo \"from-store-$2\"");
    assert_eq!(credential("NAME", &c).unwrap(), "from-store-NAME");
}

#[test]
fn an_unsafe_value_is_refused_without_echo() {
    let tmp = Tmp::new();
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.env.insert("NAME".into(), "bad value\r\nx: y".into());
    let failure = credential("NAME", &c).unwrap_err();
    assert!(!failure.message.contains("bad value"));
    c.env.insert("NAME".into(), "abc\n".into());
    assert!(credential("NAME", &c).is_err());
}

#[test]
fn a_missing_or_broken_route_table_is_a_usage_failure() {
    let tmp = Tmp::new();
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.routes = tmp.0.join("none.json");
    let (code, _, err) = main_run(&[], c);
    assert!(code == 2 && err.contains("cannot read the route table"));
    for body in ["not json", "{}"] {
        let mut c = ctx(&tmp, "https://x.example/v1");
        c.routes = tmp.file("bad.json", body);
        assert_eq!(main_run(&[], c).0, 2);
    }
}

#[test]
fn a_team_key_reports_its_team_budget() {
    let tmp = Tmp::new();
    let team = r#"{"team_info": {"max_budget": 255.0, "budget_duration": "30d", "budget_reset_at": "2026-12-01T00:00:00Z", "spend": 4.0}}"#;
    let key = r#"{"info": {"max_budget": null, "budget_duration": null, "spend": 0.5, "team_id": "team vibe/1"}}"#;
    let (url, seen) = stub(vec![(200, key.into()), (200, team.into())]);
    let (code, out, _) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 0);
    let seen = seen.join().unwrap();
    assert!(seen[0].starts_with("GET /key/info "));
    assert!(seen[1].starts_with("GET /team/info?team_id=team+vibe%2F1 "));
    assert!(out.contains("255.0") && out.contains("\"30d\"") && out.contains("4.0"));
    assert!(!out.contains(KEY));
}

#[test]
fn a_team_key_whose_team_has_no_info_fails() {
    let tmp = Tmp::new();
    let key = r#"{"info": {"team_id": "t"}}"#;
    let (url, _seen) = stub(vec![(200, key.into()), (200, "{}".into())]);
    let (code, _, err) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 1);
    assert!(err.contains("no team info"));
}

#[test]
fn a_url_with_control_characters_is_refused() {
    let tmp = Tmp::new();
    let (code, _, err) = main_run(
        &["--url", "http://127.0.0.1:1/a\nb"],
        ctx(&tmp, "https://x.example/v1"),
    );
    assert!(code == 2 && err.contains("control characters"));
}

#[test]
fn the_endpoint_reads_the_url() {
    assert_eq!(
        endpoint("HTTPS://h.example/v1/?q=1#f").unwrap(),
        "https://h.example/key/info"
    );
    assert_eq!(
        endpoint("http://[::1]:4000/x/v1").unwrap(),
        "http://[::1]:4000/x/key/info"
    );
    assert_eq!(
        endpoint("http://localhost:4000").unwrap(),
        "http://localhost:4000/key/info"
    );
    assert!(endpoint("litellm.example").is_err());
}

#[test]
fn the_command_line_is_checked() {
    let tmp = Tmp::new();
    let c = |t: &Tmp| ctx(t, "https://x.example/v1");
    let (code, out, _) = main_run(&["--help"], c(&tmp));
    assert!(code == 0 && out.contains("usage"));
    assert_eq!(main_run(&["--agent"], c(&tmp)).0, 2);
    assert_eq!(main_run(&["--bogus", "x"], c(&tmp)).0, 2);
    // --routes overrides the table
    let routes = tmp.file(
        "other.json",
        r#"{"base_url": "x", "routes": [{"agent": "other", "key_env": "K"}]}"#,
    );
    let (code, _, err) = main_run(&["--routes", routes.to_str().unwrap()], c(&tmp));
    assert!(code == 2 && err.contains("known: other"));
}

#[test]
fn the_kit_files_are_found_from_the_executable() {
    let tmp = Tmp::new();
    std::fs::create_dir_all(tmp.0.join("registry/generated/litellm")).unwrap();
    std::fs::create_dir_all(tmp.0.join("installer")).unwrap();
    std::fs::create_dir_all(tmp.0.join("scripts/target/release")).unwrap();
    tmp.file("registry/generated/litellm/routes.json", "{}");
    tmp.file("installer/agent-kit-secret", "");
    let exe = tmp.0.join("scripts/target/release/measure-litellm-budget");
    let c = real_ctx(&exe);
    assert_eq!(c.routes, tmp.0.join(ROUTES_REL));
    assert_eq!(c.secret, tmp.0.join(SECRET_REL));
    assert_eq!(find_up(&exe, "nothing/here"), None);
    // Away from a kit, the routes path stays relative and the secret is the installed helper.
    let c = real_ctx(Path::new("/nonexistent/exe"));
    assert_eq!(c.routes, PathBuf::from(ROUTES_REL));
    assert!(c.secret.ends_with(".local/bin/agent-kit-secret") || c.secret.exists());
}

#[test]
fn run_reads_the_process_environment() {
    assert_eq!(run(["--help".into()].into_iter()), 0);
}

/// A route table with the two shapes the registry carries: a budgeted route and an unbudgeted one.
fn shaped_routes(tmp: &Tmp, base: &str) -> PathBuf {
    tmp.file(
        "shaped.json",
        &format!(
            r#"{{"base_url": "{base}", "routes": [
                {{"agent": "budgeted", "key_env": "LITELLM_BUDGETED_KEY", "budget_eur": 255, "budget_duration": "30d"}},
                {{"agent": "unbudgeted", "key_env": "LITELLM_UNBUDGETED_KEY", "budget_eur": null, "budget_duration": null}}
            ]}}"#
        ),
    )
}

fn shaped_ctx(tmp: &Tmp, base: &str) -> Ctx {
    let mut c = ctx(tmp, base);
    c.routes = shaped_routes(tmp, base);
    c.env
        .insert("LITELLM_BUDGETED_KEY".into(), "sk-budgeted-1".into());
    c.env
        .insert("LITELLM_UNBUDGETED_KEY".into(), "sk-unbudgeted-1".into());
    c
}

#[test]
fn a_budgeted_route_reads_its_own_key_and_makes_one_request() {
    let tmp = Tmp::new();
    let (url, seen) = stub(vec![(200, info())]);
    let (shown, fields) = measure(
        "budgeted",
        Some(&url),
        &shaped_ctx(&tmp, "https://x.example/v1"),
    )
    .unwrap();
    let seen = seen.join().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(seen[0].contains("Authorization: Bearer sk-budgeted-1"));
    assert!(!shown.contains("sk-budgeted-1"));
    assert!(fields.contains(&("max_budget", Value::from(255.0))));
    assert!(fields.contains(&("budget_duration", Value::from("30d"))));
}

#[test]
fn an_unbudgeted_route_without_a_team_prints_null_budget() {
    let tmp = Tmp::new();
    let (url, seen) = stub(vec![(200, r#"{"info": {"spend": 0.25}}"#.into())]);
    let (shown, fields) = measure(
        "unbudgeted",
        Some(&url),
        &shaped_ctx(&tmp, "https://x.example/v1"),
    )
    .unwrap();
    assert_eq!(seen.join().unwrap().len(), 1);
    assert!(!shown.contains("sk-unbudgeted-1"));
    assert!(fields.contains(&("max_budget", Value::Null)));
    assert!(fields.contains(&("spend", Value::from(0.25))));
}

#[test]
fn a_key_that_is_not_set_is_refused_before_any_request() {
    let tmp = Tmp::new();
    let (url, seen) = stub(vec![]);
    let mut c = ctx(&tmp, "https://x.example/v1");
    c.env.remove("LITELLM_VIBE_KEY");
    let (code, out, err) = main_run(&["--url", &url], c);
    assert_eq!(code, 2);
    assert!(out.is_empty() && err.contains("LITELLM_VIBE_KEY"));
    assert!(seen.join().unwrap().is_empty());
}

#[test]
fn a_refused_team_lookup_fails_with_its_status_and_no_secret() {
    let tmp = Tmp::new();
    let key = r#"{"info": {"max_budget": null, "spend": 0.5, "team_id": "vibe-team"}}"#;
    let (url, seen) = stub(vec![
        (200, key.into()),
        (500, format!(r#"{{"error": "{KEY}"}}"#)),
    ]);
    let (code, out, err) = main_run(&["--url", &url], ctx(&tmp, "https://x.example/v1"));
    assert_eq!(code, 1);
    assert_eq!(seen.join().unwrap().len(), 2);
    assert!(err.contains("/team/info") && err.contains("HTTP 500"));
    assert!(!err.contains(KEY) && !out.contains(KEY));
}
