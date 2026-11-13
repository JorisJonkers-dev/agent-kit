//! The built-in import scan: line oriented and regex based, from a file's text alone, so the same
//! commit always gives the same imports. Ported from the node runner's `import-scan.ts`.

use crate::js_regex::Regex;
use model_gateway::text::{is_js_whitespace, js_trim, js_trim_start};
use std::collections::HashSet;
use std::sync::OnceLock;

/// The languages the built-in import scan knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanLanguage {
    Typescript,
    Python,
    Kotlin,
    Go,
    Rust,
}

impl ScanLanguage {
    pub fn name(self) -> &'static str {
        match self {
            ScanLanguage::Typescript => "typescript",
            ScanLanguage::Python => "python",
            ScanLanguage::Kotlin => "kotlin",
            ScanLanguage::Go => "go",
            ScanLanguage::Rust => "rust",
        }
    }
}

/// What kind of import one is: a Rust `mod x;` declaration is a `Module`, which names a file beside or below the declaring one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportKind {
    Import,
    Module,
}

/// One import found in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedImport {
    pub specifier: String,
    pub kind: ImportKind,
}

/// The language a path is scanned as (TypeScript and JavaScript share one scan), or `None` for one the scan does not know.
pub fn language_of(path: &str) -> Option<ScanLanguage> {
    let dot = path.rfind('.')?;
    if path.rfind('/').is_some_and(|slash| slash > dot) {
        return None;
    }
    match &path[dot..] {
        ".ts" | ".tsx" | ".mts" | ".cts" | ".js" | ".jsx" | ".mjs" | ".cjs" => Some(ScanLanguage::Typescript),
        ".py" => Some(ScanLanguage::Python),
        ".kt" | ".kts" => Some(ScanLanguage::Kotlin),
        ".go" => Some(ScanLanguage::Go),
        ".rs" => Some(ScanLanguage::Rust),
        _ => None,
    }
}

/// Strips block comments and whole-line `//` and `#` comments, keeping line breaks so a scan stays line oriented.
fn without_comments(text: &str, language: ScanLanguage) -> String {
    let blockless = if language == ScanLanguage::Python { text.to_string() } else { without_blocks(text) };
    let marker = if language == ScanLanguage::Python { "#" } else { "//" };
    let lines: Vec<&str> = blockless.split('\n').map(|line| if js_trim_start(line).starts_with(marker) { "" } else { line }).collect();
    lines.join("\n")
}

/// `text.replace(/\/\*[\s\S]*?\*\//g, block => block.replace(/[^\n]/g, ''))`: each block comment keeps only its line feeds.
fn without_blocks(text: &str) -> String {
    let mut kept = String::new();
    let mut rest = text;
    while let Some(open) = rest.find("/*") {
        let Some(length) = rest[open + 2..].find("*/") else {
            break;
        };
        let block = &rest[open..open + 2 + length + 2];
        kept.push_str(&rest[..open]);
        kept.push_str(&"\n".repeat(block.matches('\n').count()));
        rest = &rest[open + block.len()..];
    }
    kept.push_str(rest);
    kept
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern, true).expect("the import scan's patterns are valid"))
}

