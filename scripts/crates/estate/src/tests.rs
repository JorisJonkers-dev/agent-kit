use crate::testkit::run_plain;

#[test]
fn no_subcommand_or_an_unknown_one_prints_the_usage_and_exits_2() {
    for args in [&[][..], &["nope"][..]] {
        let out = run_plain(args);
        assert_eq!(out.code, 2);
        assert!(out.err.starts_with("usage: estate audit"));
    }
}

#[test]
fn help_prints_the_usage_on_stdout() {
    for flag in ["--help", "-h"] {
        let out = run_plain(&[flag]);
        assert_eq!(out.code, 0);
        assert!(out.out.contains("estate plan --project"));
    }
}

#[test]
fn run_env_reads_the_process_environment() {
    assert_eq!(crate::run_env(["--help".into()].into_iter()), 0);
}

#[test]
fn splitlines_matches_python() {
    use crate::text::splitlines;
    assert_eq!(
        splitlines("a\nb\r\nc\rd\x0be\u{2028}f"),
        ["a", "b", "c", "d", "e", "f"]
    );
    assert_eq!(splitlines("a\n"), ["a"]);
    assert_eq!(splitlines("a\n\nb"), ["a", "", "b"]);
    assert!(splitlines("").is_empty());
}

#[test]
fn shell_quote_matches_shlex() {
    use crate::text::shell_quote;
    assert_eq!(shell_quote("plain-1.0/x"), "plain-1.0/x");
    assert_eq!(shell_quote(""), "''");
    assert_eq!(shell_quote("bad deploy"), "'bad deploy'");
    assert_eq!(shell_quote("it's"), "'it'\"'\"'s'");
}

#[test]
fn expand_user_only_expands_a_leading_tilde_with_a_home() {
    use crate::text::expand_user;
    assert_eq!(expand_user("~", Some("/h")), "/h");
    assert_eq!(expand_user("~/x", Some("/h")), "/h/x");
    assert_eq!(expand_user("~x", Some("/h")), "~x");
    assert_eq!(expand_user("~", None), "~");
}

#[test]
fn which_finds_only_executable_files() {
    use crate::proc::which;
    use crate::testkit::Dir;
    let dir = Dir::new();
    dir.write("plain", "x");
    dir.tool("run", "true");
    assert!(which("run", &dir.str()).is_some());
    assert!(which("plain", &dir.str()).is_none());
    assert!(which("absent", &dir.str()).is_none());
}

#[test]
fn json_strings_escape_like_python() {
    use crate::text::json_string;
    assert_eq!(
        json_string("a\"b\\c\nd\re\tf\x08g\x0ch\x01"),
        "\"a\\\"b\\\\c\\nd\\re\\tf\\bg\\fh\\u0001\""
    );
}

#[test]
fn flags_accept_the_equals_form() {
    let out = run_plain(&["audit", "--latest-sha=zz", "."]);
    assert_eq!(out.code, 2);
    assert!(out.err.contains("--latest-sha must be 40"));
    let out = run_plain(&["audit", "--json=1", "."]);
    assert!(out.err.contains("unrecognized arguments: --json=1"));
}

fn skill_text(skill: &str) -> String {
    let path = format!(
        "{}/../../../skills/workstation/{skill}/SKILL.md",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

/// `shlex.split` for the quoting the SKILL.md commands use.
fn split_command(line: &str) -> Vec<String> {
    let (mut words, mut word, mut quote, mut started) = (Vec::new(), String::new(), None, false);
    for c in line.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                started = true;
            }
            (None, c) if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            (None, c) => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        words.push(word);
    }
    words
}

#[test]
fn the_rollback_worked_examples_really_are_refused() {
    let text = skill_text("estate-rollback");
    for heading in [
        "### Worked example: target later than the pin (refused)",
        "### Worked example: target with no Migration Proof (refused)",
    ] {
        let block = text[text.find(heading).unwrap()..]
            .split("```")
            .nth(1)
            .unwrap();
        let mut lines = block.trim().lines();
        let words = split_command(lines.next().unwrap());
        assert_eq!(&words[..2], ["estate", "plan"]);
        let args: Vec<&str> = words[1..].iter().map(String::as_str).collect();
        let out = run_plain(&args);
        assert_eq!(out.code, 1, "{heading}");
        assert_eq!(out.out, "");
        assert_eq!(out.err.trim(), lines.next().unwrap());
    }
}

#[test]
fn the_delivery_worked_examples_give_the_verdict_they_state() {
    use serde_json::json;
    let signal =
        |state: &str, url: &str| json!({"state": state, "url": url, "headSha": "a".repeat(40)});
    let input = |compose: &str,
                 status: &str,
                 deployment: &str,
                 codes: serde_json::Value,
                 issues: serde_json::Value| {
        json!({
            "project": "home-portal", "release": "v1.2.3", "sha": "a".repeat(40),
            "signals": {
                "publish": signal("success", "https://example.test/publish/1"),
                "compose": {"state": compose, "url": "https://example.test/compose/2", "headSha": "a".repeat(40), "codes": codes},
                "status": signal(status, "https://example.test/status/3"),
                "deployment": signal(deployment, "https://example.test/deploy/4"),
                "issues": issues,
            },
        })
    };
    let issue = json!([{"title": "home-portal/isolated", "url": "https://example.test/issues/1"}]);
    let text = skill_text("estate-delivery-status");
    let cases = [
        (
            "landed",
            input("clean", "success", "success", json!([]), json!([])),
        ),
        (
            "mixed",
            input("clean", "pending", "in_progress", json!([]), issue.clone()),
        ),
        (
            "not landed",
            input(
                "errors",
                "failure",
                "missing",
                json!(["E_UNKNOWN_SURFACE"]),
                issue,
            ),
        ),
    ];
    for (name, data) in cases {
        let heading = format!("### Worked example: {name}\n");
        let section = &text[text.find(&heading).unwrap() + heading.len()..];
        let section = section.split("\n#").next().unwrap();
        assert!(
            section.contains(&format!("Result: `VERDICT: {name}`")),
            "{name}"
        );
        let out = crate::testkit::run_in(&["verdict"], &data.to_string(), "", None, None);
        assert_eq!(out.code, 0, "{}", out.err);
        assert_eq!(out.out.lines().next().unwrap(), format!("VERDICT: {name}"));
    }
}
