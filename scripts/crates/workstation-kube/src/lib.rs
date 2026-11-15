//! Install a device's kubectl access: read-only for agents, admin behind sudo.
//!
//! Each device has two ServiceAccounts per cluster, declared in GitOps
//! (`cluster/flux/apps/core/workstation-access`): `<device>-ro` (view, no
//! Secrets) and `<device>-admin` (cluster-admin). `install` writes the device
//! owner's `~/.kube/config` with
//!
//! * `<cluster>`: the read-only token, inline. This is what agents use.
//! * `<cluster>-admin`: an exec credential that runs `sudo cat` on a root-only
//!   file holding the admin token. Every use asks for the sudo password, and
//!   kubectl refuses it outside a terminal.
//!
//! A previous `~/.kube/config` this tool did not write is moved, not deleted,
//! into the root-only `previous/` folder next to the admin tokens.
//!
//! Every device has its own credential set, so one device is revoked or
//! rotated without touching another. `install --rotate` deletes this device's
//! token Secrets and creates them again, then installs the new ones.
//!
//! `check` reports what an agent on the device can and cannot do, and whether
//! the CA in the device's kubeconfig still matches the cluster's.
//!
//! Tokens never reach argv or output: they travel on stdin or in a private
//! file. Admin access to mint tokens comes from `--admin-kubeconfig` (default
//! `~/.kube/config`), context `--admin-context` (default `{cluster}-admin`). A
//! kubeconfig only root can read is read with sudo.
//!
//! ```text
//! workstation-kube install --device macbook
//! workstation-kube install --device rx7900xtx --ssh deploy@100.112.121.102 --ssh-port 2222 --user ExtraToast
//! workstation-kube check --device macbook
//! ```

mod json;

use json::J;
use std::ffi::OsString;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const NAMESPACE: &str = "workstation-access";
pub const MARKER: &str = "workstation-kube";
pub const DEFAULT_CLUSTERS: [&str; 2] = ["personal", "blueshell"];
const TOKEN_WAIT_SECONDS: u64 = 60;

/// A problem the owner has to fix; printed without a backtrace.
#[derive(Debug, PartialEq, Eq)]
pub struct Failure(pub String);

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Failure {
        Failure(format!("cannot write a private temporary file: {e}"))
    }
}

type Res<T> = Result<T, Failure>;

fn fail<T>(message: String) -> Res<T> {
    Err(Failure(message))
}

/// Where the target device keeps its binaries and root-only files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub sudo: String,
    pub cat: String,
    pub secret_dir: String,
    pub root_group: String,
}

fn layout(sudo: &str, cat: &str, secret_dir: &str, root_group: &str) -> Layout {
    Layout {
        sudo: sudo.into(),
        cat: cat.into(),
        secret_dir: secret_dir.into(),
        root_group: root_group.into(),
    }
}

pub fn darwin() -> Layout {
    layout(
        "/usr/bin/sudo",
        "/bin/cat",
        "/private/etc/workstation-kube",
        "wheel",
    )
}

pub fn nixos() -> Layout {
    layout(
        "/run/wrappers/bin/sudo",
        "/run/current-system/sw/bin/cat",
        "/etc/workstation-kube",
        "root",
    )
}

pub fn linux() -> Layout {
    layout("/usr/bin/sudo", "/bin/cat", "/etc/workstation-kube", "root")
}

/// What the process around the tool provides: environment, clock, file access.
pub struct Ctx {
    /// The whole environment every child process gets.
    pub env: Vec<(String, String)>,
    pub home: String,
    pub user: Option<String>,
    pub token_wait: Duration,
    pub poll: Duration,
    pub sleep: fn(Duration),
    pub can_read: fn(&str) -> bool,
    /// The sudo a NixOS device has; its presence picks the NixOS layout.
    pub nix_sudo: String,
}

