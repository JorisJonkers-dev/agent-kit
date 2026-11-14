//! The Linux write-limit backend: Landlock, through its three syscalls.
//!
//! A write limit handles the write-side rights alone, so reads and execution
//! stay unrestricted. A confinement ([`Policy::reads`], `agent-launcher
//! confine`) also handles reading files, listing directories and executing,
//! so only the granted paths can be read or run, and, where the kernel has
//! them, connecting to a pathname UNIX socket (ABI 9) and the signal and
//! abstract-socket scopes (ABI 6). [`Policy::connect_ports`] handles TCP
//! connect and bind (ABI 4): a connect reaches only the listed ports, and a
//! bind is never allowed.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use super::{Arm, Armed, Policy, WriteLimit};

const ABI_MIN: i64 = 3;
/// The ABI that limits TCP (Linux 6.7).
const ABI_NET: i64 = 4;
/// The ABI with the signal and abstract UNIX socket scopes (Linux 6.12).
const ABI_SCOPE: i64 = 6;
/// The ABI that limits connecting to a pathname UNIX socket.
const ABI_RESOLVE_UNIX: i64 = 9;
const SYS_CREATE_RULESET: libc::c_long = libc::SYS_landlock_create_ruleset;
const SYS_ADD_RULE: libc::c_long = libc::SYS_landlock_add_rule;
const SYS_RESTRICT_SELF: libc::c_long = libc::SYS_landlock_restrict_self;
const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: u32 = 1;
const RULE_NET_PORT: u32 = 2;

const EXECUTE: u64 = 1 << 0;
const READ_FILE: u64 = 1 << 2;
const READ_DIR: u64 = 1 << 3;
const RESOLVE_UNIX: u64 = 1 << 16;
/// Reading a file and listing a directory.
const READ: u64 = READ_FILE | READ_DIR;
const NET_BIND_TCP: u64 = 1 << 0;
const NET_CONNECT_TCP: u64 = 1 << 1;
const SCOPE_ABSTRACT_UNIX_SOCKET: u64 = 1 << 0;
const SCOPE_SIGNAL: u64 = 1 << 1;
const WRITE_FILE: u64 = 1 << 1;
const REMOVE_DIR: u64 = 1 << 4;
const REMOVE_FILE: u64 = 1 << 5;
const MAKE_ANY: u64 = 0x7f << 6; // char, dir, reg, sock, fifo, block, sym
const REFER: u64 = 1 << 13;
const TRUNCATE: u64 = 1 << 14;
/// Every write-side right (ABI 3). Read and execute are left unhandled, so
/// they stay unrestricted.
const HANDLED: u64 = WRITE_FILE | REMOVE_DIR | REMOVE_FILE | MAKE_ANY | REFER | TRUNCATE;
/// The rights that apply to a file rather than a directory.
const FILE_ONLY: u64 = WRITE_FILE | TRUNCATE;

/// What a ruleset handles: every right named here is denied unless a rule
/// grants it. The kernel takes a larger structure than it knows as long as
/// the fields it does not know are zero, so all three are always passed.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RulesetAttr {
    handled_access_fs: u64,
    handled_access_net: u64,
    scoped: u64,
}

#[repr(C)]
struct NetPort {
    allowed_access: u64,
    port: u64,
}

#[repr(C, packed)]
struct PathBeneath {
    allowed_access: u64,
    parent_fd: i32,
}

/// The kernel calls, thin enough that the error mapping is all that is above them.
struct Sys;

impl Sys {
    fn call(result: libc::c_long) -> io::Result<i64> {
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(i64::from(result))
        }
    }

    fn version() -> io::Result<i64> {
        // SAFETY: a null attribute with size 0 and the VERSION flag only asks the ABI.
        Self::call(unsafe {
            libc::syscall(
                SYS_CREATE_RULESET,
                std::ptr::null::<RulesetAttr>(),
                0usize,
                CREATE_RULESET_VERSION,
            )
        })
    }

    fn create(attr: &RulesetAttr) -> io::Result<OwnedFd> {
        // SAFETY: `attr` is a valid, live attribute of the size passed.
        let fd = Self::call(unsafe {
            libc::syscall(
                SYS_CREATE_RULESET,
                attr as *const RulesetAttr,
                std::mem::size_of::<RulesetAttr>(),
                0u32,
            )
        })?;
        // SAFETY: the kernel returned a new descriptor that nothing else owns.
        Ok(unsafe { OwnedFd::from_raw_fd(fd as RawFd) })
    }

    fn add(ruleset: RawFd, access: u64, parent: RawFd) -> io::Result<()> {
        let rule = PathBeneath {
            allowed_access: access,
            parent_fd: parent,
        };
        // SAFETY: `rule` is a valid, live path-beneath rule.
        Self::call(unsafe {
            libc::syscall(
                SYS_ADD_RULE,
                ruleset,
                RULE_PATH_BENEATH,
                &rule as *const PathBeneath,
                0u32,
            )
        })
        .map(drop)
    }

    fn add_port(ruleset: RawFd, access: u64, port: u16) -> io::Result<()> {
        let rule = NetPort {
            allowed_access: access,
            port: u64::from(port),
        };
        // SAFETY: `rule` is a valid, live net-port rule.
        Self::call(unsafe {
            libc::syscall(
                SYS_ADD_RULE,
                ruleset,
                RULE_NET_PORT,
                &rule as *const NetPort,
                0u32,
            )
        })
        .map(drop)
    }

    fn restrict(ruleset: RawFd) -> io::Result<()> {
        // SAFETY: plain prctl and syscall on integers.
        Self::call(unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } as libc::c_long)?;
        Self::call(unsafe { libc::syscall(SYS_RESTRICT_SELF, ruleset, 0u32) }).map(drop)
    }
}

