//! `agent-launcher confine`: runs one command so that, as far as the kernel
//! is concerned, the whole process (and everything it starts) can read only
//! the paths it is given and the system runtime, write only the paths it is
//! given, and, with `--net-connect`, open TCP connections only to the listed
//! ports and bind none.
//!
//! ```text
//! agent-launcher confine [--check] [--read PATH]... [--run PATH]... [--run-from PROGRAM]...
//!                        [--write PATH]... [--net-connect PORT]... [--net-loopback] [--keychain] [--preferences] [--env NAME]... -- COMMAND [ARGS...]
//! ```
//!
//! * `--read`: readable (files read, directories listed), never executable.
//! * `--run`: readable and executable (an interpreter and its program).
//! * `--run-from`: what it takes to run PROGRAM (a path, or a name looked up
//!   on `PATH`), readable and executable: see [`program_grants`].
//! * `--write`: a directory that may be read, written and changed beneath, or a
//!   file that may be read and written (and nothing beside it).
//! * `--net-connect`: a TCP port a connect may reach; given once or more, no
//!   other port can be reached and no TCP port can be bound.
//! * `--net-loopback`: (macOS) the command may also bind, accept and connect on
//!   loopback (`localhost`) at any port, which a client that serves itself on a
//!   port chosen at run time needs; with `--net-connect` the rest of the network
//!   stays limited to the listed ports. Refused on Linux, where Landlock cannot
//!   name a port chosen at run time.
//! * `--keychain`: (macOS) the command may ask the login keychain for the items
//!   whose access list names it; its credentials are held there, not in a file.
//!   The keychain files are readable, nothing else of `~/Library` is. Linux
//!   keeps credentials in files, which a `--read`/`--write` names: a no-op.
//! * `--preferences`: (macOS) the command may read the system's and the
//!   managed preferences (Codex asks before it starts). A no-op on Linux.
//! * `--env`: an environment variable the command keeps, besides the few it
//!   always gets (see [`KEEP_ENV`]); given once or more.
//! * `--check`: build the confinement and say whether this machine can hold
//!   it, and list every grant, starting nothing.
//!
//! On Linux the kernel holds it (Landlock); on macOS a Seatbelt profile built
//! from `(deny default)` does, applied by `sandbox-exec` (ADR-0016), which is
//! then the program the launcher becomes and which execs the command.
//!
//! Every confinement also holds the system runtime ([`SYSTEM_RUN`], read and
//! run), the files name resolution and certificate checks read
//! ([`SYSTEM_READ`]), the random devices and `/dev/null`. Nothing under
//! `/proc` or `/sys` is granted: another process's `/proc/<pid>/environ` must
//! stay unreadable, and the programs this was traced with run without them.
//!
//! The launcher puts the confinement on itself and then becomes the command
//! (exec), so the caller's signals and timeout reach the command directly and
//! nothing is left running behind it. The command inherits no more than it is
//! given: its environment is the [`KEEP_ENV`] variables and each `--env` name
//! that the caller has set (no key, token or proxy password rides along
//! unnamed), and every open file descriptor beyond the three standard ones is
//! closed after the confinement is on, so a descriptor the caller left open
//! (a file, a socket, a pipe to another process) is not carried in.
//! When the confinement cannot be applied in full (no backend, an old kernel,
//! a path that cannot be granted), it exits 125 and starts nothing: it never
//! runs the command with less than it was asked to hold.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::writelimit::{Policy, Reads, WriteLimit};
use crate::NAME;

/// The exit code of a confinement that could not be applied: nothing started.
pub const REFUSED: i32 = 125;

