//! The employer roots (ADR-0013): the directories holding the employer's
//! clones on this machine, which the personal Gold Band instance may not read.
//!
//! They come from one declared place, the per-machine overlay
//! (`~/.config/agent-kit/overlay.yaml`, beside the keep file), under the work
//! instance: `instances.gold-band.employer_roots`, a list of absolute or `~/`
//! paths. No path is built into the launcher.
//!
//! * No overlay, or one that does not name the work instance: this is not the
//!   work machine and there is nothing to keep out of reach.
//! * An overlay that names the work instance must declare at least one root.
//!   An overlay that cannot be read or understood is refused too. Neither is
//!   ever read as "no roots": the launch fails closed.

use std::fs;
use std::path::{Path, PathBuf};

/// The work instance, whose declaration holds the roots.
const WORK_INSTANCE: &str = "gold-band";

/// The agents the work instance holds (ADR-0011); every other agent id
/// belongs to the personal instance. Naming the work agents instead of the
/// personal ones means an agent nobody classified is limited, not exempt.
pub const WORK_AGENTS: [&str; 2] = ["claude-work", "codex"];

/// Where the overlay lives for a home directory.
pub fn overlay_path(home: &Path) -> PathBuf {
    home.join(".config/agent-kit/overlay.yaml")
}

/// The declared employer roots, each made absolute (`~/` is `home`).
pub fn employer_roots(overlay: Option<&Path>, home: Option<&Path>) -> Result<Vec<PathBuf>, String> {
    let Some(file) = overlay else {
        return Ok(Vec::new());
    };
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot read the overlay {}: {e}", file.display())),
    };
    let doc: serde_yaml::Value = serde_yaml::from_str(&text)
        .map_err(|e| format!("cannot parse the overlay {}: {e}", file.display()))?;
    let Some(work) = doc.get("instances").and_then(|i| i.get(WORK_INSTANCE)) else {
        return Ok(Vec::new());
    };
    let declared = work
        .get("employer_roots")
        .and_then(serde_yaml::Value::as_sequence)
        .filter(|roots| !roots.is_empty())
        .ok_or_else(|| {
            format!(
                "the overlay {} names the work instance but declares no employer_roots, so the personal instance cannot be kept from the employer's clones",
                file.display()
            )
        })?;
    declared
        .iter()
        .map(|root| absolute(root.as_str().unwrap_or(""), home))
        .collect()
}

/// One declared root, absolute and free of `..`.
fn absolute(root: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let path = match (root.strip_prefix("~/"), home) {
        (Some(rest), Some(home)) => home.join(rest),
        (Some(_), None) => return Err(format!("employer root {root:?} needs HOME, which is not set")),
        (None, _) => PathBuf::from(root),
    };
    let plain = path
        .components()
        .all(|c| !matches!(c, std::path::Component::ParentDir));
    if !path.is_absolute() || !plain || path == Path::new("/") {
        return Err(format!(
            "employer root {root:?} must be an absolute path (or start with ~/) with no .. part, and not /"
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
