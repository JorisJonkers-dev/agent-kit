use super::*;

fn os(code: i32) -> io::Result<i64> {
    Err(io::Error::from_raw_os_error(code))
}

#[test]
fn a_recent_enough_abi_passes() {
    assert_eq!(abi_from(Ok(3)), Ok(3));
    assert_eq!(abi_from(Ok(7)), Ok(7));
}

#[test]
fn an_old_abi_is_refused_with_the_version_and_what_is_needed() {
    let message = abi_from(Ok(2)).unwrap_err();
    assert!(
        message.contains("ABI 2 is too old") && message.contains("ABI 3"),
        "{message}"
    );
}

#[test]
fn a_kernel_without_landlock_is_refused() {
    assert!(abi_from(os(libc::ENOSYS))
        .unwrap_err()
        .contains("no Landlock"));
}

#[test]
fn a_kernel_with_landlock_disabled_is_refused() {
    assert!(abi_from(os(libc::EOPNOTSUPP))
        .unwrap_err()
        .contains("disabled"));
}

#[test]
fn any_other_query_error_is_reported() {
    assert!(abi_from(os(libc::EPERM))
        .unwrap_err()
        .starts_with("cannot query Landlock"));
}

#[test]
fn the_real_kernel_answers_the_abi_query() {
    assert!(abi_from(Sys::version()).is_ok());
}

#[test]
fn bad_arguments_come_back_as_os_errors() {
    assert!(Sys::create(!0).is_err());
    assert_eq!(
        Sys::add(-1, HANDLED, -1).unwrap_err().raw_os_error(),
        Some(libc::EBADF)
    );
    // Fails on the descriptor, after the (harmless) no-new-privileges flag.
    assert_eq!(
        Sys::restrict(-1).unwrap_err().raw_os_error(),
        Some(libc::EBADF)
    );
}

#[test]
fn a_path_that_cannot_be_opened_is_named() {
    let policy = Policy {
        dirs: vec!["/nonexistent/agent-launcher".into()],
        files: vec![],
        ..Policy::default()
    };
    let message = Landlock.prepare(&policy).err().unwrap();
    assert!(message.contains("/nonexistent/agent-launcher"), "{message}");
}

#[test]
fn a_directory_right_on_a_file_is_refused_by_the_kernel() {
    let policy = Policy {
        dirs: vec!["/dev/null".into()],
        files: vec![],
        ..Policy::default()
    };
    let message = Landlock.prepare(&policy).err().unwrap();
    assert!(
        message.contains("cannot allow writes to /dev/null"),
        "{message}"
    );
}

#[test]
fn a_prepared_limit_restricts_nothing_in_this_process() {
    let granted =
        std::env::temp_dir().join(format!("agent-launcher-granted-{}", std::process::id()));
    fs::create_dir_all(&granted).unwrap();
    let policy = Policy {
        dirs: vec![granted.clone()],
        files: vec!["/dev/null".into()],
        ..Policy::default()
    };
    let _armed = Landlock.prepare(&policy).unwrap();
    let probe = std::env::temp_dir().join(format!("agent-launcher-probe-{}", std::process::id()));
    fs::write(&probe, "x").unwrap();
    fs::remove_file(&probe).unwrap();
    fs::remove_dir(&granted).unwrap();
}

#[test]
fn arming_with_something_that_is_not_a_ruleset_fails_and_restricts_nothing() {
    let not_a_ruleset = Ruleset(OwnedFd::from(fs::File::open("/dev/null").unwrap()));
    assert!(not_a_ruleset.arm().is_err());
}

#[test]
fn a_ruleset_the_kernel_refuses_is_reported() {
    let message = build(&Policy::default(), !0).err().unwrap();
    assert!(
        message.starts_with("cannot create the Landlock ruleset"),
        "{message}"
    );
}

#[test]
fn landlock_cannot_keep_the_employer_roots_unreadable_and_says_so() {
    let policy = Policy {
        read_denied: vec!["/srv/employer".into()],
        ..Policy::default()
    };
    let message = Landlock.prepare(&policy).err().unwrap();
    assert!(message.contains("cannot deny reads"), "{message}");
}
