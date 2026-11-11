//! The JavaScript string and path behaviour the state file and the page
//! depend on: its whitespace set, trim, URI encoding, UTF-16 hashing and the
//! POSIX path functions.

/// JavaScript's `\s`, and the characters `String.prototype.trim` removes.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// The line terminators a JavaScript `.`, `^` and `$` (with the `m` flag) stop at.
pub fn is_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

pub fn trim(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// The first line of `text` that is not blank, trimmed; `None` when every line is.
pub fn first_line(text: &str) -> Option<String> {
    text.split('\n').map(trim).find(|line| !line.is_empty()).map(str::to_string)
}

/// `/\w/`: ASCII letters, digits and the underscore.
pub fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `encodeURIComponent`.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The value JavaScript's `Number(digits)` gives a run of ASCII digits, as long as it fits.
pub fn parse_number(digits: &str) -> Option<u64> {
    digits.parse().ok()
}

/// FNV-1a over the text's UTF-16 code units, as 8 hex digits.
pub fn fnv_hash(text: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for unit in text.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

/// Strings ordered as JavaScript's default `sort` does: by UTF-16 code unit.
pub fn utf16_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}

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

/// A file name without a trailing `.md`, in any case.
pub fn without_md(name: &str) -> &str {
    match name.len().checked_sub(3).filter(|at| name.is_char_boundary(*at)) {
        Some(at) if name[at..].eq_ignore_ascii_case(".md") => &name[..at],
        _ => name,
    }
}
