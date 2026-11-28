//! The tests run the real `run_in` against stub servers on loopback (real curl),
//! a fake secret store and a fake passphrase gate (shell scripts), in a scratch HOME.

use super::*;
use crate::http::{config, json_object, quote, send, Jar, Req};
use serde_json::json;
use std::io::Read;
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------- scaffolding

struct Tmp(PathBuf);

impl Tmp {
    fn new() -> Tmp {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("keys-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Tmp(dir)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Buf {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[path = "../../../test-support/fake_exe.rs"]
mod fake_exe;

fn script(path: &Path, body: &str) {
    fake_exe::write_script(path, &format!("#!/bin/sh\n{body}\n"));
}

const FAKE_SECRET: &str = r#"cmd=$1; var=$2; d=$FAKE_STORE
echo "$cmd $var" >> "$d.log"
if [ -n "$FAKE_STORE_REFUSE" ]; then echo "agent-kit-secret: no usable backend" >&2; exit 2; fi
if [ -n "$FAKE_STORE_SILENT" ]; then exit 2; fi
case $cmd in
get) [ -f "$d/$var" ] || exit 1; cat "$d/$var";;
put) if [ -n "$FAKE_STORE_DROP" ]; then cat >/dev/null; else cat > "$d/$var"; fi;;
delete) [ -f "$d/$var" ] || exit 1; rm "$d/$var";;
esac"#;

const FAKE_GATE: &str = r#"case $1 in
store) [ -n "$FAKE_GATE_REFUSE" ] && exit 1; cat > "$FAKE_GATE";;
status) [ -f "$FAKE_GATE" ] && echo present || echo missing;;
esac"#;

fn real_curl() -> String {
    let path = std::env::var("PATH").unwrap();
    std::env::split_paths(&path)
        .map(|d| d.join("curl"))
        .find(|p| p.is_file())
        .expect("curl on PATH")
        .to_string_lossy()
        .into_owned()
}

struct Run {
    code: i32,
    out: String,
    err: String,
}

struct World {
    tmp: Tmp,
    env: HashMap<String, String>,
    table: PathBuf,
}

impl World {
    fn new(base: &str, table: &Value) -> World {
        let tmp = Tmp::new();
        let (bin, home, store) = (tmp.path("bin"), tmp.path("home"), tmp.path("store"));
        for dir in [&bin, &home, &store] {
            std::fs::create_dir_all(dir).unwrap();
        }
        script(&bin.join("secret"), FAKE_SECRET);
        script(&bin.join("private"), FAKE_GATE);
        script(
            &bin.join("git"),
            "[ -n \"$FAKE_GIT_EMAIL\" ] && echo \"$FAKE_GIT_EMAIL\"; exit 0",
        );
        // Logs every curl argv, then runs the real one.
        script(
            &bin.join("curl"),
            &format!(
                "echo \"$@\" >> \"$FAKE_STORE.curl\"; exec {} \"$@\"",
                real_curl()
            ),
        );
        // The installed copy env.sh reads through.
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        script(&home.join(".local/bin/agent-kit-secret"), FAKE_SECRET);
        let real = std::env::var("PATH").unwrap();
        let env: HashMap<String, String> = [
            ("PATH", format!("{}:{real}", bin.display())),
            ("HOME", home.display().to_string()),
            ("SHELL", "/bin/zsh".to_string()),
            ("FAKE_STORE", store.display().to_string()),
            ("FAKE_GATE", tmp.path("gate").display().to_string()),
            (
                "AGENT_KIT_SECRET_TOOL",
                bin.join("secret").display().to_string(),
            ),
            (
                "AGENT_KIT_PRIVATE_TOOL",
                bin.join("private").display().to_string(),
            ),
            ("AGENT_KIT_SECRETS_BACKEND", "keychain".to_string()),
            ("AGENT_KIT_AUTH_URL", base.to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let path = tmp.path("table.json");
        std::fs::write(&path, table.to_string()).unwrap();
        World {
            tmp,
            env,
            table: path,
        }
    }

    fn ctx(&self, answers: &str) -> (Ctx, Buf, Buf) {
        let (out, err) = (Buf::default(), Buf::default());
        let ctx = Ctx {
            env: self.env.clone(),
            home: self.tmp.path("home"),
            input: Box::new(std::io::Cursor::new(answers.to_string().into_bytes())),
            tty: false,
            out: Box::new(out.clone()),
            err: Box::new(err.clone()),
            host: "box.local".to_string(),
            exe: self.tmp.path("elsewhere/bin/agent-kit-keys"),
        };
        (ctx, out, err)
    }

    fn keys(&self, args: &[&str], answers: &str) -> Run {
        let (mut ctx, out, err) = self.ctx(answers);
        let table = self.table.display().to_string();
        let mut argv = vec!["--table".to_string(), table];
        argv.extend(args.iter().map(|a| a.to_string()));
        let code = run_in(&mut ctx, &argv);
        Run {
            code,
            out: out.text(),
            err: err.text(),
        }
    }

    fn stored(&self) -> BTreeMap<String, String> {
        let mut found = BTreeMap::new();
        for entry in std::fs::read_dir(self.tmp.path("store")).unwrap().flatten() {
            let value = std::fs::read_to_string(entry.path()).unwrap();
            found.insert(entry.file_name().to_string_lossy().into_owned(), value);
        }
        found
    }

    fn put(&self, name: &str, value: &str) {
        std::fs::write(self.tmp.path("store").join(name), value).unwrap();
    }

    fn home(&self, rel: &str) -> PathBuf {
        self.tmp.path("home").join(rel)
    }

    fn gate(&self) -> Option<String> {
        std::fs::read_to_string(self.tmp.path("gate")).ok()
    }

    fn logs(&self) -> String {
        let store = std::fs::read_to_string(self.tmp.path("store.log")).unwrap_or_default();
        store + &std::fs::read_to_string(self.tmp.path("store.curl")).unwrap_or_default()
    }

    fn table_entries(&self) -> Vec<Value> {
        serde_json::from_str(&std::fs::read_to_string(&self.table).unwrap()).unwrap()
    }

    fn set_table(&self, entries: &[Value]) {
        std::fs::write(&self.table, json!(entries).to_string()).unwrap();
    }
}

// ---------------------------------------------------------------- a tiny HTTP server

struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Request {
    fn bearer(&self) -> String {
        self.headers
            .get("authorization")
            .and_then(|a| a.strip_prefix("Bearer "))
            .unwrap_or("")
            .to_string()
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(json!({}))
    }
    fn cookies(&self) -> String {
        self.headers.get("cookie").cloned().unwrap_or_default()
    }
}

/// Status 0 closes the connection without an answer.
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: body.to_string(),
    }
}

fn bare(status: u16) -> Reply {
    Reply {
        status,
        headers: vec![],
        body: String::new(),
    }
}

fn with_header(mut r: Reply, name: &str, value: &str) -> Reply {
    r.headers.push((name.into(), value.into()));
    r
}

fn read_request(stream: &mut std::net::TcpStream) -> Option<Request> {
    let mut data = Vec::new();
    let mut byte = [0u8; 1];
    while !data.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        data.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&data).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next()?.split(' ');
    let (method, path) = (first.next()?.to_string(), first.next()?.to_string());
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(k, v)| (k.to_lowercase(), v.to_string()))
        .collect();
    let len: usize = headers
        .get("content-length")
        .and_then(|l| l.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

fn serve<F>(handler: F) -> String
where
    F: Fn(&Request) -> Reply + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let Some(request) = read_request(&mut stream) else {
                continue;
            };
            let r = handler(&request);
            if r.status == 0 {
                continue;
            }
            let mut text = format!(
                "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                r.status,
                r.body.len()
            );
            for (k, v) in &r.headers {
                text += &format!("{k}: {v}\r\n");
            }
            text += "\r\n";
            let _ = stream.write_all(text.as_bytes());
            let _ = stream.write_all(r.body.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}")
}

// ---------------------------------------------------------------- auth-api stub

struct AuthState {
    grants: BTreeSet<String>,
    role: String,
    totp: bool,
    minted: Vec<String>,
    valid: BTreeSet<String>,
    refused_services: BTreeSet<String>,
    revoked: Vec<String>,
    by_id: HashMap<String, String>,
    log: Vec<String>,
    me_empty: bool,
    no_mint_token: bool,
    unsafe_token: bool,
    drop_mint: bool,
    no_id: bool,
}

type SharedAuth = Arc<Mutex<AuthState>>;

fn auth_state() -> SharedAuth {
    Arc::new(Mutex::new(AuthState {
        grants: ["MEMORY_API".to_string()].into(),
        role: "ADMIN".into(),
        totp: true,
        minted: vec![],
        valid: BTreeSet::new(),
        refused_services: BTreeSet::new(),
        revoked: vec![],
        by_id: HashMap::new(),
        log: vec![],
        me_empty: false,
        no_mint_token: false,
        unsafe_token: false,
        drop_mint: false,
        no_id: false,
    }))
}

fn auth_reply(s: &mut AuthState, r: &Request) -> Reply {
    s.log.push(format!("{} {}", r.method, r.path));
    if r.headers.get("user-agent").map(String::as_str) != Some("agent-kit-keys/1") {
        return reply(403, json!({"detail": "error 1010"}));
    }
    let signed_in = r.cookies().contains("SESSION=ok");
    let xsrf = r.headers.get("x-xsrf-token").map(String::as_str) == Some("x1")
        && r.cookies().contains("XSRF-TOKEN=x1");
    let service_prefix = "/api/v1/auth/service-tokens/";
    match (r.method.as_str(), r.path.as_str()) {
        ("POST", "/api/v1/auth/session-login") => {
            let body = r.json();
            if body["password"] != "pw" {
                return reply(401, json!({"detail": "Bad credentials"}));
            }
            if s.totp && body["totpCode"] != "123456" {
                return reply(200, json!({"totpRequired": true}));
            }
            with_header(
                reply(200, json!({"success": true})),
                "Set-Cookie",
                "SESSION=ok; Path=/",
            )
        }
        ("POST", "/api/v1/auth/service-tokens") => {
            if s.drop_mint {
                return bare(0);
            }
            if !(signed_in && xsrf) {
                return reply(403, json!({"detail": "csrf"}));
            }
            let service = r.json()["service"].as_str().unwrap_or("").to_string();
            if !s.grants.contains(&service) {
                return reply(
                    403,
                    json!({"detail": format!("User does not hold the {service} service grant")}),
                );
            }
            if s.no_mint_token {
                return reply(200, json!({}));
            }
            let token = if s.unsafe_token {
                "bad token!".to_string()
            } else {
                format!("smt_{}_{}", service.to_lowercase(), s.minted.len())
            };
            s.minted.push(service.clone());
            if !s.refused_services.contains(&service) {
                s.valid.insert(token.clone());
            }
            let id = s.minted.len().to_string();
            s.by_id.insert(id.clone(), token.clone());
            if s.no_id {
                return reply(201, json!({"token": token}));
            }
            reply(201, json!({"token": token, "id": id}))
        }
        ("POST", p) if p.starts_with("/check/") => {
            if s.valid.contains(&r.bearer()) {
                reply(200, json!({}))
            } else {
                with_header(bare(302), "Location", "/login")
            }
        }
        ("GET", "/api/v1/auth/me") => {
            if !signed_in {
                return reply(401, json!({}));
            }
            let body = if s.me_empty {
                json!({})
            } else {
                json!({"id": "u1", "role": s.role})
            };
            with_header(reply(200, body), "Set-Cookie", "XSRF-TOKEN=x1; Path=/")
        }
        ("GET", "/api/v1/admin/users/u1") => reply(
            200,
            json!({"id": "u1", "servicePermissions": s.grants.iter().collect::<Vec<_>>()}),
        ),
        ("GET", "/login") => Reply {
            status: 200,
            headers: vec![],
            body: "<html>login</html>".into(),
        },
        ("GET", "/api/v1/auth/logout") => with_header(bare(302), "Location", "/login"),
        ("GET", "/mistral") => reply(if r.bearer() == "mk_good" { 200 } else { 401 }, json!({})),
        ("DELETE", p) if p.starts_with(service_prefix) => {
            let id = p[service_prefix.len()..].to_string();
            if signed_in && xsrf && s.by_id.contains_key(&id) {
                s.revoked.push(id.clone());
                let token = s.by_id.get(&id).cloned().unwrap_or_default();
                s.valid.remove(&token);
                return bare(204);
            }
            reply(403, json!({}))
        }
        ("PUT", "/api/v1/admin/users/u1/services") if signed_in && xsrf => {
            let wanted = r.json()["services"].as_array().cloned().unwrap_or_default();
            s.grants = wanted
                .iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect();
            reply(
                200,
                json!({"servicePermissions": s.grants.iter().collect::<Vec<_>>()}),
            )
        }
        ("PUT", _) => reply(403, json!({})),
        _ => reply(404, json!({})),
    }
}

