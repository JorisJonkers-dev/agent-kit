use super::*;
use crate::writelimit::{Arm, Armed};
use std::io;

fn args(items: &[&str]) -> Vec<OsString> {
    items.iter().map(OsString::from).collect()
}

/// A backend that refuses, one that arms nothing, and one whose arming fails.
enum Fake {
    Refuses,
    Fine,
    ArmFails,
}

struct Armer(bool);

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
            write: vec!["/w".into()],
            ports: Some(vec![443, 8443]),
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
    let policy = policy(&spec);
    let reads = policy.reads.as_ref().unwrap();
    assert_eq!(reads.read.last(), Some(&PathBuf::from("/p")));
    assert_eq!(reads.run.last(), Some(&PathBuf::from("/r")));
    assert!(reads.read.iter().chain(&reads.run).filter(|p| !p.starts_with("/p") && !p.starts_with("/r")).all(|p| p.exists()));
    assert!(reads.read.contains(&PathBuf::from("/dev/urandom")));
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
    assert!(summary(&policy(&spec)).ends_with("TCP connect to port(s) 443, 80"));
    assert!(summary(&Policy::default()).starts_with("0 readable path(s), 0 writable, network not limited"));
}

#[test]
fn a_command_that_cannot_be_started_or_armed_is_refused() {
    // exec only returns on failure, so this process is never replaced here.
    assert_eq!(confine(&args(&["--", "/nonexistent/agent-launcher-cmd"]), &Fake::Fine), REFUSED);
    assert_eq!(confine(&args(&["--", "/bin/sh", "-c", "exit 0"]), &Fake::ArmFails), REFUSED);
}
