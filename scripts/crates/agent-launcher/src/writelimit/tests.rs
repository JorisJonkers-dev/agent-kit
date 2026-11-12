use super::*;
use crate::{run_with, Context, LEVELS};
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;

#[path = "../../../../test-support/fake_exe.rs"]
mod fake_exe;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agent-launcher-wl-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::canonicalize(dir).unwrap()
}

/// A linked worktree the way git lays it out: the shared git directory in
/// `main/.git` (with hooks and config), the worktree's own administration
/// directory under it, and a `.git` pointer file in the worktree.
struct Tree {
    root: PathBuf,
    worktree: PathBuf,
    common: PathBuf,
    admin: PathBuf,
}

fn tree(name: &str) -> Tree {
    let root = scratch(name);
    let common = root.join("main/.git");
    let admin = common.join("worktrees/w1");
    let worktree = root.join("w1");
    for dir in [
        common.join("objects"),
        common.join("refs/heads"),
        common.join("logs"),
        common.join("hooks"),
        admin.clone(),
        worktree.clone(),
    ] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(common.join("hooks/pre-commit"), "#!/bin/sh\n").unwrap();
    fs::write(common.join("config"), "[core]\n\tbare = false\n").unwrap();
    fs::write(admin.join("commondir"), "../..\n").unwrap();
    fs::write(admin.join("HEAD"), "ref: refs/heads/w1\n").unwrap();
    fs::write(
        admin.join("gitdir"),
        format!("{}\n", worktree.join(".git").display()),
    )
    .unwrap();
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();
    Tree {
        root,
        worktree,
        common,
        admin,
    }
}

fn scope(tree: &Tree) -> Scope {
    Scope {
        temp_dir: tree.root.join("tmp"),
        worktree: tree.worktree.clone(),
        ..Scope::default()
    }
}

fn refusal(level: &str, scope: &Scope) -> String {
    policy_for(level, scope).unwrap_err()
}

// The table.

#[test]
fn read_levels_may_write_only_state_and_the_temporary_directory() {
    let t = tree("read");
    let state = t.root.join("state");
    fs::create_dir_all(&state).unwrap();
    let mut s = scope(&t);
    s.state_dirs = vec![state.clone(), t.root.join("no-such-state")];
    for level in ["read", "read-web"] {
        let policy = policy_for(level, &s).unwrap().unwrap();
        assert_eq!(policy.dirs, vec![state.clone(), s.temp_dir.clone()]);
        assert!(policy.files.iter().all(|f| f.starts_with("/dev")));
    }
}

#[test]
fn the_output_levels_add_the_output_directory() {
    let t = tree("output");
    let mut s = scope(&t);
    assert!(refusal("research", &s).contains("needs an output directory"));
    s.output_dir = Some(t.root.join("out"));
    for level in ["research", "write-scoped"] {
        let policy = policy_for(level, &s).unwrap().unwrap();
        assert_eq!(policy.dirs, vec![s.temp_dir.clone(), t.root.join("out")]);
    }
}

#[test]
fn run_one_command_adds_the_gate_output_directory() {
    let t = tree("gate");
    let mut s = scope(&t);
    assert!(refusal("run-one-command", &s).contains("needs a gate-output directory"));
    s.gate_dir = Some(t.root.join("gate"));
    let policy = policy_for("run-one-command", &s).unwrap().unwrap();
    assert_eq!(policy.dirs, vec![s.temp_dir.clone(), t.root.join("gate")]);
}

#[test]
fn write_adds_the_worktree_but_never_the_shared_hooks_or_config() {
    let t = tree("write");
    let policy = policy_for("write", &scope(&t)).unwrap().unwrap();
    assert_eq!(
        policy.dirs,
        vec![
            t.root.join("tmp"),
            t.worktree.clone(),
            t.admin.clone(),
            t.common.join("objects"),
            t.common.join("refs"),
            t.common.join("logs"),
        ]
    );
    for dir in &policy.dirs {
        assert!(
            !t.common.join("hooks").starts_with(dir) && !t.common.join("config").starts_with(dir),
            "{dir:?}"
        );
    }
}

#[test]
fn full_has_no_limit_and_an_unknown_level_has_no_row() {
    let t = tree("full");
    assert_eq!(policy_for("full", &scope(&t)).unwrap(), None);
    assert!(refusal("root", &scope(&t)).contains("no write-limit table row"));
}

#[test]
fn write_refuses_a_checkout_it_cannot_keep_hooks_out_of() {
    let t = tree("refuse");
    let mut s = scope(&t);

    s.worktree = t.root.join("missing");
    assert!(refusal("write", &s).contains("cannot resolve worktree"));

    s.worktree = t.root.join("main");
    assert!(refusal("write", &s).contains("primary checkout"));

    s.worktree = t.root.clone();
    assert!(refusal("write", &s).contains("not a git worktree"));

    fs::write(t.worktree.join(".git"), "not a pointer\n").unwrap();
    s.worktree = t.worktree.clone();
    assert!(refusal("write", &s).contains("not a gitdir pointer"));

    fs::write(t.worktree.join(".git"), "gitdir: /nonexistent/admin\n").unwrap();
    assert!(refusal("write", &s).contains("cannot resolve git directory"));

    fs::write(
        t.worktree.join(".git"),
        format!("gitdir: {}\n", t.admin.display()),
    )
    .unwrap();
    fs::remove_file(t.admin.join("commondir")).unwrap();
    assert!(refusal("write", &s).contains("has no commondir"));

    fs::write(t.admin.join("commondir"), "../../nowhere\n").unwrap();
    assert!(refusal("write", &s).contains("common git directory"));

    fs::write(t.admin.join("commondir"), "../..\n").unwrap();
    fs::write(
        t.common.join("config"),
        "[extensions]\n\tworktreeConfig = true\n",
    )
    .unwrap();
    assert!(refusal("write", &s).contains("worktreeConfig"));
}

