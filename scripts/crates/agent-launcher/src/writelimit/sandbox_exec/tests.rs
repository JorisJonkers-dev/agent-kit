use super::*;
use std::os::unix::fs::symlink;

#[path = "../../../../../test-support/fake_exe.rs"]
mod fake_exe;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agent-launcher-sx-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::canonicalize(dir).unwrap()
}

fn policy(root: &Path) -> Policy {
    for d in ["w", "t", "hooks"] {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    fs::write(root.join("w/.git"), "gitdir: x\n").unwrap();
    fs::write(root.join("dev"), "").unwrap();
    Policy {
        dirs: vec![root.join("w"), root.join("t")],
        files: vec![root.join("dev")],
        guarded: vec![root.join("w/.git")],
        denied: vec![root.join("hooks"), root.join("config")],
    }
}

#[test]
fn the_profile_grants_paths_by_parameter_and_denies_after_granting() {
    let root = scratch("profile");
    let p = profile(&policy(&root)).unwrap();
    assert_eq!(
        p.text,
        "(version 1)\n(allow default)\n(deny file-write*)\n\
         (allow file-write*\n  (subpath (param \"D0\"))\n  (subpath (param \"D1\"))\n  (literal (param \"F0\"))\n)\n\
         (deny file-write*\n  (subpath (param \"X0\"))\n  (subpath (param \"X1\"))\n  (literal (param \"G0\"))\n  (literal (param \"R0\"))\n  (literal (param \"R1\"))\n)\n\
         (deny file-link)\n"
    );
    // The paths are not in the text, so none of them can change a rule.
    assert!(!p.text.contains(root.to_str().unwrap()));
    let named: Vec<_> = p.params.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(named, ["D0", "D1", "F0", "X0", "X1", "G0", "R0", "R1"]);
    assert_eq!(p.params[0].1, root.join("w"));
    // A path that does not exist yet (a config that is not there) lands beside its parent.
    assert_eq!(p.params[4].1, root.join("config"));
}

#[test]
fn a_rule_with_no_path_is_left_out_rather_than_matching_everything() {
    assert_eq!(rule("allow", ""), "");
    let root = scratch("empty");
    let p = profile(&Policy::default()).unwrap();
    assert_eq!(
        p.text,
        "(version 1)\n(allow default)\n(deny file-write*)\n(deny file-link)\n"
    );
    assert!(p.params.is_empty());
    drop(root);
}

#[test]
fn paths_are_made_canonical_so_a_link_cannot_move_a_rule() {
    let root = scratch("links");
    fs::create_dir_all(root.join("real")).unwrap();
    symlink(root.join("real"), root.join("alias")).unwrap();
    assert_eq!(canonical(&root.join("alias")).unwrap(), root.join("real"));
    // A missing path through a link resolves beside the link's target.
    assert_eq!(
        canonical(&root.join("alias/missing")).unwrap(),
        root.join("real/missing")
    );
    let p = profile(&Policy {
        dirs: vec![root.join("alias")],
        ..Policy::default()
    })
    .unwrap();
    assert_eq!(p.params[0].1, root.join("real"));
}

#[test]
fn a_path_that_cannot_be_resolved_is_refused() {
    let root = scratch("unresolvable");
    fs::write(root.join("file"), "").unwrap();
    // The parent is missing too.
    let message = canonical(&root.join("no/such/path")).unwrap_err();
    assert!(
        message.contains("cannot resolve") && message.contains("no/such/path"),
        "{message}"
    );
    // Nothing at all to resolve.
    assert!(canonical(Path::new(""))
        .unwrap_err()
        .contains("cannot resolve"));
    // A file used as a directory is some other error.
    assert!(canonical(&root.join("file/child"))
        .unwrap_err()
        .contains("cannot resolve"));
    let policy = Policy {
        dirs: vec![root.join("no/such")],
        ..Policy::default()
    };
    assert!(profile(&policy).is_err());
}

#[test]
fn the_prefix_is_the_binary_the_profile_and_one_parameter_per_path() {
    let root = scratch("prefix");
    let p = profile(&policy(&root)).unwrap();
    let prefix = command_prefix(Path::new("/x/sandbox-exec"), &p);
    assert_eq!(prefix[0], OsString::from("/x/sandbox-exec"));
    assert_eq!(prefix[1], OsString::from("-p"));
    assert_eq!(prefix[2], OsString::from(&p.text));
    assert_eq!(prefix[3], OsString::from("-D"));
    assert_eq!(
        prefix[4],
        OsString::from(format!("D0={}", root.join("w").display()))
    );
    assert_eq!(prefix.len(), 3 + 2 * p.params.len());
}

#[test]
fn without_sandbox_exec_the_backend_refuses() {
    let root = scratch("missing");
    let message = SandboxExec::at(root.join("nope"))
        .prepare(&policy(&root))
        .err()
        .unwrap();
    assert!(
        message.contains("sandbox-exec is not available"),
        "{message}"
    );
    // Present but not executable, and a directory, are as good as absent.
    fs::write(root.join("plain"), "").unwrap();
    assert!(SandboxExec::at(root.join("plain"))
        .prepare(&policy(&root))
        .is_err());
    assert!(SandboxExec::at(&root).prepare(&policy(&root)).is_err());
}

fn fake(root: &Path, name: &str, text: &str) -> SandboxExec {
    let held = fake_exe::lock();
    let path = root.join(name);
    fake_exe::write_executable(&held, &path, text);
    SandboxExec::at(path)
}

#[test]
fn a_profile_sandbox_exec_accepts_gives_a_wrapper_for_the_command() {
    let root = scratch("ok");
    let log = root.join("log");
    let backend = fake(
        &root,
        "sandbox-exec",
        &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display()),
    );
    let _held = fake_exe::lock();
    let armed = backend.prepare(&policy(&root)).unwrap();
    assert!(armed.arm().is_ok());
    let prefix = armed.prefix();
    assert_eq!(prefix[0], root.join("sandbox-exec").into_os_string());
    assert_eq!(prefix[1], OsString::from("-p"));
    // The probe ran the same profile, then a harmless program.
    let probe = fs::read_to_string(&log).unwrap();
    assert!(
        probe.contains("(deny file-link)") && probe.trim_end().ends_with("/usr/bin/true"),
        "{probe}"
    );
}

