//! Tests for the real adapters: the PATH lookup and the programs it starts,
//! git's answers about a repository, the file system's newer methods, and
//! SHA-256 against the NIST vectors.

use super::scenarios::{Sandbox, EXEC_LOCK};
use crate::adapters::fs::SystemFiles;
use crate::adapters::git::{SystemGit, TIMEOUT_MS};
use crate::adapters::process::SystemProcess;
use crate::ports::{Files, Git, GitRemote, Process, ProcessCommand, ProcessResult};
use crate::sha256::sha256_hex;
use std::collections::HashMap;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

/// Linux can fail a spawn with ETXTBSY when another thread forks while a script
/// is still open for writing, so every "write a script, then run it" holds the
/// one lock the scenario replay holds too, across the write, the chmod and the spawn.
fn serialised() -> MutexGuard<'static, ()> {
    EXEC_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A shell script in `directory`, executable. `std::fs::write` closes its handle
/// before it returns, so nothing is open for writing when the script is spawned.
fn script(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, body).expect("write the script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("make the script executable");
    path
}

fn env_with_path(path: &str) -> HashMap<String, String> {
    HashMap::from([("PATH".to_string(), path.to_string())])
}

/// The PATH this test process runs with, so the real git can be found.
fn real_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Runs the real git in `directory`, the way the adapter does.
fn git(directory: &Path, args: &[&str]) {
    let output = std::process::Command::new("git").arg("-C").arg(directory).args(args).env("GIT_CONFIG_GLOBAL", "/dev/null").env("GIT_CONFIG_SYSTEM", "/dev/null").output().expect("run git");
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn sha256_matches_the_nist_vectors() {
    assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    // 448 bits: the length no longer fits beside the message, so it takes a second block.
    assert_eq!(
        sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(sha256_hex(&[b'a'; 1_000_000]), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
}

#[test]
fn path_lookup_finds_the_first_executable_file() {
    let sandbox = Sandbox::new();
    let first = sandbox.path.join("first");
    let second = sandbox.path.join("second");
    std::fs::create_dir_all(&first).expect("first folder");
    std::fs::create_dir_all(&second).expect("second folder");
    let _guard = serialised();
    script(&second, "tool", "#!/bin/sh\n");
    std::fs::write(first.join("plain"), "not a program\n").expect("plain file");
    std::fs::create_dir(first.join("folder")).expect("folder");
    std::fs::set_permissions(first.join("folder"), std::fs::Permissions::from_mode(0o755)).expect("chmod folder");

    let process = SystemProcess { env: env_with_path(&format!("{}:{}", text(&first), text(&second))) };
    assert_eq!(process.resolve("tool"), Some(format!("{}/tool", text(&second))));
    // A file no one may run is not a program; a directory is found, as Node's access(X_OK) finds it.
    assert_eq!(process.resolve("plain"), None);
    assert_eq!(process.resolve("folder"), Some(format!("{}/folder", text(&first))));
    assert_eq!(process.resolve("missing"), None);
}

#[test]
fn path_lookup_skips_empty_entries_and_a_path_that_is_not_there() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "tool", "#!/bin/sh\n");

    let process = SystemProcess { env: env_with_path(&format!("::{}/nowhere::{}:", text(&bin), text(&bin))) };
    assert_eq!(process.resolve("tool"), Some(format!("{}/tool", text(&bin))));

    let empty = SystemProcess { env: env_with_path("") };
    assert_eq!(empty.resolve("tool"), None);
    let without = SystemProcess { env: HashMap::new() };
    assert_eq!(without.resolve("tool"), None);
}

#[test]
fn exec_reads_what_a_program_printed_and_the_code_it_left() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let path = script(&sandbox.path, "listing", "#!/bin/sh\nprintf '%s\\n' \"$@\"\necho oops >&2\nexit 3\n");
    let process = SystemProcess { env: env_with_path(&sandbox.text()) };

    let result = process.exec(&text(&path), &["one", "two"], None).expect("the script runs");
    assert_eq!(result, ProcessResult { stdout: "one\ntwo\n".to_string(), stderr: "oops\n".to_string(), exit_code: 3, timed_out: false });
}

