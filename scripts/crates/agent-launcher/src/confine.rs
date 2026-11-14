//! `agent-launcher confine`: runs one command so that, as far as the kernel
//! is concerned, the whole process (and everything it starts) can read only
//! the paths it is given and the system runtime, write only the paths it is
//! given, and, with `--net-connect`, open TCP connections only to the listed
//! ports and bind none.
//!
//! ```text
//! agent-launcher confine [--check] [--read PATH]... [--run PATH]... [--write PATH]...
//!                        [--net-connect PORT]... -- COMMAND [ARGS...]
//! ```
//!
//! * `--read`: readable (files read, directories listed), never executable.
//! * `--run`: readable and executable (an interpreter and its program).
//! * `--write`: a directory that may be read, written and changed beneath.
//! * `--net-connect`: a TCP port a connect may reach; given once or more, no
//!   other port can be reached and no TCP port can be bound.
//! * `--check`: build the confinement and say whether this machine can hold
//!   it, starting nothing.
//!
//! Every confinement also holds the system runtime ([`SYSTEM_RUN`], read and
//! run), the files name resolution and certificate checks read
//! ([`SYSTEM_READ`]), the random devices and `/dev/null`. Nothing under
//! `/proc` or `/sys` is granted: another process's `/proc/<pid>/environ` must
//! stay unreadable, and the programs this was traced with run without them.
//!
//! The launcher puts the confinement on itself and then becomes the command
//! (exec), so the caller's signals and timeout reach the command directly and
//! nothing is left running behind it; it sets nothing in the environment.
//! When the confinement cannot be applied in full (no backend, an old kernel,
//! a path that cannot be granted), it exits 125 and starts nothing: it never
//! runs the command with less than it was asked to hold.

use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::writelimit::{Policy, Reads, WriteLimit};
use crate::NAME;

/// The exit code of a confinement that could not be applied: nothing started.
pub const REFUSED: i32 = 125;

/// The system runtime, readable and executable where it exists: the Nix
/// store (which holds no secret by design) and the usual FHS directories.
pub const SYSTEM_RUN: [&str; 6] = ["/nix/store", "/usr", "/bin", "/sbin", "/lib", "/lib64"];

/// What resolving a host name, reading the time zone and checking a TLS
/// certificate read, where it exists; and the random devices.
pub const SYSTEM_READ: [&str; 14] = [
    "/etc/ssl",
    "/etc/pki",
    "/etc/ca-certificates",
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/host.conf",
    "/etc/nsswitch.conf",
    "/etc/gai.conf",
    "/etc/services",
    "/etc/protocols",
    "/etc/localtime",
    "/etc/zoneinfo",
    "/dev/urandom",
    "/dev/random",
];

/// The devices a confined command may write.
const DEVICES: [&str; 1] = ["/dev/null"];

/// One `confine` command line.
#[derive(Debug, Default, PartialEq)]
pub struct Spec {
    pub check: bool,
    pub read: Vec<PathBuf>,
    pub run: Vec<PathBuf>,
    pub write: Vec<PathBuf>,
    pub ports: Option<Vec<u16>>,
    pub command: Vec<OsString>,
}

/// Reads the arguments after `confine`.
pub fn parse(args: &[OsString]) -> Result<Spec, String> {
    let mut spec = Spec::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let flag = arg.to_string_lossy();
        if flag == "--" {
            spec.command = rest.cloned().collect();
            break;
        }
        if flag == "--check" {
            spec.check = true;
            continue;
        }
        let value = match flag.as_ref() {
            "--read" | "--run" | "--write" | "--net-connect" => rest.next().ok_or(format!("{flag} needs a value"))?,
            _ => return Err(format!("unknown confine option {flag:?}")),
        };
        match flag.as_ref() {
            "--read" => spec.read.push(PathBuf::from(value)),
            "--run" => spec.run.push(PathBuf::from(value)),
            "--write" => spec.write.push(PathBuf::from(value)),
            _ => spec.ports.get_or_insert_with(Vec::new).push(port(value)?),
        }
    }
    if spec.command.is_empty() != spec.check {
        return Err("confine needs a command after --, or --check and no command".into());
    }
    Ok(spec)
}

fn port(value: &OsStr) -> Result<u16, String> {
    match value.to_string_lossy().parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(format!("--net-connect takes a TCP port (1-65535), not {:?}", value.to_string_lossy())),
    }
}

/// The paths of `system` that exist here.
fn existing(system: &[&str]) -> Vec<PathBuf> {
    system.iter().map(PathBuf::from).filter(|p| p.exists()).collect()
}

/// The policy for `spec`: its own grants and the system's.
pub fn policy(spec: &Spec) -> Policy {
    let mut read = existing(&SYSTEM_READ);
    read.extend(spec.read.iter().cloned());
    let mut run = existing(&SYSTEM_RUN);
    run.extend(spec.run.iter().cloned());
    Policy {
        dirs: spec.write.clone(),
        files: existing(&DEVICES),
        reads: Some(Reads { read, run }),
        connect_ports: spec.ports.clone(),
        ..Policy::default()
    }
}

/// Runs one `confine` command line: exit 2 for bad usage, [`REFUSED`] when
/// the confinement cannot be applied; otherwise it does not return, for the
/// process has become the command (unless the command cannot be started).
pub fn confine(args: &[OsString], limit: &dyn WriteLimit) -> i32 {
    let spec = match parse(args) {
        Ok(spec) => spec,
        Err(message) => {
            eprintln!("{NAME}: {message}");
            eprintln!("usage: {NAME} confine [--check] [--read PATH]... [--run PATH]... [--write PATH]... [--net-connect PORT]... -- COMMAND [ARGS...]");
            return 2;
        }
    };
    let policy = policy(&spec);
    let armed = match limit.prepare(&policy) {
        Ok(armed) => armed,
        Err(message) => {
            eprintln!("{NAME}: confine: the confinement cannot be applied here: {message}; nothing started");
            return REFUSED;
        }
    };
    if spec.check {
        println!("confine: ok, {}", summary(&policy));
        return 0;
    }
    let mut command = Command::new(&spec.command[0]);
    command.args(&spec.command[1..]);
    // SAFETY: arming only makes syscalls; the launcher is single-threaded
    // here, so the limit covers the whole process the command becomes.
    unsafe { command.pre_exec(crate::child_hook(armed)) };
    let error = command.exec();
    eprintln!("{NAME}: confine: cannot start {} under the confinement: {error}; nothing started", Path::new(&spec.command[0]).display());
    REFUSED
}

/// What a confinement grants, in one line.
fn summary(policy: &Policy) -> String {
    let reads = policy.reads.as_ref().map_or(0, |r| r.read.len() + r.run.len());
    let net = match &policy.connect_ports {
        Some(ports) => format!("TCP connect to port(s) {}", ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ")),
        None => "network not limited".into(),
    };
    format!("{reads} readable path(s), {} writable, {net}", policy.dirs.len() + policy.files.len())
}

#[cfg(test)]
mod tests;
