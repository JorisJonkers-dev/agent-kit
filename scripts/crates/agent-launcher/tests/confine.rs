//! `agent-launcher confine` under the real kernel (Landlock on Linux, a
//! Seatbelt profile through `sandbox-exec` on macOS): the launcher
//! binary is started as its own process, puts the confinement on itself and
//! becomes a probe. File probes are `/bin/sh` with `cat` and `ls`; network
//! probes are this test binary itself, re-run under the confinement as the
//! `probe` helper below (so the runner needs no `nc`, `python` or `bash`).
//! Nothing read is ever printed: a probe says only whether it got the file.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const LAUNCHER: &str = env!("CARGO_BIN_EXE_agent-launcher");
const PROBE: &str = "AGENT_LAUNCHER_CONFINE_PROBE";

/// What the operating system says to a refused file access, and the errno of a
/// refused network call: Landlock answers EACCES, Seatbelt EPERM.
#[cfg(target_os = "linux")]
const REFUSED_FILE: &str = "Permission denied";
#[cfg(target_os = "macos")]
const REFUSED_FILE: &str = "Operation not permitted";
#[cfg(target_os = "linux")]
const REFUSED_EXEC: i32 = 125;
#[cfg(target_os = "macos")]
const REFUSED_EXEC: i32 = 71;
#[cfg(target_os = "linux")]
const REFUSED_NET: i32 = 13;
#[cfg(target_os = "macos")]
const REFUSED_NET: i32 = 1;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agent-launcher-confine-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::canonicalize(dir).unwrap()
}

fn confine(options: &[&str], command: &[&str]) -> Output {
    Command::new(LAUNCHER)
        .arg("confine")
        .args(options)
        .arg("--")
        .args(command)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// A shell line that reports, for each path, `ok <path>` or `denied <path>
/// (<error>)`, never the content.
fn reads(paths: &[&Path]) -> String {
    paths
        .iter()
        .map(|p| format!("if cat '{0}' >/dev/null 2>&1; then echo 'ok {0}'; else echo \"denied {0} ($(cat '{0}' 2>&1 >/dev/null | tail -1))\"; fi; ", p.display()))
        .collect()
}

#[test]
fn a_confined_command_reads_its_grant_and_nothing_outside_it() {
    let root = scratch("reads");
    let packet = root.join("packet");
    let elsewhere = root.join("elsewhere");
    fs::create_dir_all(&packet).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    fs::write(packet.join("in.txt"), "inside").unwrap();
    fs::write(elsewhere.join("out.txt"), "outside").unwrap();
    let ssh = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".ssh"));
    let ssh_file = ssh
        .as_ref()
        .and_then(|d| fs::read_dir(d).ok())
        .and_then(|mut entries| entries.find_map(|e| e.ok().map(|e| e.path()).filter(|p| p.is_file())));
    let mut probes = vec![packet.join("in.txt"), elsewhere.join("out.txt")];
    probes.extend(ssh_file.clone());
    let refs: Vec<&Path> = probes.iter().map(PathBuf::as_path).collect();
    let script = format!(
        "{}if ls '{}' >/dev/null 2>&1; then echo 'listed elsewhere'; else echo 'not listed elsewhere'; fi",
        reads(&refs),
        elsewhere.display()
    );
    let out = confine(&["--read", packet.to_str().unwrap()], &["/bin/sh", "-c", &script]);
    let said = text(&out);
    assert!(out.status.success(), "{said}");
    assert!(said.contains(&format!("ok {}", packet.join("in.txt").display())), "{said}");
    let denied = format!("denied {} (", elsewhere.join("out.txt").display());
    assert!(said.contains(&denied) && said.contains(REFUSED_FILE), "{said}");
    assert!(said.contains("not listed elsewhere"), "{said}");
    if let Some(file) = ssh_file {
        assert!(said.contains(&format!("denied {} (", file.display())), "a file under ~/.ssh was readable");
    }
    // Unconfined, the same probe reads both: the denial is the confinement's.
    let free = Command::new("/bin/sh").arg("-c").arg(reads(&[&elsewhere.join("out.txt")])).output().unwrap();
    assert!(text(&free).starts_with("ok "), "{}", text(&free));
    fs::remove_dir_all(&root).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn another_process_environment_and_proc_are_unreadable() {
    let mut other = Command::new("sleep").arg("30").spawn().unwrap();
    let environ = format!("/proc/{}/environ", other.id());
    assert!(fs::read(&environ).is_ok(), "the test itself can read {environ}");
    let script = format!("{}{}", reads(&[Path::new(&environ)]), reads(&[Path::new("/proc/self/environ")]));
    let out = confine(&[], &["/bin/sh", "-c", &script]);
    let said = text(&out);
    let _ = other.kill();
    let _ = other.wait();
    assert!(said.contains(&format!("denied {environ} (")), "{said}");
    assert!(said.contains("denied /proc/self/environ ("), "{said}");
}