#[test]
fn exec_runs_a_program_in_the_environment_it_was_given_and_no_other() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let path = script(&sandbox.path, "environment", "#!/bin/sh\nprintf '%s|%s' \"${GREETING-unset}\" \"${HOME-unset}\"\n");
    let mut env = env_with_path(&sandbox.text());
    env.insert("GREETING".to_string(), "hello".to_string());
    let process = SystemProcess { env };

    let result = process.exec(&text(&path), &[], None).expect("the script runs");
    assert_eq!(result.stdout, "hello|unset");
    assert_eq!(result.exit_code, 0);
}

#[test]
fn exec_names_a_program_it_cannot_start() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let process = SystemProcess { env: env_with_path(&sandbox.text()) };

    let missing = sandbox.path.join("nowhere");
    assert_eq!(process.exec(&text(&missing), &[], None), Err(format!("spawn {} ENOENT", text(&missing))));

    let plain = sandbox.path.join("plain");
    std::fs::write(&plain, "not a program\n").expect("plain file");
    assert_eq!(process.exec(&text(&plain), &[], None), Err(format!("spawn {} EACCES", text(&plain))));

    // Any other failure to start is reported in the system's own words.
    let long = format!("/{}", "a".repeat(5_000));
    let error = process.exec(&long, &[], None).expect_err("no such path can be run");
    assert!(error.starts_with(&format!("spawn {long} ")), "{error}");
    assert!(error.contains("os error"), "{error}");
}

#[test]
fn exec_kills_a_program_that_outlasts_its_time_limit() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let path = script(&sandbox.path, "slow", "#!/bin/sh\nwhile :; do :; done\n");
    let process = SystemProcess { env: env_with_path(&sandbox.text()) };

    let result = process.exec(&text(&path), &[], Some(50)).expect("the script starts");
    assert!(result.timed_out);
    assert_eq!(result.exit_code, 124);
}

#[test]
fn exec_reports_the_code_node_gives_a_program_a_signal_ended() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let path = script(&sandbox.path, "killed", "#!/bin/sh\nkill -9 $$\n");
    let process = SystemProcess { env: env_with_path(&sandbox.text()) };

    let result = process.exec(&text(&path), &[], None).expect("the script starts");
    assert_eq!((result.exit_code, result.timed_out), (124, false));
}

/// A launch of the shell script `body`, which the shell reads from its own arguments.
fn shell(body: &str) -> ProcessCommand {
    ProcessCommand::new("sh", &["-c", body])
}

#[test]
fn run_passes_the_arguments_the_directory_the_added_environment_and_the_input() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let process = SystemProcess { env: HashMap::from([("PATH".to_string(), real_path()), ("KEPT".to_string(), "parent".to_string()), ("SHADOWED".to_string(), "parent".to_string())]) };
    let mut command = shell("pwd; printf '%s|%s|%s|%s\\n' \"$KEPT\" \"$SHADOWED\" \"$ADDED\" \"$1\"; cat; echo err >&2; exit 4");
    command.args.push("arg-one".to_string());
    command.args.push("arg-one".to_string());
    command.cwd = Some(text(&sandbox.path));
    command.env = [("SHADOWED", "added"), ("ADDED", "yes")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    command.stdin = Some("from stdin".to_string());

    let result = process.run(&command).expect("the shell runs");

    let directory = std::fs::canonicalize(&sandbox.path).expect("sandbox");
    assert_eq!(result.stdout, format!("{}\nparent|added|yes|arg-one\nfrom stdin", text(&directory)));
    assert_eq!(result.stderr, "err\n");
    assert_eq!((result.exit_code, result.timed_out), (4, false));
}

#[test]
fn run_closes_stdin_from_the_start_when_given_no_input_and_survives_a_program_that_never_reads_it() {
    let _guard = serialised();
    let process = SystemProcess { env: env_with_path(&real_path()) };

    let closed = process.run(&shell("cat; echo done")).expect("the shell runs");
    assert_eq!(closed.stdout, "done\n");

    let mut ignores = shell("exit 0");
    ignores.stdin = Some("x".repeat(1 << 20));
    assert_eq!(process.run(&ignores).expect("the shell runs").exit_code, 0);
}

