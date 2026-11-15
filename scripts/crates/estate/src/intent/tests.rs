use super::*;
use crate::testkit::{run_in, run_plain, spawn_lock, Dir, Out};
use std::time::Duration;

const REQUIRED: [&str; 6] = [
    "apiVersion",
    "kind",
    "schemaVersion",
    "project",
    "owner",
    "applications",
];

const FAKE_DEPLOY_KIT: &str = r#"
[ "$1" = validate ] && [ $# -ge 2 ] || { echo "E_USAGE: deploy-kit validate <paths>" >&2; exit 2; }
shift
refused=0
for f in "$@"; do
  case "$f" in
    *.project.yml)
      [ -f "$f" ] || continue
      for k in apiVersion kind schemaVersion project owner applications; do
        grep -q "^$k:" "$f" || { echo "E_REQUIRED_FIELD: $f: missing required field '$k'"; refused=1; }
      done ;;
  esac
done
exit $refused
"#;

fn scaffold_args(out: &str, overrides: &[(&str, &str)]) -> Vec<String> {
    let mut values = vec![
        ("--project", "notes"),
        ("--owner", "team-notes"),
        ("--host", "notes.example.dev"),
        ("--audience", "authenticated"),
        ("--image", "notes-api"),
        ("--process", "notes-api"),
        ("--port", "8080"),
    ];
    for (flag, value) in overrides {
        values.iter_mut().find(|(f, _)| f == flag).unwrap().1 = value;
    }
    let mut args = vec![
        "intent".to_string(),
        "scaffold".to_string(),
        "--out".to_string(),
        out.to_string(),
    ];
    for (flag, value) in values {
        args.push(flag.to_string());
        args.push(value.to_string());
    }
    args
}

fn call(args: &[String], path: &str, timeout: Option<Duration>) -> Out {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_in(&refs, "", path, timeout, None)
}

fn scaffolded() -> Dir {
    let dir = Dir::new();
    let out = call(&scaffold_args(&dir.str(), &[]), "", None);
    assert_eq!(out.code, 0, "{}", out.err);
    dir
}

fn with_tool(script: &str) -> (Dir, String) {
    let bin = Dir::new();
    bin.tool("bin/deploy-kit", script);
    let path = bin.join("bin").to_string_lossy().into_owned();
    (bin, path)
}

#[test]
fn scaffold_writes_the_project_file_and_base_env() {
    let dir = scaffolded();
    let text = std::fs::read_to_string(dir.join("deploy/notes.project.yml")).unwrap();
    for field in REQUIRED {
        assert!(
            text.contains(&format!("\n{field}:")) || text.starts_with(field),
            "{field}"
        );
    }
    for line in [
        "project: notes\n",
        "owner: team-notes\n",
        "  - id: notes\n",
        "        audience: authenticated\n",
        "          - { path: /, match: prefix, process: notes-api, surface: http }\n",
        "      - name: notes-api\n",
        "        image: notes-api\n",
        "          http: 8080\n",
        "          readiness: { path: /healthz/ready, port: 8080 }  # CHECK\n",
    ] {
        assert!(text.contains(line), "{line}");
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("deploy/env/notes-api/base.env")).unwrap(),
        "# Environment for the notes-api Process. See spec/v1/10-project-intent.md.\n"
    );
}

#[test]
fn scaffold_marks_the_author_choices_and_keeps_comments_to_pointers() {
    let dir = scaffolded();
    let text = std::fs::read_to_string(dir.join("deploy/notes.project.yml")).unwrap();
    assert!(text.matches("CHECK").count() >= 6);
    assert!(text.contains("runtime: node    # CHECK"));
    assert!(text.contains("spec/v1/10-project-intent.md"));
    assert!(
        text.lines()
            .filter(|l| l.trim_start().starts_with('#'))
            .count()
            <= 4
    );
    assert!(!text.contains("Requirements only"));
}

