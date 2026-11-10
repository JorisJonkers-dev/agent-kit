use super::*;

#[test]
fn env_reference_and_keychain_name_pass_in_secret_named_fields() {
    assert_eq!(secret_problem("env.API_TOKEN", "$API_TOKEN"), None);
    assert_eq!(secret_problem("env.API_TOKEN", "${API_TOKEN}"), None);
    assert_eq!(secret_problem("settings.keyboard", "true"), None);
}

#[test]
fn literal_in_secret_named_field_is_refused_without_printing_it() {
    for bad in ["hunter2", "hunter2 hunter2", "", "$lower", "${A", "1abc"] {
        let problem = secret_problem("env.API_TOKEN", bad).expect("refused");
        assert!(problem.contains("secret-named"));
        if !bad.is_empty() {
            assert!(!problem.contains(bad));
        }
    }
    assert!(secret_problem("env.API_KEY", "sk-aaaaaaaaaaaaaaaaaaaa").is_some());
    for name in ["env.DB_PASSWD", "env.X_CREDENTIAL", "env.GITHUB_PAT", "env.AUTH"] {
        assert!(secret_problem(name, "hunter2").is_some(), "{name}");
    }
}

#[test]
fn every_secret_shape_is_named() {
    let cases = [
        ("sk-aaaaaaaaaaaaaaaaaaaa", "openai key"),
        ("ghp_aaaaaaaaaaaaaaaaaaaaaaaa", "github token"),
        ("github_pat_aaaaaaaaaaaaaaaaaaaaaaaa", "github token"),
        ("xoxb-123", "slack token"),
        ("xoxp-123", "slack token"),
        ("AKIAAAAAAAAAAAAAAAAA", "aws access key"),
        ("Bearer abcdefghijklmnopqrstuv", "bearer token"),
        ("eyJhbGciOi.eyJzdWIi.c2ln", "jwt"),
        ("abcdefghij0123456789abcdefghij0123", "long opaque token"),
    ];
    for (value, shape) in cases {
        let problem = secret_problem("settings.note", value).expect(value);
        assert_eq!(problem, format!("settings.note holds a literal secret ({shape})"));
    }
}

#[test]
fn near_misses_and_references_are_allowed() {
    for ok in [
        "sk-short",
        "ghp_short",
        "AKIA123",
        "Bearer short",
        "bearer",
        "eyJabc",
        "eyJabc.def",
        "eyJabc.def.",
        "$HOME",
        "~/.local/bin/some-tool-with-a-long-name-0123456789",
        "/a/b/0123456789abcdefghij0123456789abcdefghij/x",
        "plain words only",
        "abcdefghijklmnopqrstuvwxyzabcdefgh",
        "0123456789012345678901234567890123",
        "abc def 0123456789012345678901234567890123 x",
    ] {
        assert_eq!(secret_problem("settings.note", ok), None, "{ok}");
    }
}

#[test]
fn opaque_runs_break_on_a_separator() {
    assert!(secret_problem("a.b", "0123456789abcdef 0123456789abcdef0123456789abcdef").is_some());
    assert!(secret_problem("a.b", "0123456789abcdef 0123456789abcdef").is_none());
}