#[test]
fn run_stops_a_program_that_outlasts_its_limit_with_sigterm_and_reports_the_signal_code() {
    let _guard = serialised();
    let process = SystemProcess { env: env_with_path(&real_path()) };
    let mut slow = shell("exec sleep 30");
    slow.timeout_ms = Some(50);

    let result = process.run(&slow).expect("the shell starts");

    assert!(result.timed_out);
    assert_eq!(result.exit_code, 124);
}

#[test]
fn run_kills_a_program_that_ignores_sigterm_once_the_grace_ends() {
    let _guard = serialised();
    let process = SystemProcess { env: env_with_path(&real_path()) };
    let mut stubborn = shell("trap '' TERM; exec sleep 30");
    stubborn.timeout_ms = Some(50);

    let started = std::time::Instant::now();
    let result = process.run_with_grace(&stubborn, 100).expect("the shell starts");

    assert!(result.timed_out);
    assert_eq!(result.exit_code, 124);
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
}

#[test]
fn run_uses_the_default_grace_for_a_program_that_exits_on_its_own() {
    let _guard = serialised();
    let process = SystemProcess { env: env_with_path(&real_path()) };
    let mut quick = shell("echo quick");
    quick.timeout_ms = Some(60_000);

    let result = process.run(&quick).expect("the shell runs");

    assert_eq!((result.stdout.as_str(), result.exit_code, result.timed_out), ("quick\n", 0, false));
}

#[test]
fn run_names_a_program_it_cannot_start_and_a_directory_that_is_not_there() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let process = SystemProcess { env: env_with_path(&real_path()) };

    let missing = sandbox.path.join("nowhere");
    assert_eq!(process.run(&ProcessCommand::new(&text(&missing), &[])), Err(format!("spawn {} ENOENT", text(&missing))));

    let plain = sandbox.path.join("plain");
    std::fs::write(&plain, "not a program\n").expect("plain file");
    assert_eq!(process.run(&ProcessCommand::new(&text(&plain), &[])), Err(format!("spawn {} EACCES", text(&plain))));

    let mut elsewhere = shell("true");
    elsewhere.cwd = Some(text(&missing));
    assert_eq!(process.run(&elsewhere), Err("spawn sh ENOENT".to_string()));
}

#[test]
fn git_names_the_root_of_a_repository_and_of_a_subdirectory() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = sandbox.path.join("repo");
    std::fs::create_dir_all(repo.join("docs")).expect("docs folder");
    git(&repo, &["init", "-q"]);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    let root = text(&std::fs::canonicalize(&repo).expect("canonical repository"));

    assert_eq!(repositories.root(&text(&repo)), Ok(Some(root.clone())));
    assert_eq!(repositories.root(&text(&repo.join("docs"))), Ok(Some(root)));
}

#[test]
fn git_says_a_path_outside_any_repository_has_no_root() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.root(&sandbox.text()), Ok(None));
}

#[test]
fn git_reports_a_path_it_cannot_read_rather_than_calling_it_no_repository() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let missing = sandbox.path.join("nowhere");
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };

    let error = repositories.root(&text(&missing)).expect_err("git cannot change to it");
    assert!(error.contains("cannot change to"), "{error}");
    assert!(error.contains("nowhere"), "{error}");
}