#[test]
fn scaffold_prints_what_it_wrote() {
    let dir = Dir::new();
    let out = call(&scaffold_args(&dir.str(), &[]), "", None);
    let root = dir.str();
    assert_eq!(
        out.out.lines().collect::<Vec<_>>(),
        [
            format!("wrote {root}/deploy/notes.project.yml"),
            format!("wrote {root}/deploy/env/notes-api/base.env")
        ]
    );
}

#[test]
fn scaffold_refuses_to_overwrite_and_leaves_the_files_alone() {
    let dir = scaffolded();
    let project = dir.join("deploy/notes.project.yml");
    std::fs::write(&project, "edited by the owner\n").unwrap();
    let out = call(&scaffold_args(&dir.str(), &[]), "", None);
    assert_eq!(out.code, 2);
    assert!(out
        .err
        .contains(&format!("refusing to overwrite {}", project.display())));
    assert!(out.err.contains("base.env"));
    assert_eq!(
        std::fs::read_to_string(&project).unwrap(),
        "edited by the owner\n"
    );
}

#[test]
fn scaffold_refuses_bad_values_and_writes_nothing() {
    let cases: [(&str, &str, &str); 14] = [
        (
            "--project",
            "Notes_App",
            "--project must be a DNS-1123 label",
        ),
        ("--process", "Bad", "--process must be a DNS-1123 label"),
        (
            "--audience",
            "everyone",
            "--audience must be one of anonymous, authenticated, internal, lan",
        ),
        ("--host", "a: b", "--host must be a lowercase DNS name"),
        (
            "--host",
            "Notes.example.dev",
            "--host must be a lowercase DNS name",
        ),
        (
            "--host",
            "bad_host.dev",
            "--host must be a lowercase DNS name",
        ),
        ("--host", "a..b", "--host must be a lowercase DNS name"),
        ("--owner", "team\nnotes", "--owner must be letters, digits"),
        ("--owner", "a: b", "--owner must be letters, digits"),
        ("--image", "img #x", "--image must be an image alias"),
        ("--image", "Img\nx", "--image must be an image alias"),
        ("--port", "0", "--port must be 1-65535"),
        ("--port", "70000", "--port must be 1-65535"),
        (
            "--project",
            "",
            "--project must be a DNS-1123 label, got ''",
        ),
    ];
    for (flag, value, message) in cases {
        let dir = Dir::new();
        let target = dir.join("repo");
        let out = call(
            &scaffold_args(target.to_str().unwrap(), &[(flag, value)]),
            "",
            None,
        );
        assert_eq!(out.code, 2, "{flag} {value}");
        assert!(out.err.contains(message), "{}", out.err);
        assert!(!target.exists());
    }
}

#[test]
fn the_value_rules_match_their_patterns_at_the_edges() {
    assert!(
        dns_label("a")
            && dns_label(&format!("a{}", "b".repeat(62)))
            && !dns_label(&format!("a{}", "b".repeat(63)))
    );
    assert!(!dns_label("a-") && !dns_label("1a") && dns_label("a-1") && !dns_label(""));
    assert!(host("a") && host("a.b-c.d") && !host("-a") && !host("a-") && !host(""));
    assert!(host(&["a"; 127].join(".")) && !host(&["a"; 128].join(".")));
    assert!(
        owner("A") && owner("a_b.c-d") && !owner("_a") && !owner("") && !owner(&"a".repeat(64))
    );
    assert!(image("a") && image("reg/img_1.x-y") && !image("-a") && !image("a-") && !image(""));
    assert!(image(&"a".repeat(200)) && !image(&"a".repeat(201)));
}

#[test]
fn repr_quotes_like_python() {
    assert_eq!(repr("a"), "'a'");
    assert_eq!(repr("it's"), "\"it's\"");
    assert_eq!(repr("a\"b'c"), "'a\"b\\'c'");
    assert_eq!(repr("a\\b\nc\rd\te"), "'a\\\\b\\nc\\rd\\te'");
}