#[test]
fn a_confined_command_writes_only_its_writable_directory() {
    let root = scratch("writes");
    let writable = root.join("w");
    fs::create_dir_all(&writable).unwrap();
    let script = format!(
        "echo x > '{0}/made' && cat '{0}/made' >/dev/null && echo wrote; echo x > '{1}/no' 2>/dev/null || echo refused",
        writable.display(),
        root.display()
    );
    let out = confine(&["--write", writable.to_str().unwrap()], &["/bin/sh", "-c", &script]);
    let said = text(&out);
    assert!(said.contains("wrote") && said.contains("refused"), "{said}");
    assert!(!root.join("no").exists());
    fs::remove_dir_all(&root).unwrap();
}

/// The network probe: run under the confinement as this test binary, it
/// connects to each port in the probe variable (`c<port>`) or binds one
/// (`b`), and prints what happened.
#[test]
fn probe() {
    let Ok(spec) = std::env::var(PROBE) else { return };
    for item in spec.split(',') {
        let udp = |port: &str| {
            let socket = UdpSocket::bind("0.0.0.0:0")?;
            socket.connect(("127.0.0.1", port.parse::<u16>().unwrap()))
        };
        let result = match (item.strip_prefix('c'), item.strip_prefix('u')) {
            (Some(port), _) => TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).map(drop),
            (None, Some(port)) => udp(port),
            (None, None) => TcpListener::bind("127.0.0.1:0").map(drop),
        };
        match result {
            Ok(()) => println!("{item}: ok"),
            Err(e) => println!("{item}: {}", e.raw_os_error().unwrap_or(-1)),
        }
    }
}

fn net_probe(options: &[&str], spec: &str) -> String {
    let me = std::env::current_exe().unwrap();
    let deps = me.parent().unwrap().to_str().unwrap().to_string();
    let mut all = vec!["--run", deps.as_str(), "--env", PROBE, "--env", "LLVM_PROFILE_FILE"];
    all.extend_from_slice(options);
    let out = Command::new(LAUNCHER)
        .arg("confine")
        .args(&all)
        .arg("--")
        .arg(&me)
        .args(["--exact", "probe", "--nocapture", "--test-threads=1"])
        .env(PROBE, spec)
        .output()
        .unwrap();
    text(&out)
}

#[test]
fn tcp_reaches_only_the_listed_ports_and_nothing_binds() {
    let allowed = TcpListener::bind("127.0.0.1:0").unwrap();
    let other = TcpListener::bind("127.0.0.1:0").unwrap();
    let (a, o) = (allowed.local_addr().unwrap().port(), other.local_addr().unwrap().port());
    let spec = format!("c{a},c{o},b");
    let said = net_probe(&["--net-connect", &a.to_string()], &spec);
    assert!(said.contains(&format!("c{a}: ok")), "{said}");
    assert!(said.contains(&format!("c{o}: {}", REFUSED_NET)), "{said}");
    assert!(said.contains(&format!("b: {}", REFUSED_NET)), "{said}");
    // Without --net-connect the network is left alone.
    let free = net_probe(&[], &spec);
    assert!(free.contains(&format!("c{o}: ok")) && free.contains("b: ok"), "{free}");
}

/// Whether this kernel's Landlock limits UDP (ABI 10 or newer).
#[cfg(target_os = "linux")]
fn landlock_limits_udp() -> bool {
    // SAFETY: a null attribute with size 0 and the VERSION flag only asks the ABI.
    let abi = unsafe { libc::syscall(libc::SYS_landlock_create_ruleset, std::ptr::null::<u8>(), 0usize, 1u32) };
    abi >= 10
}

#[cfg(target_os = "linux")]
#[test]
fn udp_reaches_only_dns_where_the_kernel_can_limit_it() {
    let spec = "u53,u5353";
    let said = net_probe(&["--net-connect", "443"], spec);
    assert!(said.contains("u53: ok"), "{said}");
    let other = if landlock_limits_udp() { REFUSED_NET.to_string() } else { "ok".to_string() };
    assert!(said.contains(&format!("u5353: {other}")), "{said}");
    // Without --net-connect UDP is left alone.
    assert!(net_probe(&[], spec).contains("u5353: ok"));
}

#[test]
fn the_command_becomes_the_process_and_its_status_is_the_exit_code() {
    let out = confine(&[], &["/bin/sh", "-c", "exit 7"]);
    assert_eq!(out.status.code(), Some(7), "{}", text(&out));
}