#[test]
fn git_names_the_remotes_of_a_repository() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = sandbox.path.join("repo");
    std::fs::create_dir_all(&repo).expect("repository folder");
    git(&repo, &["init", "-q"]);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    let path = text(&repo);

    assert_eq!(repositories.remotes(&path), Ok(Vec::new()));

    git(&repo, &["remote", "add", "origin", "git@github.com:Acme/Widgets.git"]);
    assert_eq!(
        repositories.remotes(&path),
        Ok(vec![GitRemote { name: "origin".to_string(), url: "git@github.com:Acme/Widgets.git".to_string() }])
    );

    git(&repo, &["remote", "add", "upstream", "https://example.com/other/thing.git"]);
    assert_eq!(
        repositories.remotes(&path),
        Ok(vec![
            GitRemote { name: "origin".to_string(), url: "git@github.com:Acme/Widgets.git".to_string() },
            GitRemote { name: "upstream".to_string(), url: "https://example.com/other/thing.git".to_string() },
        ])
    );

    // A url with a space in it keeps the space: only the first gap separates it from its name.
    git(&repo, &["remote", "add", "spaced", "https://example.com/a b"]);
    assert_eq!(
        repositories.remotes(&path),
        Ok(vec![
            GitRemote { name: "origin".to_string(), url: "git@github.com:Acme/Widgets.git".to_string() },
            GitRemote { name: "upstream".to_string(), url: "https://example.com/other/thing.git".to_string() },
            GitRemote { name: "spaced".to_string(), url: "https://example.com/a b".to_string() },
        ])
    );
}

#[test]
fn git_reads_only_the_remote_lines_it_understands() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(
        &bin,
        "git",
        "#!/bin/sh\n\
         printf '%s\\n' 'remote.origin.url git@github.com:Acme/Widgets.git'\n\
         printf '%s\\n' 'remote.spaced.url   https://example.com/a b'\n\
         printf '%s\\n' 'garbage'\n\
         printf '%s\\n' 'remote.nospace'\n\
         printf '%s\\n' 'remote.other.key value'\n\
         printf '%s\\n' 'remote..url https://example.com/empty-name'\n\
         printf 'remote.tab.url\\thttps://example.com/tab\\n'\n",
    );
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };

    assert_eq!(
        repositories.remotes(&sandbox.text()),
        Ok(vec![
            GitRemote { name: "origin".to_string(), url: "git@github.com:Acme/Widgets.git".to_string() },
            GitRemote { name: "spaced".to_string(), url: "https://example.com/a b".to_string() },
            GitRemote { name: "tab".to_string(), url: "https://example.com/tab".to_string() },
        ])
    );
}

#[test]
fn git_says_it_cannot_run_when_there_is_no_git_on_path() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repositories = SystemGit { env: env_with_path(&sandbox.text()), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.root(&sandbox.text()), Err("cannot run git: spawn git ENOENT".to_string()));
    assert_eq!(repositories.remotes(&sandbox.text()), Err("cannot run git: spawn git ENOENT".to_string()));
}

#[test]
fn git_reports_a_git_it_may_not_run_in_the_words_the_system_gave() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    std::fs::write(bin.join("git"), "#!/bin/sh\n").expect("git file");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };

    assert_eq!(
        repositories.root(&sandbox.text()),
        Err(format!("cannot run git: spawn git {}", std::io::Error::from_raw_os_error(13)))
    );
}

#[test]
fn git_gives_up_on_a_git_that_does_not_answer() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "git", "#!/bin/sh\nwhile :; do :; done\n");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: 100 };

    assert_eq!(repositories.root(&sandbox.text()), Err("git did not answer within 100 ms".to_string()));
}

#[test]
fn git_refuses_an_answer_with_no_root() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "git", "#!/bin/sh\nexit 0\n");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.root(&sandbox.text()), Err("git answered with no repository root".to_string()));
}

#[test]
fn git_repeats_its_own_words_for_a_failure() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "git", "#!/bin/sh\necho 'fatal: boom' >&2\nexit 2\n");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.root(&sandbox.text()), Err("fatal: boom".to_string()));
    assert_eq!(repositories.remotes(&sandbox.text()), Err("fatal: boom".to_string()));
}

#[test]
fn git_names_the_exit_code_when_it_says_nothing() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "git", "#!/bin/sh\nexit 7\n");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };
    assert_eq!(repositories.root(&sandbox.text()), Err("git exited 7".to_string()));

    // Exit code 1 is git's way of saying there is no remote, so a failure is any other code.
    script(&bin, "git", "#!/bin/sh\nexit 2\n");
    assert_eq!(repositories.remotes(&sandbox.text()), Err("git exited 2".to_string()));
}

