use super::*;
use crate::testkit::{run_in, run_plain, spawn_lock, Dir, Out};
use std::time::Duration;

const PIN_PATH: &str = "repos/JorisJonkers-dev/estate/contents/projects/grimoire/source.yaml";

const SOURCE_YAML: &str = "# GENERATED. Never hand-edit.
---
apiVersion: source.toolkit.fluxcd.io/v1
kind: OCIRepository
metadata:
  name: project-grimoire
spec:
  interval: 10m
---
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: apps-grimoire
  annotations:
    event.toolkit.fluxcd.io/repository: JorisJonkers-dev/grimoire
    event.toolkit.fluxcd.io/environment: production
    event.toolkit.fluxcd.io/version: PIN
    event.toolkit.fluxcd.io/url: https://grimoire.jorisjonkers.dev
spec:
  interval: 10m
";

const PAUSED: &str = "    estate.jorisjonkers.dev/paused-by: alice
    estate.jorisjonkers.dev/paused-at: \"2026-11-01T10:00:00Z\"
    estate.jorisjonkers.dev/paused-reason: \"bad deploy\"
";
const ROLLED: &str = "    estate.jorisjonkers.dev/rollback-version: 1.3.0
    estate.jorisjonkers.dev/rollback-fragment: sha256:abc
";

const PAUSE_CMD: &str =
    "gh workflow run pause.yml --repo JorisJonkers-dev/estate --ref main -f project=grimoire -f 'reason=bad deploy'";
const ROLLBACK_CMD: &str = "gh workflow run rollback.yml --repo JorisJonkers-dev/estate --ref main -f project=grimoire -f version=1.3.0 -f 'reason=bad deploy'";
const RESUME_CMD: &str =
    "gh workflow run resume.yml --repo JorisJonkers-dev/estate --ref main -f project=grimoire";
const PIN_NOTE: &str =
    "pin given by hand: the pin file was not read, so a recorded rollback is not checked";
const NOT_STATED: &str = "Migration Proof not stated: pass --proof-names <release the proof names> or --no-proof when the pinned release carries none";

/// A fake `gh` that logs its arguments and prints a body or fails.
struct Gh {
    dir: Dir,
    path: String,
}

impl Gh {
    fn new(body: &str, fail: bool) -> Gh {
        let dir = Dir::new();
        dir.write("body.yaml", body);
        let log = dir.join("gh.log");
        let script = format!(
            "echo \"$*\" >> {log}\n{}",
            if fail {
                "echo 'HTTP 404: Not Found' >&2\nexit 1".to_string()
            } else {
                format!("cat {}", dir.join("body.yaml").display())
            },
            log = log.display()
        );
        dir.tool("bin/gh", &script);
        let path = dir.join("bin").to_string_lossy().into_owned();
        Gh { dir, path }
    }

    fn yaml(pin: &str, extra: &str) -> Gh {
        let body = SOURCE_YAML.replace("PIN", pin).replace(
            "    event.toolkit.fluxcd.io/url",
            &format!("{extra}    event.toolkit.fluxcd.io/url"),
        );
        Gh::new(&body, false)
    }

    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("gh.log"))
            .map(|t| t.lines().map(String::from).collect())
            .unwrap_or_default()
    }

    fn run(&self, args: &[&str]) -> Out {
        let mut full = vec!["plan"];
        full.extend_from_slice(args);
        run_in(&full, "", &self.path, None, None)
    }
}

fn commands(out: &str) -> Vec<String> {
    out.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("gh workflow run"))
        .map(String::from)
        .collect()
}

const RB: [&str; 6] = [
    "--project",
    "grimoire",
    "--action",
    "rollback",
    "--reason",
    "bad deploy",
];

fn rb<'a>(more: &[&'a str]) -> Vec<&'a str> {
    let mut v: Vec<&str> = RB.to_vec();
    v.extend_from_slice(more);
    v
}

fn refused(gh: &Gh, args: &[&str], message: &str) {
    let out = gh.run(args);
    assert_eq!(out.code, 1, "{}{}", out.out, out.err);
    assert!(out.err.contains(message), "{}", out.err);
    assert!(out.err.starts_with("REFUSED: "));
    assert_eq!(out.out, "");
}

