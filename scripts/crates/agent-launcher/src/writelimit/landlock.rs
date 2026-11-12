//! The Linux write-limit backend: Landlock, through its three syscalls.

use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use super::{Arm, Armed, Policy, WriteLimit};

const ABI_MIN: i64 = 3;
const SYS_CREATE_RULESET: libc::c_long = libc::SYS_landlock_create_ruleset;
const SYS_ADD_RULE: libc::c_long = libc::SYS_landlock_add_rule;
const SYS_RESTRICT_SELF: libc::c_long = libc::SYS_landlock_restrict_self;
const CREATE_RULESET_VERSION: u32 = 1;
const RULE_PATH_BENEATH: u32 = 1;

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

#[repr(C)]
struct RulesetAttr {
    handled_access_fs: u64,
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

    fn create(handled: u64) -> io::Result<OwnedFd> {
        let attr = RulesetAttr {
            handled_access_fs: handled,
        };
        // SAFETY: `attr` is a valid, live attribute of the size passed.
        let fd = Self::call(unsafe {
            libc::syscall(
                SYS_CREATE_RULESET,
                &attr as *const RulesetAttr,
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
        abi_from(Sys::version())?;
        build(policy, HANDLED)
    }
}

/// The ruleset for `policy` handling the rights `handled`.
fn build(policy: &Policy, handled: u64) -> Result<Armed, String> {
    match Sys::create(handled) {
        Err(e) => Err(format!("cannot create the Landlock ruleset: {e}")),
        Ok(ruleset) => {
            for (paths, access) in [(&policy.dirs, handled), (&policy.files, FILE_ONLY)] {
                for path in paths {
                    grant(ruleset.as_raw_fd(), path, access)?;
                }
            }
            Ok(Box::new(Ruleset(ruleset)))
        }
    }
}

/// A finished ruleset, waiting to be put on a child.
struct Ruleset(OwnedFd);

impl Arm for Ruleset {
    fn arm(&self) -> io::Result<()> {
        Sys::restrict(self.0.as_raw_fd())
    }
}

fn grant(ruleset: RawFd, path: &Path, access: u64) -> Result<(), String> {
    // O_PATH needs no read permission and opens a device without its driver.
    let dir = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_PATH)
        .open(path)
        .map_err(|e| format!("cannot open writable path {}: {e}", path.display()))?;
    Sys::add(ruleset, access, dir.as_raw_fd())
        .map_err(|e| format!("cannot allow writes to {}: {e}", path.display()))
}

#[cfg(test)]
mod tests;
