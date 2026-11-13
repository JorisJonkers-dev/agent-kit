//! What the runner lists in a directory, which the file port the gateway shares does not.

pub trait Dirs: Send + Sync {
    /// The names of the plain files directly inside a directory, sorted as JavaScript sorts strings; empty when it cannot be read.
    fn list_files(&self, path: &str) -> Vec<String>;
}

/// The real file system. A symbolic link is not a plain file, and a name that is not UTF-8 is left out.
pub struct SystemDirs;

impl Dirs for SystemDirs {
    fn list_files(&self, path: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(path) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        // `Array.prototype.sort` compares UTF-16 code units.
        names.sort_by_key(|name| name.encode_utf16().collect::<Vec<u16>>());
        names
    }
}
