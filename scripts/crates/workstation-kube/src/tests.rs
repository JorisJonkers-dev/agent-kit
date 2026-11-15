//! The port of tests/test_workstation_kube.py: a device whose kubectl, sudo,
//! ssh, chown, getent and cat are shell stand-ins on PATH, over a state
//! directory of ServiceAccounts and token Secrets.

use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const FAKE_KUBECTL: &str = r#"#!/bin/sh
# A stand-in for kubectl over a state directory: ServiceAccounts, token Secrets, contexts.
echo "kubectl $*" >> "$FAKE_ARGV_LOG"
S=$FAKE_KUBE_STATE
ctx=personal
while [ $# -gt 0 ]; do
  case "$1" in
    --context) ctx=$2; shift 2 ;;
    --kubeconfig) shift 2 ;;
    -n) shift 2 ;;
    *) break ;;
  esac
done
admin=
cluster=$ctx
case "$ctx" in *-admin) admin=1; cluster=${ctx%-admin} ;; esac
b64() { printf %s "$1" | base64 | tr -d '\n'; }
if [ -n "$admin" ] && [ -n "$FAKE_AS_AGENT" ] && [ ! -t 0 ]; then
  echo "exec plugin cannot support interactive mode: standard input is not a terminal" >&2
  exit 1
fi
case "$1 $2" in
  "config view")
    printf '{"clusters":[{"name":"%s","cluster":{"server":"https://%s:6443","certificate-authority-data":"%s"}}]}\n' \
      "$cluster" "$cluster" "$(b64 "CA-$cluster")" ;;
  "auth can-i")
    if [ -n "$admin" ] || { [ "$3" = list ] && [ "$4" = pods ]; }; then echo yes; else echo no; fi ;;
  "get serviceaccount")
    grep -qx "$cluster/$3" "$S/sas" || exit 1 ;;
  "get secret")
    [ -f "$S/secrets/$cluster/$3" ] || exit 1
    printf '{"data":{"token":"%s"}}\n' "$(b64 "$(cat "$S/secrets/$cluster/$3")")" ;;
  "delete secret")
    rm -f "$S/secrets/$cluster/$3" ;;
  "create -f")
    n=$(($(cat "$S/minted") + 1)); echo "$n" > "$S/minted"
    name=$(grep -m1 '"name"' "$3" | sed 's/.*: "\(.*\)".*/\1/')
    mkdir -p "$S/secrets/$cluster" "$S/created"
    printf %s "TOKEN-$cluster-$name-$n" > "$S/secrets/$cluster/$name"
    cp "$3" "$S/created/$n.json" ;;
  "get configmap")
    if [ -f "$S/ca/$cluster" ]; then cat "$S/ca/$cluster"; else echo "CA-$cluster"; fi ;;
  "get namespaces")
    echo default ;;
  *)
    echo "fake kubectl: unhandled $*" >&2; exit 1 ;;
esac
"#;

// sudo and ssh run their command here; chown only records, since tests are not root.
const FAKE_SUDO: &str = r#"#!/bin/sh
echo "sudo $*" >> "$FAKE_ARGV_LOG"
noninteractive=
while [ "${1#-}" != "$1" ]; do [ "$1" = "-n" ] && noninteractive=1; [ "$1" = "-u" ] && shift; shift; done
if [ -n "$noninteractive" ] && [ -z "$FAKE_NOPASSWD" ]; then echo "sudo: a password is required" >&2; exit 1; fi
if [ -n "$FAKE_NOPASSWD" ]; then shift; exec @REAL_CAT@ "$@"; fi
exec "$@"
"#;
const FAKE_SSH: &str = r#"#!/bin/sh
echo "ssh $*" >> "$FAKE_ARGV_LOG"
for last; do :; done
exec sh -c "$last"
"#;
// The script finds a remote owner's home in passwd; the fake ssh runs here, so
// the real getent would name the real home.
const FAKE_GETENT: &str = r#"#!/bin/sh
echo "getent $*" >> "$FAKE_ARGV_LOG"
[ "$1" = passwd ] && [ -n "$2" ] || exit 2
echo "$2:x:1000:100::$FAKE_HOME:/bin/sh"
"#;
const FAKE_CHOWN: &str = r#"#!/bin/sh
echo "chown $*" >> "$FAKE_ARGV_LOG"
"#;
// Tests are not root, so file modes cannot stop an agent; this cat stands in for them.
const FAKE_CAT: &str = r#"#!/bin/sh
if [ -n "$FAKE_AS_AGENT" ]; then
  for a; do case "$a" in "$FAKE_ROOT_ONLY"*) echo "cat: $a: Permission denied" >&2; exit 1;; esac; done