fn auth_server(state: &SharedAuth) -> String {
    let state = state.clone();
    serve(move |r| auth_reply(&mut state.lock().unwrap(), r))
}

fn table(base: &str) -> Value {
    json!([
        {"env": "HINDSIGHT_API_URL", "value": "https://hindsight-api.jorisjonkers.dev", "export": true},
        {"env": "HINDSIGHT_API_TOKEN", "grant": "MEMORY_API", "check": format!("{base}/check/api"), "export": true},
        {"env": "MEMORY_MCP_TOKEN", "grant": "MEMORY_MCP", "check": format!("{base}/check/mcp"), "export": true},
        {"env": "MEMORY_PRIVATE_TOKEN", "grant": "MEMORY_PRIVATE", "check": format!("{base}/check/private"),
         "export": false, "gate": "passphrase"},
        {"env": "MISTRAL_API_KEY", "paste": "a Mistral key for this machine", "check": format!("{base}/mistral"),
         "check_method": "GET", "export": true}
    ])
}

fn world() -> (World, SharedAuth) {
    let state = auth_state();
    let base = auth_server(&state);
    (World::new(&base, &table(&base)), state)
}

const ANSWERS: &str = "me@example.com\npw\n123456\nmk_good\ny\n";

fn lines_of(out: &str) -> HashMap<String, String> {
    out.lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn ids(w: &World) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(w.home(".config/agent-kit/token-ids.json")).unwrap(),
    )
    .unwrap()
}

// ---------------------------------------------------------------- ensure

#[test]
fn ensure_mints_grants_and_stores_everything() {
    let (w, auth) = world();
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 0, "{}", run.err);
    let stored = w.stored();
    let keys: Vec<&str> = stored.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["HINDSIGHT_API_TOKEN", "MEMORY_MCP_TOKEN", "MISTRAL_API_KEY"]
    );
    // The private token is behind the gate, not in the store.
    let sealed = w.gate().unwrap();
    assert_eq!(stored["MISTRAL_API_KEY"], "mk_good");
    let s = auth.lock().unwrap();
    assert!(
        s.grants.contains("MEMORY_MCP")
            && s.grants.contains("MEMORY_PRIVATE")
            && s.grants.contains("MEMORY_API")
    );
    let mut minted = s.minted.clone();
    minted.sort();
    assert_eq!(minted, ["MEMORY_API", "MEMORY_MCP", "MEMORY_PRIVATE"]);
    assert!(s.valid.contains(&stored["HINDSIGHT_API_TOKEN"]) && s.valid.contains(&sealed));
    assert!(s.log.contains(&"GET /api/v1/auth/logout".to_string()));
    assert!(run
        .err
        .contains("keys: granted yourself MEMORY_MCP, MEMORY_PRIVATE"));
    assert!(run
        .err
        .contains("MEMORY_PRIVATE_TOKEN: minted, checked and sealed behind your passphrase"));
}

#[test]
fn a_bad_table_is_a_note_not_a_panic() {
    let (w, _) = world();
    std::fs::write(&w.table, "[{]").unwrap();
    let run = w.keys(&["status"], "");
    assert_eq!(run.code, 1);
    assert!(run.err.contains("keys: stopped:"), "{}", run.err);
    std::fs::write(&w.table, "{}").unwrap();
    assert!(w
        .keys(&["status"], "")
        .err
        .contains("is not a list of entries"));
    std::fs::remove_file(&w.table).unwrap();
    assert!(w.keys(&["status"], "").err.contains("cannot read"));
    std::fs::write(&w.table, "[{\"paste\": \"x\"}]").unwrap();
    assert!(w.keys(&["status"], "").err.contains("has no env"));
}

#[test]
fn secrets_never_reach_argv_or_output() {
    let (w, _) = world();
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 0, "{}", run.err);
    let mut secrets: Vec<String> = w.stored().into_values().collect();
    secrets.push(w.gate().unwrap());
    let logs = w.logs();
    assert!(logs.contains("--config -"), "curl was logged");
    for secret in secrets {
        assert!(!logs.contains(&secret), "{secret} reached an argv");
        assert!(!run.out.contains(&secret) && !run.err.contains(&secret));
    }
    assert!(!logs.contains("pw"), "the password reached an argv");
}

#[test]
fn a_second_run_with_everything_valid_asks_nothing() {
    let (w, auth) = world();
    w.keys(&["ensure"], ANSWERS);
    let before = auth.lock().unwrap().minted.clone();
    let logged = auth.lock().unwrap().log.len();
    let run = w.keys(&["ensure"], "");
    assert_eq!(run.code, 0, "{}", run.err);
    let s = auth.lock().unwrap();
    assert_eq!(s.minted, before);
    assert!(!s.log[logged..].iter().any(|l| l.contains("session-login")));
}

#[test]
fn an_invalid_stored_token_is_replaced_and_the_old_one_revoked() {
    let (w, auth) = world();
    w.keys(&["ensure"], ANSWERS);
    let old = w.stored()["MEMORY_MCP_TOKEN"].clone();
    let first_id = (auth
        .lock()
        .unwrap()
        .minted
        .iter()
        .position(|m| m == "MEMORY_MCP")
        .unwrap()
        + 1)
    .to_string();
    auth.lock().unwrap().valid.remove(&old);
    let run = w.keys(&["ensure"], "me@example.com\npw\n123456\n");
    assert_eq!(run.code, 0, "{}", run.err);
    let s = auth.lock().unwrap();
    assert_eq!(s.minted.iter().filter(|m| *m == "MEMORY_MCP").count(), 2);
    assert!(s.valid.contains(&w.stored()["MEMORY_MCP_TOKEN"]));
    assert_eq!(s.revoked, [first_id]);
    assert!(run
        .err
        .contains("MEMORY_MCP_TOKEN: revoked the token it replaces"));
    let ids = ids(&w);
    assert_eq!(ids["MEMORY_MCP_TOKEN"], s.minted.len().to_string());
    assert!(!ids.to_string().contains("smt_"), "ids only, never tokens");
}

#[test]
fn a_replaced_token_that_cannot_be_revoked_says_where() {
    let (w, auth) = world();
    w.keys(&["ensure"], ANSWERS);
    // The recorded id is one auth-api does not know.
    let mut recorded = ids(&w);
    recorded["MEMORY_MCP_TOKEN"] = json!("stale");
    std::fs::write(
        w.home(".config/agent-kit/token-ids.json"),
        recorded.to_string(),
    )
    .unwrap();
    auth.lock()
        .unwrap()
        .valid
        .remove(&w.stored()["MEMORY_MCP_TOKEN"]);
    let run = w.keys(&["ensure"], "me@example.com\npw\n123456\n");
    assert!(run.err.contains(
        "could not revoke the token it replaces (id stale); revoke it on the admin page"
    ));
}

#[test]
fn a_new_token_its_host_refuses_is_not_stored_and_is_revoked() {
    let (w, auth) = world();
    auth.lock()
        .unwrap()
        .refused_services
        .insert("MEMORY_MCP".into());
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(!w.stored().contains_key("MEMORY_MCP_TOKEN"));
    assert!(run
        .err
        .contains("MEMORY_MCP_TOKEN: the new token is refused by its host; not stored"));
    let s = auth.lock().unwrap();
    assert_eq!(
        s.revoked,
        [(s.minted.iter().position(|m| m == "MEMORY_MCP").unwrap() + 1).to_string()]
    );
    assert!(w.stored().contains_key("HINDSIGHT_API_TOKEN"));
}

#[test]
fn a_token_with_unexpected_characters_is_revoked_not_stored() {
    let (w, auth) = world();
    auth.lock().unwrap().unsafe_token = true;
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(run
        .err
        .contains("the minted token has unexpected characters; not stored"));
    assert!(!w.stored().contains_key("HINDSIGHT_API_TOKEN"));
    assert_eq!(auth.lock().unwrap().revoked.len(), 3);
}

#[test]
fn a_mint_without_a_token_says_why() {
    let (w, auth) = world();
    auth.lock().unwrap().no_mint_token = true;
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(
        run.err
            .contains("HINDSIGHT_API_TOKEN: mint refused: HTTP 200"),
        "{}",
        run.err
    );
}

#[test]
fn a_token_that_cannot_be_sealed_is_revoked() {
    let (mut w, auth) = world();
    w.env.insert("FAKE_GATE_REFUSE".into(), "1".into());
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(run
        .err
        .contains("MEMORY_PRIVATE_TOKEN: minted but not sealed; revoking it"));
    assert!(w.gate().is_none());
    assert_eq!(auth.lock().unwrap().revoked.len(), 1);
}

#[test]
fn a_missing_gate_tool_is_a_failed_seal_and_a_missing_status() {
    let (mut w, _) = world();
    w.env.insert(
        "AGENT_KIT_PRIVATE_TOOL".into(),
        w.tmp.path("nowhere").display().to_string(),
    );
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(run.err.contains("minted but not sealed"));
    assert_eq!(
        lines_of(&w.keys(&["status"], "").out)["MEMORY_PRIVATE_TOKEN"],
        "missing"
    );
}

