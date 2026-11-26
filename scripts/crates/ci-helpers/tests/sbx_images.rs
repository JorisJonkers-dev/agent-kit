//! The Docker Sandboxes template images build from images/sbx/<client>/Dockerfile (ported from
//! tests/test_sbx_images.py, which the Rust migration guard refuses).
//!
//! `scripts/build-sbx-images.sh` tags each image `agent-kit/<client>:<version>` (the Dockerfile's
//! `ARG VERSION=`) and `:current`. The Pi image's pins must equal the ones
//! installer/setup-workstation.sh installs, so a pin bump in setup without the image fails here.
//! Docker is replaced by a stub that records its arguments; no image is built.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const DOCKER_STUB: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$SBX_TEST_DOCKER_LOG"
"#;

fn root() -> PathBuf {
    fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
        .expect("repository root resolves")
}

fn script() -> PathBuf {
    root().join("scripts/build-sbx-images.sh")
}

fn setup() -> PathBuf {
    root().join("installer/setup-workstation.sh")
}

fn pi_dockerfile() -> PathBuf {
    root().join("images/sbx/pi/Dockerfile")
}

fn opencode_dockerfile() -> PathBuf {
    root().join("images/sbx/opencode/Dockerfile")
}

/// A scratch directory named for one test, removed when it goes out of scope.
struct Scratch(PathBuf);

impl Scratch {
    fn new(test: &str) -> Scratch {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "ci-helpers-sbx-{}-{test}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A bin directory holding a `docker` stub, the PATH that finds it, and the log the stub writes.
fn stub_path(scratch: &Path) -> (String, PathBuf) {
    let bin = scratch.join("bin");
    fs::create_dir_all(&bin).expect("bin dir");
    let docker = bin.join("docker");
    fs::write(&docker, DOCKER_STUB).expect("docker stub");
    fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).expect("chmod stub");
    (format!("{}:/usr/bin:/bin", bin.display()), scratch.join("docker.log"))
}

struct Output {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str], path: &str, log: &Path) -> Output {
    let home = log.parent().expect("log has a parent");
    let out = Command::new("/bin/sh")
        .arg(script())
        .args(args)
        .env_clear()
        .env("PATH", path)
        .env("SBX_TEST_DOCKER_LOG", log)
        .env("HOME", home)
        .output()
        .expect("run /bin/sh build-sbx-images.sh");
    Output {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8(out.stdout).expect("utf-8 stdout"),
        stderr: String::from_utf8(out.stderr).expect("utf-8 stderr"),
    }
}

/// The value of `ARG <name>=<value>` in a Dockerfile, the value a whole line with no spaces.
fn arg(dockerfile: &Path, name: &str) -> String {
    let text = read(dockerfile);
    let prefix = format!("ARG {name}=");
    text.lines()
        .filter_map(|line| line.strip_prefix(&prefix))
        .find(|value| !value.is_empty() && !value.contains(char::is_whitespace))
        .unwrap_or_else(|| panic!("{} has no ARG {name}=", dockerfile.display()))
        .to_string()
}

/// The first token after `keyword` on a line that starts with it, as `^KEYWORD\s+(\S+)` reads.
fn first_token_after<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    rest.split_whitespace().next()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every value written as `"<needle><value>"` in `text`, where the value has no quote or space.
fn quoted_pins(text: &str, needle: &str) -> BTreeSet<String> {
    text.match_indices(needle)
        .filter_map(|(at, _)| {
            let rest = &text[at + needle.len()..];
            let end = rest.find(|c: char| c == '"' || c.is_whitespace())?;
            (end > 0 && rest[end..].starts_with('"')).then(|| rest[..end].to_string())
        })
        .collect()
}

fn check_pinned_non_root(dockerfile: &Path) {
    let text = read(dockerfile);
    let froms: Vec<&str> = text
        .lines()
        .filter_map(|line| first_token_after(line, "FROM"))
        .collect();
    assert_eq!(froms, ["${NODE_IMAGE}"], "{}", dockerfile.display());

    let node_image = arg(dockerfile, "NODE_IMAGE");
    assert!(
        node_image.starts_with("node:") && !node_image.ends_with(":latest"),
        "{}: NODE_IMAGE is {node_image}",
        dockerfile.display()
    );

    let users: Vec<&str> = text
        .lines()
        .filter_map(|line| first_token_after(line, "USER"))
        .collect();
    let last = users
        .last()
        .unwrap_or_else(|| panic!("{} has no USER line", dockerfile.display()));
    assert!(
        *last != "root" && *last != "0",
        "{} ends as {last}",
        dockerfile.display()
    );
    assert!(
        !text.contains(":latest"),
        "{} names a :latest tag",
        dockerfile.display()
    );
}

