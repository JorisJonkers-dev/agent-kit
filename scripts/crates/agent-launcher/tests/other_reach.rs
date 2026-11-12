//! The reach-level cells of Pi, OpenCode, Hermes and Antigravity, proven on
//! each OS (spec #392, story 51; ticket B1-17), in the style of
//! `claude_reach.rs`.
//!
//! For every level a verified cell starts the real launcher binary under the
//! real operating-system write limit (Landlock on Linux, `sandbox-exec` on
//! macOS), on that agent's entry of the real record with only its command
//! swapped for a stub shell script. The stub never reads a credential (`HOME`
//! is a scratch directory). A level the agent cannot express is refused: the
//! record says `unsupported` with the reason, and the launcher refuses an id
//! that names it rather than starting the agent at another level.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const LAUNCHER: &str = env!("CARGO_BIN_EXE_agent-launcher");
const FILE: &str = "scripts/crates/agent-launcher/tests/other_reach.rs";
const LEVELS: [&str; 7] = [
    "read",
    "read-web",
    "research",
    "write-scoped",
    "run-one-command",
    "write",
    "full",
];
const AGENTS: [&str; 4] = ["pi", "opencode", "hermes", "antigravity"];

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

fn record_text() -> String {
    fs::read_to_string(repo_root().join("registry/reach-levels.yaml")).unwrap()
}

/// The levels an agent holds on every OS (the rest are refused).
fn held(agent: &str) -> &'static [&'static str] {
    match agent {
        "pi" => &["read", "write-scoped", "write", "full"],
        "opencode" | "hermes" => &["full"],
        _ => &[],
    }
}

