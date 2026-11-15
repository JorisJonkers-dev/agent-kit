//! Python behaviours both helpers depend on: `str.splitlines`, the text of an
//! `OSError`, and `repr(float)`.

/// Python's `str.splitlines()`: breaks at `\n`, `\r`, `\r\n` and the other
/// Unicode line boundaries, without keeping them.
pub fn splitlines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            lines.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// An I/O error the way Python prints it: `[Errno 2] No such file or
/// directory: 'path'`.
pub fn os_error(error: &std::io::Error, path: &str) -> String {
    let full = error.to_string();
    let message = full.split(" (os error").next().unwrap_or(&full);
    match error.raw_os_error() {
        Some(code) => format!("[Errno {code}] {message}: '{path}'"),
        None => format!("{message}: '{path}'"),
    }
}

/// `repr(x)` of a float: the shortest text that reads back as the same value,
/// always with a decimal point.
pub fn float_repr(x: f64) -> String {
    let text = format!("{x}");
    if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    }
}

/// Python's `round(x, digits)`: rounds the exact binary value, ties to even.
pub fn round(x: f64, digits: usize) -> f64 {
    format!("{x:.digits$}").parse().unwrap_or(x)
}
