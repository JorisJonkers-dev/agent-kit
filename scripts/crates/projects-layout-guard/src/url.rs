//! The two repository-spec patterns the guard reads, hand-matched so the crate
//! stays std-only: a GitHub URL (https, ssh or scp style) and `owner/repo`.

/// owner and repo of a GitHub URL, or `None`.
///
/// `^(?:(?:https?|ssh|git)://(?:[^@/]+@)?github\.com/|(?:[^@\s]+@)?github\.com:)`
/// `(?P<owner>[^/\s]+)/(?P<repo>[^/\s]+?)(?:\.git)?/?$`
pub fn github(url: &str) -> Option<(String, String)> {
    let mut tails: Vec<&str> = Vec::new();
    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(rest) = url.strip_prefix(scheme) {
            tails.push(rest);
            if let Some(at) = rest.find(['@', '/']) {
                if rest.as_bytes()[at] == b'@' && at > 0 {
                    tails.push(&rest[at + 1..]);
                }
            }
        }
    }
    let mut starts = vec![(url, "github.com:")];
    if let Some(at) = url.find('@') {
        if at > 0 && !url[..at].contains(char::is_whitespace) {
            starts.push((&url[at + 1..], "github.com:"));
        }
    }
    let mut candidates: Vec<&str> = tails
        .iter()
        .filter_map(|t| t.strip_prefix("github.com/"))
        .collect();
    candidates.extend(starts.iter().filter_map(|(s, p)| s.strip_prefix(p)));
    candidates.into_iter().find_map(owner_repo)
}

/// `owner/repo` with the optional `.git` and trailing slash of the tail pattern.
fn owner_repo(tail: &str) -> Option<(String, String)> {
    let (owner, rest) = tail.split_once('/')?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let bad = |s: &str| s.is_empty() || s.contains(char::is_whitespace);
    if bad(owner) || bad(rest) || rest.contains('/') {
        return None;
    }
    let repo = match rest.strip_suffix(".git") {
        Some(stem) if !stem.is_empty() => stem,
        _ => rest,
    };
    Some((owner.to_string(), repo.to_string()))
}

/// `^(?P<owner>[A-Za-z0-9_.-]+)/(?P<repo>[A-Za-z0-9_.-]+)$`
pub fn short_spec(spec: &str) -> Option<(String, String)> {
    let (owner, repo) = spec.split_once('/')?;
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    };
    (ok(owner) && ok(repo)).then(|| (owner.to_string(), repo.to_string()))
}
