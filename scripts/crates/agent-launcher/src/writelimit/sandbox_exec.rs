//! The macOS backend: the agent is started under `sandbox-exec` with a Seatbelt
//! profile. For a reach level that profile denies every file write except the
//! granted paths; for a confinement ([`Policy::reads`], ADR-0016) it is built
//! from `(deny default)` and grants only the reads, runs, writes and TCP ports
//! the policy names and the macOS runtime traced in the ADR ([`confinement`]).
//! Apple has deprecated `sandbox-exec`, but it is still shipped and is what
//! ADR-0013 names.
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
//! - the employer roots are denied for `file-read*` and `file-write*`, last of
//!   all (the read limit of the personal instance, ADR-0013): a read, a
//!   listing or a stat under them fails, and so does moving a file out, at
//!   every reach level, `full` included, where the write rules are left out
//!   and only this denial (and the link denial) remains;
//! - `file-link` is denied: a hard link to a protected file made inside a
//!   grant would be writable by its new name, since the profile checks paths
//!   and not inodes.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Arm, Armed, Policy, Reads, WriteLimit};

/// Where macOS ships it. Not looked up on `PATH`: the agent's environment
/// must not be able to substitute another program.
const SYSTEM_BINARY: &str = "/usr/bin/sandbox-exec";

/// The program the profile is proved with before the agent starts.
const PROBE: &str = "/usr/bin/true";

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

/// The absolute, link-free form of `path`, which must exist.
fn canonical(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))
}

/// The same for a path that may not exist yet: its nearest existing ancestor
/// made canonical, then the rest, so a rule for it lands where it will be.
fn canonical_or_planned(path: &Path) -> Result<PathBuf, String> {
    match (fs::canonicalize(path), path.parent(), path.file_name()) {
        (Ok(real), _, _) => Ok(real),
        (Err(e), Some(parent), Some(name)) if e.kind() == io::ErrorKind::NotFound => {
            Ok(canonical_or_planned(parent)?.join(name))
        }
        (Err(e), _, _) => Err(format!("cannot resolve {}: {e}", path.display())),
    }
}

/// `path` itself, not what it points at: its parent made canonical, then its
/// name. Where `path` is a symlink, a rule for this form keeps the link from
/// being renamed or replaced, which a rule for where it leads would not.
fn entry(path: &Path) -> PathBuf {
    let parent = path.parent().and_then(|p| fs::canonicalize(p).ok());
    match (parent, path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
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
fn rule(verb: &str, operation: &str, filters: &str) -> String {
    if filters.is_empty() {
        String::new()
    } else {
        format!("({verb} {operation}\n{filters})\n")
    }
}

/// What every confined process needs from macOS besides files, as traced under
/// `(deny default)` with `log stream` (ADR-0013): the symlinks at the root of
/// the volume that path resolution looks at (`/etc` and `/var` lead into
/// `/private`), the kernel's read-only facts (`sysctl`, which dyld and the
/// language runtimes ask), forking and signalling itself (a shell starts
/// children). No Mach service, no IPC, no process other than its own.
const RUNTIME_RULES: &str = "\
(allow file-read-data (literal \"/\"))
(allow file-read-metadata (literal \"/\") (literal \"/etc\") (literal \"/var\") (literal \"/tmp\") (literal \"/private/etc/localtime\") (literal \"/private/var/select/sh\"))
(allow sysctl-read)
(allow process-fork)
(allow signal (target self))
(allow mach-lookup (global-name \"com.apple.trustd.agent\"))
";

/// The paths of the standard streams that are files or terminals (a pipe or a
/// socket has none), as the kernel names them.
#[cfg(target_os = "macos")]
fn stdio_paths() -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut paths: Vec<PathBuf> = Vec::new();
    for fd in 0..3 {
        let mut buf = [0u8; libc::PATH_MAX as usize];
        // SAFETY: F_GETPATH writes a NUL-terminated path of at most PATH_MAX bytes into the buffer.
        let found = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) } == 0;
        let length = buf.iter().position(|b| *b == 0).unwrap_or(0);
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..length]));
        if found && path.is_absolute() && !paths.contains(&path) {
            paths.push(path);
        }
    }
    paths
}