/// The patterns of a language, in the order the node scan applies them.
fn patterns(language: ScanLanguage) -> Vec<&'static Regex> {
    static TS: [OnceLock<Regex>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    static PY: [OnceLock<Regex>; 2] = [OnceLock::new(), OnceLock::new()];
    static KT: OnceLock<Regex> = OnceLock::new();
    static GO: [OnceLock<Regex>; 2] = [OnceLock::new(), OnceLock::new()];
    static RS: [OnceLock<Regex>; 2] = [OnceLock::new(), OnceLock::new()];
    match language {
        ScanLanguage::Typescript => vec![
            regex(&TS[0], r#"^\s*(?:import|export)\b[\w\s,{}*$]*?\bfrom\s*['"]([^'"\n]+)['"]"#),
            regex(&TS[1], r#"^\s*import\s*['"]([^'"\n]+)['"]"#),
            regex(&TS[2], r#"\b(?:require|import)\s*\(\s*['"]([^'"\n]+)['"]\s*\)"#),
        ],
        ScanLanguage::Python => vec![
            regex(&PY[0], r"^\s*from\s+(\.*[\w.]*)\s+import\b"),
            regex(&PY[1], r"^\s*import\s+([\w.]+(?:\s*,\s*[\w.]+)*)"),
        ],
        ScanLanguage::Kotlin => vec![regex(&KT, r"^\s*import\s+([\w.]+)")],
        ScanLanguage::Go => vec![regex(&GO[0], r#"^\s*import\s+(?:[\w.]+\s+)?"([^"\n]+)""#), regex(&GO[1], r#"^\s+(?:[\w.]+\s+)?"([^"\n]+)"\s*$"#)],
        ScanLanguage::Rust => vec![regex(&RS[0], r"^\s*(?:pub\s+)?use\s+([\w:]+)"), regex(&RS[1], r"^\s*(?:pub\s+)?mod\s+(\w+)\s*;")],
    }
}

/// The imports of one file's text, in order of first appearance and without repeats. Comments are
/// dropped, and a specifier inside a string or template literal on a line of its own is not told
/// from a real one. A Go specifier is read inside an `import (...)` block only.
pub fn scan_imports(language: ScanLanguage, text: &str) -> Vec<ScannedImport> {
    let code = without_comments(text, language);
    let lines: Vec<char> = (if language == ScanLanguage::Go { go_import_lines(&code) } else { code }).chars().collect();
    let mut found: Vec<(usize, ScannedImport)> = Vec::new();
    for (position, pattern) in patterns(language).into_iter().enumerate() {
        let kind = if language == ScanLanguage::Rust && position == 1 { ImportKind::Module } else { ImportKind::Import };
        for hit in pattern.find_all(&lines) {
            let raw = hit.group(&lines, 1).unwrap_or_default();
            let raw = if matches!(language, ScanLanguage::Kotlin | ScanLanguage::Rust) { raw.trim_end_matches(['.', ':']).to_string() } else { raw };
            let names: Vec<String> = if language == ScanLanguage::Python && !raw.starts_with('.') { raw.split(',').map(|name| js_trim(name).to_string()).collect() } else { vec![raw] };
            found.extend(names.into_iter().map(|specifier| (hit.start, ScannedImport { specifier, kind })));
        }
    }
    found.retain(|(_, entry)| !entry.specifier.is_empty());
    found.sort_by_key(|(index, _)| *index);
    let mut seen = HashSet::new();
    found.into_iter().filter(|(_, entry)| seen.insert((entry.kind == ImportKind::Module, entry.specifier.clone()))).map(|(_, entry)| entry).collect()
}

/// Only the lines of the file's `import` declarations, so a quoted string elsewhere is not taken for an import.
fn go_import_lines(code: &str) -> String {
    let mut in_block = false;
    let kept: Vec<&str> = code
        .split('\n')
        .map(|line| {
            let start = js_trim_start(line);
            if in_block {
                in_block = !start.starts_with(')');
                return if in_block { line } else { "" };
            }
            let keyword = start.strip_prefix("import");
            if keyword.is_some_and(|after| js_trim_start(after).starts_with('(')) {
                in_block = true;
                return "";
            }
            if keyword.is_some_and(|after| after.chars().next().is_some_and(is_js_whitespace)) {
                line
            } else {
                ""
            }
        })
        .collect();
    kept.join("\n")
}

fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |slash| &path[..slash])
}

/// Joins a directory and a relative path, resolving `.` and `..`; `None` when it climbs out of the tree.
fn joined(directory: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = if directory.is_empty() { Vec::new() } else { directory.split('/').collect() };
    for part in relative.split('/') {
        if part == ".." {
            parts.pop()?;
        } else if part != "." && !part.is_empty() {
            parts.push(part);
        }
    }
    Some(parts.join("/"))
}

fn first_in(tree: &HashSet<String>, candidates: Vec<Option<String>>) -> Option<String> {
    candidates.into_iter().flatten().find(|candidate| tree.contains(candidate))
}

const TS_EXTENSIONS: [&str; 8] = [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"];

/// The repo-relative path of a tree file an import specifier names, or `None` when it names none: a
/// package, the standard library, or (for Go) a package directory, which is many files rather than one.
pub fn resolve_import(language: ScanLanguage, from: &str, found: &ScannedImport, tree: &HashSet<String>) -> Option<String> {
    let specifier = found.specifier.as_str();
    match language {
        ScanLanguage::Typescript => {
            if !specifier.starts_with('.') {
                return None;
            }
            let base = joined(dir_of(from), specifier)?;
            let stem = [".js", ".cjs", ".mjs", ".jsx"].iter().find_map(|suffix| base.strip_suffix(suffix)).unwrap_or(&base);
            let mut candidates = vec![Some(base.clone())];
            candidates.extend(TS_EXTENSIONS.iter().map(|extension| Some(format!("{stem}{extension}"))));
            candidates.extend(TS_EXTENSIONS.iter().map(|extension| Some(format!("{base}/index{extension}"))));
            first_in(tree, candidates)
        }
        ScanLanguage::Python => {
            let dots = specifier.chars().take_while(|c| *c == '.').count();
            let directory = if dots == 0 { String::new() } else { joined(dir_of(from), &vec![".."; dots - 1].join("/"))? };
            let module = specifier[dots..].split('.').collect::<Vec<_>>().join("/");
            let base: Vec<&str> = [directory.as_str(), module.as_str()].into_iter().filter(|part| !part.is_empty()).collect();
            let base = base.join("/");
            if base.is_empty() {
                None
            } else {
                first_in(tree, vec![Some(format!("{base}.py")), Some(format!("{base}/__init__.py"))])
            }
        }
        ScanLanguage::Kotlin => {
            let path = specifier.split('.').collect::<Vec<_>>().join("/");
            let mut matches = tree.iter().filter(|file| **file == format!("{path}.kt") || file.ends_with(&format!("/{path}.kt")));
            match (matches.next(), matches.next()) {
                (Some(only), None) => Some(only.clone()),
                _ => None,
            }
        }
        ScanLanguage::Go => None,
        ScanLanguage::Rust => {
            if found.kind != ImportKind::Module {
                return None;
            }
            // `mod x;` in lib.rs, main.rs or mod.rs names a file beside it; in any other file, one below its stem's directory.
            let name = from.rsplit('/').next().unwrap_or(from);
            let directory = if matches!(name, "lib.rs" | "main.rs" | "mod.rs") { dir_of(from) } else { from.strip_suffix(".rs").unwrap_or(from) };
            first_in(tree, vec![joined(directory, &format!("{specifier}.rs")), joined(directory, &format!("{specifier}/mod.rs"))])
        }
    }
}
