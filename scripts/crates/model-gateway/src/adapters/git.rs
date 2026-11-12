//! git, asked for a path's repository root and its remotes. It starts nothing else.

use crate::ports::{Git, GitRemote};
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long git may take before the gateway gives up on it.
pub const TIMEOUT_MS: u64 = 10_000;

/// How long git may take to add or remove a worktree.
pub const WORKTREE_TIMEOUT_MS: u64 = 120_000;

/// git in the environment it was given (PATH decides which git).
pub struct SystemGit {
    pub env: HashMap<String, String>,
    pub timeout_ms: u64,
}

struct Answer {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

impl SystemGit {
    fn run(&self, path: &str, args: &[&str]) -> Result<Answer, String> {
        self.run_for(path, args, self.timeout_ms)
    }

    fn run_for(&self, path: &str, args: &[&str], timeout_ms: u64) -> Result<Answer, String> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot run git: spawn git {}", if error.raw_os_error() == Some(2) { "ENOENT".to_string() } else { error.to_string() }))?;
        let mut out = child.stdout.take().expect("piped stdout");
        let mut err = child.stderr.take().expect("piped stderr");
        let stdout = std::thread::spawn(move || read_all(&mut out));
        let stderr = std::thread::spawn(move || read_all(&mut err));
        let started = Instant::now();
        let status = loop {
            if let Ok(Some(status)) = child.try_wait() {
                break status;
            }
            if started.elapsed() >= Duration::from_millis(timeout_ms) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git did not answer within {timeout_ms} ms"));
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        Ok(Answer { exit_code: status.code().unwrap_or(-1), stdout: stdout.join().unwrap_or_default(), stderr: stderr.join().unwrap_or_default() })
    }
}

fn read_all(source: &mut impl std::io::Read) -> String {
    let mut bytes = Vec::new();
    let _ = source.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

/// git's own words for a failure, or its exit code when it said nothing.
fn words(answer: &Answer) -> String {
    let message = answer.stderr.trim();
    if message.is_empty() {
        format!("git exited {}", answer.exit_code)
    } else {
        message.to_string()
    }
}

impl Git for SystemGit {
    fn root(&self, path: &str) -> Result<Option<String>, String> {
        let answer = self.run(path, &["rev-parse", "--show-toplevel"])?;
        if answer.exit_code == 0 {
            let root = answer.stdout.trim();
            if root.is_empty() {
                return Err("git answered with no repository root".to_string());
            }
            return Ok(Some(root.to_string()));
        }
        if answer.stderr.to_lowercase().contains("not a git repository") {
            return Ok(None);
        }
        Err(words(&answer))
    }

    fn remotes(&self, path: &str) -> Result<Vec<GitRemote>, String> {
        let answer = self.run(path, &["config", "--get-regexp", r"^remote\..*\.url$"])?;
        if answer.exit_code == 1 {
            return Ok(Vec::new());
        }
        if answer.exit_code != 0 {
            return Err(words(&answer));
        }
        Ok(answer.stdout.lines().filter_map(|line| parse_remote_line(line.trim())).collect())
    }

    fn remove_worktree(&self, repo: &str, path: &str) -> Result<(), String> {
        let removed = self.run_for(repo, &["worktree", "remove", "--force", path], WORKTREE_TIMEOUT_MS)?;
        if removed.exit_code == 0 {
            return Ok(());
        }
        // A worktree git still knows but refuses to remove (the repository's own, or a locked one) is never deleted by hand.
        if removed.stderr.contains("main working tree") || removed.stderr.contains("locked working tree") {
            return Err(words(&removed));
        }
        // git no longer knows the worktree (its directory moved, or a lock was left): delete it by hand and prune.
        remove_tree(path)?;
        let pruned = self.run(repo, &["worktree", "prune"])?;
        if pruned.exit_code == 0 {
            Ok(())
        } else {
            Err(words(&pruned))
        }
    }

    fn delete_branch(&self, repo: &str, branch: &str) -> Result<(), String> {
        let answer = self.run(repo, &["branch", "-D", branch])?;
        if answer.exit_code == 0 {
            Ok(())
        } else {
            Err(words(&answer))
        }
    }
}

/// `rm -rf`: a path that is not there is already gone.
fn remove_tree(path: &str) -> Result<(), String> {
    let removed = match std::fs::symlink_metadata(path) {
        Ok(kind) if kind.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    removed.map_err(|error| format!("{path}: cannot remove: {error}"))
}

/// `remote.<name>.url <url>`.
fn parse_remote_line(line: &str) -> Option<GitRemote> {
    let rest = line.strip_prefix("remote.")?;
    let (name_and_key, url) = rest.split_once(char::is_whitespace)?;
    let name = name_and_key.strip_suffix(".url")?;
    let url = url.trim();
    if name.is_empty() || url.is_empty() {
        return None;
    }
    Some(GitRemote { name: name.to_string(), url: url.to_string() })
}