#[test]
fn pause_prints_the_exact_command() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    let out = gh.run(&[
        "--project",
        "grimoire",
        "--action",
        "pause",
        "--reason",
        "bad deploy",
    ]);
    assert_eq!(out.code, 0, "{}", out.err);
    assert_eq!(commands(&out.out), [PAUSE_CMD]);
    assert!(out.out.contains(
        "pin:      1.5.0 (projects/grimoire/source.yaml, event.toolkit.fluxcd.io/version)"
    ));
    assert!(out.out.starts_with("estate rollback plan (this tool runs nothing; the owner runs the command)\nproject:  grimoire\naction:   pause\n"));
}

#[test]
fn rollback_prints_the_rollback_and_the_later_resume() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    let out = gh.run(&rb(&["--target", "1.3.0", "--no-proof"]));
    assert_eq!(out.code, 0, "{}", out.err);
    assert_eq!(commands(&out.out), [ROLLBACK_CMD, RESUME_CMD]);
    assert!(out
        .out
        .contains("target:   1.3.0 (earlier than the pin 1.5.0: ok)"));
    assert!(out
        .out
        .contains("Migration Proof: --no-proof says release 1.5.0 carries none"));
    assert!(!out.out.contains("pin given by hand"));
}

#[test]
fn rollback_with_the_proofs_release_says_ok() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    let out = gh.run(&rb(&["--target", "1.3.0", "--proof-names", "1.3.0"]));
    assert_eq!(out.code, 0, "{}", out.err);
    assert_eq!(commands(&out.out)[0], ROLLBACK_CMD);
    assert!(out
        .out
        .contains("target 1.3.0 is the release --proof-names gives: ok"));
}

#[test]
fn resume_prints_the_exact_command_and_warns_only_when_rolled_back() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    let out = gh.run(&["--project", "grimoire", "--action", "resume"]);
    assert_eq!(commands(&out.out), [RESUME_CMD]);
    assert!(!out.out.contains("WARNING"));

    let gh = Gh::yaml("1.5.0", &format!("{PAUSED}{ROLLED}"));
    let out = gh.run(&["--project", "grimoire", "--action", "resume"]);
    assert_eq!(out.code, 0, "{}", out.err);
    assert!(out
        .out
        .contains("state:    paused by alice at 2026-11-01T10:00:00Z: bad deploy"));
    assert!(out.out.contains("state:    rolled back to 1.3.0"));
    assert!(out.out.contains("WARNING: this Project is rolled back"));
    assert_eq!(commands(&out.out), [RESUME_CMD]);
}

#[test]
fn a_paused_by_without_the_rest_prints_question_marks() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "    estate.jorisjonkers.dev/paused-by: bob\n");
    let out = gh.run(&["--project", "grimoire", "--action", "resume"]);
    assert!(
        out.out.contains("state:    paused by bob at ?: \n"),
        "{}",
        out.out
    );
}

#[test]
fn a_reason_is_quoted_so_it_cannot_inject_a_second_command() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    let out = gh.run(&[
        "--project",
        "grimoire",
        "--action",
        "pause",
        "--reason",
        "x; rm -rf ~",
    ]);
    assert_eq!(out.code, 0);
    assert!(out.out.contains("-f 'reason=x; rm -rf ~'"));
    let out = gh.run(&[
        "--project",
        "grimoire",
        "--action",
        "pause",
        "--reason",
        "why",
    ]);
    assert!(out.out.contains("-f reason=why"));
}

#[test]
fn the_only_gh_call_is_a_read_only_get_of_the_contents_endpoint() {
    let _g = spawn_lock();
    let cases: Vec<Vec<&str>> = vec![
        vec!["--action", "pause", "--reason", "bad deploy"],
        vec!["--action", "resume"],
        vec![
            "--action",
            "rollback",
            "--target",
            "1.3.0",
            "--no-proof",
            "--reason",
            "bad deploy",
        ],
        vec![
            "--action",
            "rollback",
            "--target",
            "1.3.0",
            "--reason",
            "bad deploy",
        ],
        vec![
            "--action",
            "rollback",
            "--target",
            "1.3.0",
            "--no-proof",
            "--proof-names",
            "1.3.0",
            "--reason",
            "bad deploy",
        ],
        vec![
            "--action",
            "rollback",
            "--target",
            "1.6.0",
            "--no-proof",
            "--reason",
            "bad deploy",
        ],
        vec![
            "--action",
            "rollback",
            "--target",
            "1.3.0",
            "--proof-names",
            "1.4.2",
            "--reason",
            "bad deploy",
        ],
        vec!["--action", "rollback", "--reason", "bad deploy"],
        vec!["--action", "rollback", "--target", "1.3.0"],
        vec!["--action", "pause"],
    ];
    for case in cases {
        let gh = Gh::yaml("1.5.0", "");
        let mut args = vec!["--project", "grimoire"];
        args.extend(case);
        gh.run(&args);
        for call in gh.calls() {
            assert_eq!(
                call,
                format!("api {PIN_PATH} -H Accept: application/vnd.github.raw")
            );
        }
    }
}