impl Ctx {
    pub fn from_env() -> Ctx {
        let env: Vec<(String, String)> = std::env::vars().collect();
        let get = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
        Ctx {
            home: get("HOME").unwrap_or_default(),
            user: get("USER"),
            env,
            token_wait: Duration::from_secs(TOKEN_WAIT_SECONDS),
            poll: Duration::from_secs(1),
            sleep: std::thread::sleep,
            can_read: |path| std::fs::File::open(path).is_ok(),
            nix_sudo: "/run/wrappers/bin/sudo".into(),
        }
    }
}

struct Out {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Run `argv` with the context's environment. `input` goes to stdin; without
/// it stdin is the terminal, or nothing when `null_stdin`.
fn exec(ctx: &Ctx, argv: &[String], input: Option<&str>, null_stdin: bool) -> Out {
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .env_clear()
        .envs(ctx.env.iter().map(|(k, v)| (k, v)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.stdin(match (input, null_stdin) {
        (Some(_), _) => Stdio::piped(),
        (None, true) => Stdio::null(),
        (None, false) => Stdio::inherit(),
    });
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Out {
                ok: false,
                stdout: String::new(),
                stderr: format!("{}: {e}", argv[0]),
            }
        }
    };
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(data.as_bytes());
    }
    let done = child.wait_with_output().expect("wait for a started child");
    Out {
        ok: done.status.success(),
        stdout: String::from_utf8_lossy(&done.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&done.stderr).into_owned(),
    }
}

/// Python's `shlex.quote`.
pub fn quote(text: &str) -> String {
    let safe = |c: char| c.is_alphanumeric() || "_@%+=:,./-".contains(c);
    if !text.is_empty() && text.chars().all(safe) {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\"'\"'"))
    }
}