#[test]
fn exists_says_what_is_there_and_what_is_not() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    std::fs::write(sandbox.path.join("file"), "x").expect("file");
    std::fs::create_dir(sandbox.path.join("folder")).expect("folder");

    assert_eq!(files.exists("file"), Ok(true));
    assert_eq!(files.exists("folder"), Ok(true));
    assert_eq!(files.exists("missing"), Ok(false));
    // A path under a file is not there either.
    assert_eq!(files.exists("file/under"), Ok(false));

    // A directory it may not look in is an error, not a no.
    let locked = sandbox.path.join("locked");
    std::fs::create_dir(&locked).expect("locked folder");
    std::fs::write(locked.join("inside"), "x").expect("inside file");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).expect("lock the folder");
    let error = files.exists("locked/inside").expect_err("it may not look");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("unlock the folder");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("EACCES", "EACCES: permission denied, access 'locked/inside'"));
}

#[test]
fn append_creates_the_file_and_adds_to_it() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };

    assert_eq!(files.append("runs.jsonl", "one\n"), Ok(()));
    assert_eq!(files.append("runs.jsonl", "two\n"), Ok(()));
    assert_eq!(std::fs::read_to_string(sandbox.path.join("runs.jsonl")).expect("read the log"), "one\ntwo\n");

    let error = files.append("nowhere/runs.jsonl", "x").expect_err("no folder to write in");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("ENOENT", "ENOENT: no such file or directory, open 'nowhere/runs.jsonl'"));
}

/// The device that takes a write and keeps none of it: the only way to make the
/// write itself fail, rather than the open before it.
#[cfg(target_os = "linux")]
#[test]
fn append_names_a_write_that_cannot_land() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };

    let error = files.append("/dev/full", "x").expect_err("the device has no room");
    assert_eq!((error.code.as_str(), error.message.as_str()), ("ENOSPC", "ENOSPC: no space left on device, write"));
}

#[test]
fn stamp_names_what_makes_a_file_itself() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let path = sandbox.path.join("model-gateway.json");
    std::fs::write(&path, "the file itself").expect("write the file");
    std::os::unix::fs::symlink(&path, sandbox.path.join("linked.json")).expect("link to it");

    let stamp = files.stamp("linked.json").expect("a stamp of the file the link names");
    let real = std::fs::canonicalize(&path).expect("canonical file");
    let meta = std::fs::metadata(&real).expect("metadata");
    assert_eq!(stamp.realpath, text(&real));
    assert_eq!(stamp.sha256, sha256_hex(b"the file itself"));
    assert_eq!(stamp.size, 15.0);
    assert_eq!(stamp.ino, meta.ino() as f64);
    assert_eq!(stamp.ctime_ms, meta.ctime() as f64 * 1e3 + meta.ctime_nsec() as f64 / 1e6);

    assert_eq!(files.stamp("missing.toml"), None);
    // A directory has no content to hash, so it has no stamp.
    std::fs::create_dir(sandbox.path.join("folder")).expect("folder");
    assert_eq!(files.stamp("folder"), None);
}

#[test]
fn write_private_writes_a_file_only_its_owner_may_read() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let path = sandbox.path.join("model-gateway/vibe-available.json");

    assert_eq!(files.write_private("model-gateway/vibe-available.json", "{}"), Ok(()));
    assert_eq!(std::fs::read_to_string(&path).expect("read the file"), "{}");
    assert_eq!(std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(sandbox.path.join("model-gateway")).expect("metadata").permissions().mode() & 0o777, 0o700);

    // It replaces a file that is already there, and keeps it private.
    assert_eq!(files.write_private("model-gateway/vibe-available.json", "{\"a\":1}"), Ok(()));
    assert_eq!(std::fs::read_to_string(&path).expect("read the file"), "{\"a\":1}");
    assert_eq!(std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777, 0o600);
}