#[test]
fn a_refusal_made_before_reading_never_calls_gh() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    let out = gh.run(&["--project", "grimoire", "--action", "pause"]);
    assert_eq!(out.code, 1);
    assert!(gh.calls().is_empty());
}

#[test]
fn with_pin_given_gh_is_not_called_at_all() {
    let _g = spawn_lock();
    for action in ["pause", "rollback", "resume"] {
        let gh = Gh::yaml("1.5.0", "");
        let mut args = vec!["--project", "grimoire", "--action", action];
        match action {
            "rollback" => {
                args.extend(["--target", "1.3.0", "--no-proof", "--reason", "bad deploy"])
            }
            "pause" => args.extend(["--reason", "bad deploy"]),
            _ => {}
        }
        args.extend(["--pin", "1.5.0"]);
        let out = gh.run(&args);
        assert_eq!(out.code, 0, "{}", out.err);
        assert!(out.out.contains("(given with --pin (file not read))"));
        assert_eq!(out.out.matches(PIN_NOTE).count(), 1);
        assert!(gh.calls().is_empty());
    }
}

#[test]
fn rollback_refusals() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    refused(
        &gh,
        &rb(&["--no-proof", "--target", "1.6.0"]),
        "target 1.6.0 is not earlier than the pin 1.5.0",
    );
    refused(
        &gh,
        &rb(&["--no-proof", "--target", "1.5.0"]),
        "target 1.5.0 is not earlier than the pin 1.5.0",
    );
    refused(
        &gh,
        &rb(&["--target", "1.3.0", "--proof-names", "1.4.2"]),
        "target 1.3.0 is not the release the pinned release 1.5.0's Migration Proof names (1.4.2)",
    );
    refused(&gh, &rb(&["--target", "1.3.0"]), NOT_STATED);
    refused(
        &gh,
        &rb(&["--target", "1.3.0", "--no-proof", "--proof-names", "1.3.0"]),
        &format!("{NOT_STATED} (not both)"),
    );
    refused(&gh, &RB, "--action rollback needs --target");
    refused(
        &gh,
        &rb(&["--no-proof", "--target", ""]),
        "--action rollback needs --target",
    );
    for version in [
        "01.4.0", "1.04.0", "1.4.00", "1.4", "1.4.x", "1..0", "1.4.0.1",
    ] {
        refused(
            &gh,
            &rb(&["--no-proof", "--target", version]),
            &format!("--target '{version}' is not a release version"),
        );
    }
}

#[test]
fn versions_compare_numerically_not_as_text() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.9.0", "");
    refused(
        &gh,
        &rb(&["--no-proof", "--target", "1.10.0"]),
        "not earlier than the pin 1.9.0",
    );
    assert!(version_key("1.10.0") > version_key("1.9.0"));
    assert!(version_key("1.2.3") < version_key("1.2.30"));
    assert!(version_key(&format!("1.0.{}", "9".repeat(30))) > version_key("1.0.1"));
}

#[test]
fn a_missing_blank_or_multiline_reason_is_refused() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    for action in ["pause", "rollback"] {
        for reason in [
            None,
            Some(""),
            Some("   "),
            Some("two\nlines"),
            Some("a\u{85}b"),
            Some("a\u{2028}b"),
            Some("a\u{2029}b"),
            Some("a\tb"),
            Some("a\u{7f}b"),
        ] {
            let mut args = vec!["--project", "grimoire", "--action", action];
            if action == "rollback" {
                args.extend(["--target", "1.3.0", "--no-proof"]);
            }
            if let Some(r) = reason {
                args.extend(["--reason", r]);
            }
            let message = if reason.is_some_and(|r| !r.trim().is_empty()) {
                "--reason must be one line"
            } else {
                "needs --reason"
            };
            refused(&gh, &args, message);
        }
    }
}