fi
exec @REAL_CAT@ "$@"
"#;

fn real_cat() -> String {
    let path = std::env::var("PATH").unwrap_or_default();
    path.split(':')
        .map(|d| format!("{d}/cat"))
        .find(|c| Path::new(c).is_file())
        .expect("a cat on PATH")
}

static NEXT: AtomicU32 = AtomicU32::new(0);

struct Device {
    tmp: PathBuf,
    home: PathBuf,
    secret_dir: PathBuf,
    state: PathBuf,
    log: PathBuf,
    env: Vec<(String, String)>,
}

impl Drop for Device {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tmp);
    }
}

struct Ran {
    code: i32,
    out: String,
    err: String,
}

fn ctx(env: Vec<(String, String)>) -> Ctx {
    let mut ctx = Ctx::from_env();
    ctx.home = env.iter().find(|(k, _)| k == "HOME").unwrap().1.clone();
    ctx.env = env;
    ctx.sleep = |_| {};
    ctx.token_wait = Duration::from_millis(50);
    ctx.poll = Duration::from_millis(1);
    ctx
}

fn run_ctx(ctx: &Ctx, args: &[&str]) -> Ran {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = run_with(&args, ctx, &mut out, &mut err);
    Ran {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

impl Device {
    fn new() -> Device {
        let tmp = std::env::temp_dir().join(format!(
            "wk-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let home = tmp.join("home");
        let bin = tmp.join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let cat = real_cat();
        for (name, body) in [
            ("kubectl", FAKE_KUBECTL),
            ("sudo", FAKE_SUDO),
            ("ssh", FAKE_SSH),
            ("chown", FAKE_CHOWN),
            ("getent", FAKE_GETENT),
            ("cat", FAKE_CAT),
        ] {
            let file = bin.join(name);
            fs::write(&file, body.replace("@REAL_CAT@", &cat)).unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let secret_dir = tmp.join("root-only");
        let state = tmp.join("state");
        let log = tmp.join("argv.log");
        fs::create_dir_all(&state).unwrap();
        fs::write(&log, "").unwrap();
        fs::write(tmp.join("admin-config"), "{}").unwrap();
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        let mut env: Vec<(String, String)> = std::env::vars()
            .filter(|(k, _)| !k.starts_with("FAKE_") && k != "HOME" && k != "PATH")
            .collect();
        for (k, v) in [
            ("PATH", path),
            ("HOME", home.display().to_string()),
            ("FAKE_HOME", home.display().to_string()),
            ("FAKE_KUBE_STATE", state.display().to_string()),
            ("FAKE_ARGV_LOG", log.display().to_string()),
            ("FAKE_ROOT_ONLY", secret_dir.display().to_string()),
        ] {
            env.push((k.into(), v));
        }
        let device = Device {
            tmp,
            home,
            secret_dir,
            state,
            log,
            env,
        };
        let all = ["personal", "blueshell"]
            .iter()
            .flat_map(|c| ["ro", "admin"].map(move |l| format!("{c}/macbook-{l}")))
            .collect::<Vec<_>>();
        device.set_sas(&all);
        device
    }

    fn set_sas(&self, sas: &[String]) {
        fs::create_dir_all(&self.state).unwrap();
        fs::write(self.state.join("sas"), sas.join("\n") + "\n").unwrap();
        if !self.state.join("minted").exists() {
            fs::write(self.state.join("minted"), "0").unwrap();
        }
    }

    fn set_env(&mut self, key: &str, value: &str) {
        self.env.retain(|(k, _)| k != key);
        self.env.push((key.into(), value.into()));
    }

    fn run(&self, args: &[&str], agent: bool) -> Ran {
        let mut env = self.env.clone();
        if agent {
            env.push(("FAKE_AS_AGENT".into(), "1".into()));
        }
        let secret = self.secret_dir.display().to_string();
        let mut all = args.to_vec();
        all.extend(["--device", "macbook", "--secret-dir", &secret]);
        run_ctx(&ctx(env), &all)
    }

    fn install(&self, extra: &[&str]) -> Ran {
        // The owner's admin context is a plain one here: the fake has no terminal to offer sudo.
        let admin = self.tmp.join("admin-config").display().to_string();
        let mut args = vec![
            "install",
            "--admin-kubeconfig",
            &admin,
            "--admin-context",
            "{cluster}",
        ];
        args.extend(extra);
        let result = self.run(&args, true);
        assert_eq!(result.code, 0, "{}{}", result.err, result.out);
        result
    }

    fn kubeconfig(&self) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(self.home.join(".kube/config")).unwrap()).unwrap()
    }

    fn tokens(&self) -> std::collections::BTreeMap<String, String> {
        let mut found = std::collections::BTreeMap::new();
        for cluster in ["personal", "blueshell"] {
            let dir = self.state.join("secrets").join(cluster);
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries {
                let entry = entry.unwrap();
                let key = format!("{cluster}/{}", entry.file_name().to_string_lossy());
                found.insert(key, fs::read_to_string(entry.path()).unwrap());
            }
        }
        found
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).unwrap()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }
}

fn user(config: &serde_json::Value, name: &str) -> serde_json::Value {
    config["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["name"] == name)
        .unwrap()["user"]
        .clone()
}

fn context_names(config: &serde_json::Value) -> Vec<String> {
    config["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn an_agent_context_holds_the_read_only_token_inline() {
    let d = Device::new();
    d.install(&[]);
    let token = d.tokens()["personal/macbook-ro-token"].clone();
    assert_eq!(
        user(&d.kubeconfig(), "personal-macbook-ro"),
        serde_json::json!({"token": token})
    );
}

#[test]
fn an_admin_context_reads_its_token_with_sudo_from_the_root_only_folder() {
    let d = Device::new();
    let result = d.install(&[]);
    assert!(result
        .out
        .contains("personal: macbook tokens installed on this machine"));
    let exec = user(&d.kubeconfig(), "blueshell-macbook-admin")["exec"].clone();
    let last = exec["args"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        last,
        format!("{}/blueshell-admin.json", d.secret_dir.display())
    );
    assert!(exec["command"].as_str().unwrap().ends_with("/sudo"));
    assert_eq!(exec["interactiveMode"], "Always");
    assert_eq!(exec["provideClusterInfo"], false);
}

#[test]
fn the_admin_token_file_is_an_exec_credential_holding_the_admin_token() {
    let d = Device::new();
    d.install(&[]);
    let text = fs::read_to_string(d.secret_dir.join("personal-admin.json")).unwrap();
    assert!(text.ends_with('\n'));
    let credential: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(credential["kind"], "ExecCredential");
    assert_eq!(
        credential["status"]["token"],
        d.tokens()["personal/macbook-admin-token"]
    );
}

#[test]
fn the_admin_token_file_is_owner_only() {
    let d = Device::new();
    d.install(&[]);
    assert_eq!(
        Device::mode(&d.secret_dir.join("personal-admin.json")) & 0o077,
        0
    );
    assert_eq!(Device::mode(&d.secret_dir) & 0o077, 0);
}

#[test]
fn every_context_is_listed_for_both_clusters() {
    let d = Device::new();
    d.install(&[]);
    assert_eq!(
        context_names(&d.kubeconfig()),
        ["personal", "personal-admin", "blueshell", "blueshell-admin"]
    );
}

#[test]
fn no_token_ever_reaches_a_command_line() {
    let d = Device::new();
    d.install(&[]);
    let logged = d.log();
    assert!(!d.tokens().is_empty());
    for token in d.tokens().values() {
        assert!(!logged.contains(token.as_str()));
    }
}

#[test]
fn rotate_replaces_both_tokens_of_the_device() {
    let d = Device::new();
    d.install(&[]);
    let before = d.tokens();
    let result = d.install(&["--rotate"]);
    assert!(result.out.contains("tokens rotated and installed"));
    let after = d.tokens();
    assert!(before.iter().all(|(k, v)| &after[k] != v));
    assert_eq!(
        user(&d.kubeconfig(), "personal-macbook-ro")["token"],
        after["personal/macbook-ro-token"]
    );
}

#[test]
fn a_recreated_token_secret_matches_what_gitops_declares() {
    let d = Device::new();
    d.install(&[]);
    let created: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(d.state.join("created/1.json")).unwrap()).unwrap();
    assert_eq!(created["type"], "kubernetes.io/service-account-token");
    assert_eq!(
        created["metadata"]["annotations"],
        serde_json::json!({"kubernetes.io/service-account.name": "macbook-ro"})
    );
    assert_eq!(
        created["metadata"]["labels"],
        serde_json::json!({"workstation-access/device": "macbook", "workstation-access/level": "ro"})
    );
}

#[test]
fn a_kubeconfig_it_did_not_write_is_moved_to_the_root_only_folder() {
    let d = Device::new();
    fs::create_dir_all(d.home.join(".kube")).unwrap();
    fs::write(d.home.join(".kube/config"), "clusters: [old-admin-cert]\n").unwrap();
    let result = d.install(&[]);
    assert!(result.out.contains("previous kubeconfig moved to "));
    let moved: Vec<String> = fs::read_dir(d.secret_dir.join("previous"))
        .unwrap()
        .map(|e| fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    assert_eq!(moved, ["clusters: [old-admin-cert]\n"]);
}

#[test]
fn its_own_kubeconfig_is_replaced_without_keeping_a_copy() {
    let d = Device::new();
    d.install(&[]);
    d.install(&[]);
    assert!(!d.secret_dir.join("previous").exists());
}

#[test]
fn a_device_gitops_does_not_declare_fails_naming_where_to_declare_it() {
    let d = Device::new();
    d.set_sas(&[]);
    let admin = d.tmp.join("admin-config").display().to_string();
    let result = d.run(&["install", "--admin-kubeconfig", &admin], false);
    assert_eq!(result.code, 1);
    assert!(result
        .err
        .contains("cluster/flux/apps/core/workstation-access"));
    assert!(result.err.starts_with("workstation-kube: "));
}

#[test]
fn a_remote_device_is_installed_over_ssh() {
    let d = Device::new();
    let owner = std::env::var("USER").unwrap_or_else(|_| "nobody".into());
    d.install(&["--ssh", "deploy@desktop", "--user", &owner]);
    assert!(d
        .log()
        .contains("ssh -p 22 -o BatchMode=yes deploy@desktop"));
    assert!(d.secret_dir.join("personal-admin.json").exists());
    assert!(d.home.join(".kube/config").exists());
}

#[test]
fn check_passes_when_an_agent_can_read_but_not_administer() {
    let d = Device::new();
    d.install(&[]);
    let result = d.run(&["check"], true);
    assert_eq!(result.code, 0, "{}", result.out);
    assert!(!result.out.contains("FAIL"));
    assert!(result.out.contains("  ok   personal: list pods -A -> yes"));
    assert!(result
        .out
        .contains("  ok   blueshell: create deployments -n default -> no"));
}

#[test]
fn check_fails_when_an_admin_context_works_without_a_terminal() {
    let d = Device::new();
    d.install(&[]);
    let result = d.run(&["check"], false); // not an agent: the fake lets the admin context through
    assert_eq!(result.code, 1);
    assert!(result
        .out
        .contains("FAIL personal-admin refused without a terminal"));
}

#[test]
fn a_nixos_device_runs_sudo_and_cat_from_the_nix_paths() {
    let config: serde_json::Value = serde_json::from_str(&render_kubeconfig(
        "rx7900xtx",
        &nixos(),
        &[ClusterEntry {
            name: "personal".into(),
            server: "s".into(),
            ca: "c".into(),
            ro_token: "t".into(),
        }],
    ))
    .unwrap();
    let exec = user(&config, "personal-rx7900xtx-admin")["exec"].clone();
    assert_eq!(
        exec["command"].as_str().unwrap().to_string() + " " + exec["args"][0].as_str().unwrap() + " " + exec["args"][1].as_str().unwrap(),
        "/run/wrappers/bin/sudo /run/current-system/sw/bin/cat /etc/workstation-kube/personal-admin.json"
    );
}

#[test]
fn check_tells_the_owner_to_reinstall_when_the_cluster_ca_changed() {
    let d = Device::new();
    d.install(&[]);
    fs::create_dir_all(d.state.join("ca")).unwrap();
    fs::write(d.state.join("ca/personal"), "CA-rotated\n").unwrap();
    let result = d.run(&["check"], true);
    assert_eq!(result.code, 1);
    assert!(result.out.contains("FAIL personal: CA matches the cluster"));
    assert!(result.out.contains("re-run install --device macbook"));
    assert!(!result.out.contains("FAIL blueshell"));
}

#[test]
fn each_device_has_its_own_credentials_revocable_alone() {
    let d = Device::new();
    let all: Vec<String> = ["personal", "blueshell"]
        .iter()
        .flat_map(|c| {
            ["macbook", "rx7900xtx"]
                .iter()
                .flat_map(move |dev| ["ro", "admin"].map(move |l| format!("{c}/{dev}-{l}")))
        })
        .collect();
    d.set_sas(&all);
    d.install(&[]);
    let admin = d.tmp.join("admin-config").display().to_string();
    let other_dir = d.secret_dir.join("rx7900xtx").display().to_string();
    let mut env = d.env.clone();
    env.push(("FAKE_AS_AGENT".into(), "1".into()));
    let other = run_ctx(
        &ctx(env),
        &[
            "install",
            "--device",
            "rx7900xtx",
            "--secret-dir",
            &other_dir,
            "--admin-kubeconfig",
            &admin,
            "--admin-context",
            "{cluster}",
        ],
    );
    assert_eq!(other.code, 0, "{}", other.err);
    let before = d.tokens();
    let mine: Vec<_> = before.keys().filter(|k| k.contains("macbook")).collect();
    let theirs: Vec<_> = before.keys().filter(|k| k.contains("rx7900xtx")).collect();
    assert_eq!((mine.len(), theirs.len()), (4, 4));
    let distinct: std::collections::BTreeSet<_> = before.values().collect();
    assert_eq!(distinct.len(), before.len());
    d.install(&["--rotate"]);
    let after = d.tokens();
    assert!(theirs.iter().all(|k| after[*k] == before[*k]));
    assert!(mine.iter().all(|k| after[*k] != before[*k]));
}

#[test]
fn check_fails_when_sudo_would_read_the_admin_token_without_a_password() {
    let mut d = Device::new();
    d.install(&[]);
    d.set_env("FAKE_NOPASSWD", "1");
    let result = d.run(&["check"], true);
    assert_eq!(result.code, 1);
    assert!(result
        .out
        .contains("FAIL personal: admin token not readable through sudo without a password"));
}

#[test]
fn the_kubeconfig_holds_only_read_only_and_admin_contexts_and_is_owner_only() {
    let d = Device::new();
    d.install(&[]);
    assert_eq!(
        context_names(&d.kubeconfig()),
        ["personal", "personal-admin", "blueshell", "blueshell-admin"]
    );
    assert_eq!(Device::mode(&d.home.join(".kube/config")), 0o600);
}

// ----- beyond the Python suite: what the port has to cover itself -------------

fn me() -> String {
    std::env::var("USER").unwrap_or_else(|_| "nobody".into())
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

#[test]
fn the_kubeconfig_is_python_json_with_the_documented_key_order() {
    let text = render_kubeconfig(
        "d",
        &linux(),
        &[ClusterEntry {
            name: "c".into(),
            server: "s".into(),
            ca: "ca".into(),
            ro_token: "t".into(),
        }],
    );
    assert!(text.starts_with("{\n  \"apiVersion\": \"v1\",\n  \"kind\": \"Config\",\n  \"preferences\": {},\n  \"extensions\": [\n"));
    assert!(text.ends_with("  ]\n}\n"));
    assert!(text.contains("\"current-context\": \"c\""));
    let empty = render_kubeconfig("d", &linux(), &[]);
    assert!(empty.contains("\"current-context\": \"\""));
    assert!(empty.contains("\"clusters\": []"));
}

#[test]
fn json_writes_compact_and_escaped() {
    let value = J::A(vec![
        J::S("a\"b".into()),
        J::B(true),
        J::O(vec![("k".into(), J::A(vec![]))]),
    ]);
    assert_eq!(value.compact(), r#"["a\"b", true, {"k": []}]"#);
    assert_eq!(exec_credential("t"), "{\"apiVersion\": \"client.authentication.k8s.io/v1\", \"kind\": \"ExecCredential\", \"status\": {\"token\": \"t\"}}\n");
}

#[test]
fn quoting_follows_shlex() {
    assert_eq!(quote("plain-1/x.y"), "plain-1/x.y");
    assert_eq!(quote(""), "''");
    assert_eq!(quote("a b"), "'a b'");
    assert_eq!(quote("it's"), "'it'\"'\"'s'");
    assert_eq!(join(&["a", "b c"]), "a 'b c'");
}

#[test]
fn base64_decodes_with_and_without_padding() {
    assert_eq!(base64_decode("aGk=").unwrap(), b"hi");
    assert_eq!(base64_decode("aGk").unwrap(), b"hi");
    assert_eq!(base64_decode("+/\n8=").unwrap(), [0xfb, 0xff]);
    assert_eq!(base64_decode("0123").unwrap(), [0xd3, 0x5d, 0xb7]);
    assert!(base64_decode("a*").is_none());
}

#[test]
fn a_foreign_or_broken_kubeconfig_is_not_ours() {
    assert!(is_ours(r#"{"extensions":[{"name":"workstation-kube"}]}"#));
    assert!(!is_ours(r#"{"extensions":[{"name":"other"}]}"#));
    assert!(!is_ours(r#"{"extensions":7}"#));
    assert!(!is_ours("clusters: []"));
}

#[test]
fn tilde_expands_to_the_home() {
    assert_eq!(expand_user("~/.kube/config", "/h"), "/h/.kube/config");
    assert_eq!(expand_user("~", "/h"), "/h");
    assert_eq!(expand_user("~other/x", "/h"), "~other/x");
    assert_eq!(expand_user("/a", "/h"), "/a");
}

#[test]
fn the_command_line_is_parsed_like_the_python_one() {
    let o = parse(&args(&[
        "install",
        "--device",
        "m",
        "--clusters=a,b",
        "--ssh",
        "u@h",
        "--ssh-port",
        "2222",
        "--user",
        "me",
        "--secret-dir",
        "/s",
        "--rotate",
        "--admin-kubeconfig",
        "k",
        "--admin-context",
        "c",
    ]))
    .unwrap();
    assert_eq!(o.clusters, ["a", "b"]);
    assert_eq!(
        (
            o.ssh_port,
            o.ssh.as_deref(),
            o.user.as_deref(),
            o.secret_dir.as_deref()
        ),
        (2222, Some("u@h"), Some("me"), Some("/s"))
    );
    assert!(o.rotate && o.install);
    assert_eq!(
        (o.admin_kubeconfig.as_str(), o.admin_context.as_str()),
        ("k", "c")
    );
    let o = parse(&args(&["check", "--device", "m"])).unwrap();
    assert!(!o.install && !o.rotate);
    assert_eq!(o.clusters, DEFAULT_CLUSTERS);
    assert_eq!(
        (o.ssh_port, o.admin_context.as_str()),
        (22, "{cluster}-admin")
    );
    for bad in [
        args(&[]),
        args(&["frob"]),
        args(&["check", "--device", "m", "--rotate"]),
        args(&["check", "--device", "m", "--admin-context", "x"]),
        args(&["install"]),
        args(&["install", "--device"]),
        args(&["install", "--device", "m", "--ssh-port", "x"]),
        args(&["install", "--device", "m", "--bogus=1"]),
    ] {
        assert!(matches!(parse(&bad), Err(Err(_))), "{bad:?}");
    }
    assert!(matches!(parse(&args(&["--help"])), Err(Ok(_))));
    assert!(matches!(parse(&args(&["check", "-h"])), Err(Ok(_))));
}

#[test]
fn usage_errors_exit_2_and_help_exits_0() {
    let d = Device::new();
    let c = ctx(d.env.clone());
    let bad = run_ctx(&c, &["install"]);
    assert_eq!(bad.code, 2);
    assert!(bad.err.contains("--device is required"));
    let help = run_ctx(&c, &["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.out.starts_with("usage: workstation-kube"));
    assert_eq!(run(vec![OsString::from("--help")].into_iter()), 0);
}

#[test]
fn ssh_needs_a_user() {
    let d = Device::new();
    let result = run_ctx(
        &ctx(d.env.clone()),
        &["check", "--device", "m", "--ssh", "u@h"],
    );
    assert_eq!(result.code, 1);
    assert!(result.err.contains("--user is required with --ssh"));
    let mut c = ctx(d.env.clone());
    c.user = None;
    assert!(run_ctx(&c, &["check", "--device", "m"])
        .err
        .contains("--user is required"));
}

#[test]
fn a_remote_owner_without_a_home_is_refused() {
    let d = Device::new();
    let mut env = d.env.clone();
    env.retain(|(k, _)| k != "FAKE_HOME");
    env.push(("FAKE_HOME".into(), String::new()));
    let result = run_ctx(
        &ctx(env),
        &["check", "--device", "m", "--ssh", "u@h", "--user", "me"],
    );
    assert!(
        result.err.contains("no home directory for me on u@h"),
        "{}",
        result.err
    );
}

#[test]
fn a_failing_remote_command_names_the_target_and_the_step() {
    let d = Device::new();
    let mut env = d.env.clone();
    env.retain(|(k, _)| k != "PATH");
    env.push(("PATH".into(), String::new()));
    let result = run_ctx(&ctx(env), &["check", "--device", "m"]);
    assert_eq!(result.code, 1);
    assert!(
        result.err.contains("on this machine: uname -s failed"),
        "{}",
        result.err
    );
}

#[test]
fn a_darwin_target_uses_the_darwin_layout() {
    let d = Device::new();
    let bin = d.tmp.join("bin");
    fs::write(bin.join("uname"), "#!/bin/sh\necho Darwin\n").unwrap();
    fs::set_permissions(bin.join("uname"), fs::Permissions::from_mode(0o755)).unwrap();
    let c = ctx(d.env.clone());
    let target = Target::new(&c, None, 22, Some(&me()), None).unwrap();
    assert_eq!(target.layout, darwin());
    assert_eq!(linux().secret_dir, "/etc/workstation-kube");
}

#[test]
fn a_target_without_the_nix_wrapper_is_plain_linux() {
    let d = Device::new();
    let bin = d.tmp.join("bin");
    fs::write(bin.join("uname"), "#!/bin/sh\necho Linux\n").unwrap();
    fs::set_permissions(bin.join("uname"), fs::Permissions::from_mode(0o755)).unwrap();
    let mut c = ctx(d.env.clone());
    c.nix_sudo = d.tmp.join("no-wrapper").display().to_string();
    let target = Target::new(&c, None, 22, Some(&me()), None).unwrap();
    assert_eq!(target.layout, linux());
    c.nix_sudo = bin.join("sudo").display().to_string();
    let target = Target::new(&c, None, 22, Some(&me()), None).unwrap();
    assert_eq!(target.layout, nixos());
}

#[test]
fn an_admin_kubeconfig_only_root_can_read_is_copied_with_sudo() {
    let d = Device::new();
    let mut c = ctx(d.env.clone());
    c.can_read = |_| false;
    let path = d.tmp.join("admin-config").display().to_string();
    let copy;
    {
        let admin = Admin::new(&c, &path, "{cluster}").unwrap();
        copy = admin.kubeconfig.clone();
        assert_ne!(copy, path);
        assert_eq!(Device::mode(Path::new(&copy)), 0o600);
        assert_eq!(fs::read_to_string(&copy).unwrap(), "{}");
    }
    assert!(!Path::new(&copy).exists());
    let missing = d.tmp.join("nope").display().to_string();
    let Err(Failure(message)) = Admin::new(&c, &missing, "{cluster}") else {
        panic!("a missing kubeconfig must fail");
    };
    assert!(message.starts_with("cannot read admin kubeconfig"));
}

#[test]
fn an_admin_context_that_fails_names_the_cluster_and_the_command() {
    let d = Device::new();
    let c = ctx(d.env.clone());
    let admin = Admin::new(
        &c,
        &d.tmp.join("admin-config").display().to_string(),
        "{cluster}",
    )
    .unwrap();
    let Err(Failure(message)) = admin.kubectl("personal", &["frob", "a", "b", "c", "d"], true)
    else {
        panic!("an unhandled command must fail");
    };
    assert!(message.starts_with("personal: kubectl frob a b failed: fake kubectl: unhandled"));
}

#[test]
fn a_token_that_never_appears_times_out() {
    let d = Device::new();
    let c = ctx(d.env.clone());
    let admin = Admin::new(
        &c,
        &d.tmp.join("admin-config").display().to_string(),
        "{cluster}",
    )
    .unwrap();
    let Err(Failure(message)) = admin.token("personal", "macbook", "ro") else {
        panic!("no secret means no token");
    };
    assert_eq!(
        message,
        "personal: Secret workstation-access/macbook-ro-token has no token after 0s"
    );
}

#[test]
fn a_secret_with_an_unreadable_token_is_polled_until_it_times_out() {
    let d = Device::new();
    fs::create_dir_all(d.state.join("secrets/personal")).unwrap();
    // A token that is not valid UTF-8 once decoded: the fake encodes whatever bytes are in the file.
    fs::write(
        d.state.join("secrets/personal/macbook-ro-token"),
        [0xffu8, 0xfe],
    )
    .unwrap();
    let c = ctx(d.env.clone());
    let admin = Admin::new(
        &c,
        &d.tmp.join("admin-config").display().to_string(),
        "{cluster}",
    )
    .unwrap();
    assert!(admin.token("personal", "macbook", "ro").is_err());
}

#[test]
fn cluster_entries_must_be_complete_json() {
    let d = Device::new();
    let bin = d.tmp.join("bin");
    let write = |name: &str, body: &str| {
        fs::write(bin.join(name), body).unwrap();
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    };
    let c = ctx(d.env.clone());
    let admin = Admin::new(
        &c,
        &d.tmp.join("admin-config").display().to_string(),
        "{cluster}",
    )
    .unwrap();
    // A script written a moment ago can still be "text file busy" when another
    // test's fork holds its descriptor; the call is retried until it ran.
    let reason = |script: &str| {
        write("kubectl", script);
        for _ in 0..50 {
            let message = admin.cluster_entry("personal").unwrap_err().0;
            if !message.contains(" failed: ") {
                return message;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("kubectl never started");
    };
    assert!(reason("#!/bin/sh\necho 'not json'\n").contains("is not JSON"));
    assert!(reason("#!/bin/sh\necho '{\"clusters\":[]}'\n").contains("no server"));
}

#[test]
fn a_rejected_secret_manifest_fails_the_install() {
    let d = Device::new();
    let bin = d.tmp.join("bin");
    fs::write(bin.join("kubectl"), "#!/bin/sh\ncase \"$*\" in *' get secret'*) exit 1;; *create*) echo denied >&2; exit 1;; esac\n").unwrap();
    let admin = d.tmp.join("admin-config").display().to_string();
    let result = d.run(
        &[
            "install",
            "--admin-kubeconfig",
            &admin,
            "--admin-context",
            "{cluster}",
        ],
        true,
    );
    assert_eq!(result.code, 1);
    assert!(result.err.contains("kubectl create -f"), "{}", result.err);
}

#[test]
fn a_missing_program_is_a_failed_command_not_a_crash() {
    let c = ctx(vec![
        ("PATH".into(), String::new()),
        ("HOME".into(), "/".into()),
    ]);
    let out = exec(&c, &args(&["no-such-program"]), None, true);
    assert!(!out.ok);
    assert!(out.stderr.starts_with("no-such-program: "));
    assert!(out.stdout.is_empty());
}

#[test]
fn a_ca_that_is_not_valid_base64_does_not_match() {
    let d = Device::new();
    let bin = d.tmp.join("bin");
    fs::write(bin.join("kubectl"), "#!/bin/sh\necho '{\"clusters\":[{\"cluster\":{\"certificate-authority-data\":\"!!\"}}]}'\n").unwrap();
    let c = ctx(d.env.clone());
    let target = Target::new(&c, None, 22, Some(&me()), None).unwrap();
    assert!(!ca_matches(&target, "personal"));
}

#[test]
fn a_ca_that_cannot_be_read_does_not_match() {
    let d = Device::new();
    let bin = d.tmp.join("bin");
    fs::write(bin.join("kubectl"), "#!/bin/sh\nexit 1\n").unwrap();
    let c = ctx(d.env.clone());
    let target = Target::new(&c, None, 22, Some(&me()), None).unwrap();
    assert!(!ca_matches(&target, "personal"));
}

#[test]
fn a_temporary_file_that_cannot_be_written_is_a_failure() {
    let Failure(message) = Failure::from(std::io::Error::other("disk full"));
    assert_eq!(message, "cannot write a private temporary file: disk full");
}
