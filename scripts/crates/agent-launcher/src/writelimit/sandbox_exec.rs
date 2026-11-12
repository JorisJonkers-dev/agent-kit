//! The macOS write-limit backend: the agent is started under `sandbox-exec`
//! with a Seatbelt profile that denies every file write except the granted
//! paths. Apple has deprecated `sandbox-exec`, but it is still shipped and is
//! what ADR-0013 names.
//!
//! The profile text is built from fixed rules only. Every path reaches it as a
//! `-D name=value` parameter and is used as `(param "name")`, so no path is
//! ever quoted into the profile and a path cannot add a rule. The rules use
//! `subpath` and `literal` only, never a regular expression. Seatbelt checks
//! the path an operation resolves to, so every path is made canonical first
//! (`/var` is `/private/var` on macOS).
//!
//! Rules are matched last to first, so the denials come after the grants:
//!
//! - the guarded files (`.git`, `commondir`, `gitdir`) and the paths git reads
//!   its hooks and settings from are denied inside whatever grant holds them;
//! - each granted directory itself is denied (its own rename or removal) while
//!   its contents stay writable, so a granted directory cannot be moved away
//!   and replaced;
//! - `file-link` is denied: a hard link to a protected file made inside a
//!   grant would be writable by its new name, since the profile checks paths
//!   and not inodes.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Arm, Armed, Policy, WriteLimit};

/// Where macOS ships it. Not looked up on `PATH`: the agent's environment
/// must not be able to substitute another program.
const SYSTEM_BINARY: &str = "/usr/bin/sandbox-exec";

/// The macOS backend.
pub struct SandboxExec {
    binary: PathBuf,
}

impl SandboxExec {
    /// The `sandbox-exec` macOS ships.
    pub fn system() -> SandboxExec {
        SandboxExec::at(SYSTEM_BINARY)
    }

    /// A `sandbox-exec` at another path (tests).
    pub fn at(binary: impl Into<PathBuf>) -> SandboxExec {
        SandboxExec {
            binary: binary.into(),
        }
    }
}

/// The profile and the parameters it names.
#[derive(Debug, PartialEq)]
struct Profile {
    text: String,
    params: Vec<(String, PathBuf)>,
}

/// The absolute, link-free form of `path`. A path that does not exist yet is
/// resolved through its parent, so a rule for it lands where the path will be.
fn canonical(path: &Path) -> Result<PathBuf, String> {
    match fs::canonicalize(path) {
        Ok(real) => Ok(real),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                return Err(format!("cannot resolve {}: {e}", path.display()));
            };
            fs::canonicalize(parent)
                .map(|p| p.join(name))
                .map_err(|e| format!("cannot resolve {}: {e}", path.display()))
        }
        Err(e) => Err(format!("cannot resolve {}: {e}", path.display())),
    }
}

/// `path` itself, not what it points at: its parent made canonical, then its
/// name. Where `path` is a symlink, a rule for this form keeps the link from
/// being renamed or replaced, which a rule for where it leads would not.
fn entry(path: &Path) -> Result<PathBuf, String> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => fs::canonicalize(parent)
            .map(|p| p.join(name))
            .map_err(|e| format!("cannot resolve {}: {e}", path.display())),
        _ => canonical(path),
    }
}

/// The filters for one kind of path, each named by a numbered parameter.
fn filters(profile: &mut Profile, prefix: &str, filter: &str, paths: &[PathBuf]) -> String {
    let mut text = String::new();
    for (n, path) in paths.iter().enumerate() {
        let name = format!("{prefix}{n}");
        text += &format!("  ({filter} (param \"{name}\"))\n");
        profile.params.push((name, path.clone()));
    }
    text
}

/// A rule, or nothing when it names no path: an `allow` or `deny` without a
/// filter would match every path.
fn rule(verb: &str, filters: &str) -> String {
    if filters.is_empty() {
        String::new()
    } else {
        format!("({verb} file-write*\n{filters})\n")
    }
}

/// Builds the profile for `policy`.
fn profile(policy: &Policy) -> Result<Profile, String> {
    let all = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|p| canonical(p))
            .collect::<Result<Vec<_>, _>>()
    };
    let dirs = all(&policy.dirs)?;
    let files = all(&policy.files)?;
    // Each guarded file is protected as a name and as what it resolves to.
    let mut guarded = Vec::new();
    for path in &policy.guarded {
        for form in [canonical(path)?, entry(path)?] {
            if !guarded.contains(&form) {
                guarded.push(form);
            }
        }
    }
    let denied = all(&policy.denied)?;
    let mut profile = Profile {
        text: String::new(),
        params: Vec::new(),
    };
    let mut allow = filters(&mut profile, "D", "subpath", &dirs);
    allow += &filters(&mut profile, "F", "literal", &files);
    let mut deny = filters(&mut profile, "X", "subpath", &denied);
    deny += &filters(&mut profile, "G", "literal", &guarded);
    deny += &filters(&mut profile, "R", "literal", &dirs);
    profile.text = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n{}{}(deny file-link)\n",
        rule("allow", &allow),
        rule("deny", &deny)
    );
    Ok(profile)
}

/// `sandbox-exec -p <profile> -D name=value ...`, which the agent's command follows.
fn command_prefix(binary: &Path, profile: &Profile) -> Vec<OsString> {
    let mut prefix: Vec<OsString> = vec![binary.into(), "-p".into(), (&profile.text).into()];
    for (name, path) in &profile.params {
        let mut pair = OsString::from(format!("{name}="));
        pair.push(path);
        prefix.push("-D".into());
        prefix.push(pair);
    }
    prefix
}

impl WriteLimit for SandboxExec {
    fn prepare(&self, policy: &Policy) -> Result<Armed, String> {
        let executable = fs::metadata(&self.binary)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        if !executable {
            return Err(format!(
                "sandbox-exec is not available at {}",
                self.binary.display()
            ));
        }
        let prefix = command_prefix(&self.binary, &profile(policy)?);
        // Prove the profile loads (and that sandbox-exec works from here, for
        // it cannot nest) before the agent is started under it.
        let probe = Command::new(&prefix[0])
            .args(&prefix[1..])
            .arg("/usr/bin/true")
            .output()
            .map_err(|e| format!("cannot run sandbox-exec: {e}"))?;
        if !probe.status.success() {
            return Err(format!(
                "sandbox-exec cannot apply the write limit: {}",
                String::from_utf8_lossy(&probe.stderr).trim()
            ));
        }
        Ok(Box::new(Wrapper(prefix)))
    }
}

/// The finished command prefix, waiting for the agent's command.
struct Wrapper(Vec<OsString>);

impl Arm for Wrapper {
    /// Nothing to do in the child: the limit is applied by `sandbox-exec`
    /// when the child executes it.
    fn arm(&self) -> io::Result<()> {
        Ok(())
    }

    fn prefix(&self) -> Vec<OsString> {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests;