#[test]
fn a_port_that_is_not_a_number_is_a_usage_error() {
    let dir = Dir::new();
    let out = call(&scaffold_args(&dir.str(), &[("--port", "http")]), "", None);
    assert_eq!(out.code, 2);
    assert!(out
        .err
        .contains("argument --port: invalid int value: 'http'"));
    assert!(!dir.join("deploy").exists());
    let out = call(
        &scaffold_args(&dir.str(), &[("--port", " +8081 ")]),
        "",
        None,
    );
    assert_eq!(out.code, 0, "{}", out.err);
}

#[test]
fn scaffold_without_all_flags_is_a_usage_error() {
    let out = run_plain(&["intent", "scaffold", "--project", "notes"]);
    assert_eq!(out.code, 2);
    assert!(out
        .err
        .contains("the following arguments are required: --owner, --host"));
}

#[test]
fn scaffold_into_an_unwritable_place_exits_2_with_the_message() {
    let dir = Dir::new();
    dir.write("repo/deploy", "a file where the directory should be\n");
    let out = call(
        &scaffold_args(dir.join("repo").to_str().unwrap(), &[]),
        "",
        None,
    );
    assert_eq!(out.code, 2);
    assert!(out
        .err
        .contains("estate intent: cannot write the scaffold:"));
}

#[test]
fn scaffold_creates_files_exclusively_even_when_the_check_is_raced() {
    // A dangling symlink is invisible to exists() but blocks exclusive creation.
    let dir = Dir::new();
    std::fs::create_dir_all(dir.join("deploy")).unwrap();
    let victim = dir.join("victim");
    std::os::unix::fs::symlink(&victim, dir.join("deploy/notes.project.yml")).unwrap();
    let out = call(&scaffold_args(&dir.str(), &[]), "", None);
    assert_eq!(out.code, 2);
    assert!(out.err.contains("cannot write the scaffold"));
    assert!(!victim.exists());
}

#[test]
fn a_scaffolded_intent_is_accepted_by_deploy_kit() {
    let _guard = spawn_lock();
    let dir = scaffolded();
    let (_bin, path) = with_tool(FAKE_DEPLOY_KIT);
    let project = dir.join("deploy/notes.project.yml");
    let env = dir.join("deploy/env");
    let args = [
        "intent",
        "validate",
        project.to_str().unwrap(),
        env.to_str().unwrap(),
    ];
    let out = run_in(&args, "", &path, None, None);
    assert_eq!(out.code, 0, "{}{}", out.out, out.err);
    assert!(out.out.contains("deploy-kit validate: accepted (exit 0)"));
}

#[test]
fn a_missing_required_field_is_refused_with_deploy_kits_text() {
    let _guard = spawn_lock();
    let (_bin, path) = with_tool(FAKE_DEPLOY_KIT);
    for field in REQUIRED {
        let dir = scaffolded();
        let text = std::fs::read_to_string(dir.join("deploy/notes.project.yml")).unwrap();
        let kept: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with(&format!("{field}:")))
            .collect();
        let broken = dir.write("deploy/broken.project.yml", &kept.join("\n"));
        let broken = broken.to_str().unwrap();
        let out = run_in(&["intent", "validate", broken], "", &path, None, None);
        assert_eq!(out.code, 1, "{field}");
        assert!(out.out.contains(&format!(
            "E_REQUIRED_FIELD: {broken}: missing required field '{field}'"
        )));
        assert!(out.out.contains("REFUSED (exit 1)"));
    }
}

#[test]
fn validate_without_deploy_kit_on_path_exits_2_and_validates_nothing() {
    let dir = scaffolded();
    let out = run_plain(&["intent", "validate", dir.join("deploy").to_str().unwrap()]);
    assert_eq!(out.code, 2);
    assert!(out.err.contains("deploy-kit is not installed on PATH"));
    assert!(out.err.contains("Nothing was validated"));
}