#[test]
fn write_private_cleans_up_after_a_write_that_cannot_land() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    std::fs::create_dir_all(sandbox.path.join("state/folder")).expect("folder");

    // A folder is not replaced by a file, and the temporary file goes with the failure.
    let error = files.write_private("state/folder", "x").expect_err("a folder is not replaced");
    assert_eq!(error.code, "EISDIR");
    let mut left: Vec<String> = std::fs::read_dir(sandbox.path.join("state"))
        .expect("read the folder")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, vec!["folder".to_string()]);

    // A file in the way of the directory is named as the failure.
    std::fs::write(sandbox.path.join("blocker"), "x").expect("blocker file");
    let error = files.write_private("blocker/deeper/file.json", "x").expect_err("a file is not a directory");
    assert_eq!(error.code, "ENOTDIR");
    assert!(error.message.starts_with("ENOTDIR: not a directory, mkdir '"), "{}", error.message);

    // A directory it may not write in fails on the temporary file it cannot make.
    let read_only = sandbox.path.join("read-only");
    std::fs::create_dir(&read_only).expect("read-only folder");
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o555)).expect("make it read-only");
    let error = files.write_private("read-only/file.json", "x").expect_err("it may not write there");
    std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o755)).expect("unlock the folder");
    assert_eq!(error.code, "EACCES");
    assert!(error.message.starts_with("EACCES: permission denied, open '"), "{}", error.message);
}

/// A real repository with one commit, in `sandbox/repo`.
fn repository_with_a_commit(sandbox: &Sandbox) -> PathBuf {
    let repo = sandbox.path.join("repo");
    std::fs::create_dir_all(&repo).expect("repository folder");
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", "commit", "-q", "--allow-empty", "-m", "first"]);
    repo
}

#[test]
fn git_removes_a_worktree_with_its_directory_and_then_its_branch() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let worktree = sandbox.path.join("kept");
    git(&repo, &["worktree", "add", "-q", "-b", "gateway/run-1", &text(&worktree), "HEAD"]);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.remove_worktree(&text(&repo), &text(&worktree)), Ok(()));
    assert!(!worktree.exists());
    assert_eq!(repositories.delete_branch(&text(&repo), "gateway/run-1"), Ok(()));
    let error = repositories.delete_branch(&text(&repo), "gateway/run-1").expect_err("the branch is gone");
    assert!(error.contains("gateway/run-1"), "{error}");
}

#[test]
fn git_deletes_a_worktree_it_no_longer_knows_by_hand_and_prunes() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    // A directory git never made a worktree of, a plain file, and a path that is not there at all.
    let directory = sandbox.path.join("not-a-worktree");
    std::fs::create_dir_all(directory.join("inner")).expect("directory");
    std::fs::write(directory.join("inner").join("file"), "x").expect("file");
    let file = sandbox.path.join("a-file");
    std::fs::write(&file, "x").expect("file");

    assert_eq!(repositories.remove_worktree(&text(&repo), &text(&directory)), Ok(()));
    assert_eq!(repositories.remove_worktree(&text(&repo), &text(&file)), Ok(()));
    assert_eq!(repositories.remove_worktree(&text(&repo), &text(&sandbox.path.join("nowhere"))), Ok(()));
    assert!(!directory.exists() && !file.exists());
}

#[test]
fn git_reports_a_leftover_it_cannot_delete_and_a_prune_that_fails() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    // The parent of the path is a file, so the path can be neither looked at nor deleted.
    let parent = sandbox.path.join("parent-file");
    std::fs::write(&parent, "x").expect("file");
    let error = repositories.remove_worktree(&text(&repo), &text(&parent.join("child"))).expect_err("cannot delete");
    assert!(error.starts_with(&format!("{}: cannot remove: ", text(&parent.join("child")))), "{error}");

    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    script(&bin, "git", "#!/bin/sh\necho \"fatal: $*\" >&2\nexit 128\n");
    let failing = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };
    assert_eq!(
        failing.remove_worktree(&text(&repo), &text(&sandbox.path.join("gone"))),
        Err(format!("fatal: -C {} worktree prune", text(&repo)))
    );
    assert_eq!(failing.delete_branch(&text(&repo), "b"), Err(format!("fatal: -C {} branch -D b", text(&repo))));
}

