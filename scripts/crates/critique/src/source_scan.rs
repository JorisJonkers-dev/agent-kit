//! The symbol and import scan behind a code packet's repo map and context slices: by rule, from the
//! text alone, with no compiler and no model, so the same commit always gives the same scan. Each
//! language the scan knows is one scanner. Ported from the node runner's `source-scan.ts`.

use crate::js_regex::Regex;
use model_gateway::text::{is_js_whitespace, js_slice, js_trim};
use std::collections::HashSet;
use std::sync::OnceLock;

/// A top-level declaration of a source file, with the lines it spans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSymbol {
    pub name: String,
    /// The declaration keyword: `function`, `class`, `const`, `def`, ...
    pub kind: String,
    /// Its header, without its body, on one line.
    pub signature: String,
    /// The line its declaration starts on, from 1.
    pub first: usize,
    /// The last line of its body.
    pub last: usize,
}

/// What the scan reads from one source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceScan {
    pub language: &'static str,
    /// Its top-level symbols, in line order.
    pub symbols: Vec<SourceSymbol>,
    /// The modules it imports, as written.
    pub imports: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scanner {
    Typescript,
    Python,
}

/// How many lines a declaration's header may run over.
const HEADER_LINES: usize = 6;
/// The longest signature the repo map shows.
const SIGNATURE_CHARS: usize = 200;

const TS_EXTENSIONS: [&str; 8] = [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"];

fn pattern(cell: &'static OnceLock<Regex>, source: &str, multiline: bool) -> &'static Regex {
    cell.get_or_init(|| Regex::new(source, multiline).expect("the source scan's patterns are valid"))
}

fn ts_declaration() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(&CELL, r"^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(function\*?|class|interface|type|enum|const\s+enum|const|let|var|namespace)\s+([A-Za-z_$][\w$]*)", false)
}

fn ts_import() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(&CELL, r#"(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*)['"]([^'"\n]+)['"]"#, false)
}

fn py_declaration() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(&CELL, r"^(?:(async\s+def|def|class)\s+([A-Za-z_]\w*)|([A-Za-z_]\w*)\s*(?::[^=]*)?=(?!=))", false)
}

fn py_from() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(&CELL, r"^\s*from\s+(\.*[\w.]*)\s+import\s+(?:\(([^)]*)\)|([^#\n]*))", true)
}

fn py_import() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    pattern(&CELL, r"^\s*import\s+([\w.]+(?:\s*,\s*[\w.]+)*)", true)
}

/// `/\s+/g` replaced by one space.
fn collapse_white_space(text: &str) -> String {
    let mut flat = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if is_js_whitespace(c) {
            if !in_space {
                flat.push(' ');
            }
            in_space = true;
        } else {
            flat.push(c);
            in_space = false;
        }
    }
    flat
}

/// `path.posix.normalize` for the paths the scan joins.
pub fn normalize(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            ".." if !absolute => parts.push(".."),
            ".." => {}
            other => parts.push(other),
        }
    }
    let mut body = parts.join("/");
    if body.is_empty() {
        return if absolute {
            "/".to_string()
        } else if path.ends_with('/') {
            "./".to_string()
        } else {
            ".".to_string()
        };
    }
    if path.ends_with('/') {
        body.push('/');
    }
    if absolute {
        format!("/{body}")
    } else {
        body
    }
}

/// `path.posix.join(directory, relative)` for two paths that are not empty.
pub fn posix_join(directory: &str, relative: &str) -> String {
    normalize(&format!("{directory}/{relative}"))
}

/// `path.posix.dirname`.
pub fn posix_dirname(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.is_empty() {
        return ".".to_string();
    }
    let rooted = bytes[0] == b'/';
    let mut end = None;
    let mut matched_slash = true;
    for i in (1..bytes.len()).rev() {
        if bytes[i] == b'/' {
            if !matched_slash {
                end = Some(i);
                break;
            }
        } else {
            matched_slash = false;
        }
    }
    match end {
        None if rooted => "/".to_string(),
        None => ".".to_string(),
        Some(1) if rooted => "//".to_string(),
        Some(end) => path[..end].to_string(),
    }
}

/// `path.posix.extname`.
pub fn posix_extname(path: &str) -> &str {
    let bytes = path.as_bytes();
    let (mut start_dot, mut start_part, mut end, mut matched_slash, mut pre_dot) = (None, 0, None, true, 0);
    for i in (0..bytes.len()).rev() {
        if bytes[i] == b'/' {
            if !matched_slash {
                start_part = i + 1;
                break;
            }
            continue;
        }
        if end.is_none() {
            matched_slash = false;
            end = Some(i + 1);
        }
        if bytes[i] == b'.' {
            if start_dot.is_none() {
                start_dot = Some(i);
            } else if pre_dot != 1 {
                pre_dot = 1;
            }
        } else if start_dot.is_some() {
            pre_dot = -1;
        }
    }
    match (start_dot, end) {
        (Some(dot), Some(end)) if pre_dot != 0 && !(pre_dot == 1 && dot + 1 == end && dot == start_part + 1) => &path[dot..end],
        _ => "",
    }
}