#[test]
fn an_already_rolled_back_pin_is_refused() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", ROLLED);
    refused(
        &gh,
        &rb(&["--no-proof", "--target", "1.2.0"]),
        "grimoire is already rolled back to 1.3.0",
    );
    let gh = Gh::yaml(
        "1.5.0",
        "    estate.jorisjonkers.dev/rollback-fragment: sha256:abc\n",
    );
    refused(
        &gh,
        &rb(&["--no-proof", "--target", "1.2.0"]),
        "already rolled back to ?",
    );
}

#[test]
fn malformed_input_is_refused() {
    let _g = spawn_lock();
    let gh = Gh::yaml("1.5.0", "");
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (
            vec!["--project", "Grimoire", "--action", "resume"],
            "'Grimoire' is not a Project's name",
        ),
        (
            vec!["--project", "", "--action", "resume"],
            "'' is not a Project's name",
        ),
        (
            vec!["--project", "-x", "--action", "resume"],
            "'-x' is not a Project's name",
        ),
        (
            vec![
                "--project",
                "grimoire",
                "--action",
                "resume",
                "--target",
                "1.0.0",
            ],
            "only apply to --action rollback",
        ),
        (
            vec!["--project", "grimoire", "--action", "resume", "--no-proof"],
            "only apply to --action rollback",
        ),
        (
            vec![
                "--project",
                "grimoire",
                "--action",
                "pause",
                "--reason",
                "r",
                "--proof-names",
                "1.0.0",
            ],
            "only apply",
        ),
        (
            rb(&["--no-proof", "--target", "1.x"]),
            "--target '1.x' is not a release version",
        ),
        (
            rb(&["--target", "1.3.0", "--proof-names", "v1"]),
            "--proof-names 'v1' is not a release version",
        ),
        (
            vec![
                "--project",
                "grimoire",
                "--action",
                "resume",
                "--pin",
                "latest",
            ],
            "--pin 'latest' is not a release version",
        ),
    ];
    for (args, message) in cases {
        refused(&gh, &args, message);
    }
}

#[test]
fn usage_mistakes_exit_2() {
    let out = run_plain(&["plan", "--project", "grimoire", "--action", "dispatch"]);
    assert_eq!(out.code, 2);
    assert!(out
        .err
        .contains("invalid choice: 'dispatch' (choose from 'pause', 'rollback', 'resume')"));
    assert_eq!(run_plain(&["plan", "--project", "grimoire"]).code, 2);
    assert_eq!(run_plain(&["plan", "--help"]).code, 0);
}

#[test]
fn a_gh_failure_exits_1_with_no_command() {
    let _g = spawn_lock();
    let gh = Gh::new("", true);
    let out = gh.run(&["--project", "nosuch", "--action", "resume"]);
    assert_eq!(out.code, 1);
    assert!(out
        .err
        .contains("could not read projects/nosuch/source.yaml"));
    assert!(out.err.contains("HTTP 404: Not Found"));
    assert_eq!(out.out, "");
    assert_eq!(gh.calls().len(), 1);
}

#[test]
fn a_gh_failure_without_detail_says_so() {
    let _g = spawn_lock();
    let dir = Dir::new();
    dir.tool("bin/gh", "exit 3");
    let path = dir.join("bin").to_string_lossy().into_owned();
    let out = run_in(
        &["plan", "--project", "grimoire", "--action", "resume"],
        "",
        &path,
        None,
        None,
    );
    assert!(out.err.contains("(gh exit 3: no detail)"));
    dir.tool("bin/gh", "kill -9 $$");
    let out = run_in(
        &["plan", "--project", "grimoire", "--action", "resume"],
        "",
        &path,
        None,
        None,
    );
    assert!(out.err.contains("(gh exit none: no detail)"));
}

#[test]
fn gh_missing_from_path_exits_1_and_says_stop() {
    let out = run_plain(&["plan", "--project", "grimoire", "--action", "resume"]);
    assert_eq!(out.code, 1);
    assert!(out.err.contains("gh is not on PATH"));
    assert_eq!(out.out, "");
}