#[test]
fn a_retired_value_leaves_the_store_and_env_sh() {
    let (w, _) = world();
    let mut t = w.table_entries();
    t.push(json!({"env": "OLD_KEY", "retired": true}));
    t.push(json!({"env": "GONE_KEY", "retired": true}));
    w.set_table(&t);
    w.put("OLD_KEY", "old_value");
    let run = w.keys(&["write-env"], "");
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(!w.stored().contains_key("OLD_KEY"));
    assert!(run
        .err
        .contains("OLD_KEY: retired, removed from the keychain"));
    assert!(!std::fs::read_to_string(w.home(".config/agent-kit/env.sh"))
        .unwrap()
        .contains("OLD_KEY"));
    assert!(!w.keys(&["status"], "").out.contains("OLD_KEY"));
}

#[test]
fn status_reports_without_asking() {
    let (w, _) = world();
    w.keys(&["ensure"], ANSWERS);
    std::fs::remove_file(w.tmp.path("store").join("MEMORY_MCP_TOKEN")).unwrap();
    let run = w.keys(&["status"], "");
    let lines = lines_of(&run.out);
    assert_eq!(lines["HINDSIGHT_API_TOKEN"], "ok");
    assert_eq!(lines["MEMORY_MCP_TOKEN"], "missing");
    assert_eq!(lines["MISTRAL_API_KEY"], "ok");
    assert_eq!(lines["MEMORY_PRIVATE_TOKEN"], "ok");
    assert_eq!(lines["shell"], "ok");
}

#[test]
fn a_wrong_mistral_key_is_asked_again() {
    let (w, _) = world();
    let run = w.keys(
        &["ensure"],
        "me@example.com\npw\n123456\nmk_bad\nmk bad!\nmk_good\ny\n",
    );
    assert_eq!(run.code, 0, "{}", run.err);
    assert_eq!(w.stored()["MISTRAL_API_KEY"], "mk_good");
    assert_eq!(
        run.err
            .matches("MISTRAL_API_KEY: refused by its host; try again")
            .count(),
        2
    );
}

#[test]
fn a_skipped_answer_leaves_the_rest_done() {
    let (w, auth) = world();
    let run = w.keys(&["ensure"], "me@example.com\npw\n123456\n\ny\n");
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(!w.stored().contains_key("MISTRAL_API_KEY"));
    assert!(run.err.contains("MISTRAL_API_KEY: skipped"));
    assert!(auth.lock().unwrap().valid.contains(&w.gate().unwrap()));
}

#[test]
fn a_private_token_left_in_the_store_is_removed_and_replaced_behind_the_gate() {
    let (w, auth) = world();
    w.put("MEMORY_PRIVATE_TOKEN", "smt_old_exposed");
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(!w.stored().contains_key("MEMORY_PRIVATE_TOKEN"));
    assert!(auth
        .lock()
        .unwrap()
        .minted
        .contains(&"MEMORY_PRIVATE".to_string()));
    assert_ne!(w.gate().unwrap(), "smt_old_exposed");
    assert!(run.err.contains(
        "MEMORY_PRIVATE_TOKEN: removed from the keychain; a new one goes behind your passphrase"
    ));
}

#[test]
fn a_non_admin_without_a_grant_is_told_who_can_fix_it() {
    let (w, auth) = world();
    auth.lock().unwrap().role = "USER".into();
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(run.err.contains(
        "MEMORY_MCP_TOKEN: mint refused: User does not hold the MEMORY_MCP service grant"
    ));
    assert!(run.err.contains("ask an admin to grant it"));
    assert!(!auth
        .lock()
        .unwrap()
        .log
        .iter()
        .any(|l| l.starts_with("PUT")));
    assert!(
        w.stored().contains_key("HINDSIGHT_API_TOKEN"),
        "the grant you hold is still minted"
    );
}

#[test]
fn an_admin_whose_grant_did_not_take_is_told_to_ask() {
    let state = auth_state();
    let inner = state.clone();
    // The grant PUT is accepted but changes nothing.
    let base = serve(move |r| {
        let mut s = inner.lock().unwrap();
        if r.method == "PUT" {
            return reply(500, json!({}));
        }
        auth_reply(&mut s, r)
    });
    let w = World::new(&base, &table(&base));
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(
        run.err
            .contains("MEMORY_MCP_TOKEN: you do not hold MEMORY_MCP; ask an admin to grant it"),
        "{}",
        run.err
    );
    assert!(w.stored().contains_key("HINDSIGHT_API_TOKEN"));
}

#[test]
fn a_refused_password_stops_signing_in() {
    let (w, _) = world();
    let run = w.keys(&["ensure"], "me@example.com\nwrong\n\n");
    assert!(run.err.contains("sign-in refused: Bad credentials"));
    assert!(!w.stored().keys().any(|k| k.ends_with("TOKEN")));
}

#[test]
fn a_sign_in_refused_without_a_reason_names_the_status() {
    let (w, _) = world();
    let run = w.keys(&["ensure"], "me@example.com\npw\n000000\n\n");
    assert!(run.err.contains("sign-in refused: HTTP 200"), "{}", run.err);
}

#[test]
fn signing_in_without_a_session_says_so() {
    let base = serve(|r| {
        if r.path.ends_with("session-login") {
            reply(200, json!({"success": true}))
        } else {
            reply(401, json!({}))
        }
    });
    let w = World::new(&base, &table(&base));
    let run = w.keys(&["ensure"], "me@example.com\npw\ny\n");
    assert!(
        run.err
            .contains("signed in, but /auth/me answered HTTP 401"),
        "{}",
        run.err
    );
}

#[test]
fn an_empty_me_mints_nothing() {
    let (w, auth) = world();
    auth.lock().unwrap().me_empty = true;
    let run = w.keys(&["ensure"], "me@example.com\npw\n123456\nmk_good\ny\n");
    assert!(auth.lock().unwrap().minted.is_empty());
    assert_eq!(run.code, 0);
}

#[test]
fn an_unreachable_auth_api_is_a_note_and_the_run_goes_on() {
    let (mut w, _) = world();
    w.env
        .insert("AGENT_KIT_AUTH_URL".into(), "http://127.0.0.1:1".into());
    let run = w.keys(&["ensure"], "me@example.com\npw\nmk_good\ny\n");
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(run.err.contains("could not reach auth.jorisjonkers.dev"));
    assert!(run.err.contains("no tokens minted this run"));
    assert_eq!(w.stored()["MISTRAL_API_KEY"], "mk_good");
}

#[test]
fn an_auth_api_that_goes_away_mid_mint_stops_the_minting() {
    let (w, auth) = world();
    auth.lock().unwrap().drop_mint = true;
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(
        run.err.contains("the remaining tokens were not minted"),
        "{}",
        run.err
    );
    assert_eq!(w.stored()["MISTRAL_API_KEY"], "mk_good");
}

#[test]
fn the_default_email_comes_from_git() {
    let (mut w, _) = world();
    w.env
        .insert("FAKE_GIT_EMAIL".into(), "git@example.com".into());
    let run = w.keys(&["ensure"], "\npw\n123456\nmk_good\ny\n");
    assert!(
        run.err.contains("username or email [git@example.com]: "),
        "{}",
        run.err
    );
    assert_eq!(run.code, 0);
}

#[test]
fn the_terminal_hides_a_secret_answer() {
    let (w, _) = world();
    let (mut ctx, _, err) = w.ctx("hidden\n");
    ctx.tty = true;
    assert_eq!(ask(&mut ctx, "secret: ", true), "hidden");
    assert!(err.text().starts_with("secret: "));
}

// ---------------------------------------------------------------- env.sh and the shell

fn env_sh(w: &World) -> PathBuf {
    w.home(".config/agent-kit/env.sh")
}

fn shell(w: &World, program: &str, script: &str, extra: &[(&str, &str)]) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(["-c", script])
        .env_clear()
        .envs(&w.env)
        .envs(extra.iter().copied());
    let out = command.output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[test]
fn env_sh_exports_the_shared_values_and_scopes_the_private_token() {
    let (w, _) = world();
    w.keys(&["ensure"], ANSWERS);
    let stored = w.stored();
    let probe = format!(
        ". \"{}\"; printf '%s|%s|%s|%s|%s\\n' \"$HINDSIGHT_API_URL\" \"$HINDSIGHT_API_TOKEN\" \"$MEMORY_MCP_TOKEN\" \"$MISTRAL_API_KEY\" \"${{MEMORY_PRIVATE_TOKEN:-unset}}\"",
        env_sh(&w).display()
    );
    let want = format!(
        "https://hindsight-api.jorisjonkers.dev|{}|{}|mk_good|unset",
        stored["HINDSIGHT_API_TOKEN"], stored["MEMORY_MCP_TOKEN"]
    );
    for program in ["bash", "zsh", "sh"] {
        if let Some(got) = shell(&w, program, &probe, &[]).filter(|g| !g.is_empty()) {
            assert_eq!(got, want, "{program}");
        }
    }
    let text = std::fs::read_to_string(env_sh(&w)).unwrap();
    assert!(!text.contains("vibe()"));
    assert!(!text.contains("_agent_kit_secret MEMORY_PRIVATE_TOKEN"));
}

#[test]
fn env_sh_is_byte_for_byte_what_the_python_wrote() {
    let (w, _) = world();
    assert_eq!(w.keys(&["write-env"], "").code, 0);
    let text = std::fs::read_to_string(env_sh(&w)).unwrap();
    let want = "# managed by agent-kit setup-workstation.sh; rewritten on every run.\n\
# Reads the workstation's tokens and keys from the keychain. A value already\n\
# in the environment wins. An `export: false` value is never exported: the\n\
# Private Memory token is opened only by vibe-private (through agent-kit-private,\n\
# which asks for its passphrase), and vibe-study reads the Study account key\n\
# with _agent_kit_secret itself.\n\
_agent_kit_secret() { \"$HOME/.local/bin/agent-kit-secret\" get \"$1\" 2>/dev/null; }\n\
[ -n \"${HINDSIGHT_API_URL:-}\" ] || export HINDSIGHT_API_URL=\"https://hindsight-api.jorisjonkers.dev\"\n\
if [ -z \"${HINDSIGHT_API_TOKEN:-}\" ]; then _agent_kit_v=\"$(_agent_kit_secret HINDSIGHT_API_TOKEN)\"; if [ -n \"$_agent_kit_v\" ]; then export HINDSIGHT_API_TOKEN=\"$_agent_kit_v\"; fi; fi\n\
if [ -z \"${MEMORY_MCP_TOKEN:-}\" ]; then _agent_kit_v=\"$(_agent_kit_secret MEMORY_MCP_TOKEN)\"; if [ -n \"$_agent_kit_v\" ]; then export MEMORY_MCP_TOKEN=\"$_agent_kit_v\"; fi; fi\n\
if [ -z \"${MISTRAL_API_KEY:-}\" ]; then _agent_kit_v=\"$(_agent_kit_secret MISTRAL_API_KEY)\"; if [ -n \"$_agent_kit_v\" ]; then export MISTRAL_API_KEY=\"$_agent_kit_v\"; fi; fi\n\
unset _agent_kit_v\n\
case \"$(typeset -f vibe 2>/dev/null)\" in *\"agent-kit-private exec vibe\"*) unset -f vibe ;; esac\n";
    assert_eq!(text, want);
}