/// The system runtime, readable and executable where it exists: the Nix
/// store (which holds no secret by design) and the usual FHS directories.
pub const SYSTEM_RUN: [&str; 7] = ["/nix/store", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/System"];

/// What resolving a host name, reading the time zone and checking a TLS
/// certificate read, where it exists; and the random devices.
pub const SYSTEM_READ: [&str; 16] = [
    "/private/var/db/timezone",
    "/Library/Preferences/.GlobalPreferences.plist",
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

/// The environment a confined command always keeps, when the caller has it:
/// where programs are, the locale, the terminal, the time zone, the
/// temporary directory, and the files and directories of the certificate
/// store, none of which holds a secret. Everything else is named with `--env`.
pub const KEEP_ENV: [&str; 12] = [
    "PATH",
    "HOME",
    "LANG",
    "LANGUAGE",
    "TERM",
    "TZ",
    "TMPDIR",
    "NO_COLOR",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NIX_SSL_CERT_FILE",
    "NODE_EXTRA_CA_CERTS",
];

/// The devices a confined command may write.
const DEVICES: [&str; 1] = ["/dev/null"];

/// One `confine` command line.
#[derive(Debug, Default, PartialEq)]
pub struct Spec {
    pub check: bool,
    pub read: Vec<PathBuf>,
    pub run: Vec<PathBuf>,
    pub run_from: Vec<OsString>,
    pub write: Vec<PathBuf>,
    pub ports: Option<Vec<u16>>,
    pub loopback: bool,
    pub keychain: bool,
    pub preferences: bool,
    pub env: Vec<OsString>,
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
        match flag.as_ref() {
            "--check" => {
                spec.check = true;
                continue;
            }
            "--net-loopback" => {
                spec.loopback = true;
                continue;
            }
            "--keychain" => {
                spec.keychain = true;
                continue;
            }
            "--preferences" => {
                spec.preferences = true;
                continue;
            }
            _ => {}
        }
        let value = match flag.as_ref() {
            "--read" | "--run" | "--run-from" | "--write" | "--net-connect" | "--env" => rest.next().ok_or(format!("{flag} needs a value"))?,
            _ => return Err(format!("unknown confine option {flag:?}")),
        };
        match flag.as_ref() {
            "--read" => spec.read.push(PathBuf::from(value)),
            "--run" => spec.run.push(PathBuf::from(value)),
            "--run-from" => spec.run_from.push(value.clone()),
            "--write" => spec.write.push(PathBuf::from(value)),
            "--env" => spec.env.push(env_name(value)?),
            _ => spec.ports.get_or_insert_with(Vec::new).push(port(value)?),
        }
    }
    if spec.command.is_empty() != spec.check {
        return Err("confine needs a command after --, or --check and no command".into());
    }
    Ok(spec)
}

fn env_name(value: &OsStr) -> Result<OsString, String> {
    let name = value.to_string_lossy();
    if name.is_empty() || name.contains('=') {
        return Err(format!("--env takes a variable name, not {name:?}"));
    }
    Ok(value.to_os_string())
}

/// The variables a confined command gets: those of `vars` named in [`KEEP_ENV`], `LC_*` or `named`.
pub fn kept_env(vars: impl Iterator<Item = (OsString, OsString)>, named: &[OsString]) -> Vec<(OsString, OsString)> {
    vars.filter(|(name, _)| {
        let text = name.to_string_lossy();
        KEEP_ENV.contains(&text.as_ref()) || text.starts_with("LC_") || named.contains(name)
    })
    .collect()
}

/// Closes the descriptors `first` to `last`, or says why not (EINVAL when `first` is past `last`).
#[cfg(target_os = "linux")]
fn close_range(first: libc::c_uint, last: libc::c_uint) -> std::io::Result<()> {
    // SAFETY: closes descriptors only; the launcher holds none it still needs.
    match unsafe { libc::syscall(libc::SYS_close_range, first, last, 0 as libc::c_uint) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

/// Closes every open descriptor from `first` to `last` (macOS has no `close_range`): the kernel lists the ones this
/// process holds, so a descriptor above any fixed bound is closed too. Where it cannot list them it fails, and the
/// confinement is refused rather than started with descriptors the caller left open.
#[cfg(not(target_os = "linux"))]
fn close_range(first: libc::c_uint, last: libc::c_uint) -> std::io::Result<()> {
    let size = std::mem::size_of::<libc::proc_fdinfo>() as libc::c_int;
    // SAFETY: a null buffer of size 0 only asks for the size of the list.
    let bytes = unsafe { libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    if bytes <= 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Room for a few more than were counted: the listing itself may open nothing, but never trust a count to stay.
    let mut fds = vec![libc::proc_fdinfo { proc_fd: 0, proc_fdtype: 0 }; (bytes / size) as usize + 16];
    // SAFETY: the buffer holds `fds.len() * size` bytes, which is what is passed.
    let filled = unsafe { libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDLISTFDS, 0, fds.as_mut_ptr().cast(), fds.len() as libc::c_int * size) };
    if filled <= 0 {
        return Err(std::io::Error::last_os_error());
    }
    for info in &fds[..(filled / size) as usize] {
        let fd = info.proc_fd as libc::c_uint;
        if fd >= first && fd <= last {
            // SAFETY: closes a descriptor this process holds; the launcher keeps none it still needs.
            unsafe { libc::close(info.proc_fd) };
        }
    }
    Ok(())
}

/// What runs before the exec: the confinement, then closing every descriptor from `first` up.
pub(crate) fn confine_hook(armed: crate::writelimit::Armed, first: libc::c_uint) -> impl FnMut() -> std::io::Result<()> + Send + Sync + 'static {
    move || {
        armed.arm()?;
        // Once the confinement is on, this drops what the caller left open; the kernel call allocates nothing, so it
        // is safe between the confinement and the exec.
        close_range(first, libc::c_uint::MAX)
    }
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

/// Where programs are looked up and whose home must never be granted whole.
#[derive(Debug, Default)]
pub struct Env {
    pub path: Option<OsString>,
    pub home: Option<PathBuf>,
}

impl Env {
    fn from_process() -> Env {
        Env {
            path: std::env::var_os("PATH"),
            home: std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from),
        }
    }
}

/// The policy for `spec`: its own grants and the system's, or why a
/// `--run-from` program cannot be granted.
pub fn policy(spec: &Spec, env: &Env) -> Result<Policy, String> {
    // A grant is the path it resolves to, never the path it was written as, and it must exist: a link planted in a
    // directory the command may write cannot move a grant (to `~/.ssh`, say) on a later launch.
    let written: Vec<PathBuf> = spec.write.iter().map(|path| resolve_grant("--write", path)).collect::<Result<_, _>>()?;
    let writable_dirs: Vec<&PathBuf> = written.iter().filter(|path| path.is_dir()).collect();
    for path in spec.read.iter().chain(&spec.run).chain(&spec.write) {
        refuse_a_link_in(path, &writable_dirs)?;
    }
    let mut read = existing(&SYSTEM_READ);
    for path in &spec.read {
        read.push(resolve_grant("--read", path)?);
    }
    let mut run = existing(&SYSTEM_RUN);
    for path in &spec.run {
        run.push(resolve_grant("--run", path)?);
    }
    for program in &spec.run_from {
        run.extend(program_grants(program, env)?);
    }
    // A `--write` path that is a file is granted as that file alone; a directory is granted whole.
    let (written_files, written_dirs): (Vec<PathBuf>, Vec<PathBuf>) = written.into_iter().partition(|path| path.is_file());
    let mut files = existing(&DEVICES);
    files.extend(written_files);
    if spec.keychain {
        // Only the login keychain's file, not the directory that holds every keychain.
        let home = env.home.iter().map(|home| home.join("Library/Keychains/login.keychain-db"));
        read.extend(home.chain([PathBuf::from("/Library/Keychains/System.keychain")]).filter(|path| path.is_file()));
    }
    Ok(Policy {
        dirs: written_dirs,
        files,
        reads: Some(Reads { read, run }),
        connect_ports: spec.ports.clone(),
        loopback: spec.loopback,
        keychain: spec.keychain,
        preferences: spec.preferences,
        ..Policy::default()
    })
}

/// The path a grant names, resolved; one that is not there is refused (a missing `--write` file is never turned
/// into a grant of a directory that may be made at that name later).
fn resolve_grant(option: &str, path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|error| format!("{option} {}: cannot be resolved ({error}); a grant must exist", path.display()))
}

/// Refuses a grant whose path goes through a link that sits in a directory the command may write: the command
/// could have planted it.
fn refuse_a_link_in(path: &Path, writable: &[&PathBuf]) -> Result<(), String> {
    let mut walked = PathBuf::new();
    for part in path.components() {
        walked.push(part);
        let planted = fs::symlink_metadata(&walked).is_ok_and(|meta| meta.file_type().is_symlink())
            && walked.parent().and_then(|parent| fs::canonicalize(parent).ok()).is_some_and(|parent| writable.iter().any(|dir| parent.starts_with(dir)));
        if planted {
            return Err(format!("{}: goes through the link {}, which is in a directory the command may write", path.display(), walked.display()));
        }
    }
    Ok(())
}

/// The command as `sandbox-exec` is given it: a name looked up on `PATH` and every link resolved. The program
/// looks the name up after the profile is on, when the directories of `PATH` and a link in them can no longer be
/// read; a name that is not found is left for it to report.
fn resolved(program: &OsStr, env: &Env) -> OsString {
    match which(program, env) {
        Some(found) => fs::canonicalize(&found).unwrap_or(found).into_os_string(),
        None => program.to_os_string(),
    }
}

/// A file anyone may execute.
fn executable(path: &Path) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `program` itself when it names a path, else the first executable file of
/// that name in a directory of `PATH`.
fn which(program: &OsStr, env: &Env) -> Option<PathBuf> {
    if program.to_string_lossy().contains('/') {
        return Some(PathBuf::from(program)).filter(|p| executable(p));
    }
    let path = env.path.clone().unwrap_or_default();
    std::env::split_paths(&path).map(|dir| dir.join(program)).find(|p| executable(p))
}

/// Whether granting `dir` whole would grant the home directory (or `/`).
fn holds_home(dir: &Path, env: &Env) -> bool {
    dir.parent().is_none() || env.home.as_deref().is_some_and(|home| home.starts_with(dir))
}

/// A directory that holds many programs or whole installs: a prefix (`/usr/local`, `/opt/homebrew`), a shared `bin`
/// of one, the home directory or `~/.local`. Never granted whole.
fn shared(dir: &Path, env: &Env) -> bool {
    const PREFIXES: [&str; 6] = ["/", "/usr", "/usr/local", "/opt", "/opt/homebrew", "/opt/local"];
    let is_prefix = |d: &Path| {
        PREFIXES.iter().any(|p| d == Path::new(p)) || env.home.as_deref().is_some_and(|home| d == home || d == home.join(".local") || d == home.join(".cargo") || d == home.join(".nvm"))
    };
    let bin = dir.file_name().is_some_and(|n| n == "bin" || n == "sbin");
    holds_home(dir, env) || is_prefix(dir) || (bin && dir.parent().is_some_and(is_prefix))
}

/// `dir` when granting it whole is safe, else `file` alone.
fn bounded(dir: &Path, file: &Path, env: &Env) -> PathBuf {
    if shared(dir, env) { file.to_path_buf() } else { dir.to_path_buf() }
}

/// What running `program` reads and executes, with links resolved:
///
/// * the package it belongs to, the directory right under a `node_modules`
///   (or `node_modules/@scope`), else the directory it is in;
/// * for a script, its interpreter (`#!/path` or `#!/usr/bin/env NAME`, the
///   name looked up on `PATH`) with its install: the directory above its
///   `bin`, else the directory it is in.
///
/// A directory that would hold the home directory, or be `/`, is narrowed to
/// the file itself.
pub fn program_grants(program: &OsStr, env: &Env) -> Result<Vec<PathBuf>, String> {
    let name = program.to_string_lossy();
    let found = which(program, env).ok_or(format!("--run-from {name}: no executable file of that name on PATH"))?;
    // `which` found an executable file, so it resolves.
    let real = fs::canonicalize(&found).unwrap_or(found);
    let dir = real.parent().unwrap_or(&real);
    let package = real.ancestors().skip(1).find(|d| {
        let parent = |p: &Path| p.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
        let above = parent(d).unwrap_or_default();
        above == "node_modules" || (above.starts_with('@') && d.parent().and_then(parent).as_deref() == Some("node_modules"))
    });
    let mut grants = vec![bounded(package.unwrap_or(dir), &real, env)];
    if let Some(interpreter) = interpreter(&real, env) {
        let real_interpreter = fs::canonicalize(&interpreter).unwrap_or(interpreter);
        grants.push(install_of(&real_interpreter, env));
    }
    // A launcher script that hands over to another program (`exec '/venv/bin/python3' "$0"`, `exec /opt/app/bin/app`):
    // that program is run too, from the install it is written at (a virtual environment holds its libraries beside
    // its `bin`) and from the install it resolves to (the Python it links to).
    for target in exec_targets(&real) {
        let real_target = fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
        let mut installs = vec![install_of(&real_target, env)];
        // A virtual environment holds its libraries beside the `bin` the script names; that directory is granted
        // as written only when it is one (it has a `pyvenv.cfg`), never a prefix such as `/opt/homebrew`.
        let written = target.parent().and_then(Path::parent).filter(|venv| venv.join("pyvenv.cfg").is_file());
        installs.extend(written.map(|venv| bounded(venv, &target, env)));
        for install in installs {
            // A Python kept in a `python` store directory (`uv`) is reached by a link beside the one it resolves to.
            let install = match install.parent() {
                Some(store) if store.file_name().is_some_and(|n| n == "python") && !shared(store, env) => store.to_path_buf(),
                _ => install,
            };
            if !grants.contains(&install) {
                grants.push(install);
            }
        }
    }
    Ok(grants)
}

/// What running `program` needs of its install: the directory above its `bin` (a `Cellar/<pkg>/<version>`, a
/// tool's directory), else the directory it is in, and the file itself when that directory is shared.
fn install_of(program: &Path, env: &Env) -> PathBuf {
    let bin = program.parent().unwrap_or(program);
    let install = if bin.file_name().is_some_and(|n| n == "bin") { bin.parent().unwrap_or(bin) } else { bin };
    bounded(install, program, env)
}

/// The programs a launcher script runs by an absolute path with `exec`, from its first lines. Only a real command
/// counts: a line that starts with `exec` (after variable assignments), or the Python launcher's
/// `'\'\'exec' '<program>' "$0"` trick; a comment, or `exec` inside a string or another word, does not.
fn exec_targets(script: &Path) -> Vec<PathBuf> {
    let mut head = Vec::new();
    if fs::File::open(script).and_then(|file| file.take(2048).read_to_end(&mut head)).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&head);
    if !text.starts_with("#!") {
        return Vec::new();
    }
    let mut found = Vec::new();
    for line in text.lines().take(12).skip(1) {
        let mut rest = line.trim_start();
        if rest.starts_with('#') {
            continue;
        }
        // Leading `NAME=value` assignments.
        while let Some((word, tail)) = rest.split_once(' ') {
            let name = word.split_once('=').map(|(name, _)| name);
            if name.is_some_and(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')) {
                rest = tail.trim_start();
            } else {
                break;
            }
        }
        let after = if let Some(after) = rest.strip_prefix("\'\'\'exec\'") {
            after
        } else if let Some(after) = rest.strip_prefix("exec").filter(|after| after.starts_with(' ')) {
            after
        } else {
            continue;
        };
        let operand = after.trim_start();
        let path = match operand.chars().next() {
            Some(quote @ ('\'' | '"')) => operand[1..].split(quote).next().unwrap_or(""),
            _ => operand.split_whitespace().next().unwrap_or(""),
        };
        if path.starts_with('/') && !found.iter().any(|known: &PathBuf| known == Path::new(path)) {
            found.push(PathBuf::from(path));
        }
    }
    found
}

