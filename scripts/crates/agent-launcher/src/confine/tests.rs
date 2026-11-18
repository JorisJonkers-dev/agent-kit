use super::*;
use crate::writelimit::{Arm, Armed};
use std::io;
use std::os::unix::fs::PermissionsExt;

fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

/// A backend that refuses, one that arms nothing, and one whose arming fails.
enum Fake {
    Refuses,
    Fine,
    ArmFails,
    /// Starts the command through a wrapper that does not exist, as `sandbox-exec` would.
    Wrapped,
}

struct Armer(bool);

struct Wrapper;

impl Arm for Wrapper {
    fn arm(&self) -> io::Result<()> {
        Ok(())
    }

    fn prefix(&self) -> Vec<OsString> {
        args(&["/nonexistent/agent-launcher-wrapper", "-p", "profile"])
    }
}

impl Arm for Armer {
    fn arm(&self) -> io::Result<()> {
        if self.0 {
            Err(io::Error::from_raw_os_error(libc::EPERM))
        } else {
            Ok(())
        }
    }
}

impl WriteLimit for Fake {
    fn prepare(&self, _policy: &Policy) -> Result<Armed, String> {
        match self {
            Fake::Refuses => Err("no backend here".into()),
            Fake::Fine => Ok(Box::new(Armer(false))),
            Fake::ArmFails => Ok(Box::new(Armer(true))),
            Fake::Wrapped => Ok(Box::new(Wrapper)),
        }
    }
}

#[test]
fn every_option_is_read_and_the_command_follows_the_dashes() {
    let spec = parse(&args(&[
        "--read", "/p", "--run", "/r", "--write", "/w", "--net-connect", "443", "--net-connect", "8443", "--", "pi", "--read", "x",
    ]))
    .unwrap();
    assert_eq!(
        spec,
        Spec {
            check: false,
            read: vec!["/p".into()],
            run: vec!["/r".into()],
            run_from: vec![],
            write: vec!["/w".into()],
            ports: Some(vec![443, 8443]),
            env: vec![],
            command: args(&["pi", "--read", "x"]),
        }
    );
    assert_eq!(parse(&args(&["--check"])).unwrap(), Spec { check: true, ..Spec::default() });
}

#[test]
fn a_bad_command_line_is_refused_with_the_reason() {
    for (line, reason) in [
        (&["--read"][..], "--read needs a value"),
        (&["--bogus", "--", "x"][..], "unknown confine option \"--bogus\""),
        (&["--net-connect", "0", "--", "x"][..], "not \"0\""),
        (&["--net-connect", "https", "--", "x"][..], "not \"https\""),
        (&["--env", "A=b", "--", "x"][..], "--env takes a variable name"),
        (&["--env", "", "--", "x"][..], "--env takes a variable name"),
        (&["--read", "/p"][..], "needs a command after --"),
        (&["--", ][..], "needs a command after --"),
        (&["--check", "--", "x"][..], "or --check and no command"),
    ] {
        let message = parse(&args(line)).unwrap_err();
        assert!(message.contains(reason), "{line:?}: {message}");
    }
}

#[test]
fn the_policy_adds_the_system_runtime_and_devices_that_exist() {
    let spec = parse(&args(&["--read", "/p", "--run", "/r", "--write", "/w", "--", "x"])).unwrap();
    let policy = policy(&spec, &Env::default()).unwrap();
    let reads = policy.reads.as_ref().unwrap();
    assert_eq!(reads.read.last(), Some(&PathBuf::from("/p")));
    assert_eq!(reads.run.last(), Some(&PathBuf::from("/r")));
    assert!(reads.read.iter().chain(&reads.run).filter(|p| !p.starts_with("/p") && !p.starts_with("/r")).all(|p| p.exists()));
    assert!(reads.read.contains(&PathBuf::from("/dev/urandom")));
    // The macOS runtime is held where it exists, and never listed where it does not.
    assert_eq!(reads.run.contains(&PathBuf::from("/System")), Path::new("/System").exists());
    assert!(!reads.read.iter().chain(&reads.run).any(|p| p.starts_with("/proc") || p.starts_with("/sys")));
    assert_eq!((policy.dirs, policy.files, policy.connect_ports), (vec![PathBuf::from("/w")], vec![PathBuf::from("/dev/null")], None));
}

#[test]
fn bad_usage_exits_2_and_a_refused_confinement_starts_nothing() {
    assert_eq!(confine(&args(&["--nope"]), &Fake::Fine), 2);
    assert_eq!(confine(&args(&["--", "/bin/sh", "-c", "exit 0"]), &Fake::Refuses), REFUSED);
}

#[test]
fn the_check_builds_the_confinement_and_starts_nothing() {
    assert_eq!(confine(&args(&["--check"]), &Fake::Fine), 0);
    assert_eq!(confine(&args(&["--check", "--net-connect", "443"]), &Fake::Fine), 0);
    assert_eq!(confine(&args(&["--check"]), &Fake::Refuses), REFUSED);
}