#[test]
fn env_sh_drops_the_old_kit_vibe_function_but_never_one_of_your_own() {
    let (w, _) = world();
    w.keys(&["write-env"], "");
    for program in ["bash", "zsh"] {
        for (defined, survives) in [
            (
                "vibe() { command agent-kit-private exec vibe \"$@\"; }",
                false,
            ),
            ("vibe() { echo mine; }", true),
        ] {
            let probe = format!(
                "{defined}\n. \"{}\"\ntypeset -f vibe >/dev/null 2>&1 && echo kept || echo dropped",
                env_sh(&w).display()
            );
            if let Some(got) = shell(&w, program, &probe, &[]).filter(|g| !g.is_empty()) {
                assert_eq!(got, if survives { "kept" } else { "dropped" }, "{program}");
            }
        }
    }
}

fn with_study_login(w: &World) {
    let base = w.env["AGENT_KIT_AUTH_URL"].clone();
    let mut t = w.table_entries();
    t.push(
        json!({"env": "MISTRAL_STUDY_API_KEY", "login": "vibe-study --login",
                  "check": format!("{base}/mistral"), "check_method": "GET", "export": false}),
    );
    w.set_table(&t);
}

#[test]
fn a_sign_in_value_is_never_asked_for_only_pointed_at_its_sign_in() {
    let (w, _) = world();
    with_study_login(&w);
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(!w.stored().contains_key("MISTRAL_STUDY_API_KEY"));
    assert!(run
        .err
        .contains("MISTRAL_STUDY_API_KEY: sign in with  vibe-study --login"));
    assert_eq!(
        w.stored()["MISTRAL_API_KEY"],
        "mk_good",
        "the answers still reached the paste after it"
    );
}

#[test]
fn status_reports_a_sign_in_value_by_whether_it_is_stored_and_valid() {
    let (w, _) = world();
    with_study_login(&w);
    assert_eq!(
        lines_of(&w.keys(&["status"], "").out)["MISTRAL_STUDY_API_KEY"],
        "missing"
    );
    w.put("MISTRAL_STUDY_API_KEY", "mk_good");
    assert_eq!(
        lines_of(&w.keys(&["status"], "").out)["MISTRAL_STUDY_API_KEY"],
        "ok"
    );
    w.put("MISTRAL_STUDY_API_KEY", "mk_wrong");
    assert_eq!(
        lines_of(&w.keys(&["status"], "").out)["MISTRAL_STUDY_API_KEY"],
        "invalid"
    );
}

#[test]
fn the_study_account_key_is_never_exported_only_read_by_name() {
    let (w, _) = world();
    with_study_login(&w);
    w.put("MISTRAL_STUDY_API_KEY", "mk_good");
    assert_eq!(w.keys(&["write-env"], "").code, 0);
    assert!(!std::fs::read_to_string(env_sh(&w))
        .unwrap()
        .contains("export MISTRAL_STUDY_API_KEY"));
    let probe = format!(
        ". \"{}\"; printf '%s|%s\\n' \"${{MISTRAL_STUDY_API_KEY:-unset}}\" \"$(_agent_kit_secret MISTRAL_STUDY_API_KEY)\"",
        env_sh(&w).display()
    );
    if let Some(got) = shell(&w, "bash", &probe, &[]).filter(|g| !g.is_empty()) {
        assert_eq!(got, "unset|mk_good");
    }
}

#[test]
fn a_value_already_in_the_environment_wins() {
    let (w, _) = world();
    w.keys(&["ensure"], ANSWERS);
    let probe = format!(
        ". \"{}\"; printf '%s' \"$MISTRAL_API_KEY\"",
        env_sh(&w).display()
    );
    if let Some(got) =
        shell(&w, "bash", &probe, &[("MISTRAL_API_KEY", "mine")]).filter(|g| !g.is_empty())
    {
        assert_eq!(got, "mine");
    }
}

#[test]
fn the_shell_profile_sources_env_sh_once() {
    let (w, _) = world();
    w.keys(&["ensure"], ANSWERS);
    w.keys(&["ensure"], "y\n");
    let rc = std::fs::read_to_string(w.home(".zshrc")).unwrap();
    let wired: Vec<&str> = rc
        .lines()
        .filter(|l| l.contains("agent-kit/env.sh"))
        .collect();
    assert_eq!(wired, [SOURCE_LINE]);
}

#[test]
fn declining_the_profile_line_leaves_it_alone_and_says_how() {
    let (w, _) = world();
    let run = w.keys(&["ensure"], "me@example.com\npw\n123456\nmk_good\nn\n");
    assert!(!w.home(".zshrc").exists());
    assert!(run.err.contains("left ") && run.err.contains(".config/agent-kit/env.sh"));
}

#[test]
fn bash_and_zdotdir_and_unknown_shells_find_their_startup_file() {
    let (mut w, _) = world();
    w.env.insert("SHELL".into(), "/usr/bin/bash".into());
    assert_eq!(w.keys(&["ensure"], ANSWERS).code, 0);
    assert!(w.home(".bashrc").is_file());
    let (mut w, _) = world();
    w.env
        .insert("ZDOTDIR".into(), w.tmp.path("zdot").display().to_string());
    std::fs::create_dir_all(w.tmp.path("zdot")).unwrap();
    w.keys(&["ensure"], ANSWERS);
    assert!(w.tmp.path("zdot/.zshrc").is_file());
    let (mut w, _) = world();
    w.env.insert("SHELL".into(), "/bin/fish".into());
    let run = w.keys(&["ensure"], ANSWERS);
    assert!(run
        .err
        .contains("add this line to your shell's startup file"));
    assert_eq!(lines_of(&w.keys(&["status"], "").out)["shell"], "missing");
    w.env.remove("SHELL");
    assert_eq!(w.keys(&["status"], "").code, 0);
}

#[test]
fn write_env_needs_no_answers() {
    let (w, _) = world();
    let run = w.keys(&["write-env"], "");
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(std::fs::read_to_string(env_sh(&w))
        .unwrap()
        .contains("# managed by agent-kit"));
}

#[test]
fn an_unwritable_home_stops_the_run_with_a_note() {
    let (w, _) = world();
    std::fs::write(w.home(".config"), "a file where the folder should be").unwrap();
    let run = w.keys(&["write-env"], "");
    assert_eq!(run.code, 1);
    assert!(
        run.err.contains("keys: stopped: cannot write"),
        "{}",
        run.err
    );
}

// ---------------------------------------------------------------- the store

#[test]
fn without_a_usable_secret_store_the_run_stops_and_mints_nothing() {
    let (mut w, auth) = world();
    w.env.insert("FAKE_STORE_REFUSE".into(), "1".into());
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 1);
    assert!(
        run.err.contains("keys: stopped: no usable backend"),
        "{}",
        run.err
    );
    assert!(
        auth.lock().unwrap().minted.is_empty(),
        "nothing was minted that could not be kept"
    );
    w.env.insert("FAKE_STORE_SILENT".into(), "1".into());
    w.env.remove("FAKE_STORE_REFUSE");
    assert!(w
        .keys(&["status"], "")
        .err
        .contains("the secret store is not usable"));
}

#[test]
fn a_store_helper_that_will_not_run_stops_the_run() {
    let (mut w, _) = world();
    w.env.insert(
        "AGENT_KIT_SECRET_TOOL".into(),
        w.tmp.path("missing").display().to_string(),
    );
    let run = w.keys(&["status"], "");
    assert_eq!(run.code, 1);
    assert!(run.err.contains("cannot run"), "{}", run.err);
}

#[test]
fn a_value_that_does_not_read_back_is_an_error() {
    let (mut w, _) = world();
    w.env.insert("FAKE_STORE_DROP".into(), "1".into());
    let run = w.keys(&["ensure"], "me@example.com\npw\n123456\nmk_good\ny\n");
    assert_eq!(run.code, 1);
    assert!(
        run.err.contains("did not read back from the keychain"),
        "{}",
        run.err
    );
}

#[test]
fn the_store_is_named_for_its_backend() {
    let (mut w, _) = world();
    let (ctx, _, _) = w.ctx("");
    assert_eq!(Store::new(&ctx).name, "keychain");
    w.env
        .insert("AGENT_KIT_SECRETS_BACKEND".into(), "libsecret".into());
    let (ctx, _, _) = w.ctx("");
    assert_eq!(Store::new(&ctx).name, "Secret Service");
    // By platform: the keychain only on macOS with `security` on PATH.
    w.env.remove("AGENT_KIT_SECRETS_BACKEND");
    script(&w.tmp.path("bin/security"), "exit 0");
    let (ctx, _, _) = w.ctx("");
    let want = if std::env::consts::OS == "macos" {
        "keychain"
    } else {
        "Secret Service"
    };
    assert_eq!(Store::new(&ctx).name, want);
    w.env.remove("PATH");
    let (ctx, _, _) = w.ctx("");
    assert_eq!(Store::new(&ctx).name, "Secret Service");
}

#[test]
fn the_helpers_are_found_beside_the_kit_or_in_the_installed_place() {
    let (mut w, _) = world();
    w.env.remove("AGENT_KIT_SECRET_TOOL");
    let (mut ctx, _, _) = w.ctx("");
    let installed = w.home(".local/bin/agent-kit-secret");
    assert_eq!(
        store::find_tool(&ctx, "AGENT_KIT_SECRET_TOOL", "agent-kit-secret"),
        installed
    );
    let kit = w.tmp.path("kit");
    std::fs::create_dir_all(kit.join("installer")).unwrap();
    std::fs::write(kit.join("installer/agent-kit-secret"), "").unwrap();
    ctx.exe = kit.join("scripts/target/release/agent-kit-keys");
    let beside = kit.join("installer/agent-kit-secret");
    assert_eq!(
        store::find_tool(&ctx, "AGENT_KIT_SECRET_TOOL", "agent-kit-secret"),
        beside
    );
    ctx.env
        .insert("AGENT_KIT_SECRET_TOOL".into(), "/chosen".into());
    assert_eq!(
        store::find_tool(&ctx, "AGENT_KIT_SECRET_TOOL", "agent-kit-secret"),
        PathBuf::from("/chosen")
    );
    ctx.env
        .insert("AGENT_KIT_SECRET_TOOL".into(), String::new());
    assert_eq!(
        store::find_tool(&ctx, "AGENT_KIT_SECRET_TOOL", "agent-kit-secret"),
        beside
    );
}

#[test]
fn a_secrets_file_from_an_earlier_setup_moves_into_the_store_and_is_removed() {
    let (w, _) = world();
    let legacy = w.home(".config/agent-kit/secrets.env");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(
        &legacy,
        "MISTRAL_API_KEY=mk_good\nEMPTY=\nno equals sign\nHINDSIGHT_API_TOKEN=smt_kept\n",
    )
    .unwrap();
    w.put("HINDSIGHT_API_TOKEN", "smt_in_store");
    let run = w.keys(&["ensure"], "me@example.com\npw\n123456\ny\n");
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(!legacy.exists());
    assert_eq!(w.stored()["MISTRAL_API_KEY"], "mk_good");
    assert_ne!(
        w.stored()["HINDSIGHT_API_TOKEN"],
        "smt_kept",
        "a value already in the store is not overwritten"
    );
    assert!(run.err.contains("and removed the plain file"));
    assert!(!run.out.contains("mk_good") && !run.err.contains("mk_good"));
}

