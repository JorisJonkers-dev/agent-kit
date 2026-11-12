//! A remote's location, and a repository's identity from its remotes.
//!
//! The node runner reads a remote URL with two regular expressions, an scp-like
//! one first and a scheme one second. This file is the same two expressions
//! written out by hand, so the crate needs no regular expression engine.

use crate::ports::GitRemote;

/// Where a remote points: its host, owner and repository, all lowercased.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteLocation {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

/// The GitHub organisation whose repositories never send content to any critic.
pub const DENIED_OWNER: &str = "nedap";

/// What a repository's remotes say about it: the allowlist key it is known by,
/// that it belongs to the denied organisation, or that it has no remote at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepositoryIdentity {
    DeniedOwner { remote: String, owner: String, location: RemoteLocation },
    NoRemote,
    Identified { key: String, location: RemoteLocation },
}

/// Reads `host`, `owner` and `repo` from an ssh (`git@host:owner/repo.git`,
/// `ssh://git@host/owner/repo`) or https remote URL. Host aliases such as
/// `github-work` stay as written.
pub fn parse_remote(url: &str) -> Option<RemoteLocation> {
    let (host, rest) = scp_match(url).or_else(|| standard_match(url))?;
    let trimmed = rest.trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let mut segments: Vec<&str> = trimmed.split('/').filter(|segment| !segment.is_empty()).collect();
    let repo = segments.pop()?;
    let owner = segments.join("/");
    if owner.is_empty() {
        return None;
    }
    Some(RemoteLocation { host: host.to_lowercase(), owner: owner.to_lowercase(), repo: repo.to_lowercase() })
}

/// `^(?:[^@/\s]+@)?([^:/\s]+):(?!\/\/)(.+)$`: the scp-like form, `git@host:owner/repo`.
fn scp_match(url: &str) -> Option<(&str, &str)> {
    for start in user_prefixes(url, |c| c == '/' || is_space(c)) {
        let tail = &url[start..];
        let Some(end) = tail.find(|c: char| c == ':' || c == '/' || is_space(c)) else {
            continue;
        };
        if !tail[end..].starts_with(':') {
            continue;
        }
        let host = &tail[..end];
        let rest = &tail[end + 1..];
        if host.is_empty() || rest.starts_with("//") || !is_path(rest) {
            continue;
        }
        return Some((host, rest));
    }
    None
}

/// `^[a-z+]+:\/\/(?:[^@/]+@)?([^/:]+)(?::\d+)?\/(.+)$`, case-insensitively: the scheme form.
fn standard_match(url: &str) -> Option<(&str, &str)> {
    let scheme_end = url.find(|c: char| !(c.is_ascii_alphabetic() || c == '+'))?;
    if scheme_end == 0 || !url[scheme_end..].starts_with("://") {
        return None;
    }
    let tail = &url[scheme_end + 3..];
    for start in user_prefixes(tail, |c| c == '/') {
        let rest = &tail[start..];
        let Some(end) = rest.find(|c: char| c == ':' || c == '/') else {
            continue;
        };
        let host = &rest[..end];
        if host.is_empty() {
            continue;
        }
        let after = &rest[end..];
        let path = match after.strip_prefix(':') {
            None => &after[1..],
            Some(port) => {
                let digits = port.find(|c: char| !c.is_ascii_digit()).unwrap_or(port.len());
                match port[digits..].strip_prefix('/') {
                    Some(path) if digits > 0 => path,
                    _ => continue,
                }
            }
        };
        if !is_path(path) {
            continue;
        }
        return Some((host, path));
    }
    None
}

/// Where the host may start: after a `user@` prefix when there is one, else at
/// the beginning. The prefix is a run of characters `excluded` does not name,
/// so at most the first `@` can end one.
fn user_prefixes(url: &str, excluded: impl Fn(char) -> bool) -> Vec<usize> {
    let mut starts = Vec::new();
    if let Some(at) = url.find('@') {
        let prefix = &url[..at];
        if !prefix.is_empty() && !prefix.chars().any(&excluded) {
            starts.push(at + 1);
        }
    }
    starts.push(0);
    starts
}

/// `(.+)`: at least one character, and no line terminator, which `.` never matches.
fn is_path(text: &str) -> bool {
    !text.is_empty() && !text.chars().any(is_line_terminator)
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// JavaScript's `\s`.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    ) || ('\u{2000}'..='\u{200a}').contains(&c)
}

/// Decides which allowlist key a repository has. Any remote owned by `nedap`,
/// on any host or alias, denies the repository outright; otherwise the key is
/// `origin`'s location, or the first remote's when there is no `origin`.
pub fn identify_repository(remotes: &[GitRemote]) -> RepositoryIdentity {
    let located: Vec<(&str, RemoteLocation)> = remotes
        .iter()
        .filter_map(|remote| parse_remote(&remote.url).map(|location| (remote.name.as_str(), location)))
        .collect();
    if let Some((name, location)) = located.iter().find(|(_, location)| location.owner.split('/').next() == Some(DENIED_OWNER)) {
        return RepositoryIdentity::DeniedOwner { remote: name.to_string(), owner: location.owner.clone(), location: location.clone() };
    }
    let primary = located.iter().find(|(name, _)| *name == "origin").or_else(|| located.first());
    let Some((_, location)) = primary else {
        return RepositoryIdentity::NoRemote;
    };
    RepositoryIdentity::Identified { key: format!("{}/{}/{}", location.host, location.owner, location.repo), location: location.clone() }
}