/// Pi's tool allow-list for a level it holds (`None` for `full`).
fn pi_tools(level: &str) -> Option<&'static [&'static str]> {
    match level {
        "read" => Some(&["read", "grep", "find", "ls"]),
        "write-scoped" => Some(&["read", "grep", "find", "ls", "write", "edit"]),
        "write" => Some(&["read", "grep", "find", "ls", "write", "edit", "bash"]),
        _ => None,
    }
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

/// A scratch tree plus the real record with `agent`'s command swapped for the
/// stub and one id per level on this OS.
fn fixture(agent: &str, level: &str) -> Fixture {
    let base = fs::canonicalize(std::env::temp_dir())
        .unwrap()
        .join(format!(
            "other-reach-{agent}-{level}-{}",
            std::process::id()
        ));
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
        &output,
        &gate,
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

    let stub = base.join("stub.sh");
    fs::write(&stub, STUB).unwrap();
    let mut record: serde_yaml::Value = serde_yaml::from_str(&record_text()).unwrap();
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

/// The `--tools` allow-list a Pi argument list passes (`--tools a,b`).
fn pi_tool_set(args: &[String]) -> BTreeSet<String> {
    let at = args.iter().position(|a| a == "--tools").expect("--tools");
    args[at + 1].split(',').map(String::from).collect()
}

/// Proves one verified cell on this OS.
fn prove(agent: &str, level: &str) {
    let f = fixture(agent, level);
    let hooks = f.main.join(".git/hooks/pre-commit");
    let probes = vec![
        f.state.join("p-state"),
        f.output.join("p-output"),
        f.gate.join("p-gate"),
        f.worktree.join("p-worktree"),
        f.outside.join("p-outside"),
        hooks.clone(),
    ];
    let out = launch(&f, level, &probes);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the launcher exited {:?} for {agent} {level}: {stderr}\n{stdout}",
        out.status
    );

    // 1. The level's arguments, and its reach, reach the process.
    assert!(
        stdout.contains(&format!("reach={level}\n")),
        "reach: {stdout}"
    );
    let args: Vec<String> = stdout
        .lines()
        .filter_map(|l| l.strip_prefix("arg="))
        .map(String::from)
        .collect();
    let record = agent_launcher::parse(&record_text()).unwrap();
    assert_eq!(
        args, record.agents[agent].levels[level].args,
        "the stub receives exactly the record's arguments"
    );
    match (agent, pi_tools(level)) {
        ("pi", Some(tools)) => {
            let enabled: BTreeSet<String> = tools.iter().map(|t| t.to_string()).collect();
            assert_eq!(
                pi_tool_set(&args),
                enabled,
                "pi tools for {level}: {args:?}"
            );
            assert!(
                args.contains(&"--no-mcp".to_string()),
                "no ambient MCP server: {args:?}"
            );
            assert!(
                args.contains(&"--no-extensions".to_string()),
                "no extension tool: {args:?}"
            );
            assert_eq!(
                enabled.contains("bash"),
                level == "write",
                "only write has a shell: {args:?}"
            );
        }
        ("pi", None) | ("hermes", _) => {
            assert!(args.is_empty(), "full sets no restriction: {args:?}")
        }
        ("opencode", _) => assert_eq!(
            args,
            ["--auto"],
            "opencode full auto-approves what is not denied"
        ),
        _ => unreachable!(),
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
    assert_eq!(wrote, expected, "written at {agent} {level}: {stdout}");
    assert_eq!(
        denied,
        all.difference(&wrote).cloned().collect::<BTreeSet<_>>(),
        "refused at {agent} {level}: {stdout}"
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
        "{agent} {level} on {host}"
    );
    let name = format!("{agent}_reach_{}", level.replace('-', "_"));
    assert_eq!(
        record.agents[agent].levels[level].check[host],
        format!("{FILE}::{name}")
    );
    let _ = fs::remove_dir_all(&f.base);
}

/// A refused cell: unsupported with its reason, and the launcher will not
/// start the agent at that level (nothing is downgraded).
fn refuse(agent: &str, level: &str) {
    let f = fixture(agent, level);
    let host = agent_launcher::this_os();
    let record = agent_launcher::parse(&record_text()).unwrap();
    assert_eq!(
        agent_launcher::cell(&record, agent, level, host),
        "unsupported",
        "{agent} {level} on {host}"
    );
    assert!(
        record.agents[agent].levels[level].check[host].len() > 40,
        "a real reason"
    );
    let out = launch(&f, level, &[f.state.join("p-state")]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "the launcher started {agent} at refused level {level}: {stdout}"
    );
    assert!(
        !stdout.contains("reach="),
        "the stub ran at refused level {level}: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("{agent} at {level} is unsupported")),
        "the refusal names the cell: {stderr}"
    );
    let _ = fs::remove_dir_all(&f.base);
}

macro_rules! verified {
    ($name:ident, $agent:literal, $level:literal) => {
        #[test]
        fn $name() {
            prove($agent, $level);
        }
    };
}

verified!(pi_reach_read, "pi", "read");
verified!(pi_reach_write_scoped, "pi", "write-scoped");
verified!(pi_reach_write, "pi", "write");
verified!(pi_reach_full, "pi", "full");
verified!(opencode_reach_full, "opencode", "full");
verified!(hermes_reach_full, "hermes", "full");

macro_rules! refused {
    ($name:ident, $agent:literal, $level:literal) => {
        #[test]
        fn $name() {
            refuse($agent, $level);
        }
    };
}

refused!(pi_refuses_read_web, "pi", "read-web");
refused!(pi_refuses_research, "pi", "research");
refused!(pi_refuses_run_one_command, "pi", "run-one-command");
refused!(opencode_refuses_read, "opencode", "read");
refused!(opencode_refuses_write, "opencode", "write");
refused!(hermes_refuses_read, "hermes", "read");
refused!(hermes_refuses_write, "hermes", "write");
refused!(antigravity_refuses_read, "antigravity", "read");
refused!(antigravity_refuses_write, "antigravity", "write");
refused!(antigravity_refuses_full, "antigravity", "full");

/// Every cell of these four agents is verified (citing a test in this file) or
/// refused (with its reason), on both OSes; the record validates clean for
/// them; and Antigravity's Linux reason names the missing Linux command.
#[test]
fn other_reach_record_is_valid_and_complete() {
    let record = agent_launcher::parse(&record_text()).unwrap();
    let own: Vec<String> = agent_launcher::problems(&record)
        .into_iter()
        .filter(|p| AGENTS.iter().any(|a| p.starts_with(&format!("agent {a} "))))
        .collect();
    assert_eq!(own, Vec::<String>::new());
    let me = fs::read_to_string(repo_root().join(FILE)).unwrap();
    for agent in AGENTS {
        let entry = &record.agents[agent];
        assert_eq!(
            entry
                .levels
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            LEVELS.iter().copied().collect()
        );
        for (level, cells) in &entry.levels {
            for os in agent_launcher::OSES {
                let state = agent_launcher::cell(&record, agent, level, os);
                let check = &cells.check[os];
                if held(agent).contains(&level.as_str()) {
                    assert_eq!(state, "verified", "{agent} {level} {os}");
                    let (file, test) = check.split_once("::").expect("file::test");
                    assert_eq!(file, FILE);
                    assert!(
                        me.contains(&format!("verified!({test},")),
                        "{check} names a test in this file"
                    );
                } else {
                    assert_eq!(
                        state, "unsupported",
                        "{agent} {level} {os}: refused, never unverified"
                    );
                    assert!(
                        !check.contains("::") && check.len() > 40,
                        "{agent} {level} {os}: a reason, got {check:?}"
                    );
                }
            }
        }
    }
    let mac = &record.agents["antigravity"].levels["read"].check["mac"];
    assert!(!mac.contains("Linux") && !mac.starts_with('('), "{mac}");
    let linux = &record.agents["antigravity"].levels["read"].check["linux"];
    assert!(!linux.starts_with('('), "{linux}");
    assert!(
        linux.contains("#431") && linux.contains("not declared for Linux"),
        "{linux}"
    );
}