#[test]
fn a_legacy_file_that_cannot_be_read_or_removed_stops_the_run() {
    let (w, _) = world();
    let legacy = w.home(".config/agent-kit/secrets.env");
    std::fs::create_dir_all(&legacy).unwrap();
    // A directory by that name: it is not a file, so it is left alone.
    assert_eq!(w.keys(&["write-env"], "").code, 0);
    std::fs::remove_dir_all(&legacy).unwrap();
    std::fs::write(&legacy, "X=1\n").unwrap();
    std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unreadable = std::fs::read_to_string(&legacy).is_err();
    let run = w.keys(&["write-env"], "");
    std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o600)).unwrap();
    if unreadable {
        assert_eq!(run.code, 1);
    }
    let parent = legacy.parent().unwrap();
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500)).unwrap();
    let blocked = std::fs::write(parent.join("probe"), "").is_err();
    let run = w.keys(&["write-env"], "");
    std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).unwrap();
    if blocked {
        assert_eq!(run.code, 1);
        assert!(legacy.exists());
    }
}

// ---------------------------------------------------------------- revoke

#[test]
fn a_revoked_host_token_is_refused_by_its_host_and_the_next_run_mints_a_new_one() {
    let (w, auth) = world();
    w.keys(&["ensure"], ANSWERS);
    let token = w.stored()["HINDSIGHT_API_TOKEN"].clone();
    assert!(w
        .keys(&["status"], "")
        .out
        .contains("HINDSIGHT_API_TOKEN ok"));
    let run = w.keys(
        &["revoke", "HINDSIGHT_API_TOKEN"],
        "me@example.com\npw\n123456\n",
    );
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(!run.out.contains(&token) && !run.err.contains(&token));
    assert!(!auth.lock().unwrap().valid.contains(&token));
    assert!(w
        .keys(&["status"], "")
        .out
        .contains("HINDSIGHT_API_TOKEN invalid"));
    assert!(ids(&w).get("HINDSIGHT_API_TOKEN").is_none());
    assert_eq!(w.keys(&["ensure"], "me@example.com\npw\n123456\n").code, 0);
    assert!(auth
        .lock()
        .unwrap()
        .valid
        .contains(&w.stored()["HINDSIGHT_API_TOKEN"]));
    assert!(
        ids(&w).get("HINDSIGHT_API_TOKEN").is_some(),
        "the new id is recorded, so it can be revoked in turn"
    );
}

#[test]
fn revoke_refuses_a_name_that_is_not_a_minted_token() {
    let (w, _) = world();
    let run = w.keys(&["revoke", "MISTRAL_API_KEY"], "");
    assert_eq!(run.code, 2);
    assert!(run.err.contains("not a token this tool mints"));
}

#[test]
fn revoke_without_a_recorded_id_says_where_to_revoke_it() {
    let (w, _) = world();
    let run = w.keys(&["revoke", "HINDSIGHT_API_TOKEN"], "");
    assert_eq!(run.code, 1);
    assert!(run.err.contains("admin page"));
}

#[test]
fn the_command_line_is_checked() {
    let (w, _) = world();
    let run = w.keys(&["revoke"], "");
    assert_eq!(run.code, 2);
    assert!(run.err.contains("revoke needs the variable name"));
    assert_eq!(w.keys(&["frobnicate"], "").code, 2);
    let (mut ctx, _, err) = w.ctx("");
    assert_eq!(run_in(&mut ctx, &["ensure".to_string()]), 2);
    assert!(err.text().contains("--table is required") && err.text().contains("usage:"));
    assert_eq!(run_in(&mut ctx, &["--table".to_string()]), 2);
    let table = format!("--table={}", w.table.display());
    assert_eq!(run_in(&mut ctx, &[table, "status".to_string()]), 0);
}

#[test]
fn revoke_with_a_refused_sign_in_revokes_nothing() {
    let (w, auth) = world();
    w.keys(&["ensure"], ANSWERS);
    let run = w.keys(
        &["revoke", "HINDSIGHT_API_TOKEN"],
        "me@example.com\nwrong\n",
    );
    assert_eq!(run.code, 1);
    assert!(auth.lock().unwrap().revoked.is_empty());
}

#[test]
fn a_revoke_auth_api_refuses_keeps_the_id() {
    let (w, _) = world();
    w.keys(&["ensure"], ANSWERS);
    let mut recorded = ids(&w);
    recorded["HINDSIGHT_API_TOKEN"] = json!("bad/id");
    std::fs::write(
        w.home(".config/agent-kit/token-ids.json"),
        recorded.to_string(),
    )
    .unwrap();
    let run = w.keys(
        &["revoke", "HINDSIGHT_API_TOKEN"],
        "me@example.com\npw\n123456\n",
    );
    assert_eq!(run.code, 1);
    assert!(run.err.contains("refused to revoke"));
    assert!(ids(&w).get("HINDSIGHT_API_TOKEN").is_some());
}

#[test]
fn a_revoke_against_an_unreachable_auth_api_says_so() {
    let (mut w, _) = world();
    w.keys(&["ensure"], ANSWERS);
    w.env
        .insert("AGENT_KIT_AUTH_URL".into(), "http://127.0.0.1:1".into());
    let run = w.keys(&["revoke", "HINDSIGHT_API_TOKEN"], "me@example.com\npw\n");
    assert_eq!(run.code, 1);
    assert!(run.err.contains("nothing revoked"));
}

#[test]
fn without_a_private_folder_for_the_session_the_run_stops() {
    let (mut w, _) = world();
    w.keys(&["ensure"], ANSWERS);
    w.env.insert(
        "TMPDIR".into(),
        w.tmp.path("no/such/dir").display().to_string(),
    );
    let run = w.keys(&["revoke", "HINDSIGHT_API_TOKEN"], "");
    assert_eq!(run.code, 1);
    assert!(run.err.contains("no private folder for the session"));
    w.put("MEMORY_MCP_TOKEN", "gone");
    assert_eq!(w.keys(&["ensure"], "me@example.com\npw\n123456\n").code, 1);
}

// ---------------------------------------------------------------- checks

#[test]
fn an_entry_without_a_check_url_is_unchecked_and_asks_no_host() {
    let (w, _) = world();
    let (ctx, _, _) = w.ctx("");
    let entry = json!({"env": "LITELLM_VIBE_KEY", "paste": "x", "export": false});
    assert_eq!(check::check(&ctx, &entry, "sk-anything"), "unchecked");
}

#[test]
fn a_host_that_does_not_answer_leaves_a_value_unchecked() {
    let (w, _) = world();
    let (ctx, _, _) = w.ctx("");
    let entry = json!({"env": "X", "check": "http://127.0.0.1:1/x", "check_method": "GET"});
    assert_eq!(check::check(&ctx, &entry, "v"), "unchecked");
}

#[test]
fn the_mcp_check_posts_the_initialize_request() {
    let seen = Arc::new(Mutex::new(String::new()));
    let log = seen.clone();
    let base = serve(move |r| {
        let accept = r.headers.get("accept").cloned().unwrap_or_default();
        *log.lock().unwrap() = format!("{} {accept} {}", r.method, r.body);
        reply(200, json!({}))
    });
    let (w, _) = world();
    let (ctx, _, _) = w.ctx("");
    let entry = json!({"env": "X", "check": format!("{base}/mcp")});
    assert_eq!(check::check(&ctx, &entry, "v"), "ok");
    let text = seen.lock().unwrap().clone();
    assert!(
        text.starts_with("POST application/json, text/event-stream {\"jsonrpc\":\"2.0\""),
        "{text}"
    );
    assert!(text.contains("\"protocolVersion\":\"2025-03-26\""));
}

// ---------------------------------------------------------------- the http layer

#[test]
fn a_config_value_cannot_start_another_option() {
    assert_eq!(
        quote("a\"b\\c\nd\re\tf\u{b}g"),
        "\"a\\\"b\\\\c\\nd\\re\\tf\\vg\""
    );
    let req = Req {
        method: "POST",
        url: "http://x/\"; output = \"/etc/passwd",
        headers: vec![("Authorization", "Bearer a\nheader = \"evil\"".to_string())],
        body: Some("@/etc/passwd".to_string()),
        jar: None,
        timeout: 5,
    };
    let text = config(&req);
    assert!(!text.contains("\nheader = \"evil\""));
    assert!(text.contains("data-raw = \"@/etc/passwd\""));
    assert_eq!(text.matches("\noutput").count(), 0);
}

#[test]
fn json_that_is_not_an_object_is_nothing() {
    assert_eq!(json_object(b"<html>"), json!({}));
    assert_eq!(json_object(b"[1]"), json!({}));
    assert_eq!(json_object(b"{\"a\":1}"), json!({"a": 1}));
}

#[test]
fn curl_that_is_missing_or_fails_is_a_reason() {
    let (w, _) = world();
    let (mut ctx, _, _) = w.ctx("");
    let req = Req {
        method: "GET",
        url: "http://127.0.0.1:1/",
        headers: vec![],
        body: None,
        jar: None,
        timeout: 2,
    };
    assert!(send(&ctx, &req).unwrap_err().contains("Failed to connect"));
    // A curl that fails without a word.
    script(&w.tmp.path("bin/curl"), "exit 7");
    assert_eq!(send(&ctx, &req).unwrap_err(), "no answer");
    // No curl at all.
    ctx.env
        .insert("PATH".into(), w.tmp.path("empty").display().to_string());
    assert!(send(&ctx, &req).unwrap_err().contains("curl did not run"));
}

#[test]
fn the_cookie_jar_is_private_and_removed() {
    let tmp = Tmp::new();
    let base = tmp.0.display().to_string();
    let path;
    {
        let jar = Jar::new(Some(&base)).unwrap();
        path = jar.path();
        let mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
        assert_eq!(jar.cookie("XSRF-TOKEN"), None);
        std::fs::write(
            &path,
            "# Netscape\n#HttpOnly_127.0.0.1\tFALSE\t/\tFALSE\t0\tSESSION\tok\n127.0.0.1\tFALSE\t/\tFALSE\t0\tXSRF-TOKEN\tx9\nshort\tline\n",
        )
        .unwrap();
        assert_eq!(jar.cookie("XSRF-TOKEN").as_deref(), Some("x9"));
        assert_eq!(jar.cookie("SESSION").as_deref(), Some("ok"));
        assert_eq!(jar.cookie("none"), None);
    }
    assert!(!path.exists());
    assert!(Jar::new(None).is_ok());
}

// ---------------------------------------------------------------- helpers