#[test]
fn git_worktree_commands_say_when_git_cannot_run() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repositories = SystemGit { env: env_with_path(&sandbox.text()), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.remove_worktree(&sandbox.text(), "w"), Err("cannot run git: spawn git ENOENT".to_string()));
    assert_eq!(repositories.delete_branch(&sandbox.text(), "b"), Err("cannot run git: spawn git ENOENT".to_string()));
}

#[test]
fn git_never_deletes_by_hand_the_main_worktree_or_a_locked_one() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    let error = repositories.remove_worktree(&text(&repo), &text(&repo)).expect_err("the main worktree stays");
    assert!(error.contains("main working tree"), "{error}");
    assert!(repo.join(".git").exists());

    let locked = sandbox.path.join("locked");
    git(&repo, &["worktree", "add", "-q", "-b", "gateway/locked", &text(&locked), "HEAD"]);
    git(&repo, &["worktree", "lock", &text(&locked)]);
    let error = repositories.remove_worktree(&text(&repo), &text(&locked)).expect_err("a locked worktree stays");
    assert!(error.contains("locked"), "{error}");
    assert!(locked.exists());
}

/// The canonical text of a path, as git names it.
fn canonical(path: &Path) -> String {
    text(&std::fs::canonicalize(path).expect("canonical path"))
}

#[test]
fn git_names_the_git_directory_of_a_repository_and_of_a_worktree_of_it() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    let root = canonical(&repo);

    assert_eq!(repositories.git_dir(&text(&repo)), Ok(format!("{root}/.git")));

    let worktree = sandbox.path.join("tree");
    git(&repo, &["worktree", "add", "-q", "-b", "gateway/one", &text(&worktree), "HEAD"]);
    let git_dir = repositories.git_dir(&text(&worktree)).expect("the worktree's own git directory");
    assert!(git_dir.starts_with(&format!("{root}/.git/worktrees/")), "{git_dir}");
    assert_ne!(git_dir, root);
}

#[test]
fn git_creates_a_worktree_on_a_branch_of_its_own_off_the_base_it_is_given() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    let worktree = sandbox.path.join("gateway");

    assert_eq!(repositories.create_worktree(&text(&repo), &text(&worktree), "gateway/one", "main"), Ok(()));
    assert!(worktree.join(".git").exists());
    let branch = std::process::Command::new("git").arg("-C").arg(&worktree).args(["rev-parse", "--abbrev-ref", "HEAD"]).output().expect("git");
    assert_eq!(String::from_utf8_lossy(&branch.stdout).trim(), "gateway/one");

    // A base git does not know is refused in git's own words.
    let error = repositories.create_worktree(&text(&repo), &text(&sandbox.path.join("other")), "gateway/two", "no-such-ref").expect_err("no such base");
    assert!(error.contains("no-such-ref"), "{error}");
}

#[test]
fn git_stages_everything_and_lists_what_it_staged_against_a_base() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    std::fs::write(repo.join("a.txt"), "one").expect("a file");
    std::fs::create_dir_all(repo.join("b")).expect("a folder");
    std::fs::write(repo.join("b/c.txt"), "two").expect("a file");

    assert_eq!(repositories.stage_all(&text(&repo)), Ok(()));
    let mut against_base = repositories.staged_files(&text(&repo), Some("main")).expect("listed");
    against_base.sort();
    assert_eq!(against_base, vec!["a.txt".to_string(), "b/c.txt".to_string()]);
    // Without a base it lists what is staged against HEAD, which is the same change here.
    let mut against_head = repositories.staged_files(&text(&repo), None).expect("listed");
    against_head.sort();
    assert_eq!(against_head, vec!["a.txt".to_string(), "b/c.txt".to_string()]);

    let error = repositories.staged_files(&text(&repo), Some("no-such-ref")).expect_err("no such base");
    assert!(error.contains("no-such-ref"), "{error}");
}

#[test]
fn git_lists_the_worktrees_it_knows_and_skips_the_ones_whose_directory_is_gone() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };
    let root = canonical(&repo);
    assert_eq!(repositories.list_worktrees(&text(&repo)), Ok(vec![root.clone()]));

    let kept = sandbox.path.join("kept");
    git(&repo, &["worktree", "add", "-q", "-b", "gateway/one", &text(&kept), "HEAD"]);
    let gone = sandbox.path.join("gone");
    git(&repo, &["worktree", "add", "-q", "-b", "gateway/two", &text(&gone), "HEAD"]);
    std::fs::remove_dir_all(&gone).expect("the worktree's directory goes");

    assert_eq!(repositories.list_worktrees(&text(&repo)), Ok(vec![root, canonical(&kept)]));
}

