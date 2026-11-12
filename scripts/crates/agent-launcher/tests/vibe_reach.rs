//! The Vibe reach-level cells, proven on each OS (spec #392, story 51; ticket
//! B1-16).
//!
//! For each reach level and for both Vibe agents (`vibe`, and `vibe-study`,
//! which runs the same CLI under the Study account) this starts the real
//! launcher binary, under the real operating-system write limit (Landlock on
//! Linux, `sandbox-exec` on macOS), on the agent's entry of the real support
//! record (`registry/reach-levels.yaml`) with only its command swapped for a
//! stub. The stub is a shell script, not Vibe: it never reads `~/.vibe` or any
//! key (`HOME` is a scratch directory). It reports the arguments and reach it
//! was started with and tries to write to each directory a probe names.
//!
//! A supported level checks that the record's tool arguments reach the
//! process exactly, that they hold the level (tool list, approval mode), that
//! the writes the OS allows are exactly ADR-0013's table, and that the cell
//! for this OS is `verified` and cites the test. A level Vibe cannot hold
//! (`read-web`, `research`, `run-one-command`) is recorded `unsupported` with
//! its reason, and the launcher refuses it before starting anything.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeSet;
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
const AGENTS: [&str; 2] = ["vibe", "vibe-study"];
/// The levels Vibe cannot hold, and a word each reason must contain.
const REFUSED: [(&str, &str); 3] = [
    ("read-web", "web_search"),
    ("research", "web_search"),
    ("run-one-command", "pre-approves"),
];
/// Every Vibe tool whose enabling widens what an agent may do.
const POWERFUL: [&str; 5] = ["bash", "edit", "write_file", "web_fetch", "web_search"];

/// Prints the reach and each argument up to `::`, then tries to create each
/// path after it. `denied` is the operating system's own refusal.
const STUB: &str = r#"echo "reach=$AGENT_KIT_REACH"
n=0
for a in "$@"; do
  [ "$a" = "::" ] && break
  echo "arg=$a"
  n=$((n + 1))
done
shift $((n + 1))
for p in "$@"; do
  if out=$( ( : > "$p" ) 2>&1 ); then
    echo "wrote $p"
  else
    case "$out" in
      *"Permission denied"*|*"Operation not permitted"*) echo "denied $p" ;;
      *) echo "failed $p: $out" ;;
    esac
  fi
done
"#;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

/// The tools a level enables, from the table the ADR and docs/RUST.md give
/// (`None` for `full`, which sets no tool list).
fn tools_of(level: &str) -> Option<&'static [&'static str]> {
    Some(match level {
        "read" => &["read_file", "grep"],
        "write-scoped" => &["read_file", "grep", "write_file", "edit"],
        "write" => &["read_file", "grep", "write_file", "edit", "bash"],
        _ => return None,
    })
}

