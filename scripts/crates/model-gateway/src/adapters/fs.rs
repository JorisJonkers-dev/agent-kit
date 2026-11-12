use crate::ports::{Files, FsError};
use crate::text::{resolve};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};

/// The real file system. A relative path is read against `cwd`, the working directory the command runs in.
pub struct SystemFiles {
    pub cwd: String,
}

/// The errno numbers the gateway can meet, with the code and the words Node gives them.
const ERRORS: [(i32, &str, &str); 11] = [
    (1, "EPERM", "operation not permitted"),
    (2, "ENOENT", "no such file or directory"),
    (13, "EACCES", "permission denied"),
    (17, "EEXIST", "file already exists"),
    (20, "ENOTDIR", "not a directory"),
    (21, "EISDIR", "illegal operation on a directory"),
    (28, "ENOSPC", "no space left on device"),
    (30, "EROFS", "read-only file system"),
    (36, "ENAMETOOLONG", "name too long"),
    (39, "ENOTEMPTY", "directory not empty"),
    (40, "ELOOP", "too many symbolic links encountered"),
];

/// The error as Node words it: `ENOENT: no such file or directory, open '<path>'`.
pub fn fail(error: &std::io::Error, syscall: &str, path: Option<&str>) -> FsError {
    let (code, text) = error
        .raw_os_error()
        .and_then(|number| ERRORS.iter().find(|(value, _, _)| *value == number))
        .map_or(("UNKNOWN", "unknown error"), |(_, code, text)| (*code, *text));
    let place = path.map(|path| format!(" '{path}'")).unwrap_or_default();
    FsError { code: code.to_string(), message: format!("{code}: {text}, {syscall}{place}") }
}

/// A temporary file name beside `path`, as `writeFileAtomic` makes one: twelve hex digits and `.tmp`.
fn temporary_name(path: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let random = RandomState::new().build_hasher().finish() ^ COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{path}.{:012x}.tmp", random & 0xffff_ffff_ffff)
}

impl Files for SystemFiles {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        let mut file = std::fs::File::open(resolve(&self.cwd, path)).map_err(|error| fail(&error, "open", Some(path)))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|error| fail(&error, "read", None))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Writes a temporary file beside the target, then renames it over it, so a reader sees the old file or the new one
    /// and never half of either. A failed write removes its temporary file.
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        let target = resolve(&self.cwd, path);
        let temporary = temporary_name(&target);
        let result = write_then_rename(&temporary, &target, content);
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    fn mkdir_all(&self, path: &str) -> Result<(), FsError> {
        std::fs::create_dir_all(resolve(&self.cwd, path)).map_err(|error| fail(&error, "mkdir", Some(path)))
    }

    fn create_exclusive(&self, path: &str) -> Result<bool, FsError> {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(resolve(&self.cwd, path)) {
            Ok(_) => Ok(true),
            Err(error) if error.raw_os_error() == Some(17) => Ok(false),
            Err(error) => Err(fail(&error, "open", Some(path))),
        }
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        match std::fs::remove_file(resolve(&self.cwd, path)) {
            Ok(()) => Ok(()),
            Err(error) if error.raw_os_error() == Some(2) => Ok(()),
            Err(error) => Err(fail(&error, "unlink", Some(path))),
        }
    }

    fn modified_ms(&self, path: &str) -> Option<i64> {
        let modified = std::fs::metadata(resolve(&self.cwd, path)).ok()?.modified().ok()?;
        Some(modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
    }
}

/// Writes `content` to `temporary`, then renames it over `target`.
pub fn write_then_rename(temporary: &str, target: &str, content: &str) -> Result<(), FsError> {
    std::fs::write(temporary, content).map_err(|error| fail(&error, "open", Some(temporary)))?;
    std::fs::rename(temporary, target).map_err(|error| fail(&error, "rename", Some(&format!("{temporary}' -> '{target}"))))
}
