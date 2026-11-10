use super::*;

fn answer_args(first: Option<&str>, second: Option<&str>) -> (String, i32) {
    answer(first.map(OsString::from), second.map(OsString::from))
}

#[test]
fn version_reports_the_crate_and_exits_zero() {
    let (message, code) = answer_args(Some("--version"), None);
    assert_eq!(code, 0);
    assert_eq!(message, format!("{NAME} {}", env!("CARGO_PKG_VERSION")));
    assert_eq!(run(vec![OsString::from("--version")]), 0);
}

#[test]
fn validate_reports_an_existing_path() {
    let (message, code) = validate(OsStr::new("."));
    assert_eq!(code, 0);
    assert!(message.contains(". exists"));
}

#[test]
fn validate_rejects_a_missing_path() {
    let (message, code) = validate(OsStr::new("no-such-path-agent-kit-setup"));
    assert_eq!(code, 1);
    assert!(message.contains("does not exist"));
    assert_eq!(
        run(vec![
            "validate".into(),
            "no-such-path-agent-kit-setup".into()
        ]),
        1
    );
}

#[test]
fn validate_without_a_path_is_bad_usage() {
    let (message, code) = answer_args(Some("validate"), None);
    assert_eq!(code, 2);
    assert!(message.contains("needs a path"));
}

#[test]
fn no_or_unknown_command_is_bad_usage() {
    assert_eq!(answer_args(None, None).1, 2);
    assert!(answer_args(None, None).0.contains("usage:"));
    assert_eq!(answer_args(Some("dance"), Some("x")).1, 2);
    assert_eq!(run(std::iter::empty()), 2);
}