#[test]
fn a_profile_sandbox_exec_rejects_is_refused_with_its_message() {
    let root = scratch("rejected");
    let backend = fake(
        &root,
        "sandbox-exec",
        "#!/bin/sh\necho 'unknown operation file-link' >&2\nexit 65\n",
    );
    let _held = fake_exe::lock();
    let message = backend.prepare(&policy(&root)).err().unwrap();
    assert!(
        message.contains("cannot apply the write limit") && message.contains("unknown operation"),
        "{message}"
    );
}

#[test]
fn a_sandbox_exec_that_cannot_run_is_refused() {
    let root = scratch("cannot-run");
    let backend = fake(&root, "sandbox-exec", "#!/nonexistent/interpreter\n");
    let _held = fake_exe::lock();
    let message = backend.prepare(&policy(&root)).err().unwrap();
    assert!(message.contains("cannot run sandbox-exec"), "{message}");
}

#[test]
fn a_policy_with_an_unresolvable_path_is_refused_before_anything_runs() {
    let root = scratch("unresolved");
    let backend = fake(&root, "sandbox-exec", "#!/bin/sh\nexit 0\n");
    let _held = fake_exe::lock();
    let bad = Policy {
        dirs: vec![root.join("no/such")],
        ..Policy::default()
    };
    assert!(backend
        .prepare(&bad)
        .err()
        .unwrap()
        .contains("cannot resolve"));
}

#[test]
fn the_system_backend_is_the_one_macos_ships() {
    assert_eq!(
        SandboxExec::system().binary,
        PathBuf::from("/usr/bin/sandbox-exec")
    );
}