#[test]
fn a_confinement_that_cannot_be_applied_starts_nothing() {
    let root = scratch("refused");
    let marker = root.join("ran");
    let script = format!(": > '{}'", marker.display());
    let out = confine(&["--read", "/nonexistent/agent-launcher-grant", "--write", root.to_str().unwrap()], &["/bin/sh", "-c", &script]);
    assert_eq!(out.status.code(), Some(125), "{}", text(&out));
    assert!(text(&out).contains("/nonexistent/agent-launcher-grant") && text(&out).contains("nothing started"));
    assert!(!marker.exists());
    let check = Command::new(LAUNCHER).args(["confine", "--check", "--net-connect", "443"]).output().unwrap();
    assert!(check.status.success() && text(&check).starts_with("confine: ok,"), "{}", text(&check));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn run_from_grants_what_a_packaged_script_needs_and_nothing_else_does() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("run-from");
    let package = root.join("lib/node_modules/tool");
    fs::create_dir_all(package.join("bin")).unwrap();
    fs::write(package.join("data.txt"), "packaged").unwrap();
    let script = package.join("bin/tool");
    fs::write(&script, format!("#!/bin/sh\ncat '{}'\n", package.join("data.txt").display())).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let granted = confine(&["--run-from", script.to_str().unwrap()], &[script.to_str().unwrap()]);
    assert_eq!((granted.status.code(), String::from_utf8_lossy(&granted.stdout).as_ref()), (Some(0), "packaged"), "{}", text(&granted));
    // Without the grant the script cannot even be executed, and the launcher says so.
    let refused = confine(&[], &[script.to_str().unwrap()]);
    // sandbox-exec, which is what runs the command on macOS, ends with its own EX_OSERR (71) when the exec is denied.
    assert_eq!(refused.status.code(), Some(REFUSED_EXEC), "{}", text(&refused));
    assert!(text(&refused).contains(REFUSED_FILE), "{}", text(&refused));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_command_inherits_no_secret_variables_and_no_open_descriptors() {
    let out = Command::new(LAUNCHER)
        .args(["confine", "--env", "KEPT_BY_NAME", "--", "/bin/sh", "-c", "echo \"secret=${LEAKY_SECRET-unset} named=${KEPT_BY_NAME-unset} path=${PATH:+set} locale=${LC_ALL-unset}\""])
        .env("LEAKY_SECRET", "hunter2")
        .env("KEPT_BY_NAME", "yes")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = text(&out);
    assert!(said.contains("secret=unset named=yes path=set locale=C"), "{said}");
    assert!(!said.contains("hunter2"), "{said}");
    // A descriptor the caller left open (7, a file here) is closed before the command starts.
    let script = format!("exec 7</etc/hosts; exec {LAUNCHER} confine -- /bin/sh -c 'if read -r line <&7 2>/dev/null; then echo fd7=open; else echo fd7=closed; fi'");
    let out = Command::new("/bin/sh").args(["-c", &script]).stdin(Stdio::null()).output().unwrap();
    assert!(text(&out).contains("fd7=closed"), "{}", text(&out));
}

/// macOS has no UDP rule to carve DNS out with: name resolution goes through
/// the mDNSResponder socket, so a direct UDP send to any port is refused.
#[cfg(target_os = "macos")]
#[test]
fn udp_is_refused_on_macos_because_dns_goes_through_the_resolver_socket() {
    let said = net_probe(&["--net-connect", "443"], "u53,u5353");
    assert!(said.contains(&format!("u53: {REFUSED_NET}")), "{said}");
    assert!(said.contains(&format!("u5353: {REFUSED_NET}")), "{said}");
    assert!(net_probe(&[], "u5353").contains("u5353: ok"));
}

/// Neither the home directory nor a sibling checkout can be listed or read,
/// and the confined process cannot reach a name it could resolve outside.
#[test]
fn the_home_directory_and_a_sibling_checkout_are_out_of_reach() {
    let root = scratch("sibling");
    let checkout = root.join("checkout");
    let sibling = root.join("sibling-checkout");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(sibling.join(".git")).unwrap();
    fs::write(sibling.join("secret.txt"), "sibling").unwrap();
    let home = PathBuf::from(std::env::var_os("HOME").unwrap());
    let script = format!(
        "{}for d in '{}' '{}'; do if ls \"$d\" >/dev/null 2>&1; then echo \"listed $d\"; else echo \"unlisted $d\"; fi; done",
        reads(&[&sibling.join("secret.txt")]),
        home.display(),
        sibling.display()
    );
    let out = confine(&["--read", checkout.to_str().unwrap()], &["/bin/sh", "-c", &script]);
    let said = text(&out);
    assert!(said.contains(&format!("denied {} (", sibling.join("secret.txt").display())), "{said}");
    assert!(said.contains(&format!("unlisted {}", home.display())), "{said}");
    assert!(said.contains(&format!("unlisted {}", sibling.display())), "{said}");
    fs::remove_dir_all(&root).unwrap();
}