fn scanner_of(path: &str) -> Option<Scanner> {
    match posix_extname(path) {
        extension if TS_EXTENSIONS.contains(&extension) => Some(Scanner::Typescript),
        ".py" => Some(Scanner::Python),
        _ => None,
    }
}

/// The name of the language a path is scanned as, by its extension.
pub fn language_of(path: &str) -> Option<&'static str> {
    scanner_of(path).map(Scanner::language)
}

impl Scanner {
    fn language(self) -> &'static str {
        match self {
            Scanner::Typescript => "typescript",
            Scanner::Python => "python",
        }
    }

    /// The top-level declaration a line at column 0 starts, if it starts one.
    fn declaration(self, line: &str) -> Option<(String, String)> {
        let chars: Vec<char> = line.chars().collect();
        match self {
            Scanner::Typescript => {
                let hit = ts_declaration().find_at(&chars, 0)?;
                Some((collapse_white_space(&hit.group(&chars, 1).unwrap_or_default()), hit.group(&chars, 2).unwrap_or_default()))
            }
            Scanner::Python => {
                let hit = py_declaration().find_at(&chars, 0)?;
                match hit.group(&chars, 3) {
                    None => Some((collapse_white_space(&hit.group(&chars, 1).unwrap_or_default()), hit.group(&chars, 2).unwrap_or_default())),
                    Some(name) => Some(("assignment".to_string(), name)),
                }
            }
        }
    }

    /// The declaration's header, from the lines it starts on, without its body.
    fn signature(self, header: &str, kind: &str) -> String {
        match self {
            Scanner::Typescript if kind == "type" => header.split('\n').next().unwrap_or_default().to_string(),
            Scanner::Typescript if matches!(kind, "const" | "let" | "var") => value_signature(header),
            Scanner::Typescript => cut_at(header, '{').to_string(),
            Scanner::Python if kind == "assignment" => cut_at(header, '=').to_string(),
            Scanner::Python => cut_at(header, ':').to_string(),
        }
    }

    fn imports(self, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        match self {
            Scanner::Typescript => ts_import().find_all(&chars).iter().map(|hit| hit.group(&chars, 1).unwrap_or_default()).collect(),
            Scanner::Python => {
                let mut found = Vec::new();
                for hit in py_from().find_all(&chars) {
                    let module = hit.group(&chars, 1).unwrap_or_default();
                    let listed = hit.group(&chars, 2).or_else(|| hit.group(&chars, 3)).unwrap_or_default();
                    found.push(module.clone());
                    for name in listed.split(',').map(|name| js_trim(name).split(is_js_whitespace).next().unwrap_or_default()) {
                        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                            found.push(format!("{module}{}{name}", if module.ends_with('.') { "" } else { "." }));
                        }
                    }
                }
                let mut plain = Vec::new();
                for hit in py_import().find_all(&chars) {
                    plain.extend(hit.group(&chars, 1).unwrap_or_default().split(',').map(|name| js_trim(name).to_string()));
                }
                found.extend(plain);
                found
            }
        }
    }

    /// The tracked path an import names, if it names one.
    fn resolve(self, from: &str, specifier: &str, tracked: &HashSet<String>) -> Option<String> {
        match self {
            Scanner::Typescript => {
                if !specifier.starts_with('.') {
                    return None;
                }
                let target = posix_join(&posix_dirname(from), specifier);
                let stem = [".js", ".cjs", ".mjs", ".jsx"].iter().find_map(|suffix| target.strip_suffix(suffix)).unwrap_or(&target).to_string();
                let mut candidates = vec![target.clone()];
                candidates.extend(TS_EXTENSIONS.iter().map(|extension| format!("{stem}{extension}")));
                candidates.extend(TS_EXTENSIONS.iter().map(|extension| format!("{target}/index{extension}")));
                candidates.into_iter().find(|candidate| tracked.contains(candidate))
            }
            Scanner::Python => {
                let dots = specifier.chars().take_while(|c| *c == '.').count();
                let module = specifier[dots..].split('.').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("/");
                let files = |base: &str| if base.is_empty() { Vec::new() } else { vec![format!("{base}.py"), format!("{base}/__init__.py")] };
                if dots > 0 {
                    let mut directory = posix_dirname(from);
                    for _ in 1..dots {
                        directory = posix_dirname(&directory);
                    }
                    let base = if module.is_empty() { directory } else { posix_join(&directory, &module) };
                    return files(if base == "." { "" } else { &base }).into_iter().find(|candidate| tracked.contains(candidate));
                }
                let candidates = files(&module);
                if let Some(exact) = candidates.iter().find(|candidate| tracked.contains(*candidate)) {
                    return Some(exact.clone());
                }
                let mut nested = tracked.iter().filter(|path| candidates.iter().any(|candidate| path.ends_with(&format!("/{candidate}"))));
                match (nested.next(), nested.next()) {
                    (Some(only), None) => Some(only.clone()),
                    _ => None,
                }
            }
        }
    }
}