struct Fixture {
    base: PathBuf,
    record: PathBuf,
    worktree: PathBuf,
    main: PathBuf,
    state: PathBuf,
    output: PathBuf,
    gate: PathBuf,
    outside: PathBuf,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.test",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env_remove("GIT_DIR")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A scratch tree: a git repository with a linked worktree (the launcher's
/// working directory), directories for state, output and gate output, one
/// outside all of them, and the record the launcher reads.
fn fixture(agent: &str, level: &str) -> Fixture {
    let base = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!("vibe-reach-{level}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let main = base.join("main");
    let worktree = base.join("w");
    let (state, output, gate, outside) = (
        base.join("state"),
        base.join("out"),
        base.join("gate"),
        base.join("outside"),
    );
    for dir in [
        &main,
        &base.join("home"),
        &base.join("tmp"),
        &state,
        &outside,
    ] {
        fs::create_dir_all(dir).unwrap();
    }
    git(&main, &["init", "-q"]);
    git(
        &main,
        &["commit", "-q", "--allow-empty", "--no-verify", "-m", "base"],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().unwrap(),
            "-b",
            "feature",
        ],
    );

    // The real record, with the Claude command swapped for the stub, and one id
    // for each level on this OS.
    let stub = base.join("stub.sh");
    fs::write(&stub, STUB).unwrap();
    let text = fs::read_to_string(repo_root().join("registry/reach-levels.yaml")).unwrap();
    let mut record: serde_yaml::Value = serde_yaml::from_str(&text).unwrap();
    record["agents"][agent]["command"] =
        serde_yaml::from_str(&format!("[/bin/sh, {}]", stub.display())).unwrap();
    let host = agent_launcher::this_os();
    let mut ids = String::new();
    for l in LEVELS {
        ids += &format!("{l}-id: {{agent: {agent}, reach: {l}, hosts: [{host}]");
        if l == "research" || l == "write-scoped" {
            ids += &format!(", output_dir: {}", output.display());
        }
        ids += "}\n";
    }
    record["ids"] = serde_yaml::from_str(&ids).unwrap();
    let file = base.join("reach.yaml");
    fs::write(&file, serde_yaml::to_string(&record).unwrap()).unwrap();
    Fixture {
        base,
        record: file,
        worktree,
        main,
        state,
        output,
        gate,
        outside,
    }
}

/// Starts the level's id through the launcher, as its own process, with the
/// probes after `--`, and returns its output.
fn launch(f: &Fixture, level: &str, probes: &[PathBuf]) -> Output {
    Command::new(LAUNCHER)
        .args([
            "run".as_ref(),
            f.record.as_os_str(),
            format!("{level}-id").as_ref(),
            "--".as_ref(),
            "::".as_ref(),
        ])
        .args(probes)
        .current_dir(&f.worktree)
        .env("HOME", f.base.join("home"))
        .env("XDG_CONFIG_HOME", f.base.join("home/.config"))
        .env("TMPDIR", f.base.join("tmp"))
        .env("AGENT_KIT_STATE_DIRS", &f.state)
        .env("AGENT_KIT_GATE_DIR", &f.gate)
        .env_remove("GIT_DIR")
        .output()
        .unwrap()
}

/// The values of a repeated flag, `--flag=a --flag=b`.
fn flag_set(args: &[String], flag: &str) -> BTreeSet<String> {
    let prefix = format!("{flag}=");
    args.iter()
        .filter_map(|a| a.strip_prefix(&prefix))
        .map(String::from)
        .collect()
}

/// Proves one level on this OS.
fn prove(agent: &str, level: &str) {
    let f = fixture(agent, level);
    let hooks = f.main.join(".git/hooks/pre-commit");
    // Every directory a level might be granted, and two it never is.
    let probes = vec![
        f.state.join("p-state"),
        f.output.join("p-output"),
        f.gate.join("p-gate"),
        f.worktree.join("p-worktree"),
        f.outside.join("p-outside"),
        hooks.clone(),
    ];
    fs::create_dir_all(&f.output).unwrap();
    fs::create_dir_all(&f.gate).unwrap();
    let out = launch(&f, level, &probes);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the launcher exited {:?} for {agent} {level}: {stderr}\n{stdout}",
        out.status
    );

    // 1. The level's tool arguments, and its reach, reach the process.
    assert!(
        stdout.contains(&format!("reach={level}\n")),
        "reach: {stdout}"
    );
    let args: Vec<String> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("arg="))
        .map(String::from)
        .collect();
    let text = fs::read_to_string(repo_root().join("registry/reach-levels.yaml")).unwrap();
    let record = agent_launcher::parse(&text).unwrap();
    assert_eq!(
        args, record.agents[agent].levels[level].args,
        "the stub receives exactly the record's arguments"
    );
    let hold = |flag: &str| flag_set(&args, flag);
    match tools_of(level) {
        None => assert_eq!(args, ["--auto-approve"], "full sets no tool list"),
        Some(tools) => {
            let enabled: BTreeSet<String> = tools.iter().map(|t| t.to_string()).collect();
            assert_eq!(
                hold("--enabled-tools"),
                enabled,
                "tools for {level}: {args:?}"
            );
            let denied = hold("--disabled-tools");
            let powerful_off: BTreeSet<String> = POWERFUL
                .iter()
                .filter(|t| !enabled.contains(**t))
                .map(|t| t.to_string())
                .collect();
            assert_eq!(
                denied, powerful_off,
                "every powerful tool the level lacks is denied: {args:?}"
            );
            let approves_all = args.contains(&"--auto-approve".to_string());
            assert_eq!(
                approves_all,
                level == "write",
                "only write approves the shell: {args:?}"
            );
            let edits_only = args.contains(&"--agent=accept-edits".to_string());
            assert_eq!(
                edits_only,
                level == "write-scoped",
                "write-scoped approves edits only: {args:?}"
            );
            assert!(
                !args
                    .iter()
                    .any(|a| a.contains("--yolo") || a.contains("bypass")),
                "{args:?}"
            );
        }
    }

    // 2. The OS permits exactly ADR-0013's writes for the level.
    let named = |verb: &str| -> BTreeSet<String> {
        stdout
            .lines()
            .filter_map(|l| l.strip_prefix(verb))
            .map(|p| {
                p.strip_prefix(f.base.to_str().unwrap())
                    .unwrap_or(p)
                    .to_string()
            })
            .collect()
    };
    let (wrote, denied) = (named("wrote "), named("denied "));
    assert!(
        !stdout.contains("failed "),
        "a probe failed for a reason other than the OS refusing it: {stdout}"
    );
    let (state, output, gate, worktree, outside, hook) = (
        "/state/p-state",
        "/out/p-output",
        "/gate/p-gate",
        "/w/p-worktree",
        "/outside/p-outside",
        "/main/.git/hooks/pre-commit",
    );
    let expected: &[&str] = match level {
        "read" => &[state],
        "write-scoped" => &[state, output],
        "write" => &[state, worktree],
        _ => &[state, output, gate, worktree, outside, hook],
    };
    let all: BTreeSet<String> = [state, output, gate, worktree, outside, hook]
        .iter()
        .map(|p| p.to_string())
        .collect();
    let expected: BTreeSet<String> = expected.iter().map(|p| p.to_string()).collect();
    assert_eq!(wrote, expected, "written at {level}: {stdout}");
    assert_eq!(
        denied,
        all.difference(&wrote).cloned().collect::<BTreeSet<_>>(),
        "refused at {level}: {stdout}"
    );
    assert_eq!(
        hooks.exists(),
        level == "full",
        "the shared hooks directory is only writable at full"
    );

    // 3. The record's cell for this OS is verified and cites this very test.
    let host = agent_launcher::this_os();
    assert_eq!(
        agent_launcher::cell(&record, agent, level, host),
        "verified",
        "{level} on {host}"
    );
    let cited = &record.agents[agent].levels[level].check[host];
    let name = format!("vibe_reach_{}", level.replace('-', "_"));
    assert_eq!(
        cited,
        &format!("scripts/crates/agent-launcher/tests/vibe_reach.rs::{name}")
    );
    let _ = fs::remove_dir_all(&f.base);
}

