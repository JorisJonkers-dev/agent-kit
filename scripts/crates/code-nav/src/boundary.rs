//! The read boundary: which paths the server may open.
//!
//! The allowlist is a set of canonical roots. A request is admitted only when
//! its canonical path is under one of them, no symlink on the way leads out,
//! and the file is not a secret. Everything else is a [`Refusal`].

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Why a path was not admitted.
#[derive(Debug, PartialEq)]
pub enum Refusal {
    /// The path is not under any allowlisted root.
    OutsideAllowlist(PathBuf),
    /// The path is under a root but a symlink on it leads out of every root.
    SymlinkOut(PathBuf),
    /// The path names a secret file or sits in a secret directory.
    SecretFile(PathBuf),
    /// The path is under a root but does not exist.
    Missing(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::OutsideAllowlist(p) => write!(f, "refused: {} is outside the allowed roots", p.display()),
            Refusal::SymlinkOut(p) => write!(f, "refused: {} is a symlink that leaves the allowed roots", p.display()),
            Refusal::SecretFile(p) => write!(f, "refused: {} is a secret file", p.display()),
            Refusal::Missing(p) => write!(f, "{} does not exist", p.display()),
        }
    }
}

/// The allowlist of canonical roots.
#[derive(Debug)]
pub struct Boundary {
    roots: Vec<PathBuf>,
}

impl Boundary {
    /// Canonicalise `roots`; each must be an existing directory.
    pub fn new(roots: &[PathBuf]) -> Result<Boundary, String> {
        let mut canonical = Vec::new();
        for root in roots {
            let c = root.canonicalize().map_err(|e| format!("root {}: {e}", root.display()))?;
            if !c.is_dir() {
                return Err(format!("root {} is not a directory", root.display()));
            }
            canonical.push(c);
        }
        if canonical.is_empty() {
            return Err("no roots".to_string());
        }
        Ok(Boundary { roots: canonical })
    }

    /// The canonical roots.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Admit `requested` (absolute, or relative to the first root) and return
    /// its canonical path.
    pub fn admit(&self, requested: &Path) -> Result<PathBuf, Refusal> {
        let lexical = normalise(&self.roots[0].join(requested));
        let lexically_inside = self.root_of(&lexical).is_some();
        let canonical = match lexical.canonicalize() {
            Ok(c) => c,
            Err(_) if lexically_inside => return Err(Refusal::Missing(lexical)),
            Err(_) => return Err(Refusal::OutsideAllowlist(lexical)),
        };
        let Some(root) = self.root_of(&canonical) else {
            return Err(if lexically_inside {
                Refusal::SymlinkOut(lexical)
            } else {
                Refusal::OutsideAllowlist(lexical)
            });
        };
        let relative = canonical.strip_prefix(root).unwrap_or(&canonical);
        let named = lexical.file_name().map(Path::new).unwrap_or(Path::new(""));
        if is_secret(relative) || is_secret(named) {
            return Err(Refusal::SecretFile(lexical));
        }
        Ok(canonical)
    }

    /// The root `canonical` is under, if any.
    fn root_of(&self, path: &Path) -> Option<&Path> {
        self.roots.iter().map(PathBuf::as_path).find(|r| path.starts_with(r))
    }
}

/// `path` with `.` and `..` resolved without touching the file system.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        // `components` already drops interior `.`; the path is absolute, so
        // there is no leading one.
        match c {
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Directories whose whole content is secret.
const SECRET_DIRS: [&str; 3] = [".ssh", ".gnupg", ".aws"];
/// File names that are secret as they stand.
const SECRET_NAMES: [&str; 7] =
    [".netrc", ".npmrc", ".pypirc", ".git-credentials", "credentials", "credentials.json", "secrets.json"];
/// Extensions of key and certificate-store files.
const SECRET_EXTENSIONS: [&str; 6] = ["pem", "key", "p12", "pfx", "kdbx", "keystore"];
/// `.env` variants that hold no value and may be read.
const ENV_TEMPLATES: [&str; 3] = [".env.example", ".env.sample", ".env.template"];

/// True when any component of `path` is a secret directory, or its file name
/// is a secret name: `.env` and `.env.*` (except templates), SSH private
/// keys, key and certificate stores, and credential files.
pub fn is_secret(path: &Path) -> bool {
    let names: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    let Some((file, dirs)) = names.split_last() else {
        return false;
    };
    if dirs.iter().any(|d| SECRET_DIRS.contains(&d.as_str())) || SECRET_DIRS.contains(&file.as_str()) {
        return true;
    }
    if ENV_TEMPLATES.contains(&file.as_str()) {
        return false;
    }
    let extension = file.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    file == ".env"
        || file.starts_with(".env.")
        || file.starts_with("id_rsa")
        || file.starts_with("id_ed25519")
        || file.starts_with("id_ecdsa")
        || SECRET_NAMES.contains(&file.as_str())
        || SECRET_EXTENSIONS.contains(&extension)
}
