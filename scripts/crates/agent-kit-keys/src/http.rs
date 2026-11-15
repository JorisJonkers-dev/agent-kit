//! HTTP through `curl`, with the whole request (URL, bearer, cookies, body) on
//! curl's stdin as a config, so no secret is ever in an argv. Redirects are not
//! followed: forward-auth sends a bad token to a login page that answers 200.

use crate::Ctx;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Cloudflare refuses a default client agent.
pub const AGENT: &str = "agent-kit-keys/1";

pub struct Req<'a> {
    pub method: &'a str,
    pub url: &'a str,
    pub headers: Vec<(&'a str, String)>,
    pub body: Option<String>,
    pub jar: Option<&'a Path>,
    pub timeout: u32,
}

#[derive(Debug)]
pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

/// A value for a double-quoted curl config string: nothing in it can end the
/// string or start another option.
pub fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{b}' => out.push_str("\\v"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

pub fn config(req: &Req) -> String {
    let mut out = format!(
        "url = {}\nrequest = {}\n",
        quote(req.url),
        quote(req.method)
    );
    out += &format!("user-agent = {}\n", quote(AGENT));
    for (name, value) in &req.headers {
        out += &format!("header = {}\n", quote(&format!("{name}: {value}")));
    }
    if let Some(body) = &req.body {
        out += &format!("data-raw = {}\n", quote(body));
    }
    if let Some(jar) = req.jar {
        let jar = quote(&jar.to_string_lossy());
        out += &format!("cookie = {jar}\ncookie-jar = {jar}\n");
    }
    out += "write-out = \"\\n%{http_code}\"\n";
    out
}

/// The reply, or why there was none (curl's own words).
pub fn send(ctx: &Ctx, req: &Req) -> Result<Resp, String> {
    let failed = |e: std::io::Error| format!("curl did not run: {e}");
    let mut child = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--max-time",
            &req.timeout.to_string(),
            "--config",
            "-",
        ])
        .env_clear()
        .envs(&ctx.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    // A curl that dies early closes the pipe; its exit status says so.
    let _ = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(config(req).as_bytes()));
    let out = child.wait_with_output().map_err(failed)?;
    let mut body = out.stdout;
    let code = body
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|at| body.split_off(at))
        .and_then(|tail| String::from_utf8_lossy(&tail).trim().parse::<u16>().ok());
    match code {
        Some(status) if out.status.success() => Ok(Resp { status, body }),
        _ => {
            let why = String::from_utf8_lossy(&out.stderr)
                .trim()
                .replace("curl: ", "");
            Err(if why.is_empty() {
                "no answer".to_string()
            } else {
                why
            })
        }
    }
}

/// A JSON object, or nothing: sign-out, for one, ends on an HTML page.
pub fn json_object(raw: &[u8]) -> Value {
    match serde_json::from_slice::<Value>(raw) {
        Ok(v @ Value::Object(_)) => v,
        _ => Value::Object(Default::default()),
    }
}

/// A private temporary directory holding the cookie jar of one sign-in.
pub struct Jar {
    dir: PathBuf,
}

impl Jar {
    /// Made under `base` (the run's TMPDIR, else the system one).
    pub fn new(base: Option<&str>) -> std::io::Result<Jar> {
        use std::os::unix::fs::DirBuilderExt;
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let base = base.map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!("agent-kit-keys-{}-{n}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        Ok(Jar { dir })
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join("cookies")
    }

    /// The value of the first cookie called `name` (Netscape jar format).
    pub fn cookie(&self, name: &str) -> Option<String> {
        let text = std::fs::read_to_string(self.path()).ok()?;
        text.lines().find_map(|line| {
            let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
            let fields: Vec<&str> = line.split('\t').collect();
            (fields.len() == 7 && fields[5] == name).then(|| fields[6].to_string())
        })
    }
}

impl Drop for Jar {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
