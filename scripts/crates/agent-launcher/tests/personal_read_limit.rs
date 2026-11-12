//! The read limit of the personal Gold Band instance on the work Mac (spec
//! #392, ticket B1-17; ADR-0013, #396).
//!
//! The launcher is started as its own process, under the real operating-system
//! limit, with a per-machine overlay that declares an employer root (a scratch
//! directory here, never a real clone) and a stub agent that tries to read it.
//! The stub is a shell script. HOME is a scratch directory too, so no real
//! overlay, keep file or credential is read.
//!
//! On macOS a personal-instance agent, at every reach level including `full`,
//! cannot read, list, stat, link or move anything under the employer root,
//! and still reads its own repository, the toolchain and its own state
//! directory; a work-instance agent is not limited. This is the proof the
//! `macos` CI job runs. On Linux, which holds no employer clones, the launcher
//! refuses a declared root rather than start without the limit.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LAUNCHER: &str = env!("CARGO_BIN_EXE_agent-launcher");
const LEVELS: [&str; 7] = [
    "read",
    "read-web",
    "research",
    "write-scoped",
    "run-one-command",
    "write",
    "full",
];

/// Tries each probe `<kind>:<path>` and says what happened. `r` reads a file,
/// `l` lists a directory, `ln` hard-links `a|b`, `mv` moves `a|b`.
const STUB: &str = r#"for spec in "$@"; do
  p="${spec#*:}"
  case "$spec" in
    r:*)  if head -c 1 "$p" >/dev/null 2>&1; then echo "ok $spec"; else echo "refused $spec"; fi ;;
    l:*)  if ls "$p" >/dev/null 2>&1; then echo "ok $spec"; else echo "refused $spec"; fi ;;
    ln:*) if ln "${p%%|*}" "${p#*|}" 2>/dev/null; then echo "ok $spec"; else echo "refused $spec"; fi ;;
    mv:*) if mv "${p%%|*}" "${p#*|}" 2>/dev/null; then echo "ok $spec"; else echo "refused $spec"; fi ;;
  esac
done
"#;

