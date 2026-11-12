//! The POSIX path functions the state directory and the atomic write depend on.

/// An absolute POSIX path with `.`, `..` and repeated separators removed, as `path.normalize` does.
pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

/// `path.join` for absolute results.
pub fn join(parts: &[&str]) -> String {
    normalize(&parts.join("/"))
}

/// `path.resolve(cwd, path)`: `path` itself when absolute, else beneath the absolute `cwd`.
pub fn resolve(cwd: &str, path: &str) -> String {
    if path.starts_with('/') {
        normalize(path)
    } else {
        normalize(&format!("{cwd}/{path}"))
    }
}

/// `path.basename`.
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed.rsplit_once('/').map_or(trimmed, |(_, name)| name)
}

/// `path.dirname` for an absolute path.
pub fn dirname(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    let head = trimmed.rfind('/').map_or("", |at| &trimmed[..at]).trim_end_matches('/');
    if head.is_empty() {
        "/"
    } else {
        head
    }
}

/// Whether JavaScript's `String.prototype.trim` counts the character as white space: the
/// `WhiteSpace` and `LineTerminator` productions. Rust's `char::is_whitespace` differs in two
/// places: it takes U+0085 (which JavaScript does not) and not U+FEFF (which JavaScript does).
pub fn is_js_whitespace(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

/// `String.prototype.trim`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

/// `String.prototype.trimStart`.
pub fn js_trim_start(text: &str) -> &str {
    text.trim_start_matches(is_js_whitespace)
}

/// `String.prototype.slice(start, end)`: indexes count UTF-16 code units, a negative one counts
/// from the end, and a missing `end` is the end. A cut through a surrogate pair leaves half a
/// pair, which a Rust string cannot hold, so that half reads as U+FFFD.
pub fn js_slice(text: &str, start: f64, end: Option<f64>) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    let length = units.len() as f64;
    let resolve = |index: f64| {
        let whole = if index.is_nan() { 0.0 } else { index.trunc() };
        if whole < 0.0 {
            (length + whole).max(0.0)
        } else {
            whole.min(length)
        }
    };
    let from = resolve(start);
    let to = end.map_or(length, resolve);
    if from >= to {
        return String::new();
    }
    String::from_utf16_lossy(&units[from as usize..to as usize])
}
