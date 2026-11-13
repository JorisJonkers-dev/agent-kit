//! Paths inside a repository. The node runner's `application/paths.ts`.

use crate::error::CritiqueError;
use model_gateway::text::resolve as resolve_path;

/// The repository-relative form of `path`; a path outside the repository is an error.
pub fn inside_repository(root: &str, path: &str) -> Result<String, CritiqueError> {
    let absolute = resolve_path(root, path);
    let relative = if absolute == root {
        Some("")
    } else {
        absolute.strip_prefix(&format!("{}/", root.trim_end_matches('/')))
    };
    match relative {
        Some(relative) if !relative.starts_with("..") => Ok(relative.to_string()),
        _ => Err(CritiqueError::new("outside-repository", format!("{path} is outside the repository"))),
    }
}

/// `path.join`: the parts joined with `/` and normalised, staying relative when the first non-empty part is. The
/// gateway's own `join` is for absolute results only.
pub fn join(parts: &[&str]) -> String {
    let joined = parts.iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join("/");
    if joined.is_empty() {
        return ".".to_string();
    }
    let absolute = joined.starts_with('/');
    let mut kept: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." if kept.last().is_some_and(|last| *last != "..") => {
                kept.pop();
            }
            ".." if absolute => {}
            other => kept.push(other),
        }
    }
    let body = kept.join("/");
    let trailing = if joined.ends_with('/') && !body.is_empty() { "/" } else { "" };
    match (absolute, body.is_empty()) {
        (true, true) => "/".to_string(),
        (true, false) => format!("/{body}{trailing}"),
        (false, true) => ".".to_string(),
        (false, false) => format!("{body}{trailing}"),
    }
}
