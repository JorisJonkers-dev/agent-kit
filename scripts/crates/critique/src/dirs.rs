//! What the runner lists in a directory, which the file port the gateway shares does not.

pub trait Dirs: Send + Sync {
    /// Whether the path is a directory, or a link to one.
    fn is_dir(&self, path: &str) -> bool;
    /// The names of the plain files directly inside a directory, sorted as JavaScript sorts strings; empty when it cannot be read.
    fn list_files(&self, path: &str) -> Vec<String>;
    /// The names of the directories directly inside a directory (a link to one is not), sorted as JavaScript sorts strings; empty when it cannot be read.
    fn list_directories(&self, path: &str) -> Vec<String>;
    /// Every file under the directory, at any depth, relative to it with `/`, sorted as JavaScript sorts strings: a
    /// symbolic link counts as a file and is not followed; none when the directory does not exist.
    fn list_tree(&self, dir: &str) -> Vec<String>;
    /// What a file holds, or nothing when it cannot be read.
    fn read_bytes(&self, path: &str) -> Option<Vec<u8>>;
}

/// The real file system. A symbolic link is not a plain file, and a name that is not UTF-8 is left out.
pub struct SystemDirs;

impl Dirs for SystemDirs {
    fn is_dir(&self, path: &str) -> bool {
        std::fs::metadata(path).is_ok_and(|kind| kind.is_dir())
    }

    fn list_files(&self, path: &str) -> Vec<String> {
        names_where(path, |kind| kind.is_file())
    }

    fn list_directories(&self, path: &str) -> Vec<String> {
        names_where(path, |kind| kind.is_dir())
    }

    fn list_tree(&self, dir: &str) -> Vec<String> {
        let mut found = Vec::new();
        collect_tree(std::path::Path::new(dir), "", &mut found);
        found.sort_by_key(|name| name.encode_utf16().collect::<Vec<u16>>());
        found
    }

    fn read_bytes(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }
}

/// The files under `dir`, as paths relative to the starting directory (`prefix` is the part walked so far).
fn collect_tree(dir: &std::path::Path, prefix: &str, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let relative = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            collect_tree(&entry.path(), &relative, found);
        } else {
            found.push(relative);
        }
    }
}

/// The names of the entries directly inside a directory whose type `accept`s, sorted as JavaScript sorts strings.
fn names_where(path: &str, accept: fn(&std::fs::FileType) -> bool) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| accept(&kind)))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    // `Array.prototype.sort` compares UTF-16 code units.
    names.sort_by_key(|name| name.encode_utf16().collect::<Vec<u16>>());
    names
}