/// Only macOS names a descriptor's path, and only macOS needs it.
#[cfg(not(target_os = "macos"))]
fn stdio_paths() -> Vec<PathBuf> {
    Vec::new()
}

/// What `--keychain` adds: the Security server, which a client's `security` call (and a native keychain read)
/// asks for the items whose access list names it. The keychain files are read grants, added by the launcher.
const KEYCHAIN_RULES: &str = "(allow mach-lookup (global-name \"com.apple.SecurityServer\"))\n";

/// What `--preferences` adds: the preferences daemon, which a native client (Codex) asks for the system's and the
/// managed (device-management) preferences before it starts.
const PREFERENCES_RULES: &str = "(allow user-preference-read)
(allow mach-lookup (global-name \"com.apple.cfprefsd.daemon\") (global-name \"com.apple.cfprefsd.agent\"))
(allow file-read* (subpath \"/Library/Managed Preferences\"))
(allow ipc-posix-shm-read-data (ipc-posix-name-prefix \"apple.cfprefs.\"))
";

/// The directory sockets name resolution talks to: the only way off the
/// machine that is not a TCP connect, and only granted with a port.
const DNS_SOCKET: &str = "/private/var/run/mDNSResponder";

/// The head of a confinement profile: everything denied but what `reads`, the
/// writable paths and the network grant. `dirs` and `files` are the writable
/// paths, already made canonical.
fn confinement(
    profile: &mut Profile,
    policy: &Policy,
    reads: &Reads,
    dirs: &[PathBuf],
    files: &[PathBuf],
    writable: &str,
) -> Result<String, String> {
    let strict = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|p| canonical(p))
            .collect::<Result<Vec<_>, _>>()
    };
    let read = strict(&reads.read)?;
    let run = strict(&reads.run)?;
    // A runtime walks every ancestor of a path to resolve it (Node's realpath
    // lstats each one), so each ancestor may be stat'ed (never listed or read).
    let mut ancestors: Vec<PathBuf> = Vec::new();
    for path in read.iter().chain(&run).chain(dirs).chain(files) {
        for ancestor in path.ancestors().skip(1) {
            let ancestor = ancestor.to_path_buf();
            if ancestor.parent().is_some() && !ancestors.contains(&ancestor) {
                ancestors.push(ancestor);
            }
        }
    }
    let mut text = format!("(version 1)\n(deny default)\n{RUNTIME_RULES}");
    // The files the process was started with as stdin, stdout and stderr: an
    // `fstat` on an open descriptor is checked against its path, and Node
    // aborts at start when it cannot stat a standard stream that is a file.
    text += &rule("allow", "file-read-metadata file-ioctl", &filters(profile, "S", "literal", &stdio_paths()));
    text += &rule("allow", "file-read-metadata", &filters(profile, "A", "literal", &ancestors));
    text += &rule("allow", "file-read*", &filters(profile, "P", "subpath", &read));
    text += &rule(
        "allow",
        "file-read* file-map-executable process-exec",
        &filters(profile, "Q", "subpath", &run),
    );
    text += &rule("allow", "file-read* file-write*", writable);
    // A connect is allowed to the listed ports on any host (as Landlock's
    // `ConnectTcp` does, which names no host); names are resolved through
    // mDNSResponder, and nothing is ever bound. Without a port list the
    // network is left alone, as on Linux.
    match &policy.connect_ports {
        Some(ports) => {
            for port in ports {
                text += &format!("(allow network-outbound (remote tcp \"*:{port}\"))\n");
            }
            text += &format!(
                "(allow network-outbound (remote unix-socket (path-literal \"{DNS_SOCKET}\")))\n"
            );
        }
        None => text += "(allow network*)\n",
    }
    if policy.keychain {
        text += KEYCHAIN_RULES;
    }
    if policy.preferences {
        text += PREFERENCES_RULES;
    }
    // `/System` is granted whole, and the user's volume is mounted beneath it
    // (`/System/Volumes/Data`, which firmlinks `/Users`, `/Library` and
    // `/private` to the paths the rest of the system sees). Seatbelt resolves
    // a firmlink to its path before it matches, so reading `$HOME` through
    // that prefix is refused already (`tests/confine.rs` proves it); this
    // denial after the grants keeps it that way if that ever changes, and
    // keeps the swap volume out of reach.
    text += "(deny file-read* file-map-executable process-exec\n  (subpath \"/System/Volumes/Data\")\n  (subpath \"/System/Volumes/VM\")\n)\n";
    Ok(text)
}

/// Builds the profile for `policy`.
fn profile(policy: &Policy) -> Result<Profile, String> {
    // A grant must exist to be granted.
    let strict = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|p| canonical(p))
            .collect::<Result<Vec<_>, _>>()
    };
    let dirs = strict(&policy.dirs)?;
    let files = strict(&policy.files)?;
    // Each guarded file is protected as a name and as what it resolves to.
    let mut guarded = Vec::new();
    for path in &policy.guarded {
        for form in [canonical_or_planned(path)?, entry(path)] {
            if !guarded.contains(&form) {
                guarded.push(form);
            }
        }
    }
    // Each denied path is kept both as what it resolves to and by its own
    // name, so a link at the name cannot be removed and replaced.
    let mut denied = Vec::new();
    for path in &policy.denied {
        for form in [canonical_or_planned(path)?, entry(path)] {
            if !denied.contains(&form) {
                denied.push(form);
            }
        }
    }
    let mut profile = Profile {
        text: String::new(),
        params: Vec::new(),
    };
    let mut allow = filters(&mut profile, "D", "subpath", &dirs);
    allow += &filters(&mut profile, "F", "literal", &files);
    let mut deny = filters(&mut profile, "X", "subpath", &denied);
    deny += &filters(&mut profile, "G", "literal", &guarded);
    deny += &filters(&mut profile, "R", "literal", &dirs);
    let roots: Vec<PathBuf> = policy
        .read_denied
        .iter()
        .map(|p| canonical_or_planned(p))
        .collect::<Result<_, _>>()?;
    // Writes are denied there too, so a file cannot be moved out of a root and
    // read under its new name; links are denied for the same reason.
    let read_rule = rule("deny", "file-read* file-write*", &filters(&mut profile, "E", "subpath", &roots));
    if let Some(reads) = &policy.reads {
        let head = confinement(&mut profile, policy, reads, &dirs, &files, &allow)?;
        profile.text = format!(
            "{head}{}{read_rule}(deny file-link)\n",
            rule("deny", "file-write*", &deny)
        );
        return Ok(profile);
    }
    let write_rules = if policy.writes_unlimited {
        String::new()
    } else {
        format!(
            "(deny file-write*)\n{}{}(deny file-link)\n",
            rule("allow", "file-write*", &allow),
            rule("deny", "file-write*", &deny)
        )
    };
    let link = if policy.writes_unlimited && !roots.is_empty() { "(deny file-link)\n" } else { "" };
    profile.text = format!("(version 1)\n(allow default)\n{write_rules}{read_rule}{link}");
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
        // A port limit is only built into a confinement's profile; a reach
        // level's profile leaves the network alone, so a policy that asks for
        // one without a confinement is refused instead of losing the limit.
        if policy.connect_ports.is_some() && policy.reads.is_none() {
            return Err("a network limit (--net-connect) is held on macOS only with a confinement; it was asked for without one".into());
        }
        let executable = fs::metadata(&self.binary)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        if !executable {
            return Err(format!(
                "sandbox-exec is not available at {}",
                self.binary.display()
            ));
        }
        let mut profile = profile(policy)?;
        let prefix = command_prefix(&self.binary, &profile);
        // Prove the profile loads (and that sandbox-exec works from here, for
        // it cannot nest) before the agent is started under it. A confinement
        // denies every exec, so its probe is the same profile with one more
        // rule that lets `true` run: the probe then proves the runtime set
        // holds, and the agent's own profile gains nothing.
        if policy.reads.is_some() {
            profile.text += &format!("(allow file-read* file-map-executable process-exec (literal \"{PROBE}\"))\n");
        }
        let probe_prefix = command_prefix(&self.binary, &profile);
        let probe = Command::new(&probe_prefix[0])
            .args(&probe_prefix[1..])
            .arg(PROBE)
            .output()
            .map_err(|e| format!("cannot run sandbox-exec: {e}"))?;
        if !probe.status.success() {
            return Err(format!(
                "sandbox-exec cannot apply the profile: {}",
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
