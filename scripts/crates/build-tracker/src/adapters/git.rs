use crate::ports::{Process, RepositoryLocation, Repositories};
use crate::spec::{repository_name, RepositoryName};
use crate::text::{basename, dirname, is_space, is_terminator, trim};

/// Finds a directory's repository with git: its root, the directory's place under it, and its remote.
pub struct GitRepositories<P: Process> {
    pub process: P,
}

impl<P: Process> Repositories for GitRepositories<P> {
    fn locate(&self, dir: &str) -> Result<Option<RepositoryLocation>, String> {
        let Some(paths) = self.git(dir, &["rev-parse", "--path-format=absolute", "--show-toplevel", "--git-common-dir", "--show-prefix"])? else {
            return Ok(None);
        };
        let mut lines = paths.split('\n');
        let mut next = || lines.next().unwrap_or("").to_string();
        let (root, common_dir, prefix) = (next(), next(), next());
        let remotes = self.git(dir, &["config", "--get-regexp", r"^remote\..*\.url$"])?.unwrap_or_default();
        let main_root = if basename(&common_dir) == ".git" { dirname(&common_dir).to_string() } else { common_dir };
        Ok(Some(RepositoryLocation { main_root, prefix, remote: primary_remote(&remotes), root }))
    }
}

impl<P: Process> GitRepositories<P> {
    /// The command's output, or `None` when git exits non-zero.
    fn git(&self, dir: &str, args: &[&str]) -> Result<Option<String>, String> {
        let mut all = vec!["-C".to_string(), dir.to_string()];
        all.extend(args.iter().map(|arg| arg.to_string()));
        let result = self.process.exec("git", &all)?;
        Ok((result.exit_code == 0).then_some(result.stdout))
    }
}

/// `^remote\.(.+)\.url\s+(.+)$` on a trimmed line: the remote's name and its location.
fn remote_line(line: &str) -> Option<(&str, &str)> {
    if line.chars().any(is_terminator) {
        return None;
    }
    let rest = line.strip_prefix("remote.")?;
    // The name is greedy: the last `.url` followed by whitespace and more text wins.
    rest.rmatch_indices(".url").filter(|(at, _)| *at > 0).find_map(|(at, marker)| {
        let after = &rest[at + marker.len()..];
        let location = after.trim_start_matches(is_space);
        (location.len() < after.len() && !location.is_empty()).then_some((&rest[..at], location))
    })
}

/// `origin`'s location, or the first remote's when there is no `origin`.
fn primary_remote(config: &str) -> Option<RepositoryName> {
    let remotes: Vec<(&str, RepositoryName)> = config
        .split('\n')
        .filter_map(|line| remote_line(trim(line)).and_then(|(name, location)| parse_remote(location).map(|repository| (name, repository))))
        .collect();
    let chosen = remotes.iter().find(|(name, _)| *name == "origin").or(remotes.first());
    chosen.map(|(_, repository)| repository.clone())
}

/// `^[^@/\s]+@([^:/\s]+):(.+)$`: `git@host:owner/repo.git`.
fn scp_like(url: &str) -> Option<(&str, &str)> {
    let (user, rest) = url.split_once('@')?;
    if user.is_empty() || user.contains(|c: char| c == '/' || is_space(c)) {
        return None;
    }
    let (host, path) = rest.split_once(':')?;
    let host_ok = !host.is_empty() && !host.contains(|c: char| c == '/' || is_space(c));
    (host_ok && !path.is_empty()).then_some((host, path))
}

/// `^[a-z+]+:\/\/(?:[^@/]+@)?([^/:]+)(?::\d+)?\/(.+)$`, case-insensitive: `https://host/owner/repo.git`.
fn with_scheme(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphabetic() || c == '+') {
        return None;
    }
    let userless = rest.split_once('@').filter(|(user, _)| !user.is_empty() && !user.contains('/')).map(|(_, tail)| tail);
    userless.into_iter().chain(std::iter::once(rest)).find_map(|tail| {
        let host_end = tail.find(['/', ':'])?;
        let (host, after) = tail.split_at(host_end);
        let after = match after.strip_prefix(':') {
            Some(port) => {
                let digits = port.bytes().take_while(u8::is_ascii_digit).count();
                if digits == 0 {
                    return None;
                }
                &port[digits..]
            }
            None => after,
        };
        let path = after.strip_prefix('/')?;
        (!host.is_empty() && !path.is_empty()).then_some((host, path))
    })
}

/// Reads `git@host:owner/repo.git`, `https://host/owner/repo.git` or `ssh://git@host/owner/repo`.
pub fn parse_remote(url: &str) -> Option<RepositoryName> {
    if url.chars().any(is_terminator) {
        return None;
    }
    let (host, path) = scp_like(url).or_else(|| with_scheme(url))?;
    let without_slashes = path.trim_end_matches('/');
    let without_git = without_slashes.strip_suffix(".git").unwrap_or(without_slashes);
    let mut segments: Vec<&str> = without_git.split('/').filter(|segment| !segment.is_empty()).collect();
    let repo = segments.pop()?;
    let owner = segments.join("/");
    if owner.is_empty() {
        return None;
    }
    Some(repository_name(host, &owner, repo))
}
