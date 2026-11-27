//! The setup canary crate: the Rust workspace's proof of path.
//!
//! The standards repo's ADR-0014 puts all of agent-kit's Rust in one workspace
//! under `scripts/`. This member is deliberately trivial and std-only, with no
//! dependency of any kind.
//! `--version` proves the build end to end; the command also validates a path.
//! New Rust joins this workspace as a new member, never as its own Cargo
//! project.
//!
//! The claim store writes one lease per key as a git commit on
//! `refs/claims/<key>`. Creating or replacing that ref with a
//! `--force-with-lease` push is the atomic lock: a push fails when somebody
//! else moved the ref first, and the loser reads the winning lease back.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::lease::{Clock, Lease};

/// Runs one git command with `stdin` and returns its stdout, or its stderr.
pub trait Git {
    fn git(&self, args: &[&str], stdin: &str) -> Result<String, String>;
}

/// A [`Git`] that runs the real executable in a caller-owned scratch directory.
pub struct RealGit {
    pub dir: PathBuf,
}

impl RealGit {
    fn exec(&self, args: &[&str], stdin: &str) -> std::io::Result<std::process::Output> {
        let mut child = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().expect("stdin is piped");
        input.write_all(stdin.as_bytes())?;
        drop(input);
        child.wait_with_output()
    }
}

impl Git for RealGit {
    fn git(&self, args: &[&str], stdin: &str) -> Result<String, String> {
        let output = self
            .exec(args, stdin)
            .map_err(|error| format!("git: {error}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches('\n')
            .to_string())
    }
}

/// The remote, the clock and the git scratch directory used for one key space.
pub struct Store<'a> {
    pub git: &'a dyn Git,
    pub clock: &'a dyn Clock,
    pub url: String,
}

/// What an acquire did: won with its new commit, or lost to a holder.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Won(String),
    Lost(Lease),
}

/// The ref that holds the lease for `key`.
pub fn refname(key: &str) -> String {
    format!("refs/claims/{key}")
}

impl Store<'_> {
    /// Pushes a new value, or reads back who held the ref when the push lost.
    fn push(&self, key: &str, expected: &str, refspec: &str, sha: &str) -> Result<Outcome, String> {
        let reference = refname(key);
        let lease_arg = format!("--force-with-lease={reference}:{expected}");
        if self
            .git
            .git(&["push", &lease_arg, &self.url, refspec], "")
            .is_ok()
        {
            return Ok(Outcome::Won(sha.to_string()));
        }
        match self.current(key)? {
            Some((_, holder)) => Ok(Outcome::Lost(holder)),
            None => Err(format!("claim {key} vanished after a failed push")),
        }
    }

    /// The current commit and lease for `key`, if the remote has one.
    pub fn current(&self, key: &str) -> Result<Option<(String, Lease)>, String> {
        let reference = refname(key);
        let remote = self.git.git(&["ls-remote", &self.url, &reference], "")?;
        if remote.is_empty() {
            return Ok(None);
        }
        let sha = remote
            .split_whitespace()
            .next()
            .ok_or_else(|| format!("malformed ls-remote: {remote}"))?;
        self.git.git(&["fetch", &self.url, &reference], "")?;
        let message = self
            .git
            .git(&["log", "-1", "--format=%B", "FETCH_HEAD"], "")?;
        Ok(Some((sha.to_string(), Lease::parse(&message)?)))
    }

    /// Commits an empty tree holding `lease`, optionally on top of `parent`.
    pub fn commit(&self, lease: &Lease, parent: Option<&str>) -> Result<String, String> {
        self.commit_message(&lease.message(), parent)
    }

    /// Builds an empty-tree commit with this message on top of `parent`.
    pub fn commit_message(&self, message: &str, parent: Option<&str>) -> Result<String, String> {
        let tree = self.git.git(&["mktree"], "")?;
        // The configured identity is used when there is one (a hook may check it); a fallback
        // keeps the command working on a machine with no git identity.
        let configured = self
            .git
            .git(&["config", "user.name"], "")
            .map(|name| !name.is_empty())
            .unwrap_or(false);
        let mut args: Vec<&str> = Vec::new();
        if !configured {
            args.extend_from_slice(&["-c", "user.name=claim", "-c", "user.email=claim@localhost"]);
        }
        args.extend_from_slice(&["commit-tree", &tree]);
        if let Some(parent) = parent {
            args.extend_from_slice(&["-p", parent]);
        }
        args.extend_from_slice(&["-F", "-"]);
        self.git.git(&args, message)
    }

    /// Takes an unheld or expired claim, or reports the active holder.
    pub fn acquire(
        &self,
        key: &str,
        device: &str,
        run: &str,
        issue: u64,
        lease_seconds: u64,
    ) -> Result<Outcome, String> {
        let reference = refname(key);
        let lease = Lease {
            device: device.to_string(),
            run: run.to_string(),
            issue,
            expires: self.clock.now() + lease_seconds,
        };
        match self.current(key)? {
            None => {
                let sha = self.commit(&lease, None)?;
                self.push(key, "", &format!("{sha}:{reference}"), &sha)
            }
            Some((old_sha, holder)) => {
                if !holder.expired(self.clock.now()) {
                    return Ok(Outcome::Lost(holder));
                }
                let sha = self.commit(&lease, Some(&old_sha))?;
                self.push(key, &old_sha, &format!("{sha}:{reference}"), &sha)
            }
        }
    }

    /// Extends a claim held by this device and run, keeping its issue.
    pub fn renew(
        &self,
        key: &str,
        device: &str,
        run: &str,
        lease_seconds: u64,
    ) -> Result<String, String> {
        let (old_sha, holder) = self.current(key)?.ok_or_else(|| "no claim".to_string())?;
        if holder.device != device || holder.run != run {
            return Err("not held by this device and run".to_string());
        }
        let lease = Lease {
            device: holder.device,
            run: holder.run,
            issue: holder.issue,
            expires: self.clock.now() + lease_seconds,
        };
        let sha = self.commit(&lease, Some(&old_sha))?;
        let reference = refname(key);
        match self.push(key, &old_sha, &format!("{sha}:{reference}"), &sha)? {
            Outcome::Won(sha) => Ok(sha),
            Outcome::Lost(_) => Err(format!("claim {key} changed while renewing")),
        }
    }

    /// Deletes the claim ref, returning the lease it held.
    pub fn release(&self, key: &str) -> Result<Lease, String> {
        let (sha, lease) = self.current(key)?.ok_or_else(|| "no claim".to_string())?;
        let reference = refname(key);
        match self.push(key, &sha, &format!(":{reference}"), &sha)? {
            Outcome::Won(_) => Ok(lease),
            Outcome::Lost(_) => Err(format!("claim {key} changed while releasing")),
        }
    }

    /// Every claim on the remote, sorted by key.
    pub fn list(&self) -> Result<Vec<(String, Lease)>, String> {
        let remote = self
            .git
            .git(&["ls-remote", &self.url, "refs/claims/*"], "")?;
        let mut found = Vec::new();
        for line in remote.lines() {
            let field = line
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| format!("malformed ls-remote line: {line}"))?;
            let key = field
                .strip_prefix("refs/claims/")
                .ok_or_else(|| format!("not a claim ref: {field}"))?;
            if let Some((_, lease)) = self.current(key)? {
                found.push((key.to_string(), lease));
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(found)
    }
}
