//! What the runner asks git: a path's remotes, its repository root and its
//! common directory. The words of a failure are the node adapter's.

use model_gateway::adapters::fs::fail;
use model_gateway::ports::GitRemote;
use model_gateway::text::js_trim;
use std::collections::HashMap;
use std::process::{Command, Stdio};

pub trait Git: Send + Sync {
    fn remotes(&self, cwd: &str) -> Result<Vec<GitRemote>, String>;
    fn common_dir(&self, cwd: &str) -> Result<String, String>;
    fn root(&self, cwd: &str) -> Result<String, String>;
}

/// git in the environment it was given (PATH decides which git).
pub struct SystemGit {
    pub env: HashMap<String, String>,
}

impl SystemGit {
    /// Runs git in `cwd`; an exit code outside `accepted` fails with the words the node adapter's `GitCommandError` has.
    fn run(&self, cwd: &str, args: &[&str], accepted: &[i32]) -> Result<String, String> {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("spawn git {}", fail(&error, "spawn", None).code))?;
        let code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if accepted.contains(&code) {
            return Ok(stdout);
        }
        Err(failure_words(args, cwd, code, &stdout, &String::from_utf8_lossy(&output.stderr)))
    }
}

/// `git <args> failed in <cwd>: <detail>`: what git said on stderr, else on stdout, else its exit code.
pub fn failure_words(args: &[&str], cwd: &str, code: i32, stdout: &str, stderr: &str) -> String {
    let detail = [js_trim(stderr), js_trim(stdout)].into_iter().find(|text| !text.is_empty()).map_or_else(|| format!("exit code {code}"), str::to_string);
    format!("git {} failed in {cwd}: {detail}", args.join(" "))
}

/// `remote.<name>.url <url>`, as `/^remote\.(.+)\.url (.+)$/` reads a line: the name runs to the last `.url `.
pub fn parse_remote_line(line: &str) -> Option<GitRemote> {
    let rest = line.strip_prefix("remote.")?;
    let dot = |c: char| matches!(c, '\r' | '\u{2028}' | '\u{2029}');
    let (name, url) = rest.rmatch_indices(".url ").find_map(|(at, found)| {
        let (name, url) = (&rest[..at], &rest[at + found.len()..]);
        (!name.is_empty() && !url.is_empty() && !name.contains(dot) && !url.contains(dot)).then_some((name, url))
    })?;
    Some(GitRemote { name: name.to_string(), url: url.to_string() })
}

impl Git for SystemGit {
    fn remotes(&self, cwd: &str) -> Result<Vec<GitRemote>, String> {
        // `git config --get-regexp` exits 1 when nothing matches.
        let listing = self.run(cwd, &["config", "--get-regexp", r"^remote\..*\.url$"], &[0, 1])?;
        Ok(listing.split('\n').filter_map(|line| parse_remote_line(js_trim(line))).collect())
    }

    fn common_dir(&self, cwd: &str) -> Result<String, String> {
        self.run(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"], &[0]).map(|out| js_trim(&out).to_string())
    }

    fn root(&self, cwd: &str) -> Result<String, String> {
        self.run(cwd, &["rev-parse", "--show-toplevel"], &[0]).map(|out| js_trim(&out).to_string())
    }
}
