use crate::ports::{Files, FsError};
use crate::text::{basename, dirname, resolve};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Read;

/// The real file system. A relative path is read against `cwd`, the working directory the command runs in.
pub struct SystemFiles {
    pub cwd: String,
}

/// The error as Node words it: `ENOENT: no such file or directory, open '<path>'`.
pub fn fail(error: &std::io::Error, syscall: &str, path: Option<&str>) -> FsError {
    let (code, text) = match error.raw_os_error() {
        Some(2) => ("ENOENT", "no such file or directory"),
        Some(13) => ("EACCES", "permission denied"),
        Some(17) => ("EEXIST", "file already exists"),
        Some(20) => ("ENOTDIR", "not a directory"),
        Some(21) => ("EISDIR", "illegal operation on a directory"),
        Some(39) => ("ENOTEMPTY", "directory not empty"),
        _ => ("UNKNOWN", "unknown error"),
    };
    let place = path.map(|path| format!(" '{path}'")).unwrap_or_default();
    FsError { code: code.to_string(), message: format!("{code}: {text}, {syscall}{place}") }
}

impl Files for SystemFiles {
    fn read_text(&self, path: &str) -> Result<String, FsError> {
        let mut file = std::fs::File::open(resolve(&self.cwd, path)).map_err(|error| fail(&error, "open", Some(path)))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|error| fail(&error, "read", None))?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn list_files(&self, dir: &str) -> Result<Vec<String>, FsError> {
        let entries = std::fs::read_dir(resolve(&self.cwd, dir)).map_err(|error| fail(&error, "scandir", Some(dir)))?;
        let names = entries.filter_map(Result::ok).filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()));
        Ok(names.map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
    }

    /// Writes a temporary file in the same directory, then renames it over the target, so a reader sees the old file or
    /// the new one and never half of either. A failed write removes its temporary file.
    fn write_atomic(&self, path: &str, content: &str) -> Result<(), FsError> {
        let target = resolve(&self.cwd, path);
        let dir = dirname(&target);
        std::fs::create_dir_all(dir).map_err(|error| fail(&error, "mkdir", Some(dir)))?;
        let random = RandomState::new().build_hasher().finish();
        let temporary = format!("{dir}/.{}.{:012x}.tmp", basename(&target), random & 0xffff_ffff_ffff);
        let result = write_then_rename(&temporary, &target, content);
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }
}

/// Writes `content` to `temporary`, then renames it over `target`.
pub fn write_then_rename(temporary: &str, target: &str, content: &str) -> Result<(), FsError> {
    std::fs::write(temporary, content).map_err(|error| fail(&error, "open", Some(temporary)))?;
    std::fs::rename(temporary, target).map_err(|error| fail(&error, "rename", Some(&format!("{temporary}' -> '{target}"))))
}