fn join<S: AsRef<str>>(args: &[S]) -> String {
    args.iter()
        .map(|a| quote(a.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

/// The device being installed: this machine, or one reached over ssh.
pub struct Target<'a> {
    ctx: &'a Ctx,
    ssh: Option<String>,
    ssh_port: u32,
    user: String,
    pub layout: Layout,
    home: String,
    group: String,
}

impl<'a> Target<'a> {
    pub fn new(
        ctx: &'a Ctx,
        ssh: Option<&str>,
        ssh_port: u32,
        user: Option<&str>,
        secret_dir: Option<&str>,
    ) -> Res<Target<'a>> {
        let owner = match (user, ssh) {
            (Some(user), _) => Some(user.to_string()),
            (None, None) => ctx.user.clone(),
            (None, Some(_)) => None,
        };
        let Some(owner) = owner.filter(|o| !o.is_empty()) else {
            return fail("--user is required with --ssh".into());
        };
        let mut target = Target {
            ctx,
            ssh: ssh.map(str::to_string),
            ssh_port,
            user: owner,
            layout: linux(),
            home: String::new(),
            group: String::new(),
        };
        target.layout = target.detect_layout()?;
        if let Some(dir) = secret_dir {
            target.layout.secret_dir = dir.to_string();
        }
        // This machine: $HOME. A remote device: the owner's passwd entry.
        target.home = if ssh.is_none() {
            ctx.home.clone()
        } else {
            let script = format!("getent passwd {} | cut -d: -f6", quote(&target.user));
            target.sh(&script, None, false, true)?.trim().to_string()
        };
        if target.home.is_empty() {
            return fail(format!(
                "no home directory for {} on {}",
                target.user,
                target.name()
            ));
        }
        let script = format!("id -gn {}", quote(&target.user));
        target.group = target.sh(&script, None, false, true)?.trim().to_string();
        Ok(target)
    }

    fn detect_layout(&self) -> Res<Layout> {
        if self.sh("uname -s", None, false, true)?.trim() == "Darwin" {
            return Ok(darwin());
        }
        let probe = format!(
            "test -x {} && echo nixos || true",
            quote(&self.ctx.nix_sudo)
        );
        if self.sh(&probe, None, false, true)?.trim() == "nixos" {
            return Ok(nixos());
        }
        Ok(linux())
    }

    pub fn name(&self) -> &str {
        self.ssh.as_deref().unwrap_or("this machine")
    }

    /// Run a shell script on the target, as root when asked; data goes to stdin.
    fn sh(&self, script: &str, data: Option<&str>, root: bool, check: bool) -> Res<String> {
        let mut command = if root {
            strings(&["sudo", "sh", "-c", script])
        } else {
            strings(&["sh", "-c", script])
        };
        if let Some(ssh) = &self.ssh {
            command = vec![
                "ssh".into(),
                "-p".into(),
                self.ssh_port.to_string(),
                "-o".into(),
                "BatchMode=yes".into(),
                ssh.clone(),
                join(&command),
            ];
        }
        let result = exec(self.ctx, &command, data, false);
        if check && !result.ok {
            let first = script.split(';').next().unwrap_or_default();
            return fail(format!(
                "on {}: {first} failed: {}",
                self.name(),
                result.stderr.trim()
            ));
        }
        Ok(result.stdout)
    }

    pub fn write_root_file(&self, path: &str, data: &str) -> Res<()> {
        let d = quote(&self.layout.secret_dir);
        let p = quote(path);
        let g = &self.layout.root_group;
        let script = format!(
            "umask 077; mkdir -p {d}; chown 0:{g} {d}; chmod 700 {d}; cat > {p}; chown 0:{g} {p}; chmod 600 {p}"
        );
        self.sh(&script, Some(data), true, true).map(|_| ())
    }

    /// Write the owner's kubeconfig; move a foreign one aside. Returns where it went.
    pub fn install_kubeconfig(&self, data: &str) -> Res<Option<String>> {
        let kube = format!("{}/.kube", self.home);
        let config = format!("{kube}/config");
        let mut moved = None;
        let existing = self.sh(
            &format!("cat {} 2>/dev/null || true", quote(&config)),
            None,
            true,
            true,
        )?;
        if !existing.trim().is_empty() && !is_ours(&existing) {
            let previous = format!("{}/previous", self.layout.secret_dir);
            let stamp = exec(self.ctx, &strings(&["date", "+%Y%m%dT%H%M%S"]), None, true);
            let target = format!("{previous}/config.{}", stamp.stdout.trim());
            let g = &self.layout.root_group;
            let secret_dir = quote(&self.layout.secret_dir);
            let script = format!(
                "umask 077; mkdir -p {p}; chown -R 0:{g} {secret_dir}; chmod 700 {secret_dir} {p}; mv {c} {m}; chmod 600 {m}",
                p = quote(&previous),
                c = quote(&config),
                m = quote(&target),
            );
            self.sh(&script, None, true, true)?;
            moved = Some(target);
        }
        let (u, g) = (quote(&self.user), quote(&self.group));
        let script = format!(
            "umask 077; mkdir -p {k}; chown {u}:{g} {k}; chmod 700 {k}; cat > {c}; chown {u}:{g} {c}; chmod 600 {c}",
            k = quote(&kube),
            c = quote(&config),
        );
        self.sh(&script, Some(data), true, true)?;
        Ok(moved)
    }

    /// Run a command as the device owner, with no terminal, as an agent would.
    fn as_user(&self, args: &[&str]) -> Out {
        let inner = format!("{} </dev/null", join(args));
        let command = match &self.ssh {
            Some(ssh) => {
                let inner = format!("sudo -u {} sh -c {}", quote(&self.user), quote(&inner));
                vec![
                    "ssh".into(),
                    "-p".into(),
                    self.ssh_port.to_string(),
                    "-o".into(),
                    "BatchMode=yes".into(),
                    ssh.clone(),
                    inner,
                ]
            }
            None => vec!["sh".into(), "-c".into(), inner],
        };
        exec(self.ctx, &command, None, true)
    }
}

/// Whether a kubeconfig is one this tool wrote (it carries the marker extension).
pub fn is_ours(kubeconfig: &str) -> bool {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(kubeconfig) else {
        return false;
    };
    doc.get("extensions")
        .and_then(|e| e.as_array())
        .is_some_and(|list| list.iter().any(|e| e.get("name") == Some(&MARKER.into())))
}

/// kubectl with the owner's admin credentials, used to mint device tokens.
pub struct Admin<'a> {
    ctx: &'a Ctx,
    kubeconfig: String,
    temp: Option<String>,
    context_format: String,
}