// The kernel's enforcement, through the launcher, with a stub agent.

/// Each argument is one attempt, in order, and sets its bit in the exit code
/// when it worked: `w:PATH` truncates or creates a file, `rm:PATH` deletes
/// one, `mv:PATH` renames it, `hl:PATH` makes a hard link beside it, `hx:SRC|DST` hard links SRC as DST, `sl:TARGET|LINK` makes that symlink, `wl:PATH` appends to it, `ln:PATH` makes a symlink beside it, and
/// `e:PATH` sets its bit when a write fails with "Permission denied" (Landlock) or
/// "Operation not permitted" (Seatbelt), the operating-system error. A `rm:` target must exist.
const STUB: &str = r#"#!/bin/sh
mask=0
bit=1
for a in "$@"; do
  p="${a#*:}"
  case "$a" in
    w:*) ( : > "$p" ) 2>/dev/null && mask=$((mask + bit)) ;;
    e:*) case "$( ( : > "$p" ) 2>&1 )" in *"Permission denied"*|*"Operation not permitted"*) mask=$((mask + bit)) ;; esac ;;
    rm:*) rm -f -- "$p" 2>/dev/null; [ -e "$p" ] || mask=$((mask + bit)) ;;
    mv:*) mv -- "$p" "$p.moved" 2>/dev/null && mask=$((mask + bit)) ;;
    hl:*) ln -- "$p" "$p.hard" 2>/dev/null && mask=$((mask + bit)) ;;
    hx:*) ln -- "${p%%|*}" "${p#*|}" 2>/dev/null && mask=$((mask + bit)) ;;
    sl:*) ln -s -- "${p%%|*}" "${p#*|}" 2>/dev/null && mask=$((mask + bit)) ;;
    wl:*) ( echo x >> "$p" ) 2>/dev/null && mask=$((mask + bit)) ;;
    ln:*) ln -s x "$p.link" 2>/dev/null && mask=$((mask + bit)) ;;
  esac
  bit=$((bit * 2))
done
exit $mask
"#;

/// Starts the stub at `level` and returns its exit code (the bitmask above).
fn attempt(dir: &Path, level: &str, output: &Path, context: &Context, ops: &[String]) -> i32 {
    let host = crate::this_os();
    let _held = fake_exe::lock();
    let stub = dir.join("stub");
    fake_exe::write_executable(&_held, &stub, STUB);
    let mut record = format!(
        "agents:\n  stub:\n    command: [{}]\n    levels:\n",
        stub.display()
    );
    let mut ids = String::from("ids:\n");
    for l in LEVELS {
        record += &format!("      {l}: {{support: {{linux: verified, mac: verified}}}}\n");
        ids += &format!("  stub-{l}:\n    agent: stub\n    reach: {l}\n    hosts: [{host}]\n");
        if l == "research" || l == "write-scoped" {
            ids += &format!("    output_dir: {}\n", output.display());
        }
    }
    let file = dir.join("reach.yaml");
    fs::write(&file, record + &ids).unwrap();
    let mut argv: Vec<OsString> = vec![
        "run".into(),
        file.into_os_string(),
        format!("stub-{level}").into(),
        "--".into(),
    ];
    argv.extend(ops.iter().map(OsString::from));
    run_with(argv, crate::this_os(), &*native(), context)
}

fn context(t: &Tree) -> Context {
    Context {
        worktree: t.worktree.clone(),
        temp_root: t.root.join("temps"),
        ..Context::default()
    }
}

