use super::*;
use std::os::unix::fs::symlink;

use crate::writelimit::tests::fake_exe;

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
        ..Policy::default()
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
    assert_eq!(rule("allow", "file-write*", ""), "");
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
        canonical_or_planned(&root.join("alias/missing/deeper")).unwrap(),
        root.join("real/missing/deeper")
    );
    let p = profile(&Policy {
        dirs: vec![root.join("alias")],
        ..Policy::default()
    })
    .unwrap();
    assert_eq!(p.params[0].1, root.join("real"));
}

#[test]
fn a_guarded_link_is_protected_by_name_and_by_target() {
    let root = scratch("guarded-link");
    fs::write(root.join("real"), "").unwrap();
    symlink(root.join("real"), root.join("pointer")).unwrap();
    let p = profile(&Policy {
        guarded: vec![root.join("pointer"), root.join("real")],
        ..Policy::default()
    })
    .unwrap();
    let guarded: Vec<_> = p
        .params
        .iter()
        .filter(|(n, _)| n.starts_with('G'))
        .collect();
    // The target once, though two entries name it; the link's own name too.
    assert_eq!(guarded.len(), 2, "{guarded:?}");
    assert!(guarded.iter().any(|(_, v)| *v == root.join("real")));
    assert!(guarded.iter().any(|(_, v)| *v == root.join("pointer")));
    // A path with no parent or name is its own entry.
    assert_eq!(entry(Path::new("/")), PathBuf::from("/"));
}

