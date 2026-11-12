//! The POSIX path functions the state directory and the atomic write depend on.

/// An absolute POSIX path with `.`, `..` and repeated separators removed, as `path.normalize` does.
pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `path.join` for absolute results.
pub fn join(parts: &[&str]) -> String {
    normalize(&parts.join("/"))
}

/// `path.resolve(cwd, path)`: `path` itself when absolute, else beneath the absolute `cwd`.
pub fn resolve(cwd: &str, path: &str) -> String {
    if path.starts_with('/') {
        normalize(path)
    } else {
        normalize(&format!("{cwd}/{path}"))
    }
}

/// `path.basename`.
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit_once('/').map_or(trimmed, |(_, name)| name)
}

/// `path.dirname` for an absolute path.
pub fn dirname(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    let head = trimmed.rfind('/').map_or("", |at| &trimmed[..at]).trim_end_matches('/');
    if head.is_empty() {
        "/"
    } else {
        head
    }
}
