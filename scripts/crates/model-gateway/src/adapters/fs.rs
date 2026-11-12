use crate::ports::{FileStamp, Files, FsError};
use crate::text::{join, resolve};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};

/// The real file system. A relative path is read against `cwd`, the working directory the command runs in.
pub struct SystemFiles {
    pub cwd: String,
}

/// The errno numbers that differ between Linux and macOS.
#[cfg(target_os = "macos")]
const NUMBERS: (i32, i32, i32) = (63, 66, 62);
#[cfg(not(target_os = "macos"))]
const NUMBERS: (i32, i32, i32) = (36, 39, 40);
pub const ENAMETOOLONG: i32 = NUMBERS.0;
pub const ENOTEMPTY: i32 = NUMBERS.1;
pub const ELOOP: i32 = NUMBERS.2;

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
    (ENAMETOOLONG, "ENAMETOOLONG", "name too long"),
    (ENOTEMPTY, "ENOTEMPTY", "directory not empty"),
    (ELOOP, "ELOOP", "too many symbolic links encountered"),
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

/// The six letters or digits `mkdtemp` ends a name with.
fn random_letters() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut random = RandomState::new().build_hasher().finish() ^ COUNTER.fetch_add(1, Ordering::Relaxed);
    (0..6)
        .map(|_| {
            let letter = ALPHABET[(random % ALPHABET.len() as u64) as usize];
            random /= ALPHABET.len() as u64;
            char::from(letter)
        })
        .collect()
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

    fn exists(&self, path: &str) -> Result<bool, FsError> {
        match std::fs::metadata(resolve(&self.cwd, path)) {
            Ok(_) => Ok(true),
            Err(error) if matches!(error.raw_os_error(), Some(2) | Some(20)) => Ok(false),
            Err(error) => Err(fail(&error, "access", Some(path))),
        }
    }

    fn append(&self, path: &str, content: &str) -> Result<(), FsError> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(resolve(&self.cwd, path))
            .map_err(|error| fail(&error, "open", Some(path)))?;
        file.write_all(content.as_bytes()).map_err(|error| fail(&error, "write", None))
    }

    fn stamp(&self, path: &str) -> Option<FileStamp> {
        use std::os::unix::fs::MetadataExt;
        let real = std::fs::canonicalize(resolve(&self.cwd, path)).ok()?;
        let meta = std::fs::metadata(&real).ok()?;
        let bytes = std::fs::read(&real).ok()?;
        Some(FileStamp {
            realpath: real.to_string_lossy().into_owned(),
            ctime_ms: meta.ctime() as f64 * 1e3 + meta.ctime_nsec() as f64 / 1e6,
            ino: meta.ino() as f64,
            sha256: crate::sha256::sha256_hex(&bytes),
            size: meta.size() as f64,
        })
    }

    fn write_private(&self, path: &str, content: &str) -> Result<(), FsError> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let target = resolve(&self.cwd, path);
        let directory = crate::text::dirname(&target).to_string();
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&directory).map_err(|error| fail(&error, "mkdir", Some(&directory)))?;
        let temporary = temporary_name(&target);
        let result = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| fail(&error, "open", Some(&temporary)))
            .and_then(|_| write_then_rename(&temporary, &target, content));
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    /// `mkdir -p` of the file's directory, then the file itself, in place: what `writeFile` does.
    fn write_text(&self, path: &str, content: &str) -> Result<(), FsError> {
        let target = resolve(&self.cwd, path);
        let directory = crate::text::dirname(&target).to_string();
        std::fs::create_dir_all(&directory).map_err(|error| fail(&error, "mkdir", Some(&directory)))?;
        std::fs::write(&target, content).map_err(|error| fail(&error, "open", Some(path)))
    }

    /// `mkdtemp`: the directory is made at once, so two callers never get the same one (a name taken is an error, which
    /// six random characters make as unlikely as `mkdtemp`'s own retry makes it).
    fn temp_dir(&self, parent: &str, prefix: &str) -> Result<String, FsError> {
        let base = join(&[&resolve(&self.cwd, parent), prefix]);
        let candidate = format!("{base}{}", random_letters());
        std::fs::DirBuilder::new().create(&candidate).map(|()| candidate).map_err(|error| fail(&error, "mkdtemp", Some(&format!("{base}XXXXXX"))))
    }

    fn remove(&self, path: &str) -> Result<(), FsError> {
        let target = resolve(&self.cwd, path);
        let removed = match std::fs::symlink_metadata(&target) {
            Ok(kind) if kind.is_dir() => std::fs::remove_dir_all(&target),
            Ok(_) => std::fs::remove_file(&target),
            Err(error) if error.raw_os_error() == Some(2) => Ok(()),
            Err(error) => Err(error),
        };
        removed.map_err(|error| fail(&error, "rm", Some(path)))
    }
}

/// Writes `content` to `temporary`, then renames it over `target`.
pub fn write_then_rename(temporary: &str, target: &str, content: &str) -> Result<(), FsError> {
    std::fs::write(temporary, content).map_err(|error| fail(&error, "open", Some(temporary)))?;
    std::fs::rename(temporary, target).map_err(|error| fail(&error, "rename", Some(&format!("{temporary}' -> '{target}"))))
}