#[test]
fn a_denied_link_is_kept_by_name_and_by_target() {
    let root = scratch("denied-link");
    fs::write(root.join("real"), "").unwrap();
    symlink(root.join("real"), root.join("alternates")).unwrap();
    let p = profile(&Policy {
        denied: vec![root.join("alternates"), root.join("real")],
        ..Policy::default()
    })
    .unwrap();
    let denied: Vec<_> = p.params.iter().filter(|(n, _)| n.starts_with('X')).collect();
    assert_eq!(denied.len(), 2, "{denied:?}");
    assert!(denied.iter().any(|(_, v)| *v == root.join("alternates")));
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
    assert!(canonical_or_planned(Path::new(""))
        .unwrap_err()
        .contains("cannot resolve"));
    // A file used as a directory is some other error.
    assert!(canonical_or_planned(&root.join("file/child"))
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
    let path = root.join(name);
    fake_exe::write_script(&path, text);
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
        message.contains("cannot apply the profile") && message.contains("unknown operation"),
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

#[test]
fn the_employer_roots_are_denied_for_reads_and_writes_after_everything_else() {
    let root = scratch("read-limit");
    let mut policy = policy(&root);
    policy.read_denied = vec![root.join("employer"), root.join("nowhere-yet/clones")];
    let p = profile(&policy).unwrap();
    assert!(
        p.text.ends_with(
            "(deny file-link)\n(deny file-read* file-write*\n  (subpath (param \"E0\"))\n  (subpath (param \"E1\"))\n)\n"
        ),
        "{}",
        p.text
    );
    // The roots are parameters, never text, and a root that is not there yet
    // lands beside its parent.
    assert!(!p.text.contains(root.to_str().unwrap()));
    assert!(p.params.contains(&("E0".into(), root.join("employer"))));
    assert!(p.params.contains(&("E1".into(), root.join("nowhere-yet/clones"))));
}

#[test]
fn unlimited_writes_leave_only_the_read_denial_and_the_link_denial() {
    let root = scratch("read-only-limit");
    let p = profile(&Policy {
        read_denied: vec![root.join("employer")],
        writes_unlimited: true,
        ..Policy::default()
    })
    .unwrap();
    assert_eq!(
        p.text,
        "(version 1)\n(allow default)\n\
         (deny file-read* file-write*\n  (subpath (param \"E0\"))\n)\n(deny file-link)\n"
    );
    // With nothing to keep unreadable either, there is no rule at all.
    let none = profile(&Policy {
        writes_unlimited: true,
        ..Policy::default()
    })
    .unwrap();
    assert_eq!(none.text, "(version 1)\n(allow default)\n");
}

#[test]
fn an_employer_root_that_cannot_be_resolved_is_refused() {
    let root = scratch("read-limit-bad");
    fs::write(root.join("file"), "").unwrap();
    let policy = Policy {
        read_denied: vec![root.join("file/child")],
        ..Policy::default()
    };
    assert!(profile(&policy).unwrap_err().contains("cannot resolve"));
}

fn confined(root: &Path, ports: Option<Vec<u16>>) -> Policy {
    for d in ["read", "run", "write"] {
        fs::create_dir_all(root.join(d)).unwrap();
    }
    fs::write(root.join("dev"), "").unwrap();
    Policy {
        dirs: vec![root.join("write")],
        files: vec![root.join("dev")],
        reads: Some(Reads {
            read: vec![root.join("read")],
            run: vec![root.join("run")],
        }),
        connect_ports: ports,
        ..Policy::default()
    }
}

/// The parameters named with `prefix`, in order.
fn values(p: &Profile, prefix: char) -> Vec<&PathBuf> {
    p.params.iter().filter(|(n, _)| n.starts_with(prefix)).map(|(_, v)| v).collect()
}

#[test]
fn a_confinement_denies_everything_and_grants_reads_runs_writes_and_ports() {
    let root = scratch("confined");
    let p = profile(&confined(&root, Some(vec![443, 8080]))).unwrap();
    assert!(p.text.starts_with("(version 1)\n(deny default)\n"), "{}", p.text);
    assert!(!p.text.contains("(allow default)"), "{}", p.text);
    // Reads and runs are separate grants: only a run grant may be executed or mapped.
    assert!(p.text.contains("(allow file-read*\n  (subpath (param \"P0\"))\n)\n"), "{}", p.text);
    assert!(
        p.text.contains("(allow file-read* file-map-executable process-exec\n  (subpath (param \"Q0\"))\n)\n"),
        "{}",
        p.text
    );
    assert!(
        p.text.contains("(allow file-read* file-write*\n  (subpath (param \"D0\"))\n  (literal (param \"F0\"))\n)\n"),
        "{}",
        p.text
    );
    assert_eq!(values(&p, 'P'), [&root.join("read")]);
    assert_eq!(values(&p, 'Q'), [&root.join("run")]);
    assert_eq!(values(&p, 'D'), [&root.join("write")]);
    // The network: the listed ports to any host, the resolver socket, and nothing else.
    assert!(p.text.contains("(allow network-outbound (remote tcp \"*:443\"))\n"), "{}", p.text);
    assert!(p.text.contains("(allow network-outbound (remote tcp \"*:8080\"))\n"), "{}", p.text);
    assert!(
        p.text.contains("(allow network-outbound (remote unix-socket (path-literal \"/private/var/run/mDNSResponder\")))\n"),
        "{}",
        p.text
    );
    assert!(!p.text.contains("(allow network*)") && !p.text.contains("network-bind"), "{}", p.text);
    // Hard links are denied, as in the write limit, and the denial comes last.
    assert!(p.text.ends_with("(deny file-link)\n"), "{}", p.text);
    // No path is quoted into the profile.
    assert!(!p.text.contains(root.to_str().unwrap()));
}

#[test]
fn without_a_port_list_the_network_is_left_alone() {
    let root = scratch("confined-net");
    let p = profile(&confined(&root, None)).unwrap();
    assert!(p.text.contains("(allow network*)\n") && !p.text.contains("remote tcp"), "{}", p.text);
}

#[test]
fn the_runtime_rules_are_the_traced_set_and_nothing_wider() {
    for rule in [
        "(allow file-read-data (literal \"/\"))",
        "(allow sysctl-read)",
        "(allow process-fork)",
        "(allow signal (target self))",
        "(allow mach-lookup (global-name \"com.apple.trustd.agent\"))",
    ] {
        assert!(RUNTIME_RULES.contains(rule), "{rule}");
    }
    // Reads of the home directory, a bind and a wildcard Mach lookup are never part of it.
    for wider in ["file-read*", "network", "(allow mach-lookup)", "process-exec", "file-write"] {
        assert!(!RUNTIME_RULES.contains(wider), "{wider}");
    }
}

#[test]
fn every_ancestor_of_a_grant_may_be_stat_but_not_read() {
    let root = scratch("ancestors");
    let p = profile(&confined(&root, None)).unwrap();
    let ancestors = values(&p, 'A');
    // A runtime walks each ancestor to resolve a path (Node's realpath); each is listed once.
    assert!(ancestors.contains(&&root) && !ancestors.contains(&&root.join("read")), "{ancestors:?}");
    let mut once = ancestors.clone();
    once.dedup();
    assert_eq!(once.len(), ancestors.len(), "{ancestors:?}");
    assert!(ancestors.iter().all(|a| a.parent().is_some()), "the root is in the runtime rules, not here");
    assert!(p.text.contains("(allow file-read-metadata\n  (literal (param \"A0\"))"), "{}", p.text);
    // Only metadata: no ancestor is granted a read.
    let granted_reads = p.text.split("(allow file-read*").count() - 1;
    assert_eq!(granted_reads, 3, "{}", p.text);
}

#[test]
fn a_grant_that_cannot_be_resolved_refuses_the_confinement() {
    let root = scratch("confined-bad");
    let mut policy = confined(&root, None);
    policy.reads = Some(Reads { read: vec![root.join("nowhere")], run: vec![] });
    assert!(profile(&policy).unwrap_err().contains("cannot resolve"));
    policy.reads = Some(Reads { read: vec![], run: vec![root.join("nowhere")] });
    assert!(profile(&policy).unwrap_err().contains("cannot resolve"));
}

#[test]
fn the_standard_streams_that_are_files_may_be_stat_and_a_pipe_needs_nothing() {
    // The test's own streams may be a pipe, a file or a terminal; whichever, every path found is absolute and listed once.
    let paths = stdio_paths();
    assert!(paths.iter().all(|p| p.is_absolute()), "{paths:?}");
    let mut once = paths.clone();
    once.dedup();
    assert_eq!(once.len(), paths.len());
    let root = scratch("stdio");
    let p = profile(&confined(&root, None)).unwrap();
    assert_eq!(values(&p, 'S'), paths.iter().collect::<Vec<_>>());
    assert_eq!(p.text.contains("file-read-metadata file-ioctl"), !paths.is_empty());
}

#[cfg(target_os = "macos")]
#[test]
fn a_standard_stream_that_is_a_file_is_found_by_its_path() {
    use std::os::fd::AsRawFd;
    let root = scratch("stdio-file");
    let file = fs::File::create(root.join("out")).unwrap();
    // SAFETY: duplicates the descriptor over stderr in a child-free test thread; restored below.
    let saved = unsafe { libc::dup(2) };
    assert!(unsafe { libc::dup2(file.as_raw_fd(), 2) } == 2);
    let found = stdio_paths();
    // SAFETY: puts stderr back.
    unsafe { libc::dup2(saved, 2) };
    unsafe { libc::close(saved) };
    assert!(found.contains(&root.join("out")), "{found:?}");
}

#[test]
fn the_probe_of_a_confinement_may_run_true_and_the_agents_profile_may_not() {
    let root = scratch("probe");
    let log = root.join("log");
    let backend = fake(
        &root,
        "sandbox-exec",
        &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display()),
    );
    let _held = fake_exe::lock();
    let armed = backend.prepare(&confined(&root, Some(vec![443]))).unwrap();
    let probe = fs::read_to_string(&log).unwrap();
    assert!(
        probe.contains("(allow file-read* file-map-executable process-exec (literal \"/usr/bin/true\"))")
            && probe.trim_end().ends_with("/usr/bin/true"),
        "{probe}"
    );
    // The profile the agent gets is the one without that rule.
    let prefix = armed.prefix();
    assert!(!prefix[2].to_string_lossy().contains("/usr/bin/true"), "{:?}", prefix[2]);
    assert!(prefix[2].to_string_lossy().contains("(deny default)"));
}

#[test]
fn a_confinement_sandbox_exec_cannot_apply_is_refused_with_its_message() {
    let root = scratch("confined-rejected");
    let backend = fake(&root, "sandbox-exec", "#!/bin/sh\necho 'sandbox_apply: Operation not permitted' >&2\nexit 71\n");
    let _held = fake_exe::lock();
    let message = backend.prepare(&confined(&root, Some(vec![443]))).err().unwrap();
    assert!(message.contains("cannot apply the profile") && message.contains("sandbox_apply"), "{message}");
}

#[test]
fn a_port_limit_without_a_confinement_is_refused_not_dropped() {
    let root = scratch("ports-only");
    let backend = fake(&root, "sandbox-exec", "#!/bin/sh\nexit 0\n");
    let _held = fake_exe::lock();
    let mut policy = policy(&root);
    policy.connect_ports = Some(vec![443]);
    let message = backend.prepare(&policy).err().unwrap();
    assert!(message.contains("--net-connect") && message.contains("confinement"), "{message}");
}

#[test]
fn the_user_volume_and_the_swap_volume_are_denied_after_the_grants() {
    let root = scratch("volumes");
    let p = profile(&confined(&root, None)).unwrap();
    let deny = "(deny file-read* file-map-executable process-exec\n  (subpath \"/System/Volumes/Data\")\n  (subpath \"/System/Volumes/VM\")\n)\n";
    let at = p.text.find(deny).expect(&p.text);
    assert!(at > p.text.find("(allow file-read* file-map-executable process-exec").unwrap());
    assert!(at > p.text.find("(allow network*)").unwrap());
}

#[test]
fn the_keychain_and_the_preferences_are_allowed_only_when_asked_for() {
    let root = scratch("confined-services");
    let plain = profile(&confined(&root, Some(vec![443]))).unwrap();
    assert!(!plain.text.contains("SecurityServer") && !plain.text.contains("cfprefsd"), "{}", plain.text);
    let asked = profile(&Policy { keychain: true, preferences: true, ..confined(&root, Some(vec![443])) }).unwrap();
    assert!(asked.text.contains("(allow mach-lookup (global-name \"com.apple.SecurityServer\"))"), "{}", asked.text);
    assert!(asked.text.contains("com.apple.cfprefsd.daemon") && asked.text.contains("(allow user-preference-read)"), "{}", asked.text);
    // Still no bind, and the volume denial still comes after every grant.
    assert!(!asked.text.contains("network-bind"), "{}", asked.text);
    assert!(asked.text.find("SecurityServer").unwrap() < asked.text.find("(deny file-read* file-map-executable process-exec").unwrap());
}