#[test]
fn small_helpers_agree_with_the_python() {
    assert!(is_safe("sk-abc_1.2~+/=") && !is_safe("") && !is_safe("a b") && !is_safe("a\"b"));
    assert!(truthy(&json!("x")) && !truthy(&json!("")) && !truthy(&json!(0)) && truthy(&json!(1)));
    assert!(!truthy(&Value::Null) && !truthy(&json!(false)) && truthy(&json!(true)));
    assert!(
        !truthy(&json!([]))
            && truthy(&json!([1]))
            && !truthy(&json!({}))
            && truthy(&json!({"a": 1}))
    );
    assert_eq!(id_text(&json!("a")), "a");
    assert_eq!(id_text(&Value::Null), "");
    assert_eq!(id_text(&json!(7)), "7");
    let (w, _) = world();
    let (mut ctx, _, _) = w.ctx("");
    assert_eq!(ctx.host_name(), "box");
    ctx.host = String::new();
    assert_eq!(ctx.host_name(), "host");
    ctx.host = "BOX".into();
    assert_eq!(ctx.host_name(), "box");
    assert_eq!(ask(&mut ctx, "x", false), "");
}

#[test]
fn the_command_line_entry_point_reports_usage() {
    assert_eq!(run(["--bogus"].into_iter().map(OsString::from)), 2);
    assert_eq!(NAME, "agent-kit-keys");
}

// ---------------------------------------------------------------- LiteLLM

const ADMIN: &str = "sk-admin-1h";
const MASTER: &str = "sk-master";

#[derive(Default)]
struct ProxyState {
    teams: Vec<Value>,
    keys: BTreeMap<String, Value>,
    users: BTreeSet<String>,
    refuse: BTreeSet<String>,
    calls: Vec<String>,
    mint_empty: bool,
    unsafe_key: bool,
    refuse_delete: bool,
    refuse_team_new: bool,
    refuse_team_update: bool,
    refuse_user_new: bool,
    html_ok: bool,
    team_list_wrapped: bool,
}

type SharedProxy = Arc<Mutex<ProxyState>>;

fn by_alias(s: &ProxyState, alias: &str) -> Vec<String> {
    s.keys
        .iter()
        .filter(|(_, v)| v["key_alias"] == alias)
        .map(|(k, _)| k.clone())
        .collect()
}

fn proxy_reply(s: &mut ProxyState, r: &Request) -> Reply {
    let (path, query) = r.path.split_once('?').unwrap_or((r.path.as_str(), ""));
    s.calls.push(format!("{} {path}", r.method));
    if path == "/key/info" {
        return match s.keys.get(&r.bearer()) {
            Some(key) => reply(200, json!({"info": {"models": key["models"]}})),
            None => reply(401, json!({})),
        };
    }
    if s.html_ok && path == "/html" {
        return Reply {
            status: 200,
            headers: vec![],
            body: "<html>".into(),
        };
    }
    let bearer = r.bearer();
    let master = bearer == MASTER && (path.starts_with("/user/") || path == "/key/generate");
    if (bearer != ADMIN && !master) || s.refuse.contains(path) {
        return reply(403, json!({"error": "not allowed"}));
    }
    match path {
        "/user/info" => {
            let id = query.strip_prefix("user_id=").unwrap_or("");
            reply(if s.users.contains(id) { 200 } else { 404 }, json!({}))
        }
        "/user/new" => {
            if s.refuse_user_new {
                return reply(500, json!({"detail": "database down"}));
            }
            let id = r.json()["user_id"].as_str().unwrap_or("").to_string();
            s.users.insert(id);
            reply(200, r.json())
        }
        "/team/list" => {
            let teams = json!(s.teams);
            reply(
                200,
                if s.team_list_wrapped {
                    json!({"teams": teams})
                } else {
                    teams
                },
            )
        }
        "/team/new" => {
            if s.refuse_team_new {
                return reply(500, json!({}));
            }
            let mut team = r.json();
            team["team_id"] = json!(format!("team-{}", s.teams.len()));
            s.teams.push(team.clone());
            reply(200, team)
        }
        "/team/update" => {
            if s.refuse_team_update {
                return reply(500, json!({}));
            }
            let body = r.json();
            let team = s
                .teams
                .iter_mut()
                .find(|t| t["team_id"] == body["team_id"])
                .unwrap();
            for (k, v) in body.as_object().unwrap() {
                team[k] = v.clone();
            }
            reply(200, json!({}))
        }
        "/key/list" => {
            let alias = query
                .strip_prefix("key_alias=")
                .unwrap_or("")
                .replace("%40", "@");
            reply(200, json!({"keys": by_alias(s, &alias)}))
        }
        "/key/delete" => {
            if s.refuse_delete {
                return reply(500, json!({}));
            }
            for alias in r.json()["key_aliases"].as_array().unwrap() {
                for key in by_alias(s, alias.as_str().unwrap()) {
                    s.keys.remove(&key);
                }
            }
            reply(200, json!({"deleted_keys": []}))
        }
        "/key/generate" => {
            if master {
                return reply(200, json!({"key": ADMIN}));
            }
            if s.mint_empty {
                return reply(200, json!({}));
            }
            let body = r.json();
            let alias = body["key_alias"].as_str().unwrap_or("").to_string();
            let key = if s.unsafe_key {
                "bad key!".to_string()
            } else {
                format!("sk-{}-{}", alias.replace('@', "-at-"), s.keys.len())
            };
            s.keys.insert(key.clone(), body);
            reply(200, json!({"key": key}))
        }
        _ => reply(404, json!({})),
    }
}

fn proxy_server(state: &SharedProxy) -> String {
    let state = state.clone();
    serve(move |r| proxy_reply(&mut state.lock().unwrap(), r))
}

fn litellm_entry(url: &str, env: &str, agent: &str, budget: Option<f64>) -> Value {
    let mut team = json!({"team_alias": agent, "models": ["canonical"]});
    if let Some(b) = budget {
        team["max_budget"] = json!(b);
        team["budget_duration"] = json!("30d");
    }
    json!({"env": env, "mint": "litellm", "check": format!("{url}/key/info"), "check_method": "GET",
           "export": false, "litellm": {"proxy": url, "team": team}})
}

fn litellm_world() -> (World, SharedProxy) {
    let state: SharedProxy = Arc::new(Mutex::new(ProxyState::default()));
    let url = proxy_server(&state);
    let table = json!([
        litellm_entry(&url, "LITELLM_VIBE_KEY", "vibe", Some(255.0)),
        litellm_entry(&url, "LITELLM_LOCAL_KEY", "local", None),
    ]);
    (World::new(&url, &table), state)
}

fn mint_with(w: &World, entries: &[Value], wanted: &[&str], token: &str) -> (String, R<()>) {
    let (mut ctx, _, err) = w.ctx("");
    let list: Vec<&Value> = entries.iter().collect();
    let keys = Store::new(&ctx);
    let wanted: Vec<String> = wanted.iter().map(|s| s.to_string()).collect();
    let got = litellm::mint_litellm(&mut ctx, &list, &wanted, token, &keys);
    (err.text(), got)
}

fn mint(w: &World, wanted: &[&str], token: &str, only_first: bool) -> String {
    let all = w.table_entries();
    let take = if only_first { 1 } else { all.len() };
    mint_with(w, &all[..take], wanted, token).0
}

#[test]
fn missing_keys_get_a_team_each_and_a_key_named_for_this_host() {
    let (w, proxy) = litellm_world();
    mint(&w, &["LITELLM_VIBE_KEY", "LITELLM_LOCAL_KEY"], ADMIN, false);
    let s = proxy.lock().unwrap();
    let teams: HashMap<String, &Value> = s
        .teams
        .iter()
        .map(|t| (t["team_alias"].as_str().unwrap().to_string(), t))
        .collect();
    assert_eq!(teams["vibe"]["max_budget"], json!(255.0));
    assert_eq!(teams["vibe"]["budget_duration"], "30d");
    assert!(teams["local"].get("max_budget").is_none());
    let stored = w.stored();
    let vibe = &s.keys[&stored["LITELLM_VIBE_KEY"]];
    assert_eq!(vibe["key_alias"], "vibe@box");
    assert_eq!(vibe["team_id"], teams["vibe"]["team_id"]);
    // A key follows its team's access groups, so a change of the team's models needs no new key.
    assert_eq!(vibe["models"], json!(["all-team-models"]));
    assert_eq!(
        s.keys[&stored["LITELLM_LOCAL_KEY"]]["key_alias"],
        "local@box"
    );
}

#[test]
fn the_paid_key_is_a_team_holding_paid_alone_and_is_minted_like_the_tiers() {
    let state: SharedProxy = Arc::new(Mutex::new(ProxyState::default()));
    let url = proxy_server(&state);
    let mut entry = litellm_entry(&url, "LITELLM_PAID_KEY", "paid", None);
    entry["litellm"]["team"]["models"] = json!(["paid"]);
    let w = World::new(&url, &json!([entry]));
    mint(&w, &["LITELLM_PAID_KEY"], ADMIN, false);
    let s = state.lock().unwrap();
    let team = s
        .teams
        .iter()
        .find(|t| t["team_alias"] == "paid")
        .expect("a team named paid");
    assert_eq!(team["models"], json!(["paid"]));
    assert!(team.get("max_budget").is_none());
    let stored = w.stored();
    let key = &s.keys[&stored["LITELLM_PAID_KEY"]];
    assert_eq!(key["key_alias"], "paid@box");
    assert_eq!(key["team_id"], team["team_id"]);
}

#[test]
fn the_auto_key_is_a_team_holding_public_repo_free_and_paid() {
    let state: SharedProxy = Arc::new(Mutex::new(ProxyState::default()));
    let url = proxy_server(&state);
    let mut entry = litellm_entry(&url, "LITELLM_AUTO_KEY", "auto", None);
    entry["litellm"]["team"]["models"] = json!(["public-repo", "free", "paid"]);
    let w = World::new(&url, &json!([entry]));
    mint(&w, &["LITELLM_AUTO_KEY"], ADMIN, false);
    let s = state.lock().unwrap();
    let team = s
        .teams
        .iter()
        .find(|t| t["team_alias"] == "auto")
        .expect("a team named auto");
    assert_eq!(team["models"], json!(["public-repo", "free", "paid"]));
    let stored = w.stored();
    assert_eq!(s.keys[&stored["LITELLM_AUTO_KEY"]]["key_alias"], "auto@box");
}

#[test]
fn the_old_shared_key_is_deleted_and_a_drifted_budget_is_put_back() {
    let (w, proxy) = litellm_world();
    {
        let mut s = proxy.lock().unwrap();
        s.teams.push(json!({"team_alias": "vibe", "team_id": "t-old", "models": ["canonical"], "max_budget": 100, "budget_duration": "30d"}));
        s.keys
            .insert("sk-shared".into(), json!({"key_alias": "vibe"}));
    }
    let said = mint(&w, &["LITELLM_VIBE_KEY"], ADMIN, true);
    let s = proxy.lock().unwrap();
    assert!(!s.keys.contains_key("sk-shared"));
    assert_eq!(s.teams[0]["max_budget"], json!(255.0));
    assert_eq!(s.teams.len(), 1);
    assert_eq!(s.keys[&w.stored()["LITELLM_VIBE_KEY"]]["team_id"], "t-old");
    assert!(
        said.contains("deleted the shared key vibe")
            && said.contains("set max_budget on team vibe"),
        "{said}"
    );
}