#[test]
fn sbx_dry_run_prints_tags_and_contexts_and_builds_nothing() {
    let scratch = Scratch::new("dry-run");
    let (path, log) = stub_path(&scratch.0);
    let result = run(&["--dry-run"], &path, &log);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(!log.exists(), "dry run must not call docker");
    let lines: Vec<&str> = result.stdout.lines().collect();
    let pi_version = arg(&pi_dockerfile(), "VERSION");
    let oc_version = arg(&opencode_dockerfile(), "VERSION");
    let pi_line = lines
        .iter()
        .find(|line| line.contains("agent-kit/pi:"))
        .expect("a pi build line");
    let oc_line = lines
        .iter()
        .find(|line| line.contains("agent-kit/opencode:"))
        .expect("an opencode build line");
    let pi_dir = pi_dockerfile().parent().unwrap().to_path_buf();
    let oc_dir = opencode_dockerfile().parent().unwrap().to_path_buf();
    assert!(pi_line.contains(&format!("-t agent-kit/pi:{pi_version} -t agent-kit/pi:current")));
    assert!(pi_line.ends_with(&pi_dir.display().to_string()));
    assert!(pi_line.contains(&format!("-f {}", pi_dockerfile().display())));
    assert!(oc_line.contains(&format!(
        "-t agent-kit/opencode:{oc_version} -t agent-kit/opencode:current"
    )));
    assert!(oc_line.ends_with(&oc_dir.display().to_string()));
}

#[test]
fn sbx_client_flag_builds_only_that_client() {
    let scratch = Scratch::new("client-flag");
    let (path, log) = stub_path(&scratch.0);
    let result = run(&["--dry-run", "--client", "pi"], &path, &log);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("agent-kit/pi:"));
    assert!(!result.stdout.contains("agent-kit/opencode:"));
}

#[test]
fn sbx_real_run_calls_docker_build_with_both_tags() {
    let scratch = Scratch::new("real-run");
    let (path, log) = stub_path(&scratch.0);
    let result = run(&["--client", "opencode"], &path, &log);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let oc_version = arg(&opencode_dockerfile(), "VERSION");
    let oc_dir = opencode_dockerfile().parent().unwrap().to_path_buf();
    let calls = read(&log);
    let calls: Vec<&str> = calls.lines().collect();
    assert_eq!(
        calls,
        [format!(
            "build -f {} -t agent-kit/opencode:{oc_version} -t agent-kit/opencode:current {}",
            opencode_dockerfile().display(),
            oc_dir.display()
        )]
    );
}

#[test]
fn sbx_missing_docker_fails_with_message() {
    let scratch = Scratch::new("no-docker");
    let empty = scratch.0.join("empty");
    fs::create_dir_all(&empty).expect("empty dir");
    let result = run(&[], &empty.display().to_string(), &scratch.0.join("docker.log"));

    assert_ne!(result.code, 0);
    assert!(
        result.stderr.contains("docker is not on PATH"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn sbx_unknown_client_is_bad_usage() {
    let scratch = Scratch::new("unknown-client");
    let (path, log) = stub_path(&scratch.0);
    let result = run(&["--client", "nope"], &path, &log);

    assert_eq!(result.code, 2);
    assert!(
        result.stderr.contains("no image for client 'nope'"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn sbx_pi_pins_match_setup() {
    let setup_text = read(&setup());
    let pi_pins = quoted_pins(&setup_text, "\"@earendil-works/pi-coding-agent@");
    let acp_pins = quoted_pins(&setup_text, "\"pi-acp@");
    let pi_version = arg(&pi_dockerfile(), "VERSION");
    let acp_version = arg(&pi_dockerfile(), "PI_ACP_VERSION");
    assert_eq!(pi_pins, BTreeSet::from([pi_version]));
    assert_eq!(acp_pins, BTreeSet::from([acp_version]));
}

#[test]
fn sbx_pi_image_is_pinned_and_runs_as_non_root() {
    check_pinned_non_root(&pi_dockerfile());
}

#[test]
fn sbx_opencode_image_is_pinned_and_runs_as_non_root() {
    check_pinned_non_root(&opencode_dockerfile());
}

#[test]
fn sbx_build_script_is_posix_sh() {
    let text = read(&script());
    assert_eq!(text.lines().next(), Some("#!/bin/sh"));
    let mode = fs::metadata(script()).expect("script metadata").permissions().mode();
    assert_ne!(mode & 0o111, 0, "build-sbx-images.sh is not executable");
}