impl Drop for Admin<'_> {
    fn drop(&mut self) {
        if let Some(path) = &self.temp {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A fresh private file name under the temporary directory.
fn private_file(prefix: &str, suffix: &str, data: &str) -> std::io::Result<String> {
    use std::os::unix::fs::OpenOptionsExt;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let path = std::env::temp_dir().join(format!(
        "{prefix}{}-{n}-{nanos}{suffix}",
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(data.as_bytes())?;
    Ok(path.to_string_lossy().into_owned())
}

impl<'a> Admin<'a> {
    pub fn new(ctx: &'a Ctx, kubeconfig: &str, context_format: &str) -> Res<Admin<'a>> {
        let mut admin = Admin {
            ctx,
            kubeconfig: kubeconfig.to_string(),
            temp: None,
            context_format: context_format.to_string(),
        };
        if !(ctx.can_read)(kubeconfig) {
            // A break-glass kubeconfig only root can read: copy it to a private temp file.
            let data = exec(ctx, &strings(&["sudo", "cat", kubeconfig]), None, false);
            if !data.ok {
                return fail(format!(
                    "cannot read admin kubeconfig {kubeconfig}: {}",
                    data.stderr.trim()
                ));
            }
            let copy = private_file("workstation-kube-", "", &data.stdout)?;
            admin.kubeconfig = copy.clone();
            admin.temp = Some(copy);
        }
        Ok(admin)
    }

    /// stdin stays the terminal: an exec credential behind sudo needs it.
    fn kubectl(&self, cluster: &str, args: &[&str], check: bool) -> Res<Out> {
        let context = self.context_format.replace("{cluster}", cluster);
        let mut argv = strings(&["kubectl", "--kubeconfig", &self.kubeconfig, "--context"]);
        argv.push(context);
        argv.extend(args.iter().map(|a| a.to_string()));
        let result = exec(self.ctx, &argv, None, false);
        if check && !result.ok {
            return fail(format!(
                "{cluster}: kubectl {} failed: {}",
                args.iter().take(3).copied().collect::<Vec<_>>().join(" "),
                result.stderr.trim()
            ));
        }
        Ok(result)
    }

    pub fn cluster_entry(&self, cluster: &str) -> Res<(String, String)> {
        let out = self.kubectl(
            cluster,
            &["config", "view", "--raw", "--minify", "-o", "json"],
            true,
        )?;
        let view: serde_json::Value = serde_json::from_str(&out.stdout)
            .map_err(|e| Failure(format!("{cluster}: kubectl config view is not JSON: {e}")))?;
        let entry = &view["clusters"][0]["cluster"];
        match (
            entry["server"].as_str(),
            entry["certificate-authority-data"].as_str(),
        ) {
            (Some(server), Some(ca)) => Ok((server.to_string(), ca.to_string())),
            _ => fail(format!(
                "{cluster}: the admin context has no server and certificate-authority-data"
            )),
        }
    }

    pub fn require_service_accounts(&self, cluster: &str, device: &str) -> Res<()> {
        for level in ["ro", "admin"] {
            let name = format!("{device}-{level}");
            let found = self.kubectl(
                cluster,
                &[
                    "-n",
                    NAMESPACE,
                    "get",
                    "serviceaccount",
                    &name,
                    "-o",
                    "name",
                ],
                false,
            )?;
            if !found.ok {
                return fail(format!(
                    "{cluster}: ServiceAccount {NAMESPACE}/{name} does not exist. Declare the device in cluster/flux/apps/core/workstation-access and let Flux apply it first."
                ));
            }
        }
        Ok(())
    }

    pub fn recreate_token_secret(&self, cluster: &str, device: &str, level: &str) -> Res<()> {
        let name = format!("{device}-{level}-token");
        self.kubectl(
            cluster,
            &[
                "-n",
                NAMESPACE,
                "delete",
                "secret",
                &name,
                "--ignore-not-found",
            ],
            true,
        )?;
        self.ensure_token_secret(cluster, device, level)
    }

    pub fn ensure_token_secret(&self, cluster: &str, device: &str, level: &str) -> Res<()> {
        let name = format!("{device}-{level}-token");
        let held = self.kubectl(
            cluster,
            &["-n", NAMESPACE, "get", "secret", &name, "-o", "name"],
            false,
        )?;
        if held.ok {
            return Ok(());
        }
        let secret = token_secret(device, level).pretty();
        // A file, not stdin: stdin must stay the terminal for a sudo-backed context.
        let file = private_file("workstation-kube-secret-", ".json", &secret)?;
        let created = self.kubectl(cluster, &["create", "-f", &file], true);
        let _ = std::fs::remove_file(&file);
        created.map(|_| ())
    }

    pub fn token(&self, cluster: &str, device: &str, level: &str) -> Res<String> {
        let name = format!("{device}-{level}-token");
        let deadline = Instant::now() + self.ctx.token_wait;
        loop {
            let got = self.kubectl(
                cluster,
                &["-n", NAMESPACE, "get", "secret", &name, "-o", "json"],
                false,
            )?;
            if got.ok {
                let doc: serde_json::Value = serde_json::from_str(&got.stdout).unwrap_or_default();
                let decoded = doc["data"]["token"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .and_then(base64_decode)
                    .and_then(|bytes| String::from_utf8(bytes).ok());
                if let Some(token) = decoded {
                    return Ok(token);
                }
            }
            if Instant::now() > deadline {
                return fail(format!(
                    "{cluster}: Secret {NAMESPACE}/{name} has no token after {}s",
                    self.ctx.token_wait.as_secs()
                ));
            }
            (self.ctx.sleep)(self.ctx.poll);
        }
    }
}

/// Standard base64, padding optional; `None` on a character outside the alphabet.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut bits, mut have) = (0u32, 0u32);
    for c in text.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'\n' | b'\r' => continue,
            _ => return None,
        };
        bits = (bits << 6) | u32::from(v);
        have += 6;
        if have >= 8 {
            have -= 8;
            out.push((bits >> have) as u8);
            bits &= (1 << have) - 1;
        }
    }
    Some(out)
}

fn s(text: &str) -> J {
    J::S(text.to_string())
}

fn obj(fields: Vec<(&str, J)>) -> J {
    J::O(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

/// The token Secret exactly as GitOps declares it, so Flux adopts it unchanged.
pub fn token_secret(device: &str, level: &str) -> J {
    let labels = obj(vec![
        ("workstation-access/device", s(device)),
        ("workstation-access/level", s(level)),
    ]);
    obj(vec![
        ("apiVersion", s("v1")),
        ("kind", s("Secret")),
        ("type", s("kubernetes.io/service-account-token")),
        (
            "metadata",
            obj(vec![
                ("name", s(&format!("{device}-{level}-token"))),
                ("namespace", s(NAMESPACE)),
                ("labels", labels),
                (
                    "annotations",
                    obj(vec![(
                        "kubernetes.io/service-account.name",
                        s(&format!("{device}-{level}")),
                    )]),
                ),
            ]),
        ),
    ])
}

pub fn exec_credential(token: &str) -> String {
    let doc = obj(vec![
        ("apiVersion", s("client.authentication.k8s.io/v1")),
        ("kind", s("ExecCredential")),
        ("status", obj(vec![("token", s(token))])),
    ]);
    format!("{}\n", doc.compact())
}

pub fn admin_token_path(layout: &Layout, cluster: &str) -> String {
    format!("{}/{cluster}-admin.json", layout.secret_dir)
}

/// One cluster of the rendered kubeconfig: name, server, CA data, read-only token.
pub struct ClusterEntry {
    pub name: String,
    pub server: String,
    pub ca: String,
    pub ro_token: String,
}

/// The device's kubeconfig: inline read-only users, sudo-gated admin users.
pub fn render_kubeconfig(device: &str, layout: &Layout, clusters: &[ClusterEntry]) -> String {
    let (mut users, mut contexts, mut entries) = (vec![], vec![], vec![]);
    for c in clusters {
        let name = &c.name;
        users.push(obj(vec![
            ("name", s(&format!("{name}-{device}-ro"))),
            ("user", obj(vec![("token", s(&c.ro_token))])),
        ]));
        users.push(obj(vec![
            ("name", s(&format!("{name}-{device}-admin"))),
            (
                "user",
                obj(vec![(
                    "exec",
                    obj(vec![
                        ("apiVersion", s("client.authentication.k8s.io/v1")),
                        ("command", s(&layout.sudo)),
                        (
                            "args",
                            J::A(vec![s(&layout.cat), s(&admin_token_path(layout, name))]),
                        ),
                        ("interactiveMode", s("Always")),
                        ("provideClusterInfo", J::B(false)),
                    ]),
                )]),
            ),
        ]));
        contexts.push(obj(vec![
            ("name", s(name)),
            (
                "context",
                obj(vec![
                    ("cluster", s(name)),
                    ("user", s(&format!("{name}-{device}-ro"))),
                ]),
            ),
        ]));
        contexts.push(obj(vec![
            ("name", s(&format!("{name}-admin"))),
            (
                "context",
                obj(vec![
                    ("cluster", s(name)),
                    ("user", s(&format!("{name}-{device}-admin"))),
                ]),
            ),
        ]));
        entries.push(obj(vec![
            ("name", s(name)),
            (
                "cluster",
                obj(vec![
                    ("server", s(&c.server)),
                    ("certificate-authority-data", s(&c.ca)),
                ]),
            ),
        ]));
    }
    let first = clusters.first().map(|c| c.name.as_str()).unwrap_or("");
    let doc = obj(vec![
        ("apiVersion", s("v1")),
        ("kind", s("Config")),
        ("preferences", obj(vec![])),
        (
            "extensions",
            J::A(vec![obj(vec![
                ("name", s(MARKER)),
                ("extension", obj(vec![("device", s(device))])),
            ])]),
        ),
        ("current-context", s(first)),
        ("clusters", J::A(entries)),
        ("users", J::A(users)),
        ("contexts", J::A(contexts)),
    ]);
    format!("{}\n", doc.pretty())
}

/// Where a path starting with `~` goes: the owner's home.
fn expand_user(path: &str, home: &str) -> String {
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{home}{rest}"),
        _ => path.to_string(),
    }
}

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub install: bool,
    pub device: String,
    pub clusters: Vec<String>,
    pub ssh: Option<String>,
    pub ssh_port: u32,
    pub user: Option<String>,
    pub secret_dir: Option<String>,
    pub rotate: bool,
    pub admin_kubeconfig: String,
    pub admin_context: String,
}

const USAGE: &str = "usage: workstation-kube {install,check} --device DEVICE [--clusters A,B] [--ssh USER@HOST] [--ssh-port N] [--user USER] [--secret-dir DIR]\n       install only: [--rotate] [--admin-kubeconfig PATH] [--admin-context FORMAT]";

/// `Err(Ok(text))` is a request for help, `Err(Err(text))` a usage error.
pub fn parse(args: &[String]) -> Result<Options, Result<String, String>> {
    let Some(command) = args.first() else {
        return Err(Err("a command is required: install or check".into()));
    };
    let install = match command.as_str() {
        "-h" | "--help" => return Err(Ok(USAGE.to_string())),
        "install" => true,
        "check" => false,
        other => return Err(Err(format!("unknown command {other:?}"))),
    };
    let mut opts = Options {
        install,
        device: String::new(),
        clusters: DEFAULT_CLUSTERS.iter().map(|c| c.to_string()).collect(),
        ssh: None,
        ssh_port: 22,
        user: None,
        secret_dir: None,
        rotate: false,
        admin_kubeconfig: "~/.kube/config".into(),
        admin_context: "{cluster}-admin".into(),
    };
    let mut device = None;
    let mut rest = args[1..].iter();
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (arg.as_str(), None),
        };
        if flag == "-h" || flag == "--help" {
            return Err(Ok(USAGE.to_string()));
        }
        if flag == "--rotate" && install {
            opts.rotate = true;
            continue;
        }
        let known = [
            "--device",
            "--clusters",
            "--ssh",
            "--ssh-port",
            "--user",
            "--secret-dir",
        ];
        let install_only = ["--admin-kubeconfig", "--admin-context"];
        if !known.contains(&flag) && !(install && install_only.contains(&flag)) {
            return Err(Err(format!("unrecognized argument {arg:?}")));
        }
        let Some(value) = inline.or_else(|| rest.next().cloned()) else {
            return Err(Err(format!("{flag} needs a value")));
        };
        match flag {
            "--device" => device = Some(value),
            "--clusters" => opts.clusters = value.split(',').map(str::to_string).collect(),
            "--ssh" => opts.ssh = Some(value),
            "--ssh-port" => match value.parse() {
                Ok(port) => opts.ssh_port = port,
                Err(_) => return Err(Err(format!("--ssh-port: invalid int value {value:?}"))),
            },
            "--user" => opts.user = Some(value),
            "--secret-dir" => opts.secret_dir = Some(value),
            "--admin-kubeconfig" => opts.admin_kubeconfig = value,
            _ => opts.admin_context = value,
        }
    }
    match device {
        Some(device) => {
            opts.device = device;
            Ok(opts)
        }
        None => Err(Err("--device is required".into())),
    }
}

fn line(out: &mut dyn Write, text: &str, ok: bool) -> u32 {
    let _ = writeln!(out, "  {} {text}", if ok { "ok  " } else { "FAIL" });
    u32::from(!ok)
}

fn install(opts: &Options, ctx: &Ctx, out: &mut dyn Write) -> Res<i32> {
    let target = Target::new(
        ctx,
        opts.ssh.as_deref(),
        opts.ssh_port,
        opts.user.as_deref(),
        opts.secret_dir.as_deref(),
    )?;
    let admin = Admin::new(
        ctx,
        &expand_user(&opts.admin_kubeconfig, &ctx.home),
        &opts.admin_context,
    )?;
    let device = &opts.device;
    let mut clusters = vec![];
    for cluster in &opts.clusters {
        admin.require_service_accounts(cluster, device)?;
        for level in ["ro", "admin"] {
            if opts.rotate {
                admin.recreate_token_secret(cluster, device, level)?;
            } else {
                admin.ensure_token_secret(cluster, device, level)?;
            }
        }
        let (server, ca) = admin.cluster_entry(cluster)?;
        let ro_token = admin.token(cluster, device, "ro")?;
        clusters.push(ClusterEntry {
            name: cluster.clone(),
            server,
            ca,
            ro_token,
        });
        let admin_token = admin.token(cluster, device, "admin")?;
        target.write_root_file(
            &admin_token_path(&target.layout, cluster),
            &exec_credential(&admin_token),
        )?;
        let _ = writeln!(
            out,
            "{cluster}: {device} tokens {}installed on {}",
            if opts.rotate { "rotated and " } else { "" },
            target.name()
        );
    }
    let config = render_kubeconfig(device, &target.layout, &clusters);
    if let Some(moved) = target.install_kubeconfig(&config)? {
        let _ = writeln!(out, "previous kubeconfig moved to {moved} (root only)");
    }
    Ok(report(&target, device, &opts.clusters, out))
}

/// What an agent on the device must be able to do, and what it must not.
const EXPECTED: [(&[&str], &str); 4] = [
    (&["auth", "can-i", "list", "pods", "-A"], "yes"),
    (&["auth", "can-i", "get", "secrets", "-A"], "no"),
    (
        &["auth", "can-i", "create", "deployments", "-n", "default"],
        "no",
    ),
    (&["auth", "can-i", "delete", "pods", "-n", "default"], "no"),
];

/// Whether the CA in the device's kubeconfig is the CA the cluster publishes.
fn ca_matches(target: &Target, cluster: &str) -> bool {
    let view = target.as_user(&[
        "kubectl",
        "--context",
        cluster,
        "config",
        "view",
        "--raw",
        "--minify",
        "-o",
        "json",
    ]);
    let published = target.as_user(&[
        "kubectl",
        "--context",
        cluster,
        "-n",
        "default",
        "get",
        "configmap",
        "kube-root-ca.crt",
        "-o",
        r"jsonpath={.data.ca\.crt}",
    ]);
    if !view.ok || !published.ok {
        return false; // an unknown authority fails the call itself
    }
    let doc: serde_json::Value = serde_json::from_str(&view.stdout).unwrap_or_default();
    let held = doc["clusters"][0]["cluster"]["certificate-authority-data"]
        .as_str()
        .and_then(base64_decode);
    held.is_some_and(|held| String::from_utf8_lossy(&held).trim() == published.stdout.trim())
}

fn report(target: &Target, device: &str, clusters: &[String], out: &mut dyn Write) -> i32 {
    let mut failures = 0;
    for cluster in clusters {
        let path = admin_token_path(&target.layout, cluster);
        let plain = target.as_user(&["cat", &path]);
        failures += line(
            out,
            &format!("{cluster}: admin token unreadable without sudo"),
            !plain.ok,
        );
        // sudo -n fails when it would need a password; a NOPASSWD or cached grant would let an agent through.
        let silent = target.as_user(&["sudo", "-n", &target.layout.cat, &path]);
        failures += line(
            out,
            &format!("{cluster}: admin token not readable through sudo without a password"),
            !silent.ok,
        );
        for (kubectl_args, want) in EXPECTED {
            let mut argv = vec!["kubectl", "--context", cluster];
            argv.extend(kubectl_args);
            let got = target.as_user(&argv);
            let last = got.stdout.trim().lines().last().unwrap_or("").to_string();
            failures += line(
                out,
                &format!("{cluster}: {} -> {want}", kubectl_args[2..].join(" ")),
                last == want,
            );
        }
        if ca_matches(target, cluster) {
            line(out, &format!("{cluster}: CA matches the cluster"), true);
        } else {
            failures += line(out, &format!("{cluster}: CA matches the cluster"), false);
            let _ = writeln!(
                out,
                "       the cluster CA changed or cannot be read: re-run install --device {device}"
            );
        }
        let admin_context = format!("{cluster}-admin");
        let refused =
            target.as_user(&["kubectl", "--context", &admin_context, "get", "namespaces"]);
        failures += line(
            out,
            &format!("{cluster}-admin refused without a terminal"),
            !refused.ok,
        );
    }
    i32::from(failures > 0)
}

fn check(opts: &Options, ctx: &Ctx, out: &mut dyn Write) -> Res<i32> {
    let target = Target::new(
        ctx,
        opts.ssh.as_deref(),
        opts.ssh_port,
        opts.user.as_deref(),
        opts.secret_dir.as_deref(),
    )?;
    Ok(report(&target, &opts.device, &opts.clusters, out))
}

/// The command with a context: returns the exit status.
pub fn run_with(args: &[String], ctx: &Ctx, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let opts = match parse(args) {
        Ok(opts) => opts,
        Err(Ok(help)) => {
            let _ = writeln!(out, "{help}");
            return 0;
        }
        Err(Err(message)) => {
            let _ = writeln!(err, "{USAGE}\nworkstation-kube: error: {message}");
            return 2;
        }
    };
    let result = if opts.install {
        install(&opts, ctx, out)
    } else {
        check(&opts, ctx, out)
    };
    result.unwrap_or_else(|Failure(message)| {
        let _ = writeln!(err, "workstation-kube: {message}");
        1
    })
}

pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    run_with(
        &args,
        &Ctx::from_env(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )
}

#[cfg(test)]
mod tests;