#[test]
fn a_gh_that_cannot_be_run_or_does_not_answer_is_refused() {
    let _g = spawn_lock();
    let dir = Dir::new();
    let tool = dir.write("bin/gh", "#!/nonexistent/interpreter\n");
    std::fs::set_permissions(&tool, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = dir.join("bin").to_string_lossy().into_owned();
    let args = ["plan", "--project", "grimoire", "--action", "resume"];
    assert!(run_in(&args, "", &path, None, None)
        .err
        .contains("could not run gh"));
    dir.tool("bin/gh", "exec sleep 30");
    let out = run_in(&args, "", &path, Some(Duration::from_millis(200)), None);
    assert_eq!(out.code, 1);
    assert!(out.err.contains("did not answer within"));
}

#[test]
fn an_unusable_pin_file_is_refused() {
    let _g = spawn_lock();
    let cases = [
        ("kind: Kustomization\n", "found 0"),
        ("    event.toolkit.fluxcd.io/version: 1.0.0\n    event.toolkit.fluxcd.io/version: 1.1.0\n", "found 2"),
        ("    event.toolkit.fluxcd.io/version: latest\n", "the pin 'latest'"),
        ("    event.toolkit.fluxcd.io/version: \"\"\n", "found 0"),
        ("    event.toolkit.fluxcd.io/version: 1.0.0 trailing\n", "found 0"),
        ("    event.toolkit.fluxcd.io/version:\n", "found 0"),
    ];
    for (body, message) in cases {
        let gh = Gh::new(body, false);
        let out = gh.run(&["--project", "grimoire", "--action", "resume"]);
        assert_eq!(out.code, 1, "{body}");
        assert!(out.err.contains(message), "{}", out.err);
        assert_eq!(out.out, "");
    }
}

#[test]
fn pin_and_annotation_lines_are_read_like_the_original_patterns() {
    assert_eq!(
        pin_of("  event.toolkit.fluxcd.io/version: \"2.0.1\"  # tag"),
        Some("2.0.1")
    );
    assert_eq!(
        pin_of("event.toolkit.fluxcd.io/version:\t'2.0.1'"),
        Some("2.0.1")
    );
    assert_eq!(
        pin_of("event.toolkit.fluxcd.io/version: 2.0.1#x"),
        Some("2.0.1")
    );
    assert_eq!(pin_of("x: event.toolkit.fluxcd.io/version: 2.0.1"), None);
    assert_eq!(
        annotation_of("  estate.jorisjonkers.dev/paused-by: alice"),
        Some(("paused-by", "alice"))
    );
    assert_eq!(
        annotation_of("estate.jorisjonkers.dev/paused-reason: \"bad deploy\"  "),
        Some(("paused-reason", "bad deploy"))
    );
    assert_eq!(annotation_of("estate.jorisjonkers.dev/x:"), Some(("x", "")));
    assert_eq!(annotation_of("estate.jorisjonkers.dev/Bad: x"), None);
    assert_eq!(annotation_of("estate.jorisjonkers.dev/: x"), None);
    assert_eq!(annotation_of("estate.jorisjonkers.dev/nocolon"), None);
    assert_eq!(annotation_of("other: x"), None);
}

#[test]
fn a_pin_file_with_windows_line_endings_is_read() {
    let _g = spawn_lock();
    let gh = Gh::new("    event.toolkit.fluxcd.io/version: 2.0.1\r\n    estate.jorisjonkers.dev/paused-by: carol\r\n", false);
    let out = gh.run(&["--project", "grimoire", "--action", "resume"]);
    assert_eq!(out.code, 0, "{}", out.err);
    assert!(out.out.contains("pin:      2.0.1"));
    assert!(out.out.contains("paused by carol at ?"));
}

#[test]
fn the_tool_never_names_a_dispatching_gh_call() {
    let code = include_str!("../plan.rs");
    let production = code.split("#[cfg(test)]").next().unwrap();
    assert_eq!(production.matches("run_capture(").count(), 1);
    assert!(production.contains("\"api\".to_string()"));
    assert!(!production.contains("--method"));
    assert!(!production.contains("\"-X\""));
    assert!(!production.contains("-XPOST"));
}