fn ops(items: &[&Path]) -> Vec<String> {
    items.iter().map(|p| format!("w:{}", p.display())).collect()
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn read_levels_cannot_write_outside_their_temporary_directory() {
    let t = tree("k-read");
    fs::create_dir_all(t.root.join("temps")).unwrap();
    let outside = t.root.join("outside");
    for level in ["read", "read-web"] {
        // Writing to a file next to the worktree fails; the dev null device works.
        let ops = ops(&[&outside, &t.worktree.join("f"), Path::new("/dev/null")]);
        assert_eq!(
            attempt(&t.root, level, &t.root, &context(&t), &ops),
            0b100,
            "{level}"
        );
        assert!(!outside.exists());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn output_levels_write_only_their_output_directory() {
    let t = tree("k-output");
    let out = t.root.join("out");
    let outside = t.root.join("outside");
    for level in ["research", "write-scoped"] {
        let ops = ops(&[&out.join("report"), &outside, &t.worktree.join("f")]);
        assert_eq!(
            attempt(&t.root, level, &out, &context(&t), &ops),
            0b001,
            "{level}"
        );
        assert!(out.join("report").exists() && !outside.exists());
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn run_one_command_writes_only_its_gate_directory() {
    let t = tree("k-gate");
    let gate = t.root.join("gate");
    let mut c = context(&t);
    c.gate_dir = Some(gate.clone());
    let ops = ops(&[
        &gate.join("out"),
        &t.root.join("outside"),
        &t.worktree.join("f"),
    ]);
    assert_eq!(
        attempt(&t.root, "run-one-command", &t.root, &c, &ops),
        0b001
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn write_changes_the_worktree_but_not_hooks_or_config() {
    let t = tree("k-write");
    let hook = t.common.join("hooks/pre-commit");
    let config = t.common.join("config");
    let mut attempts = ops(&[
        &t.worktree.join("file"),
        &t.admin.join("index.lock"),
        &t.common.join("refs/heads/w1"),
        &t.common.join("objects/ab"),
        &t.root.join("outside"),
        &hook,
        &config,
        &t.common.join("hooks/post-commit"),
        &t.common.join("config.lock"),
        &t.common.join("HEAD"),
    ]);
    attempts.extend([
        format!("rm:{}", hook.display()),
        format!("rm:{}", config.display()),
        format!("mv:{}", hook.display()),
        format!("mv:{}", config.display()),
        format!("ln:{}", t.common.join("hooks/pre-push").display()),
    ]);
    // Exit codes hold 8 bits; run the first and second halves separately.
    let first = attempt(&t.root, "write", &t.root, &context(&t), &attempts[..8]);
    assert_eq!(
        first, 0b0000_1111,
        "worktree, admin dir, refs and objects are writable"
    );
    let second = attempt(&t.root, "write", &t.root, &context(&t), &attempts[8..]);
    assert_eq!(
        second, 0,
        "no write to the common git directory's hooks or config succeeded"
    );
    assert_eq!(fs::read_to_string(&hook).unwrap(), "#!/bin/sh\n");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "[core]\n\tbare = false\n"
    );
    assert!(!t.common.join("hooks/post-commit").exists() && !t.common.join("config.lock").exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn full_writes_anywhere() {
    let t = tree("k-full");
    let ops = ops(&[&t.root.join("outside"), &t.common.join("config")]);
    assert_eq!(attempt(&t.root, "full", &t.root, &context(&t), &ops), 0b11);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn write_in_a_primary_checkout_is_refused_and_nothing_starts() {
    let t = tree("k-primary");
    let mut c = context(&t);
    c.worktree = t.root.join("main");
    let marker = t.root.join("marker");
    assert_eq!(attempt(&t.root, "write", &t.root, &c, &ops(&[&marker])), 1);
    assert!(!marker.exists());
}

// A real git commit works under `write`, and the hook and config are out of reach.

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn git_commits_in_a_worktree_but_cannot_install_hooks_or_change_config() {
    let host = crate::this_os();
    let _held = fake_exe::lock();
    let root = scratch("git");
    let git = |dir: &Path, args: &[&str]| {
        let status = std::process::Command::new("git")
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
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    let main = root.join("main");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let worktree = root.join("w");
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
    drop(_held);

    let script = format!(
        "git -c user.name=t -c user.email=t@example.test commit -q --allow-empty -m work || exit 1\n\
         echo x > f && git add f && git -c user.name=t -c user.email=t@example.test commit -q -m file || exit 2\n\
         git config core.hooksPath /tmp 2>/dev/null && exit 3\n\
         mkdir -p {m}/.git/hooks 2>/dev/null; echo x > {m}/.git/hooks/pre-commit 2>/dev/null && exit 4\n\
         exit 0\n",
        m = main.display()
    );
    let stub_dir = root.join("stub");
    fs::create_dir_all(&stub_dir).unwrap();
    let c = Context {
        worktree: worktree.clone(),
        temp_root: root.join("temps"),
        ..Context::default()
    };
    let code = {
        let held = fake_exe::lock();
        let sh = stub_dir.join("agent");
        fake_exe::write_executable(
            &held,
            &sh,
            &format!("#!/bin/sh\ncd {}\n{script}", worktree.display()),
        );
        let yaml = format!(
            "agents:\n  a:\n    command: [{}]\n    levels:\n      write: {{support: {{linux: verified, mac: verified}}}}\nids:\n  a-write:\n    agent: a\n    reach: write\n    hosts: [{host}]\n",
            sh.display()
        );
        let file = root.join("reach.yaml");
        fs::write(&file, yaml).unwrap();
        run_with(
            [
                OsString::from("run"),
                file.into_os_string(),
                "a-write".into(),
            ],
            host,
            &*native(),
            &c,
        )
    };
    assert_eq!(
        code, 0,
        "exit 1/2: a commit failed under the limit; 3: config was changed; 4: a hook was installed"
    );
    assert!(!main.join(".git/hooks/pre-commit").exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn links_and_renames_cannot_reach_hooks_config_or_the_git_directories() {
    let t = tree("k-links");
    let hook = t.common.join("hooks/pre-commit");
    let config = t.common.join("config");
    let w = &t.worktree;
    let attempts: Vec<String> = vec![
        // A hard link to a protected file, made inside the worktree.
        format!("hx:{}|{}", hook.display(), w.join("h").display()),
        format!("hx:{}|{}", config.display(), w.join("c").display()),
        // Creates its own file when the link was refused; the hook stays as it was.
        format!("wl:{}", w.join("h").display()),
        // A symlink is made, but a write through it lands on the hook.
        format!("sl:{}|{}", hook.display(), w.join("s").display()),
        format!("wl:{}", w.join("s").display()),
        // A parent of the worktree, and the shared git directory, cannot be renamed.
        format!("mv:{}", t.root.join("main").display()),
        format!("mv:{}", t.common.display()),
    ];
    assert_eq!(
        attempt(&t.root, "write", &t.root, &context(&t), &attempts),
        0b000_1100,
        "only the symlink itself and the new file h may be made"
    );
    assert_eq!(fs::read_to_string(&hook).unwrap(), "#!/bin/sh\n");
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        "[core]\n\tbare = false\n"
    );
    assert!(t.common.exists() && !t.root.join("main.moved").exists());
    let more: Vec<String> = [
        &t.worktree,
        &t.admin,
        &t.common.join("refs"),
        &t.common.join("objects"),
        &t.common.join("logs"),
    ]
    .iter()
    .map(|p| format!("mv:{}", p.display()))
    .collect();
    assert_eq!(
        attempt(&t.root, "write", &t.root, &context(&t), &more),
        0,
        "no granted directory can be moved away"
    );
    assert!(t.worktree.exists() && t.admin.exists());
}

#[cfg(target_os = "macos")]
#[test]
fn the_files_that_locate_the_git_directory_cannot_be_changed_at_all() {
    let t = tree("k-pointers");
    let pointer = t.worktree.join(".git");
    let attempts: Vec<String> = vec![
        format!("w:{}", pointer.display()),
        format!("rm:{}", pointer.display()),
        format!("mv:{}", pointer.display()),
        format!("w:{}", t.admin.join("commondir").display()),
        format!("rm:{}", t.admin.join("commondir").display()),
        format!("w:{}", t.admin.join("gitdir").display()),
    ];
    let before = fs::read(&pointer).unwrap();
    // 0 also means the launcher found nothing to put back.
    assert_eq!(
        attempt(&t.root, "write", &t.root, &context(&t), &attempts),
        0
    );
    assert_eq!(fs::read(&pointer).unwrap(), before);
    assert_eq!(
        fs::read_to_string(t.admin.join("commondir")).unwrap(),
        "../..\n"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn a_hooks_path_outside_the_worktree_cannot_be_written() {
    let t = tree("k-hookspath");
    let hooks = t.root.join("shared-hooks");
    fs::create_dir_all(&hooks).unwrap();
    fs::write(
        t.common.join("config"),
        format!("[core]\n\thooksPath = {}\n", hooks.display()),
    )
    .unwrap();
    // The agent may not add a hook there, though the directory is beside its worktree.
    let attempts = ops(&[&hooks.join("pre-commit"), &t.worktree.join("f")]);
    assert_eq!(
        attempt(&t.root, "write", &t.root, &context(&t), &attempts),
        0b10
    );
    assert!(!hooks.join("pre-commit").exists());
}

// The launcher with a backend that wraps the command.

/// A `sandbox-exec` stand-in: accepts and logs the probe, then for the real
/// start skips the profile options and runs the command.
const FAKE_SANDBOX_EXEC: &str = r#"#!/bin/sh
while [ "$1" = -p ] || [ "$1" = -D ]; do shift 2; done
case "$1" in
  /usr/bin/true) exit 0 ;;
esac
echo wrapped >> "$WRAP_LOG"
exec "$@"
"#;

#[test]
fn a_wrapping_backend_starts_the_agent_through_its_prefix() {
    let t = tree("k-wrapped");
    let held = fake_exe::lock();
    let wrapper = t.root.join("sandbox-exec");
    fake_exe::write_executable(&held, &wrapper, FAKE_SANDBOX_EXEC);
    let stub = t.root.join("stub");
    fake_exe::write_executable(&held, &stub, "#!/bin/sh\necho \"$@\" > \"$OUT\"\n");
    let log = t.root.join("wrap.log");
    let out = t.root.join("out.txt");
    std::env::set_var("WRAP_LOG", &log);
    std::env::set_var("OUT", &out);
    let host = crate::this_os();
    let yaml = format!(
        "agents:\n  a:\n    command: [{}, first]\n    levels:\n      read: {{support: {{linux: verified, mac: verified}}}}\nids:\n  a-read:\n    agent: a\n    reach: read\n    hosts: [{host}]\n",
        stub.display()
    );
    let file = t.root.join("reach.yaml");
    fs::write(&file, yaml).unwrap();
    let code = run_with(
        [
            OsString::from("run"),
            file.into_os_string(),
            "a-read".into(),
            "--".into(),
            "second".into(),
        ],
        host,
        &SandboxExec::at(&wrapper),
        &context(&t),
    );
    std::env::remove_var("WRAP_LOG");
    std::env::remove_var("OUT");
    assert_eq!(code, 0);
    assert_eq!(fs::read_to_string(&log).unwrap(), "wrapped\n");
    assert_eq!(fs::read_to_string(&out).unwrap(), "first second\n");
}

#[test]
fn an_agent_command_that_the_wrapper_would_read_as_an_option_is_refused() {
    let t = tree("k-dash");
    let held = fake_exe::lock();
    let wrapper = t.root.join("sandbox-exec");
    fake_exe::write_executable(&held, &wrapper, FAKE_SANDBOX_EXEC);
    let log = t.root.join("wrap.log");
    std::env::set_var("WRAP_LOG", &log);
    let host = crate::this_os();
    let yaml = format!(
        "agents:\n  a:\n    command: [-p, '(allow default)']\n    levels:\n      read: {{support: {{linux: verified, mac: verified}}}}\nids:\n  a-read:\n    agent: a\n    reach: read\n    hosts: [{host}]\n"
    );
    let file = t.root.join("reach.yaml");
    fs::write(&file, yaml).unwrap();
    let code = run_with(
        [
            OsString::from("run"),
            file.into_os_string(),
            "a-read".into(),
        ],
        host,
        &SandboxExec::at(&wrapper),
        &context(&t),
    );
    std::env::remove_var("WRAP_LOG");
    assert_eq!(code, 1);
    assert!(!log.exists(), "nothing was started under the wrapper");
}

#[test]
fn the_launcher_refuses_to_start_without_sandbox_exec() {
    let t = tree("k-no-sandbox-exec");
    let marker = t.root.join("marker");
    let held = fake_exe::lock();
    let stub = t.root.join("stub");
    fake_exe::write_executable(&held, &stub, STUB);
    let host = crate::this_os();
    let yaml = format!(
        "agents:\n  a:\n    command: [{}]\n    levels:\n      read: {{support: {{linux: verified, mac: verified}}}}\nids:\n  a-read:\n    agent: a\n    reach: read\n    hosts: [{host}]\n",
        stub.display()
    );
    let file = t.root.join("reach.yaml");
    fs::write(&file, yaml).unwrap();
    let code = run_with(
        [
            OsString::from("run"),
            file.into_os_string(),
            "a-read".into(),
            "--".into(),
            format!("w:{}", marker.display()).into(),
        ],
        host,
        &SandboxExec::at(t.root.join("missing/sandbox-exec")),
        &context(&t),
    );
    assert_eq!(code, 1);
    assert!(!marker.exists(), "the agent must not have started");
}

#[test]
fn the_write_policy_denies_the_hooks_and_config_git_uses() {
    let t = tree("denied");
    let hooks = t.root.join("elsewhere");
    fs::write(
        t.common.join("config"),
        format!("[core]\n\thooksPath = {}\n", hooks.display()),
    )
    .unwrap();
    let policy = policy_for("write", &scope(&t)).unwrap().unwrap();
    assert_eq!(
        policy.denied,
        vec![
            t.common.join("hooks"),
            t.common.join("config"),
            hooks,
            t.admin.join("config.worktree"),
            t.common.join("objects/info/alternates"),
        ]
    );
}

#[test]
fn a_backend_for_no_os_refuses_and_the_native_one_exists() {
    assert!(Unsupported
        .prepare(&Policy::default())
        .err()
        .unwrap()
        .contains("no write-limit backend"));
    // The backend for this OS answers; on the supported OSes an empty policy is accepted.
    let answer = native().prepare(&Policy::default());
    assert_eq!(
        answer.is_ok(),
        cfg!(target_os = "linux") || cfg!(target_os = "macos")
    );
}

// The launcher with a backend that cannot hold the limit.

struct Unavailable;

impl WriteLimit for Unavailable {
    fn prepare(&self, _policy: &Policy) -> Result<Armed, String> {
        Err("this kernel has no Landlock".into())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn the_launcher_refuses_to_start_when_the_backend_is_unavailable() {
    let t = tree("k-unavailable");
    let marker = t.root.join("out/marker");
    let host = crate::this_os();
    let _held = fake_exe::lock();
    let stub = t.root.join("stub");
    fake_exe::write_executable(&_held, &stub, STUB);
    let yaml = format!(
        "agents:\n  a:\n    command: [{}]\n    levels:\n      research: {{support: {{linux: verified, mac: verified}}}}\n      full: {{support: {{linux: verified, mac: verified}}}}\nids:\n  a-research:\n    agent: a\n    reach: research\n    output_dir: {}\n    hosts: [{host}]\n  a-full:\n    agent: a\n    reach: full\n    hosts: [{host}]\n",
        stub.display(),
        t.root.join("out").display()
    );
    let file = t.root.join("reach.yaml");
    fs::write(&file, yaml).unwrap();
    let go = |id: &str| {
        run_with(
            [
                OsString::from("run"),
                file.clone().into_os_string(),
                id.into(),
                "--".into(),
                format!("w:{}", marker.display()).into(),
            ],
            host,
            &Unavailable,
            &context(&t),
        )
    };
    // No limit, no agent: exit 1 and the stub never ran (it would have exited 1 too, so look for its side effect).
    assert_eq!(go("a-research"), 1);
    assert!(!marker.exists(), "the agent must not have started");
    // `full` takes no limit, so it does not need the backend.
    assert_eq!(
        go("a-full"),
        1,
        "the stub ran and created the marker, exit 1 as its bit 1 is set"
    );
    assert!(marker.exists());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_run_that_cannot_make_its_directories_is_refused() {
    let t = tree("k-mkdir");
    let blocker = t.root.join("blocker");
    fs::write(&blocker, "x").unwrap();
    // The output directory sits beneath a regular file.
    assert_eq!(
        attempt(&t.root, "research", &blocker.join("out"), &context(&t), &[]),
        1
    );
}

#[test]
fn the_context_of_this_process_reads_its_environment() {
    std::env::set_var("AGENT_KIT_STATE_DIRS", "/a::/b");
    std::env::set_var("AGENT_KIT_GATE_DIR", "/gate");
    let c = Context::from_env();
    std::env::remove_var("AGENT_KIT_STATE_DIRS");
    std::env::remove_var("AGENT_KIT_GATE_DIR");
    assert_eq!(c.state_dirs, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    assert_eq!(c.gate_dir, Some(PathBuf::from("/gate")));
    assert_eq!(c.worktree, PathBuf::from("."));
}

struct Fine;

impl Arm for Fine {
    fn arm(&self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn the_child_hook_arms_the_limit() {
    assert!(crate::child_hook(Box::new(Fine))().is_ok());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_write_outside_the_allowed_paths_fails_with_the_operating_system_error() {
    let t = tree("k-errno");
    let outside = format!("e:{}", t.root.join("outside").display());
    let inside = format!("e:{}", t.worktree.join("inside").display());
    // Denied outside (bit set), allowed inside (no error, bit clear).
    assert_eq!(
        attempt(&t.root, "write", &t.root, &context(&t), &[outside, inside]),
        0b01
    );
}

#[cfg(target_os = "linux")]
#[test]
fn redirecting_git_is_undone_and_reported() {
    let t = tree("k-redirect");
    let pointer = fs::read(t.worktree.join(".git")).unwrap();
    let commondir = fs::read(t.admin.join("commondir")).unwrap();
    let fake = t.worktree.join("fake");
    // The agent points git at a directory it controls and replaces `.git`.
    let script = format!(
        "mkdir -p {f}/hooks; echo x > {f}/hooks/pre-commit; echo {f} > {a}/commondir; rm -f {w}/.git; mkdir {w}/.git; echo y > {w}/.git/config",
        f = fake.display(), a = t.admin.display(), w = t.worktree.display()
    );
    let host = crate::this_os();
    let _held = fake_exe::lock();
    let stub = t.root.join("redirect");
    fake_exe::write_executable(&_held, &stub, &format!("#!/bin/sh\n{script}\n"));
    let yaml = format!(
        "agents:\n  a:\n    command: [{}]\n    levels:\n      write: {{support: {{linux: verified, mac: verified}}}}\nids:\n  a-write:\n    agent: a\n    reach: write\n    hosts: [{host}]\n",
        stub.display()
    );
    let file = t.root.join("reach.yaml");
    fs::write(&file, yaml).unwrap();
    let code = run_with(
        [
            OsString::from("run"),
            file.into_os_string(),
            "a-write".into(),
        ],
        host,
        &*native(),
        &context(&t),
    );
    assert_eq!(code, 1, "tampering is reported");
    assert_eq!(fs::read(t.worktree.join(".git")).unwrap(), pointer);
    assert_eq!(fs::read(t.admin.join("commondir")).unwrap(), commondir);
}

#[test]
fn the_guard_restores_only_what_changed() {
    let dir = scratch("guard");
    let (same, edited, gone, missing) = (
        dir.join("same"),
        dir.join("edited"),
        dir.join("gone"),
        dir.join("missing"),
    );
    for f in [&same, &edited, &gone] {
        fs::write(f, "v1").unwrap();
    }
    let policy = Policy {
        guarded: vec![same.clone(), edited.clone(), gone.clone(), missing],
        ..Policy::default()
    };
    let guard = Guard::take(&policy);
    fs::write(&edited, "v2").unwrap();
    fs::remove_file(&gone).unwrap();
    fs::create_dir(&gone).unwrap();
    assert_eq!(
        guard.restore(),
        vec![edited.display().to_string(), gone.display().to_string()]
    );
    assert_eq!(fs::read_to_string(&edited).unwrap(), "v1");
    assert_eq!(fs::read_to_string(&gone).unwrap(), "v1");
    assert!(guard.restore().is_empty());
}

#[test]
fn write_guards_the_files_that_locate_the_git_directory() {
    let t = tree("guarded");
    let policy = policy_for("write", &scope(&t)).unwrap().unwrap();
    assert_eq!(
        policy.guarded,
        vec![
            t.worktree.join(".git"),
            t.admin.join("commondir"),
            t.admin.join("gitdir"),
            t.common.join("objects/info")
        ]
    );
}

#[test]
fn a_relative_gitdir_pointer_is_taken_from_the_worktree() {
    let t = tree("relative");
    fs::write(
        t.worktree.join(".git"),
        "gitdir: ../main/.git/worktrees/w1\n",
    )
    .unwrap();
    assert!(policy_for("write", &scope(&t)).unwrap().is_some());
}

#[test]
fn a_missing_logs_directory_is_made_and_a_broken_config_is_refused() {
    let t = tree("logs");
    fs::remove_dir(t.common.join("logs")).unwrap();
    assert!(policy_for("write", &scope(&t))
        .unwrap()
        .unwrap()
        .dirs
        .contains(&t.common.join("logs")));
    fs::remove_dir_all(t.common.join("logs")).unwrap();
    fs::write(t.common.join("logs"), "x").unwrap();
    assert!(refusal("write", &scope(&t)).contains("cannot create"));
    fs::remove_file(t.common.join("config")).unwrap();
    assert!(refusal("write", &scope(&t)).contains("cannot read the repository config"));
}

#[test]
fn a_config_that_includes_another_file_is_refused() {
    let t = tree("include");
    fs::write(t.common.join("config"), "[include]\n\tpath = extra\n").unwrap();
    assert!(refusal("write", &scope(&t)).contains("includes another file"));
}

#[test]
fn a_writable_directory_inside_a_git_directory_is_refused() {
    let t = tree("dotgit");
    let mut s = scope(&t);
    s.output_dir = Some(t.common.join("hooks"));
    assert!(refusal("research", &s).contains("inside a .git directory"));
}

#[test]
fn the_run_directory_is_new_and_private() {
    let dir = scratch("private");
    let made = dir.join("run");
    crate::private_dir(&dir, &made).unwrap();
    assert_eq!(
        fs::metadata(&made).unwrap().permissions().mode() & 0o777,
        0o700
    );
    // Already there (say, planted): refused. Root beneath a file: refused.
    assert!(crate::private_dir(&dir, &made).is_err());
    assert!(crate::private_dir(&dir.join("run/x"), &made).is_err());
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_run_whose_temporary_directory_cannot_be_made_is_refused() {
    let t = tree("k-notemp");
    let blocker = t.root.join("blocker");
    fs::write(&blocker, "x").unwrap();
    let mut c = context(&t);
    c.temp_root = blocker.join("temps");
    assert_eq!(attempt(&t.root, "read", &t.root, &c, &[]), 1);
}

#[test]
fn write_refuses_a_git_directory_that_is_not_laid_out_as_git_makes_it() {
    let t = tree("topology");
    let s = scope(&t);

    // Residue of a redirect: commondir names a directory inside the worktree.
    let fake = t.worktree.join("fake");
    fs::create_dir_all(fake.join("worktrees")).unwrap();
    std::os::unix::fs::symlink(&t.admin, fake.join("worktrees/w1")).unwrap();
    fs::write(t.admin.join("commondir"), format!("{}\n", fake.display())).unwrap();
    assert!(refusal("write", &s).contains("not laid out as git makes"));
    fs::write(t.admin.join("commondir"), "../..\n").unwrap();

    // The directory does not point back at this worktree, or says nothing.
    fs::write(
        t.admin.join("gitdir"),
        format!("{}/other/.git\n", t.root.display()),
    )
    .unwrap();
    assert!(refusal("write", &s).contains("does not point back"));
    fs::remove_file(t.admin.join("gitdir")).unwrap();
    assert!(refusal("write", &s).contains("has no gitdir"));
    fs::write(
        t.admin.join("gitdir"),
        format!("{}\n", t.worktree.join(".git").display()),
    )
    .unwrap();

    // A shared directory that is a symlink to hooks.
    fs::remove_dir_all(t.common.join("objects")).unwrap();
    std::os::unix::fs::symlink(t.common.join("hooks"), t.common.join("objects")).unwrap();
    assert!(refusal("write", &s).contains("is a symlink"));
}

#[test]
fn write_refuses_hooks_the_agent_could_reach() {
    let s_of = |t: &Tree| scope(t);

    // core.hooksPath inside the worktree (relative, as Husky sets it) or absolute.
    let t = tree("hp-relative");
    fs::write(t.common.join("config"), "[core]\n\thooksPath = .husky\n").unwrap();
    assert!(refusal("write", &s_of(&t)).contains("lies inside the writable"));
    let t = tree("hp-absolute");
    let inside = t.common.join("objects/hooks");
    fs::write(
        t.common.join("config"),
        format!("[core]\n\tHooksPath = \"{}\"\n", inside.display()),
    )
    .unwrap();
    assert!(refusal("write", &s_of(&t)).contains("lies inside the writable"));

    // A hooksPath outside the grants, even under ~, is fine.
    let t = tree("hp-outside");
    fs::write(
        t.common.join("config"),
        format!(
            "[core]\n\thooksPath = {}\n",
            t.root.join("global-hooks").display()
        ),
    )
    .unwrap();
    assert!(policy_for("write", &s_of(&t)).is_ok());
    let t = tree("hp-home");
    fs::write(
        t.common.join("config"),
        "[core]\n\thooksPath = ~/.config/git/hooks\n",
    )
    .unwrap();
    assert!(policy_for("write", &s_of(&t)).is_ok());
    assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    assert!(hooks_paths("[core]\n\tbare = false\n").is_empty());

    // common/hooks that is a symlink into the worktree.
    let t = tree("hooks-link");
    fs::create_dir_all(t.worktree.join("hooks")).unwrap();
    fs::remove_dir_all(t.common.join("hooks")).unwrap();
    std::os::unix::fs::symlink(t.worktree.join("hooks"), t.common.join("hooks")).unwrap();
    assert!(refusal("write", &s_of(&t)).contains("lies inside the writable"));

    // common/config that is a symlink into the worktree.
    let t = tree("config-link");
    fs::write(t.worktree.join("cfg"), "[core]\n").unwrap();
    fs::remove_file(t.common.join("config")).unwrap();
    std::os::unix::fs::symlink(t.worktree.join("cfg"), t.common.join("config")).unwrap();
    assert!(refusal("write", &s_of(&t)).contains("lies inside the writable"));

    // A worktree root that is the hooks directory itself.
    let t = tree("root-hooks");
    let mut s = s_of(&t);
    fs::write(
        t.common.join("hooks/.git"),
        format!("gitdir: {}\n", t.admin.display()),
    )
    .unwrap();
    fs::write(
        t.admin.join("gitdir"),
        format!("{}\n", t.common.join("hooks/.git").display()),
    )
    .unwrap();
    s.worktree = t.common.join("hooks");
    assert!(refusal("write", &s).contains("lies inside the writable"));
}

#[test]
fn write_refuses_refs_and_logs_symlinks_and_a_misnamed_admin_directory() {
    for shared in ["refs", "logs"] {
        let t = tree(&format!("link-{shared}"));
        fs::remove_dir_all(t.common.join(shared)).unwrap();
        std::os::unix::fs::symlink(t.common.join("hooks"), t.common.join(shared)).unwrap();
        assert!(
            refusal("write", &scope(&t)).contains("is a symlink"),
            "{shared}"
        );
    }
    // An admin directory not under a `worktrees` directory.
    let t = tree("misnamed");
    let moved = t.common.join("wt/w1");
    fs::create_dir_all(t.common.join("wt")).unwrap();
    fs::rename(&t.admin, &moved).unwrap();
    fs::write(
        t.worktree.join(".git"),
        format!("gitdir: {}\n", moved.display()),
    )
    .unwrap();
    fs::write(moved.join("commondir"), "../..\n").unwrap();
    assert!(refusal("write", &scope(&t)).contains("not laid out"));
}

#[test]
fn a_guarded_file_that_cannot_be_put_back_is_reported_as_such() {
    let dir = scratch("guard-fail");
    let sub = dir.join("sub");
    fs::create_dir_all(&sub).unwrap();
    let file = sub.join("f");
    fs::write(&file, "v1").unwrap();
    let guard = Guard::take(&Policy {
        guarded: vec![file.clone()],
        ..Policy::default()
    });
    fs::remove_dir_all(&sub).unwrap();
    let report = guard.restore();
    assert_eq!(report.len(), 1);
    assert!(report[0].contains("could NOT be put back"), "{report:?}");
}

#[test]
fn every_hooks_path_is_checked_and_global_config_counts() {
    // Git uses the last value, so the first being harmless is no excuse.
    let t = tree("hp-twice");
    let outside = t.root.join("safe");
    fs::write(
        t.common.join("config"),
        format!(
            "[core]\n\thooksPath = {}\n\thooksPath = .husky\n",
            outside.display()
        ),
    )
    .unwrap();
    assert!(refusal("write", &scope(&t)).contains("lies inside the writable"));

    // A relative hooksPath in the global config runs from the worktree.
    let t = tree("hp-global");
    let global = t.root.join("gitconfig");
    fs::write(&global, "[core]\n\thooksPath = .githooks\n").unwrap();
    let mut s = scope(&t);
    s.other_configs = vec![global, t.root.join("no-such-config")];
    assert!(refusal("write", &s).contains("lies inside the writable"));

    // A hooksPath that is a link leading nowhere is refused.
    let t = tree("hp-dangling");
    let link = t.root.join("link");
    std::os::unix::fs::symlink(t.worktree.join("missing"), &link).unwrap();
    fs::write(
        t.common.join("config"),
        format!("[core]\n\thooksPath = {}\n", link.display()),
    )
    .unwrap();
    assert!(refusal("write", &scope(&t)).contains("dangling link"));
}

#[test]
fn the_context_names_the_users_git_configs() {
    let c = Context::from_env();
    assert!(c.other_configs.contains(&PathBuf::from("/etc/gitconfig")));
}

#[test]
fn git_config_files_follow_home_and_xdg() {
    let p = |s: &str| Some(PathBuf::from(s));
    let etc = PathBuf::from("/etc/gitconfig");
    assert_eq!(crate::configs_in(None, None), vec![etc.clone()]);
    assert_eq!(
        crate::configs_in(p("/h"), None),
        vec![
            etc.clone(),
            "/h/.gitconfig".into(),
            "/h/.config/git/config".into()
        ]
    );
    assert_eq!(
        crate::configs_in(p("/h"), p("/x")),
        vec![etc, "/h/.gitconfig".into(), "/x/git/config".into()]
    );
}