#[test]
fn validate_names_a_path_that_does_not_exist() {
    let dir = scaffolded();
    let gone = dir.join("nope.project.yml");
    let out = run_plain(&["intent", "validate", gone.to_str().unwrap()]);
    assert_eq!(out.code, 2);
    assert!(out
        .err
        .contains(&format!("no such path: {}", gone.display())));
    let extra = dir.join("gone");
    let out = run_plain(&[
        "intent",
        "validate",
        dir.join("deploy").to_str().unwrap(),
        extra.to_str().unwrap(),
    ]);
    assert!(out
        .err
        .contains(&format!("no such path: {}", extra.display())));
}

#[test]
fn validate_times_out_and_exits_2() {
    let _guard = spawn_lock();
    let dir = scaffolded();
    let (_bin, path) = with_tool("exec sleep 30");
    let deploy = dir.join("deploy");
    let args = ["intent", "validate", deploy.to_str().unwrap()];
    let out = run_in(&args, "", &path, Some(Duration::from_millis(200)), None);
    assert_eq!(out.code, 2);
    assert!(out.err.contains("did not finish within"));
    assert!(out.err.contains("Nothing was validated"));
}

#[test]
fn validate_reports_a_deploy_kit_that_cannot_be_run() {
    let _guard = spawn_lock();
    let dir = scaffolded();
    let bin = Dir::new();
    let tool = bin.write("bin/deploy-kit", "#!/nonexistent/interpreter\n");
    std::fs::set_permissions(&tool, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = bin.join("bin").to_string_lossy().into_owned();
    let out = run_in(
        &["intent", "validate", dir.join("deploy").to_str().unwrap()],
        "",
        &path,
        None,
        None,
    );
    assert_eq!(out.code, 2);
    assert!(out.err.contains("cannot run deploy-kit"));
}

#[test]
fn the_fake_deploy_kit_exits_2_without_paths() {
    let _guard = spawn_lock();
    let (bin, _path) = with_tool(FAKE_DEPLOY_KIT);
    let bare = std::process::Command::new(bin.join("bin/deploy-kit"))
        .arg("validate")
        .output()
        .unwrap();
    assert_eq!(bare.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bare.stderr).contains("E_USAGE"));
}

#[test]
fn validate_mirrors_the_exit_status_and_passes_the_output_through() {
    let _guard = spawn_lock();
    let dir = scaffolded();
    let cases = [
        (0, 0, "accepted (exit 0)"),
        (1, 1, "REFUSED (exit 1)"),
        (2, 2, "called wrongly (exit 2)"),
        (3, 2, "unexpected exit 3"),
    ];
    for (code, returned, line) in cases {
        let (_bin, path) = with_tool(&format!(
            "echo deploy-kit-said-this\necho and-this >&2\nexit {code}"
        ));
        let out = run_in(
            &["intent", "validate", dir.join("deploy").to_str().unwrap()],
            "",
            &path,
            None,
            None,
        );
        assert_eq!(out.code, returned);
        assert_eq!(out.out.lines().next(), Some("deploy-kit-said-this"));
        assert!(out.out.contains(line));
        assert_eq!(out.err, "and-this\n");
    }
}

#[test]
fn a_deploy_kit_killed_by_a_signal_is_an_unexpected_exit() {
    let _guard = spawn_lock();
    let dir = scaffolded();
    let (_bin, path) = with_tool("kill -9 $$");
    let out = run_in(
        &["intent", "validate", dir.join("deploy").to_str().unwrap()],
        "",
        &path,
        None,
        None,
    );
    assert_eq!(out.code, 2);
    assert!(out.out.contains("unexpected exit -9"));
}

#[test]
fn the_intent_command_needs_a_known_subcommand() {
    assert_eq!(run_plain(&["intent"]).code, 2);
    assert_eq!(run_plain(&["intent", "bogus"]).code, 2);
    let out = run_plain(&["intent", "--help"]);
    assert_eq!(
        (out.code, out.out.as_str()),
        (0, "usage: estate intent {scaffold,validate} ...\n")
    );
    assert_eq!(run_plain(&["intent", "validate"]).code, 2);
    assert_eq!(run_plain(&["intent", "validate", "--help"]).code, 0);
}