#[test]
fn a_budget_that_only_differs_in_its_number_type_is_not_drift() {
    let (w, proxy) = litellm_world();
    proxy.lock().unwrap().teams.push(json!({"team_alias": "vibe", "team_id": "t", "models": ["canonical"], "max_budget": 255, "budget_duration": "30d"}));
    let said = mint(&w, &[], ADMIN, true);
    assert!(!said.contains("set "), "{said}");
    assert!(litellm::same(&json!(255), &json!(255.0)) && !litellm::same(&json!(1), &json!(2)));
    assert!(litellm::same(&json!("a"), &json!("a")));
}

#[test]
fn an_old_cap_is_cleared_when_the_route_has_none() {
    let (w, proxy) = litellm_world();
    proxy.lock().unwrap().teams.push(json!({"team_alias": "local", "team_id": "t-local", "models": ["canonical"], "max_budget": 9, "budget_duration": "30d"}));
    let all = w.table_entries();
    let (said, _) = mint_with(&w, &all[1..], &[], ADMIN);
    assert_eq!(proxy.lock().unwrap().teams[0]["max_budget"], Value::Null);
    assert!(
        said.contains("set budget_duration, max_budget on team local"),
        "{said}"
    );
}

#[test]
fn a_team_that_cannot_be_updated_keeps_its_old_settings() {
    let (w, proxy) = litellm_world();
    {
        let mut s = proxy.lock().unwrap();
        s.teams.push(json!({"team_alias": "vibe", "team_id": "t", "models": ["canonical"], "max_budget": 1, "budget_duration": "30d"}));
        s.refuse_team_update = true;
    }
    assert!(mint(&w, &[], ADMIN, true)
        .contains("team vibe could not be updated (HTTP 500); it keeps its old settings"));
}

#[test]
fn a_team_that_cannot_be_made_is_skipped() {
    let (w, proxy) = litellm_world();
    proxy.lock().unwrap().refuse_team_new = true;
    let said = mint(&w, &["LITELLM_VIBE_KEY"], ADMIN, true);
    assert!(said.contains("team vibe could not be made (HTTP 500)"));
    assert!(w.stored().is_empty());
}

#[test]
fn a_team_list_wrapped_in_an_object_is_read() {
    let (w, proxy) = litellm_world();
    proxy.lock().unwrap().team_list_wrapped = true;
    mint(&w, &["LITELLM_VIBE_KEY"], ADMIN, true);
    assert!(w.stored().contains_key("LITELLM_VIBE_KEY"));
}

#[test]
fn a_key_this_host_lost_is_replaced_not_duplicated() {
    let (w, proxy) = litellm_world();
    proxy
        .lock()
        .unwrap()
        .keys
        .insert("sk-lost".into(), json!({"key_alias": "vibe@box"}));
    mint(&w, &["LITELLM_VIBE_KEY"], ADMIN, true);
    let s = proxy.lock().unwrap();
    assert_eq!(
        by_alias(&s, "vibe@box"),
        [w.stored()["LITELLM_VIBE_KEY"].clone()]
    );
}

#[test]
fn a_key_that_cannot_be_deleted_mints_nothing() {
    let (w, proxy) = litellm_world();
    {
        let mut s = proxy.lock().unwrap();
        s.keys
            .insert("sk-lost".into(), json!({"key_alias": "vibe@box"}));
        s.keys
            .insert("sk-shared".into(), json!({"key_alias": "vibe"}));
        s.refuse_delete = true;
    }
    let said = mint(&w, &["LITELLM_VIBE_KEY"], ADMIN, true);
    assert!(
        said.contains("the shared key vibe could not be deleted"),
        "{said}"
    );
    assert!(
        said.contains("the old key vibe@box could not be deleted; nothing minted"),
        "{said}"
    );
    assert!(w.stored().is_empty());
}