macro_rules! level {
    ($name:ident, $level:literal) => {
        #[test]
        fn $name() {
            for agent in AGENTS {
                prove(agent, $level);
            }
        }
    };
}

level!(vibe_reach_read, "read");
level!(vibe_reach_write_scoped, "write-scoped");
level!(vibe_reach_write, "write");
level!(vibe_reach_full, "full");

/// A level Vibe cannot hold is recorded unsupported on each OS with its
/// reason, never verified, and the launcher refuses to start it.
#[test]
fn vibe_reach_refused_levels_say_why_and_start_nothing() {
    let text = fs::read_to_string(repo_root().join("registry/reach-levels.yaml")).unwrap();
    let record = agent_launcher::parse(&text).unwrap();
    for agent in AGENTS {
        for (level, word) in REFUSED {
            for os in agent_launcher::OSES {
                assert_eq!(
                    agent_launcher::cell(&record, agent, level, os),
                    "unsupported",
                    "{agent} {level} on {os}"
                );
                let reason = &record.agents[agent].levels[level].check[os];
                assert!(reason.contains(word), "{agent} {level} on {os}: {reason}");
            }
            let f = fixture(agent, level);
            let out = launch(&f, level, &[]);
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(!out.status.success(), "{agent} {level} must be refused");
            assert!(
                !stdout.contains("reach="),
                "nothing starts for {agent} {level}: {stdout}"
            );
            assert_eq!(out.status.code(), Some(1), "a refusal exits 1: {out:?}");
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains("nothing started"), "refusal message: {stderr}");
            let _ = fs::remove_dir_all(&f.base);
        }
    }
}

/// The record is clean for both Vibe agents, covers every level, and every
/// check it cites names a test in this file.
#[test]
fn vibe_reach_record_is_valid_and_cites_real_checks() {
    let text = fs::read_to_string(repo_root().join("registry/reach-levels.yaml")).unwrap();
    let record = agent_launcher::parse(&text).unwrap();
    let own: Vec<String> = agent_launcher::problems(&record)
        .into_iter()
        .filter(|p| p.starts_with("agent vibe"))
        .collect();
    assert_eq!(own, Vec::<String>::new());
    let me =
        fs::read_to_string(repo_root().join("scripts/crates/agent-launcher/tests/vibe_reach.rs"))
            .unwrap();
    for agent in AGENTS {
        let spec = &record.agents[agent];
        assert_eq!(spec.command, [agent]);
        assert_eq!(
            spec.levels
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            LEVELS.iter().copied().collect()
        );
        for (level, entry) in &spec.levels {
            for os in agent_launcher::OSES {
                let state = agent_launcher::cell(&record, agent, level, os);
                assert!(
                    ["verified", "unsupported"].contains(&state),
                    "{agent} {level} on {os} is {state}"
                );
                if state == "verified" {
                    let (file, test) = entry.check[os].split_once("::").expect("file::test");
                    assert!(repo_root().join(file).exists(), "{}", entry.check[os]);
                    assert!(
                        me.contains(&format!("level!({test},")),
                        "{} names a test in this file",
                        entry.check[os]
                    );
                }
            }
        }
    }
}