/// Reads the kernel's answer to an ABI query as a refusal message, or the ABI.
fn abi_from(result: io::Result<i64>) -> Result<i64, String> {
    match result {
        Ok(abi) if abi >= ABI_MIN => Ok(abi),
        Ok(abi) => Err(format!(
            "Landlock ABI {abi} is too old; the write limit needs ABI {ABI_MIN} or newer (Linux 6.2)"
        )),
        Err(e) if e.raw_os_error() == Some(libc::ENOSYS) => {
            Err("this kernel has no Landlock (Linux 5.13 or newer built with it is needed)".into())
        }
        Err(e) if e.raw_os_error() == Some(libc::EOPNOTSUPP) => {
            Err("Landlock is built into this kernel but disabled (add it to the lsm= boot list)".into())
        }
        Err(e) => Err(format!("cannot query Landlock: {e}")),
    }
}

/// The Linux backend.
pub struct Landlock;

impl WriteLimit for Landlock {
    fn prepare(&self, policy: &Policy) -> Result<Armed, String> {
        // Landlock only grants: it cannot carve a directory out of the reads
        // the agent already has. The Linux workstation holds no employer
        // clones (ADR-0011), so asking it to keep one unreadable is refused.
        if !policy.read_denied.is_empty() {
            return Err("Landlock cannot deny reads under the employer roots; the read limit is held on the work Mac (sandbox-exec) only".into());
        }
        let abi = abi_from(Sys::version())?;
        build(policy, handled_for(policy, abi)?)
    }
}

/// What the ruleset for `policy` handles on a kernel of Landlock ABI `abi`,
/// or why that kernel cannot hold it.
fn handled_for(policy: &Policy, abi: i64) -> Result<RulesetAttr, String> {
    if policy.connect_ports.is_some() && abi < ABI_NET {
        return Err(format!(
            "Landlock ABI {abi} cannot limit the network; the confinement needs ABI {ABI_NET} or newer (Linux 6.7)"
        ));
    }
    let confined = policy.reads.is_some();
    let resolve_unix = if abi >= ABI_RESOLVE_UNIX { RESOLVE_UNIX } else { 0 };
    let scoped = if abi >= ABI_SCOPE { SCOPE_ABSTRACT_UNIX_SOCKET | SCOPE_SIGNAL } else { 0 };
    Ok(RulesetAttr {
        handled_access_fs: if confined { HANDLED | READ | EXECUTE | resolve_unix } else { HANDLED },
        handled_access_net: if policy.connect_ports.is_some() { NET_BIND_TCP | NET_CONNECT_TCP } else { 0 },
        scoped: if confined { scoped } else { 0 },
    })
}

/// The ruleset for `policy` handling the rights `handled`. A granted
/// directory gets every write-side right and, in a confinement, reading too;
/// a read grant reads, a run grant reads and executes. A file gets only the
/// rights that apply to a file.
fn build(policy: &Policy, handled: RulesetAttr) -> Result<Armed, String> {
    let reads = handled.handled_access_fs & READ;
    let read_file = reads & READ_FILE;
    let ruleset = Sys::create(&handled).map_err(|e| format!("cannot create the Landlock ruleset: {e}"))?;
    let fd = ruleset.as_raw_fd();
    for path in &policy.dirs {
        grant(fd, path, HANDLED | reads, "writable")?;
    }
    for path in &policy.files {
        grant(fd, path, FILE_ONLY | read_file, "writable")?;
    }
    let none = super::Reads::default();
    let granted = policy.reads.as_ref().unwrap_or(&none);
    for (paths, access) in [(&granted.read, READ), (&granted.run, READ | EXECUTE)] {
        for path in paths {
            grant(fd, path, access, "readable")?;
        }
    }
    for port in policy.connect_ports.iter().flatten() {
        Sys::add_port(fd, NET_CONNECT_TCP, *port)
            .map_err(|e| format!("cannot allow TCP connections to port {port}: {e}"))?;
    }
    Ok(Box::new(Ruleset(ruleset)))
}

/// A finished ruleset, waiting to be put on a child.
struct Ruleset(OwnedFd);

impl Arm for Ruleset {
    fn arm(&self) -> io::Result<()> {
        Sys::restrict(self.0.as_raw_fd())
    }
}

/// Grants `access` beneath `path`. A read grant on a file drops the right to
/// list, which only a directory has; a write grant keeps its directory
/// rights, so the kernel refuses one on a file.
fn grant(ruleset: RawFd, path: &Path, access: u64, kind: &str) -> Result<(), String> {
    // O_PATH needs no read permission and opens a device without its driver.
    let opened = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(path)
        .map_err(|e| format!("cannot open {kind} path {}: {e}", path.display()))?;
    let is_dir = opened.metadata().map(|m| m.is_dir()).unwrap_or(false);
    let access = if is_dir || access & READ == 0 { access } else { access & !READ_DIR };
    Sys::add(ruleset, access, opened.as_raw_fd())
        .map_err(|e| format!("cannot grant {kind} access to {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