#[test]
fn the_summary_names_the_grants_and_the_network() {
    let spec = parse(&args(&["--net-connect", "443", "--net-connect", "80", "--check"])).unwrap();
    assert!(summary(&policy(&spec, &Env::default()).unwrap()).ends_with("TCP connect to port(s) 443, 80"));
    assert!(summary(&Policy::default()).starts_with("0 readable path(s), 0 writable, network not limited"));
}

#[test]
fn a_command_that_cannot_be_started_or_armed_is_refused() {
    // exec only returns on failure, so this process is never replaced here.
    // The hook closes descriptors from the one given up, so a far one keeps this process's own.
    assert_eq!(confine_from(&args(&["--", "/nonexistent/agent-launcher-cmd"]), &Fake::Fine, 1 << 20), REFUSED);
    assert_eq!(confine(&args(&["--", "/bin/sh", "-c", "exit 0"]), &Fake::ArmFails), REFUSED);
    // A backend that starts the command through a wrapper (sandbox-exec) is refused the same way when that cannot start.
    assert_eq!(confine_from(&args(&["--", "/bin/sh", "-c", "exit 0"]), &Fake::Wrapped, 1 << 20), REFUSED);
}

#[test]
fn the_command_a_wrapper_gets_is_found_on_path_with_its_links_resolved() {
    let root = scratch("resolved");
    let real = file(&root.join("real/tool"), "#!/bin/sh\n", 0o755);
    fs::create_dir_all(root.join("bin")).unwrap();
    std::os::unix::fs::symlink(&real, root.join("bin/tool")).unwrap();
    let env = env_with(&[&root.join("bin")], None);
    assert_eq!(resolved(OsStr::new("tool"), &env), real.into_os_string());
    // A path is resolved too, and a name that is nowhere is left for the program to report.
    assert_eq!(resolved(root.join("bin/tool").as_os_str(), &env), root.join("real/tool").into_os_string());
    assert_eq!(resolved(OsStr::new("no-such-tool"), &env), OsString::from("no-such-tool"));
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agent-launcher-cf-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::canonicalize(dir).unwrap()
}

fn file(path: &Path, text: &str, mode: u32) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    path.to_path_buf()
}

fn env_with(path: &[&Path], home: Option<&Path>) -> Env {
    Env {
        path: Some(std::env::join_paths(path).unwrap()),
        home: home.map(Path::to_path_buf),
    }
}