#[test]
fn git_reads_the_status_of_a_worktree_as_git_prints_it() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repo = repository_with_a_commit(&sandbox);
    let repositories = SystemGit { env: env_with_path(&real_path()), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.status(&text(&repo)), Ok(String::new()));
    std::fs::write(repo.join("new.txt"), "x").expect("a file");
    assert_eq!(repositories.status(&text(&repo)), Ok("?? new.txt\n".to_string()));
}

#[test]
fn git_reads_only_the_worktree_blocks_it_understands() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(
        &bin,
        "git",
        "#!/bin/sh\n\
         printf '%s\\n' 'prunable' 'worktree /one' 'HEAD abc' 'prunable gitdir file points to non-existent location' 'worktree /two' 'HEAD def' 'worktree /three' 'prunable'\n",
    );
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };

    // A block before any worktree line is nothing, and a prunable block is skipped.
    assert_eq!(repositories.list_worktrees(&sandbox.text()), Ok(vec!["/two".to_string()]));
}

#[test]
fn git_reads_the_staged_names_it_is_given_and_drops_the_empty_ones() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "git", "#!/bin/sh\nprintf 'a.txt\\0\\0b/c.txt\\0'\n");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.staged_files(&sandbox.text(), Some("main")), Ok(vec!["a.txt".to_string(), "b/c.txt".to_string()]));
}

#[test]
fn git_refuses_a_git_directory_it_cannot_name() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "git", "#!/bin/sh\nexit 0\n");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };

    assert_eq!(repositories.git_dir(&sandbox.text()), Err("git exited 0".to_string()));
}

#[test]
fn git_repeats_its_own_words_for_the_worktree_and_staging_commands() {
    let sandbox = Sandbox::new();
    let bin = sandbox.path.join("bin");
    std::fs::create_dir_all(&bin).expect("bin folder");
    let _guard = serialised();
    script(&bin, "git", "#!/bin/sh\necho 'fatal: boom' >&2\nexit 2\n");
    let repositories = SystemGit { env: env_with_path(&text(&bin)), timeout_ms: TIMEOUT_MS };
    let path = sandbox.text();

    assert_eq!(repositories.git_dir(&path), Err("fatal: boom".to_string()));
    assert_eq!(repositories.create_worktree(&path, "w", "b", "main"), Err("fatal: boom".to_string()));
    assert_eq!(repositories.stage_all(&path), Err("fatal: boom".to_string()));
    assert_eq!(repositories.staged_files(&path, None), Err("fatal: boom".to_string()));
    assert_eq!(repositories.list_worktrees(&path), Err("fatal: boom".to_string()));
    assert_eq!(repositories.status(&path), Err("fatal: boom".to_string()));
}

#[test]
fn git_worktree_and_staging_commands_say_when_git_cannot_run() {
    let sandbox = Sandbox::new();
    let _guard = serialised();
    let repositories = SystemGit { env: env_with_path(&sandbox.text()), timeout_ms: TIMEOUT_MS };
    let path = sandbox.text();

    assert_eq!(repositories.git_dir(&path), Err("cannot run git: spawn git ENOENT".to_string()));
    assert_eq!(repositories.create_worktree(&path, "w", "b", "main"), Err("cannot run git: spawn git ENOENT".to_string()));
    assert_eq!(repositories.stage_all(&path), Err("cannot run git: spawn git ENOENT".to_string()));
    assert_eq!(repositories.staged_files(&path, Some("main")), Err("cannot run git: spawn git ENOENT".to_string()));
    assert_eq!(repositories.list_worktrees(&path), Err("cannot run git: spawn git ENOENT".to_string()));
    assert_eq!(repositories.status(&path), Err("cannot run git: spawn git ENOENT".to_string()));
}