/// The tracked file an import of a file names, by the file's language.
pub fn resolve_import(from: &str, specifier: &str, tracked: &HashSet<String>) -> Option<String> {
    scanner_of(from)?.resolve(from, specifier, tracked)
}

/// Whether a line starts a top-level statement: it starts at column 0 and does not close a bracket.
fn starts_statement(line: &str) -> bool {
    line.chars().next().is_some_and(|c| !is_js_whitespace(c) && !matches!(c, ')' | ']' | '}'))
}

/// The top-level symbols and imports of a file in a language the scan knows. A top-level declaration
/// starts at column 0 and runs to the last line before the next line at column 0 that does not close
/// a bracket, without the blank lines at its end.
pub fn scan_source(path: &str, text: &str) -> Option<SourceScan> {
    let scanner = scanner_of(path)?;
    let lines: Vec<&str> = text.split('\n').collect();
    let starts: Vec<usize> = (0..lines.len()).filter(|index| starts_statement(lines[*index])).collect();
    let mut symbols = Vec::new();
    for (i, start) in starts.iter().copied().enumerate() {
        let Some((kind, name)) = scanner.declaration(lines[start]) else {
            continue;
        };
        let mut end = starts.get(i + 1).copied().unwrap_or(lines.len()) - 1;
        while end > start && js_trim(lines[end]).is_empty() {
            end -= 1;
        }
        let header = lines[start..=end.min(start + HEADER_LINES - 1)].join("\n");
        symbols.push(SourceSymbol { first: start + 1, signature: shorten(&scanner.signature(&header, &kind)), kind, last: end + 1, name });
    }
    Some(SourceScan { imports: scanner.imports(text), language: scanner.language(), symbols })
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Whether a text mentions a name as a whole word.
pub fn mentions(text: &str, name: &str) -> bool {
    whole_word(&text.chars().collect::<Vec<_>>(), name)
}

fn whole_word(text: &[char], name: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    (0..=text.len().saturating_sub(name.len())).any(|at| {
        text.len() >= name.len()
            && text[at..at + name.len()] == name[..]
            && !(at > 0 && is_name_char(text[at - 1]))
            && !text.get(at + name.len()).is_some_and(|c| is_name_char(*c))
    })
}

/// The lines, from 1, of a text that mention a name as a whole word.
pub fn mention_lines(text: &str, name: &str) -> Vec<usize> {
    text.split('\n').enumerate().filter(|(_, line)| whole_word(&line.chars().collect::<Vec<_>>(), name)).map(|(index, _)| index + 1).collect()
}

/// The header up to the first `stop` outside brackets.
fn cut_at(header: &str, stop: char) -> &str {
    let chars: Vec<(usize, char)> = header.char_indices().collect();
    let mut depth = 0usize;
    for (position, (offset, c)) in chars.iter().copied().enumerate() {
        let after_equals = position > 0 && chars[position - 1].1 == '=';
        let next = chars.get(position + 1).map(|(_, next)| *next);
        if matches!(c, '(' | '[' | '<') {
            depth += 1;
        } else if matches!(c, ')' | ']' | '>') && !after_equals {
            depth = depth.saturating_sub(1);
        } else if c == stop && depth == 0 && !(stop == '=' && next.is_none_or(|next| next == '=' || next == '>')) {
            return &header[..offset];
        }
    }
    header
}

/// A variable's name and type, and an arrow function's parameters and return type.
fn value_signature(header: &str) -> String {
    let name = cut_at(header, '=');
    let rest = &header[name.len()..];
    let value = model_gateway::text::js_trim_start(&rest[rest.chars().next().map_or(0, char::len_utf8)..]);
    static ARROW: OnceLock<Regex> = OnceLock::new();
    let starts_a_function = pattern(&ARROW, r"^(?:async\s+)?(?:\(|<|[A-Za-z_$][\w$]*\s*=>)", false);
    let chars: Vec<char> = value.chars().collect();
    if starts_a_function.find_at(&chars, 0).is_none() {
        return name.to_string();
    }
    match value.find("=>") {
        None => name.to_string(),
        Some(arrow) => format!("{} = {}", name.trim_end_matches(is_js_whitespace), &value[..arrow + 2]),
    }
}

fn shorten(signature: &str) -> String {
    let flat = collapse_white_space(signature);
    let flat = js_trim(&flat);
    if flat.encode_utf16().count() <= SIGNATURE_CHARS {
        flat.to_string()
    } else {
        format!("{}…", js_slice(flat, 0.0, Some((SIGNATURE_CHARS - 1) as f64)))
    }
}