#[test]
fn a_present_key_is_kept_while_the_team_still_converges() {
    let (w, proxy) = litellm_world();
    mint(&w, &[], ADMIN, true);
    let s = proxy.lock().unwrap();
    assert_eq!(
        s.teams
            .iter()
            .map(|t| t["team_alias"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["vibe"]
    );
    assert!(s.keys.is_empty() && w.stored().is_empty());
}

#[test]
fn a_token_without_team_access_mints_nothing_and_stops_at_once() {
    let (w, proxy) = litellm_world();
    proxy.lock().unwrap().refuse.insert("/team/list".into());
    let said = mint(&w, &["LITELLM_VIBE_KEY", "LITELLM_LOCAL_KEY"], ADMIN, false);
    let s = proxy.lock().unwrap();
    assert!(s.keys.is_empty() && w.stored().is_empty());
    assert_eq!(s.calls, ["GET /team/list"]);
    assert!(said.contains("HTTP 403: not allowed"), "{said}");
    assert!(said.contains(
        "nothing minted (registry/generated/litellm/make-admin-token.sh makes the token)"
    ));
}

#[test]
fn a_proxy_that_does_not_answer_stops_the_rest() {
    let (w, _) = litellm_world();
    let mut all = w.table_entries();
    all[0]["litellm"]["proxy"] = json!("http://127.0.0.1:1");
    let (said, got) = mint_with(&w, &all, &["LITELLM_VIBE_KEY"], ADMIN);
    assert!(got.is_ok());
    assert!(said.contains("could not reach the LiteLLM proxy"));
}

#[test]
fn a_store_that_refuses_a_minted_key_stops_the_run() {
    let (mut w, _) = litellm_world();
    w.env.insert("FAKE_STORE_DROP".into(), "1".into());
    let all = w.table_entries();
    let (_, got) = mint_with(&w, &all[..1], &["LITELLM_VIBE_KEY"], ADMIN);
    assert!(matches!(got, Err(Fail::Stop(_))));
}

#[test]
fn a_proxy_that_mints_nothing_usable_stores_nothing() {
    let (w, proxy) = litellm_world();
    proxy.lock().unwrap().mint_empty = true;
    assert!(mint(&w, &["LITELLM_VIBE_KEY"], ADMIN, true)
        .contains("the proxy did not mint a usable key for vibe@box"));
    {
        let mut s = proxy.lock().unwrap();
        s.mint_empty = false;
        s.unsafe_key = true;
    }
    assert!(mint(&w, &["LITELLM_VIBE_KEY"], ADMIN, true).contains("did not mint a usable key"));
    assert!(w.stored().is_empty());
}

#[test]
fn a_new_key_the_proxy_refuses_is_deleted_again() {
    let (w, proxy) = litellm_world();
    let mut all = w.table_entries();
    // The check goes to a host that refuses every key.
    let refuse = serve(|_| reply(401, json!({})));
    all[0]["check"] = json!(format!("{refuse}/key/info"));
    let (said, _) = mint_with(&w, &all[..1], &["LITELLM_VIBE_KEY"], ADMIN);
    assert!(said.contains("the new key vibe@box is refused by the proxy; not stored"));
    assert!(proxy.lock().unwrap().keys.is_empty() && w.stored().is_empty());
}

#[test]
fn a_reply_that_is_not_json_or_empty_is_read_as_nothing() {
    let (w, proxy) = litellm_world();
    proxy.lock().unwrap().html_ok = true;
    let url = w.table_entries()[0]["litellm"]["proxy"]
        .as_str()
        .unwrap()
        .to_string();
    let (ctx, _, _) = w.ctx("");
    let api = litellm::LiteLLM::new(&format!("{url}/"), ADMIN);
    assert_eq!(
        api.call(&ctx, "GET", "/html", None).unwrap(),
        (0, Value::Null)
    );
    let empty = serve(|_| bare(204));
    let api = litellm::LiteLLM::new(&empty, ADMIN);
    assert_eq!(
        api.call(&ctx, "GET", "/x", None).unwrap(),
        (204, Value::Null)
    );
    let flat = serve(|_| reply(500, json!(["not", "an", "object"])));
    let api = litellm::LiteLLM::new(&flat, ADMIN);
    assert_eq!(api.call(&ctx, "GET", "/x", None).unwrap(), (500, json!({})));
}

#[test]
fn the_reason_for_a_refusal_is_short() {
    assert_eq!(
        litellm::why(&json!({"error": {"message": "nope"}})),
        ": nope"
    );
    assert_eq!(litellm::why(&json!({"error": {"type": "x"}})), "");
    assert_eq!(litellm::why(&json!({"detail": "d"})), ": d");
    assert_eq!(litellm::why(&json!({})), "");
    assert_eq!(
        litellm::why(&json!({"error": "x".repeat(300)}))
            .chars()
            .count(),
        202
    );
    assert_eq!(litellm::why(&json!({"error": 5})), ": 5");
    assert_eq!(litellm::encode("a b@c"), "a%20b%40c");
}

// ---------------------------------------------------------------- ensure_litellm

fn ensure_world(todo: &[&str]) -> (World, SharedProxy, BTreeSet<String>) {
    let (w, proxy) = litellm_world();
    (w, proxy, todo.iter().map(|s| s.to_string()).collect())
}

fn ensure_litellm_run(w: &World, todo: &BTreeSet<String>, answers: &str) -> (String, R<()>) {
    let (mut ctx, _, err) = w.ctx(answers);
    let entries = w.table_entries();
    let keys = Store::new(&ctx);
    let got = ensure_litellm(&mut ctx, &entries, todo, &keys);
    (err.text(), got)
}

#[test]
fn without_a_token_nothing_is_asked_when_no_key_is_missing() {
    let (w, proxy, todo) = ensure_world(&[]);
    let (said, got) = ensure_litellm_run(&w, &todo, "");
    assert!(got.is_ok());
    assert!(proxy.lock().unwrap().calls.is_empty());
    assert!(said.is_empty(), "{said}");
}

#[test]
fn a_skipped_token_mints_nothing() {
    let (w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    let (said, _) = ensure_litellm_run(&w, &todo, "\n");
    assert!(said.contains("Paste a LiteLLM admin token to mint LITELLM_VIBE_KEY"));
    assert!(said.contains("LITELLM_VIBE_KEY: skipped (no LiteLLM admin token)"));
    assert!(proxy.lock().unwrap().calls.is_empty() && w.stored().is_empty());
}

#[test]
fn a_token_in_the_environment_mints_without_asking() {
    let (mut w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    w.env
        .insert("LITELLM_ADMIN_TOKEN".into(), format!(" {ADMIN} "));
    let (said, got) = ensure_litellm_run(&w, &todo, "");
    assert!(got.is_ok());
    assert!(!said.contains("Paste"));
    let s = proxy.lock().unwrap();
    assert_eq!(
        s.keys[&w.stored()["LITELLM_VIBE_KEY"]]["key_alias"],
        "vibe@box"
    );
}

#[test]
fn a_pasted_token_mints() {
    let (w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    let (_, got) = ensure_litellm_run(&w, &todo, &format!("{ADMIN}\n"));
    assert!(got.is_ok());
    assert!(proxy
        .lock()
        .unwrap()
        .keys
        .contains_key(&w.stored()["LITELLM_VIBE_KEY"]));
}

#[test]
fn a_token_with_unexpected_characters_mints_nothing() {
    let (w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    let (said, _) = ensure_litellm_run(&w, &todo, "bad token!\n");
    assert!(said.contains("the LiteLLM admin token has unexpected characters; nothing minted"));
    assert!(proxy.lock().unwrap().calls.is_empty());
}

#[test]
fn the_master_key_in_the_environment_mints_the_management_key_in_process() {
    let (mut w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    w.env.insert("LITELLM_MASTER_KEY".into(), MASTER.into());
    let (said, got) = ensure_litellm_run(&w, &todo, "");
    assert!(got.is_ok());
    assert!(!said.contains("Paste"), "{said}");
    assert!(
        said.contains("minted a one-hour LiteLLM management key from the master key (not stored)")
    );
    let s = proxy.lock().unwrap();
    assert!(s.users.contains("agent-kit-setup"));
    let first: Vec<&str> = s.calls.iter().take(3).map(String::as_str).collect();
    assert_eq!(
        first,
        ["GET /user/info", "POST /user/new", "POST /key/generate"]
    );
    assert_eq!(
        s.keys[&w.stored()["LITELLM_VIBE_KEY"]]["key_alias"],
        "vibe@box"
    );
    // Only the minted keys are stored: neither the master key nor the management key.
    assert!(!w.stored().values().any(|v| v == MASTER || v == ADMIN));
    assert!(!w.logs().contains(MASTER) && !w.logs().contains(ADMIN));
}

#[test]
fn the_master_key_in_the_store_is_used_and_the_setup_user_made_once() {
    let (w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    w.put("LITELLM_MASTER_KEY", MASTER);
    proxy.lock().unwrap().users.insert("agent-kit-setup".into());
    let (said, _) = ensure_litellm_run(&w, &todo, "");
    assert!(said.contains("from the master key"));
    assert!(!proxy
        .lock()
        .unwrap()
        .calls
        .contains(&"POST /user/new".to_string()));
    assert_eq!(
        w.stored()["LITELLM_MASTER_KEY"],
        MASTER,
        "the store keeps what it held"
    );
}

#[test]
fn a_master_key_the_proxy_refuses_falls_back_to_asking() {
    let (mut w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    w.env.insert("LITELLM_MASTER_KEY".into(), "sk-wrong".into());
    let (said, _) = ensure_litellm_run(&w, &todo, &format!("{ADMIN}\n"));
    assert!(
        said.contains("could not make the agent-kit-setup user (HTTP 403: not allowed)"),
        "{said}"
    );
    assert!(said.contains("Paste a LiteLLM admin token"));
    assert!(proxy
        .lock()
        .unwrap()
        .keys
        .contains_key(&w.stored()["LITELLM_VIBE_KEY"]));
    // A user the proxy cannot make, and a key it will not mint.
    proxy.lock().unwrap().refuse_user_new = true;
    w.env.insert("LITELLM_MASTER_KEY".into(), MASTER.into());
    let (said, _) = ensure_litellm_run(&w, &todo, "\n");
    assert!(
        said.contains("could not make the agent-kit-setup user (HTTP 500: database down)"),
        "{said}"
    );
    {
        let mut s = proxy.lock().unwrap();
        s.refuse_user_new = false;
        s.refuse.insert("/key/generate".into());
        s.users.insert("agent-kit-setup".into());
    }
    let (said, _) = ensure_litellm_run(&w, &todo, "\n");
    assert!(
        said.contains("could not mint a management key (HTTP 403: not allowed)"),
        "{said}"
    );
}

#[test]
fn a_master_key_with_unexpected_characters_is_not_used() {
    let (mut w, proxy, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    w.env.insert("LITELLM_MASTER_KEY".into(), "bad key!".into());
    let (said, _) = ensure_litellm_run(&w, &todo, "\n");
    assert!(said.contains("LITELLM_MASTER_KEY has unexpected characters; not used"));
    assert!(proxy.lock().unwrap().calls.is_empty());
}

#[test]
fn a_master_key_against_an_unreachable_proxy_falls_back_to_asking() {
    let (mut w, _, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    let mut all = w.table_entries();
    all[0]["litellm"]["proxy"] = json!("http://127.0.0.1:1");
    w.set_table(&all);
    w.env.insert("LITELLM_MASTER_KEY".into(), MASTER.into());
    let (said, _) = ensure_litellm_run(&w, &todo, "\n");
    assert!(
        said.contains("could not reach the LiteLLM proxy")
            && said.contains("no management key minted"),
        "{said}"
    );
    assert!(said.contains("Paste a LiteLLM admin token"));
}

#[test]
fn an_unusable_store_stops_the_master_key_lookup() {
    let (mut w, _, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    w.env.insert("FAKE_STORE_REFUSE".into(), "1".into());
    let (_, got) = ensure_litellm_run(&w, &todo, "");
    assert!(got.is_err());
}

#[test]
fn a_table_without_a_proxy_has_no_master_key_route() {
    let (w, _, todo) = ensure_world(&["LITELLM_VIBE_KEY"]);
    let mut all = w.table_entries();
    all[0]["litellm"]["proxy"] = json!("");
    w.set_table(&all);
    w.put("LITELLM_MASTER_KEY", MASTER);
    let (said, _) = ensure_litellm_run(&w, &todo, "\n");
    assert!(said.contains("Paste a LiteLLM admin token"));
}

#[test]
fn ensure_gets_litellm_keys_end_to_end() {
    let (mut w, proxy) = litellm_world();
    w.env.insert("LITELLM_ADMIN_TOKEN".into(), ADMIN.into());
    let run = w.keys(&["ensure"], "y\n");
    assert_eq!(run.code, 0, "{}", run.err);
    assert_eq!(proxy.lock().unwrap().keys.len(), 2);
    assert!(run.err.contains(
        "LITELLM_VIBE_KEY: minted vibe@box in team vibe, checked and stored in the keychain"
    ));
    // A second run: both stored keys are valid, so nothing is minted again.
    let calls = proxy.lock().unwrap().calls.len();
    assert_eq!(w.keys(&["ensure"], "").code, 0);
    let s = proxy.lock().unwrap();
    assert_eq!(s.keys.len(), 2);
    assert!(
        !s.calls[calls..].contains(&"POST /key/generate".to_string()),
        "{:?}",
        &s.calls[calls..]
    );
}

#[test]
fn a_token_minted_without_an_id_is_stored_but_not_recorded() {
    let (w, auth) = world();
    auth.lock().unwrap().no_id = true;
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(w.stored().contains_key("HINDSIGHT_API_TOKEN"));
    assert!(!w.home(".config/agent-kit/token-ids.json").exists());
}

#[test]
fn an_admin_whose_user_record_cannot_be_read_still_grants() {
    let state = auth_state();
    let inner = state.clone();
    let base = serve(move |r| {
        let mut s = inner.lock().unwrap();
        if r.method == "GET" && r.path == "/api/v1/admin/users/u1" {
            return reply(404, json!({}));
        }
        auth_reply(&mut s, r)
    });
    let w = World::new(&base, &table(&base));
    let run = w.keys(&["ensure"], ANSWERS);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(state.lock().unwrap().grants.contains("MEMORY_MCP"));
}

#[test]
fn a_failure_passes_through_the_helper_that_is_not_for_it() {
    assert_eq!(Fail::Unreachable("a".into()).unreachable().unwrap(), "a");
    assert!(matches!(
        Fail::Refused("a".into()).unreachable(),
        Err(Fail::Refused(_))
    ));
    assert_eq!(Fail::Refused("b".into()).refused().unwrap(), "b");
    assert!(matches!(
        Fail::Stop("b".into()).refused(),
        Err(Fail::Stop(_))
    ));
}

#[test]
fn a_stored_key_scoped_narrower_than_its_team_is_minted_again() {
    let (mut w, proxy, todo) = ensure_world(&[]);
    w.env.insert("LITELLM_ADMIN_TOKEN".into(), ADMIN.into());
    // An old key that names its own access groups, one that follows its team, and a stored value the proxy refuses.
    let host = w.ctx("").0.host_name();
    let host_alias = |agent: &str| format!("{agent}@{host}");
    {
        let mut s = proxy.lock().unwrap();
        s.keys.insert("sk-old".into(), json!({"key_alias": host_alias("vibe"), "models": ["canonical"]}));
        s.keys.insert("sk-team".into(), json!({"key_alias": host_alias("local"), "models": ["all-team-models"]}));
    }
    w.put("LITELLM_VIBE_KEY", "sk-old");
    w.put("LITELLM_LOCAL_KEY", "sk-team");
    let (said, got) = ensure_litellm_run(&w, &todo, "");
    assert!(got.is_ok());
    assert!(said.contains("LITELLM_VIBE_KEY: the key is scoped to canonical, not to its team's models; minting it again"), "{said}");
    assert!(!said.contains("LITELLM_LOCAL_KEY: the key is scoped"), "{said}");
    let stored = w.stored();
    assert_ne!(stored["LITELLM_VIBE_KEY"].trim(), "sk-old");
    assert_eq!(stored["LITELLM_LOCAL_KEY"].trim(), "sk-team");
    let s = proxy.lock().unwrap();
    assert!(!s.keys.contains_key("sk-old"));
    assert_eq!(s.keys[stored["LITELLM_VIBE_KEY"].trim()]["models"], json!(["all-team-models"]));
}

#[test]
fn a_stored_key_the_proxy_does_not_describe_or_that_is_unsafe_is_left_alone() {
    let (w, proxy, todo) = ensure_world(&[]);
    w.put("LITELLM_VIBE_KEY", "sk-unknown");
    w.put("LITELLM_LOCAL_KEY", "bad key!");
    let (said, got) = ensure_litellm_run(&w, &todo, "");
    assert!(got.is_ok());
    assert!(said.is_empty(), "{said}");
    assert_eq!(proxy.lock().unwrap().calls, ["GET /key/info"]);
}

#[test]
fn a_non_utf8_environment_entry_is_skipped_not_a_panic() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let bad_value = OsStr::from_bytes(b"\x9E\xE9e").to_os_string();
    let bad_name = OsStr::from_bytes(b"BAD\xFF").to_os_string();
    let vars = vec![
        (OsString::from("PACKAGES_PROXY_TOKEN"), bad_value),
        (bad_name, OsString::from("unused")),
        (OsString::from("KEEP_ME"), OsString::from("yes")),
    ];
    let env = utf8_env(vars.into_iter());
    assert_eq!(env.get("KEEP_ME").map(String::as_str), Some("yes"));
    assert!(!env.contains_key("PACKAGES_PROXY_TOKEN"));
    assert_eq!(env.len(), 1);
}

#[test]
fn the_process_environment_is_read_without_panicking() {
    assert!(process_env().contains_key("PATH"));
}
