//! POSIX path helpers with the semantics of Python's `os.path`, which the
//! guard's decisions were written against.

/// `os.path.join` for two parts.
pub fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_string()
    } else if a.is_empty() || a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

/// `os.path.normpath`.
pub fn normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let mut initial = usize::from(path.starts_with('/'));
    if path.starts_with("//") && !path.starts_with("///") {
        initial = 2;
    }
    let mut comps: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(comp);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let body = format!("{}{}", "/".repeat(initial), comps.join("/"));
    if body.is_empty() {
        ".".to_string()
    } else {
        body
    }
}

/// `os.path.expanduser` for `~` and `~/...`; `~user` is left alone.
fn expanduser(path: &str, home: &str) -> String {
    let rest = if path == "~" {
        Some("")
    } else {
        path.strip_prefix("~/").map(|_| &path[1..])
    };
    match rest {
        Some(rest) => {
            let joined = format!("{}{}", home.trim_end_matches('/'), rest);
            if joined.is_empty() {
                "/".to_string()
            } else {
                joined
            }
        }
        None => path.to_string(),
    }
}

/// The guard's `norm`: expand `~`, anchor a relative path at `cwd`, normalise.
pub fn norm(path: &str, cwd: &str, home: &str) -> String {
    let path = expanduser(path, home);
    normpath(&join(cwd, &path))
}

/// `os.path.dirname`.
pub fn dirname(path: &str) -> String {
    let head = &path[..path.rfind('/').map_or(0, |i| i + 1)];
    if head.chars().all(|c| c == '/') {
        head.to_string()
    } else {
        head.trim_end_matches('/').to_string()
    }
}

/// `os.path.basename`.
pub fn basename(path: &str) -> &str {
    &path[path.rfind('/').map_or(0, |i| i + 1)..]
}

/// `os.path.relpath(path, start)` for absolute inputs.
pub fn relpath(path: &str, start: &str) -> String {
    let p: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let s: Vec<&str> = start.split('/').filter(|c| !c.is_empty()).collect();
    let common = p.iter().zip(&s).take_while(|(a, b)| a == b).count();
    let mut rel: Vec<&str> = vec![".."; s.len() - common];
    rel.extend(&p[common..]);
    if rel.is_empty() {
        ".".to_string()
    } else {
        rel.join("/")
    }
}