struct Fixture {
    base: PathBuf,
    record: PathBuf,
    worktree: PathBuf,
    employer: PathBuf,
    state: PathBuf,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.test", "-c", "init.defaultBranch=main"])
        .args(args)
        .env_remove("GIT_DIR")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A scratch machine: an employer root with a secret in it, a personal
/// repository (a linked worktree, which `write` needs), a state directory,
/// the overlay naming the employer root, and a record with one stub agent
/// for the personal instance and one for the work instance, each at every level.
fn fixture(name: &str, overlay: &str) -> Fixture {
    let base = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("personal-read-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let (main, worktree) = (base.join("main"), base.join("w"));
    let (employer, state) = (base.join("employer"), base.join("state"));
    for dir in [&main, &employer, &state, &base.join("home/.config/agent-kit"), &base.join("tmp")] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(employer.join("secret.txt"), "employer code\n").unwrap();
    fs::write(state.join("config.json"), "{}\n").unwrap();
    git(&main, &["init", "-q"]);
    fs::write(main.join("README"), "personal\n").unwrap();
    git(&main, &["add", "README"]);
    git(&main, &["commit", "-q", "--no-verify", "-m", "base"]);
    git(&main, &["worktree", "add", "-q", worktree.to_str().unwrap(), "-b", "feature"]);
    fs::write(
        base.join("home/.config/agent-kit/overlay.yaml"),
        overlay.replace("EMPLOYER", employer.to_str().unwrap()),
    )
    .unwrap();

    let stub = base.join("stub.sh");
    fs::write(&stub, STUB).unwrap();
    let host = agent_launcher::this_os();
    let mut record = String::from("agents:\n");
    let mut ids = String::from("ids:\n");
    for agent in ["personal-stub", "claude-work"] {
        record += &format!("  {agent}:\n    command: [/bin/sh, {}]\n    levels:\n", stub.display());
        for l in LEVELS {
            record += &format!("      {l}: {{support: {{linux: verified, mac: verified}}}}\n");
            ids += &format!("  {agent}-{l}: {{agent: {agent}, reach: {l}, hosts: [{host}]");
            if l == "research" || l == "write-scoped" {
                ids += &format!(", output_dir: {}", base.join("out").display());
            }
            ids += "}\n";
        }
    }
    let file = base.join("reach.yaml");
    fs::write(&file, record + &ids).unwrap();
    Fixture { base, record: file, worktree, employer, state }
}

const OVERLAY: &str = "instances:\n  gold-band:\n    employer_roots: [EMPLOYER]\n";

fn launcher(f: &Fixture, args: &[&str]) -> Command {
    let mut command = Command::new(LAUNCHER);
    command
        .args(args)
        .current_dir(&f.worktree)
        .env("HOME", f.base.join("home"))
        .env("XDG_CONFIG_HOME", f.base.join("home/.config"))
        .env("TMPDIR", f.base.join("tmp"))
        .env("AGENT_KIT_STATE_DIRS", &f.state)
        .env_remove("AGENT_KIT_OVERLAY")
        .env_remove("GIT_DIR");
    command
}

fn run(f: &Fixture, id: &str, probes: &[String]) -> Output {
    let mut args = vec!["run", f.record.to_str().unwrap(), id, "--"];
    args.extend(probes.iter().map(String::as_str));
    launcher(f, &args).output().unwrap()
}

fn say(out: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// What each probe did, `ok` or `refused`.
fn outcomes(out: &Output) -> Vec<String> {
    String::from_utf8_lossy(&out.stdout).lines().map(String::from).collect()
}

#[cfg(target_os = "macos")]
#[test]
fn a_personal_instance_launch_cannot_read_the_employer_root_at_any_reach_level() {
    for level in LEVELS {
        let f = fixture(level, OVERLAY);
        let secret = f.employer.join("secret.txt");
        fs::create_dir_all(f.base.join("out")).unwrap();
        std::os::unix::fs::symlink(&secret, f.worktree.join("alias")).unwrap();
        let probes: Vec<String> = [
            format!("r:{}", secret.display()),
            format!("l:{}", f.employer.display()),
            format!("r:{}", f.worktree.join("alias").display()),
            format!("ln:{}|{}", secret.display(), f.worktree.join("hard").display()),
            format!("mv:{}|{}", secret.display(), f.worktree.join("moved").display()),
            // What it still needs: its repository, the toolchain, its config.
            format!("r:{}", f.worktree.join("README").display()),
            format!("l:{}", f.worktree.display()),
            "r:/bin/sh".to_string(),
            "r:/usr/bin/true".to_string(),
            format!("r:{}", f.state.join("config.json").display()),
        ]
        .into();
        let out = run(&f, &format!("personal-stub-{level}"), &probes);
        assert!(out.status.success(), "{level}: {}", say(&out));
        let got = outcomes(&out);
        let want: Vec<String> = probes
            .iter()
            .enumerate()
            .map(|(n, p)| format!("{} {p}", if n < 5 { "refused" } else { "ok" }))
            .collect();
        assert_eq!(got, want, "{level}");
        assert_eq!(fs::read_to_string(&secret).unwrap(), "employer code\n", "{level}: left alone");
        assert!(!f.worktree.join("hard").exists() && !f.worktree.join("moved").exists(), "{level}");
        let _ = fs::remove_dir_all(&f.base);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn a_work_instance_agent_still_reads_the_employer_root() {
    let f = fixture("work", OVERLAY);
    let probe = format!("r:{}", f.employer.join("secret.txt").display());
    let out = run(&f, "claude-work-read", &[probe.clone()]);
    assert!(out.status.success(), "{}", say(&out));
    assert_eq!(outcomes(&out), [format!("ok {probe}")]);
    let _ = fs::remove_dir_all(&f.base);
}

#[cfg(target_os = "macos")]
#[test]
fn the_check_for_setup_passes_where_sandbox_exec_works() {
    let f = fixture("check", OVERLAY);
    let out = launcher(&f, &["read-limit-check"]).output().unwrap();
    assert!(out.status.success(), "{}", say(&out));
    assert!(say(&out).contains("read limit: ok, 1 employer root(s)"), "{}", say(&out));
    let _ = fs::remove_dir_all(&f.base);
}

#[cfg(target_os = "linux")]
#[test]
fn linux_refuses_a_declared_employer_root_instead_of_starting_without_the_limit() {
    let f = fixture("linux", OVERLAY);
    let secret = format!("r:{}", f.employer.join("secret.txt").display());
    for level in ["read", "full"] {
        let out = run(&f, &format!("personal-stub-{level}"), &[secret.clone()]);
        assert_eq!(out.status.code(), Some(1), "{level}: {}", say(&out));
        assert!(say(&out).contains("cannot deny reads"), "{}", say(&out));
        assert!(outcomes(&out).is_empty(), "the agent must not start: {}", say(&out));
    }
    let out = launcher(&f, &["read-limit-check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", say(&out));
    assert!(say(&out).contains("must not start"), "{}", say(&out));
    let _ = fs::remove_dir_all(&f.base);
}

#[test]
fn a_work_machine_overlay_without_roots_refuses_the_personal_instance() {
    let f = fixture("noroots", "instances:\n  gold-band: {}\n");
    let out = run(&f, "personal-stub-read", &["r:/bin/sh".into()]);
    assert_eq!(out.status.code(), Some(1), "{}", say(&out));
    assert!(say(&out).contains("declares no employer_roots"), "{}", say(&out));
    assert!(outcomes(&out).is_empty(), "the agent must not start: {}", say(&out));
    let out = launcher(&f, &["read-limit-check"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", say(&out));
    let _ = fs::remove_dir_all(&f.base);
}

#[test]
fn a_machine_without_a_work_instance_needs_no_read_limit() {
    let f = fixture("none", "instances: {}\n");
    let out = run(&f, "personal-stub-read", &["r:/bin/sh".into()]);
    assert!(out.status.success(), "{}", say(&out));
    assert_eq!(outcomes(&out), ["ok r:/bin/sh"]);
    let out = launcher(&f, &["read-limit-check"]).output().unwrap();
    assert!(out.status.success(), "{}", say(&out));
    let _ = fs::remove_dir_all(&f.base);
}