/// The interpreter a script's `#!` line names, as found here; none for a
/// file that is not a script or names one that cannot be found.
fn interpreter(script: &Path, env: &Env) -> Option<PathBuf> {
    let mut head = Vec::new();
    fs::File::open(script).ok()?.take(256).read_to_end(&mut head).ok()?;
    let line = String::from_utf8_lossy(&head).lines().next()?.strip_prefix("#!")?.to_string();
    let mut words = line.split_whitespace();
    let first = words.next()?;
    if !first.ends_with("/env") {
        return which(OsStr::new(first), env);
    }
    let name = words.find(|w| !w.starts_with('-'))?;
    which(OsStr::new(name), env)
}

/// Runs one `confine` command line: exit 2 for bad usage, [`REFUSED`] when
/// the confinement cannot be applied; otherwise it does not return, for the
/// process has become the command (unless the command cannot be started).
pub fn confine(args: &[OsString], limit: &dyn WriteLimit) -> i32 {
    confine_from(args, limit, 3)
}

/// [`confine`], closing the descriptors from `first_fd` up before the exec (a test passes one far above its own).
fn confine_from(args: &[OsString], limit: &dyn WriteLimit, first_fd: libc::c_uint) -> i32 {
    let spec = match parse(args) {
        Ok(spec) => spec,
        Err(message) => {
            eprintln!("{NAME}: {message}");
            eprintln!("usage: {NAME} confine [--check] [--read PATH]... [--run PATH]... [--run-from PROGRAM]... [--write PATH]... [--net-connect PORT]... [--net-loopback] [--keychain] [--preferences] [--env NAME]... -- COMMAND [ARGS...]");
            return 2;
        }
    };
    let armed = match policy(&spec, &Env::from_process()).and_then(|policy| limit.prepare(&policy).map(|armed| (armed, policy))) {
        Ok(armed) => armed,
        Err(message) => {
            eprintln!("{NAME}: confine: the confinement cannot be applied here: {message}; nothing started");
            return REFUSED;
        }
    };
    let (armed, policy) = armed;
    if spec.check {
        println!("confine: ok, {}", summary(&policy));
        print!("{}", grants(&policy));
        return 0;
    }
    // A backend that confines through another program (sandbox-exec) names it
    // and its leading arguments; the command follows them.
    let prefix = armed.prefix();
    let mut command = match prefix.split_first() {
        Some((program, leading)) => {
            let mut command = Command::new(program);
            command.args(leading).arg(resolved(&spec.command[0], &Env::from_process())).args(&spec.command[1..]);
            command
        }
        None => {
            let mut command = Command::new(&spec.command[0]);
            command.args(&spec.command[1..]);
            command
        }
    };
    command.env_clear().envs(kept_env(std::env::vars_os(), &spec.env));
    // SAFETY: arming only makes syscalls; the launcher is single-threaded
    // here, so the limit covers the whole process the command becomes.
    unsafe { command.pre_exec(confine_hook(armed, first_fd)) };
    let error = command.exec();
    eprintln!("{NAME}: confine: cannot start {} under the confinement: {error}; nothing started", Path::new(&spec.command[0]).display());
    // The hook may have closed the descriptor the ruleset is held in; dropping it would abort instead of exiting.
    std::mem::forget(command);
    REFUSED
}

/// Each grant on its own line: `read`, `run` or `write`, then the path.
fn grants(policy: &Policy) -> String {
    let none = Reads::default();
    let reads = policy.reads.as_ref().unwrap_or(&none);
    let line = |kind: &str, path: &PathBuf| format!("{kind} {}\n", path.display());
    let mut text: String = reads.read.iter().map(|p| line("read", p)).collect();
    text += &reads.run.iter().map(|p| line("run", p)).collect::<String>();
    text + &policy.dirs.iter().chain(&policy.files).map(|p| line("write", p)).collect::<String>()
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