#[test]
fn a_node_package_grants_its_package_and_its_interpreters_install() {
    let root = scratch("pkg");
    let node = file(&root.join("nodejs/bin/node"), "\x7fELF", 0o755);
    let cli = file(&root.join("lib/node_modules/@scope/tool/dist/bundle/cli.js"), "#!/usr/bin/env -S node --flag\n", 0o755);
    fs::create_dir_all(root.join("bin")).unwrap();
    std::os::unix::fs::symlink(&cli, root.join("bin/tool")).unwrap();
    let env = env_with(&[&root.join("bin"), &root.join("nodejs/bin")], None);
    let grants = program_grants(OsStr::new("tool"), &env).unwrap();
    assert_eq!(grants, vec![root.join("lib/node_modules/@scope/tool"), root.join("nodejs")]);
    let plain = file(&root.join("lib/node_modules/plain/cli.js"), "#!/usr/bin/env node\n", 0o755);
    assert_eq!(program_grants(plain.as_os_str(), &env).unwrap(), vec![root.join("lib/node_modules/plain"), root.join("nodejs")]);
    let _ = node;
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_script_with_an_absolute_interpreter_or_none_found_and_a_binary() {
    let root = scratch("interp");
    let interp = file(&root.join("tools/interp"), "", 0o755);
    let script = file(&root.join("app/run"), &format!("#!{} -x\n", interp.display()), 0o755);
    let env = env_with(&[], None);
    assert_eq!(program_grants(script.as_os_str(), &env).unwrap(), vec![root.join("app"), root.join("tools")]);
    for (name, text) in [("missing", "#!/usr/bin/env no-such-interpreter\n"), ("options", "#!/usr/bin/env -S\n"), ("empty", ""), ("bare", "#!\n"), ("binary", "\x7fELF")] {
        let program = file(&root.join(format!("{name}/p")), text, 0o755);
        assert_eq!(program_grants(program.as_os_str(), &env).unwrap(), vec![root.join(name)], "{name}");
    }
    // Executable but unreadable: its interpreter cannot be read, so none is granted.
    let hidden = file(&root.join("hidden/p"), "#!/bin/sh\n", 0o111);
    assert_eq!(program_grants(hidden.as_os_str(), &env).unwrap(), vec![root.join("hidden")]);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_grant_that_would_hold_the_home_directory_is_narrowed_to_the_file() {
    let root = scratch("home");
    let interp = file(&root.join("bin/interp"), "", 0o755);
    let script = file(&root.join("run"), &format!("#!{}\n", interp.display()), 0o755);
    let env = env_with(&[], Some(&root.join("sub")));
    assert_eq!(program_grants(script.as_os_str(), &env).unwrap(), vec![script.clone(), interp.clone()]);
    assert!(holds_home(Path::new("/"), &Env::default()));
    assert!(!holds_home(&root, &Env::default()));
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn a_program_that_is_not_there_or_not_executable_is_refused() {
    let root = scratch("missing");
    let data = file(&root.join("data"), "", 0o644);
    let env = env_with(&[&root], None);
    for program in [OsStr::new("no-such-program"), data.as_os_str(), OsStr::new("data")] {
        let message = program_grants(program, &env).unwrap_err();
        assert!(message.contains("no executable file of that name on PATH"), "{message}");
    }
    let spec = Spec { run_from: vec!["no-such-program".into()], check: true, ..Spec::default() };
    assert!(policy(&spec, &env).is_err());
    let none = Env::default();
    assert!(which(OsStr::new("sh"), &none).is_none());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_policy_runs_from_each_program_and_the_option_is_read() {
    let root = scratch("policy");
    let tool = file(&root.join("t/tool"), "", 0o755);
    let spec = parse(&args(&["--run-from", tool.to_str().unwrap(), "--check"])).unwrap();
    assert_eq!(spec.run_from, vec![tool.clone().into_os_string()]);
    let policy = policy(&spec, &Env::default()).unwrap();
    assert_eq!(policy.reads.unwrap().run.last(), Some(&root.join("t")));
    assert!(Env::from_process().path.is_some());
    assert_eq!(confine(&args(&["--check", "--run-from", "/nonexistent/agent-launcher-tool"]), &Fake::Fine), REFUSED);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn the_grant_list_names_every_path_by_kind() {
    let spec = parse(&args(&["--read", "/p", "--run", "/r", "--write", "/w", "--check"])).unwrap();
    let listed = grants(&policy(&spec, &Env::default()).unwrap());
    assert!(listed.contains("read /p\n") && listed.contains("run /r\n") && listed.contains("write /w\n") && listed.contains("write /dev/null\n"), "{listed}");
    assert_eq!(grants(&Policy::default()), "");
}

fn vars(items: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    items.iter().map(|(k, v)| (OsString::from(k), OsString::from(v))).collect()
}

#[test]
fn the_command_keeps_the_plain_variables_the_locale_and_the_ones_it_is_named() {
    let spec = parse(&args(&["--env", "PI_CODING_AGENT_DIR", "--env", "UNSET", "--", "x"])).unwrap();
    assert_eq!(spec.env, args(&["PI_CODING_AGENT_DIR", "UNSET"]));
    let held = kept_env(
        vars(&[("PATH", "/bin"), ("LC_ALL", "C"), ("LITELLM_KEY", "k"), ("GH_TOKEN", "t"), ("PI_CODING_AGENT_DIR", "/a"), ("HTTPS_PROXY", "p")]).into_iter(),
        &spec.env,
    );
    assert_eq!(held, vars(&[("PATH", "/bin"), ("LC_ALL", "C"), ("PI_CODING_AGENT_DIR", "/a")]));
    assert!(kept_env(vars(&[("SECRET", "x")]).into_iter(), &[]).is_empty());
}

#[test]
fn descriptors_from_the_given_one_up_are_closed_after_the_confinement_is_on() {
    let own = |fd: libc::c_int| unsafe { libc::fcntl(fd, libc::F_GETFD) } != -1;
    // SAFETY: duplicates a standard descriptor to one far above any the test runner holds.
    let high = unsafe { libc::fcntl(0, libc::F_DUPFD, 900) };
    assert!(high >= 900 && own(high));
    assert!(confine_hook(Box::new(Armer(false)), 900)().is_ok());
    assert!(!own(high));
    assert!(own(0) && own(1) && own(2));
    // An arming that fails closes nothing and says so.
    // SAFETY: as above.
    let kept = unsafe { libc::fcntl(0, libc::F_DUPFD, 900) };
    assert!(confine_hook(Box::new(Armer(true)), 900)().is_err());
    assert!(own(kept));
    // SAFETY: closes the descriptor made above.
    unsafe { libc::close(kept) };
    // A descriptor far above any fixed bound is closed too (macOS used to stop at 65535).
    let mut limit = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
    // SAFETY: reads and raises this process's soft limit for descriptors to make room for a high one.
    unsafe {
        libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit);
        limit.rlim_cur = limit.rlim_max.min(200_000);
        libc::setrlimit(libc::RLIMIT_NOFILE, &limit);
    }
    // SAFETY: as above, to a descriptor past 65536.
    let far = unsafe { libc::fcntl(0, libc::F_DUPFD, 70_000) };
    if far >= 70_000 {
        assert!(confine_hook(Box::new(Armer(false)), 900)().is_ok());
        assert!(!own(far));
    }
    #[cfg(target_os = "linux")]
    assert_eq!(close_range(5, 3).unwrap_err().raw_os_error(), Some(libc::EINVAL));
}
