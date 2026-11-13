//! Which source file imports which, where their top-level symbols are, and the repo map a code packet freezes. The
//! node runner's `domain/repo-map.ts`.

use crate::code_facts::code_unit_order;
use crate::ranking::{RankedFile, RankedSymbol};
use crate::source_scan::{mentions, resolve_import, scan_source, SourceScan, SourceSymbol};
use crate::tree_sample::bytes;
use std::collections::{HashMap, HashSet};

/// A file the scan read, and the tracked files its imports name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedFile {
    pub path: String,
    pub text: String,
    pub scan: SourceScan,
    pub imports: Vec<String>,
}

/// Which source file imports which, and where their top-level symbols are, from the symbol and import scan. A file
/// references a symbol of another when it imports that file and names the symbol as a whole word.
pub struct SourceGraph {
    /// In the order the texts were given; a text in a language the scan does not know is not here.
    pub files: Vec<ScannedFile>,
    index: HashMap<String, usize>,
    importers_of: HashMap<String, Vec<String>>,
}

impl SourceGraph {
    /// Scans every file in a language the scan knows; other files are not in the graph.
    pub fn build(texts: &[(String, String)]) -> SourceGraph {
        let tracked: HashSet<String> = texts.iter().map(|(path, _)| path.clone()).collect();
        let mut files: Vec<ScannedFile> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        for (path, text) in texts {
            let Some(scan) = scan_source(path, text) else { continue };
            let mut imports: Vec<String> = Vec::new();
            for specifier in &scan.imports {
                if let Some(target) = resolve_import(path, specifier, &tracked) {
                    if target != *path && !imports.contains(&target) {
                        imports.push(target);
                    }
                }
            }
            imports.sort_by(|a, b| code_unit_order(a, b));
            let file = ScannedFile { imports, path: path.clone(), scan, text: text.clone() };
            match index.get(path) {
                Some(at) => files[*at] = file,
                None => {
                    index.insert(path.clone(), files.len());
                    files.push(file);
                }
            }
        }
        let mut importers_of: HashMap<String, Vec<String>> = HashMap::new();
        for file in &files {
            for target in &file.imports {
                importers_of.entry(target.clone()).or_default().push(file.path.clone());
            }
        }
        SourceGraph { files, importers_of, index }
    }

    pub fn file(&self, path: &str) -> Option<&ScannedFile> {
        self.index.get(path).map(|at| &self.files[*at])
    }

    /// The files that import a file, sorted.
    pub fn importers(&self, path: &str) -> Vec<String> {
        let mut found = self.importers_of.get(path).cloned().unwrap_or_default();
        found.sort_by(|a, b| code_unit_order(a, b));
        found
    }

    /// The other files that import a file and name one of its symbols.
    pub fn references(&self, path: &str, name: &str) -> Vec<String> {
        self.importers(path).into_iter().filter(|importer| mentions(self.file(importer).map_or("", |file| file.text.as_str()), name)).collect()
    }

    /// The top-level symbol whose lines hold a line of a file.
    pub fn enclosing(&self, path: &str, line: usize) -> Option<&SourceSymbol> {
        self.file(path)?.scan.symbols.iter().find(|symbol| symbol.first <= line && line <= symbol.last)
    }

    /// Where a name a file uses is declared: in the file itself, else in the first file it imports that declares it.
    pub fn definition(&self, path: &str, name: &str) -> Option<(&str, &SourceSymbol)> {
        let imports = self.file(path).map(|file| file.imports.as_slice()).unwrap_or_default();
        std::iter::once(path).chain(imports.iter().map(String::as_str)).find_map(|candidate| {
            let file = self.file(candidate)?;
            let symbol = file.scan.symbols.iter().find(|declared| declared.name == name)?;
            Some((file.path.as_str(), symbol))
        })
    }
}

/// One symbol of the repo map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMapSymbol {
    pub name: String,
    pub kind: String,
    pub signature: String,
    /// The line its declaration starts on.
    pub line: usize,
    /// How many other files reference it.
    pub references: usize,
}

/// One source file of the repo map, with its symbols, most referenced first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoMapEntry {
    pub path: String,
    /// How many other files reference one of its symbols.
    pub references: usize,
    pub symbols: Vec<RepoMapSymbol>,
}

/// Every source file the scan reads, ranked by how many other files reference its symbols, most first, then by path.
pub fn build_repo_map(graph: &SourceGraph) -> Vec<RepoMapEntry> {
    let mut entries: Vec<RepoMapEntry> = graph
        .files
        .iter()
        .map(|file| {
            let mut referencing: HashSet<String> = HashSet::new();
            let mut symbols: Vec<RepoMapSymbol> = file
                .scan
                .symbols
                .iter()
                .map(|symbol| {
                    let references = graph.references(&file.path, &symbol.name);
                    referencing.extend(references.iter().cloned());
                    RepoMapSymbol { kind: symbol.kind.clone(), line: symbol.first, name: symbol.name.clone(), references: references.len(), signature: symbol.signature.clone() }
                })
                .collect();
            symbols.sort_by(|a, b| b.references.cmp(&a.references).then(a.line.cmp(&b.line)));
            RepoMapEntry { path: file.path.clone(), references: referencing.len(), symbols }
        })
        .collect();
    entries.sort_by(|a, b| b.references.cmp(&a.references).then_with(|| code_unit_order(&a.path, &b.path)));
    entries
}

/// The repo map as frozen, and how many files its cap left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedRepoMap {
    pub text: String,
    pub listed: usize,
    pub dropped: usize,
    /// Whose map it is: code-nav's, or the runner's own scan when code-nav was unavailable.
    pub source: &'static str,
}

/// The packet path of the repo map.
pub const REPO_MAP_PATH: &str = "tree/REPO-MAP.md";

/// The repo map as a packet file: a heading per file, in rank order, with a line per symbol. Files go in while the
/// map fits under its cap; the rest, the least referenced, are left out and counted at its end.
pub fn render_repo_map(entries: &[RepoMapEntry], commit: &str, cap_bytes: i64) -> RenderedRepoMap {
    let head = [
        "# Repo map".to_string(),
        String::new(),
        format!("Every source file the runner's symbol scan reads at commit `{commit}`, with its top-level symbols and their signatures. Files are ranked by how many other files reference their symbols, most first; a file references a symbol when it imports the symbol's file and names it. Each symbol's line is its line in `tree/<path>`, which holds the file when the packet cap let it in. This map is a fact the runner computed."),
        String::new(),
    ]
    .join("\n");
    let blocks: Vec<String> = entries.iter().map(render_entry).collect();
    cap_blocks(&head, &blocks, cap_bytes, "No source file in a language the scan reads.\n", "scan")
}

/// The repo map from code-nav's `map` tool, in code-nav's rank order: a heading per file with the symbols code-nav
/// lists for it, under the same cap. `files` and `symbols` are already limited to the paths the tree holds.
pub fn render_code_nav_map(files: &[RankedFile], symbols: &[RankedSymbol], commit: &str, cap_bytes: i64) -> RenderedRepoMap {
    let head = [
        "# Repo map".to_string(),
        String::new(),
        format!("Files and symbols ranked by code-nav's map, which reads the repository's working tree, so a line can differ from `tree/<path>` (read at commit `{commit}`) where the working tree has uncommitted edits: a symbol's references are the whole-word occurrences of its name in the other scanned files, a file's score is the sum over the symbols it declares, most referenced first. A symbol's line is its line in the working tree; `tree/<path>` holds the file when the packet cap let it in. The map lists the most referenced files and symbols only. This map is a fact the runner read from code-nav."),
        String::new(),
    ]
    .join("\n");
    let blocks: Vec<String> = files
        .iter()
        .map(|file| {
            let listed: Vec<String> = symbols
                .iter()
                .filter(|symbol| symbol.path == file.path)
                .map(|symbol| format!("- `{}` ({}, line {}, {} refs)", symbol.name, symbol.kind, number_text(symbol.line), number_text(symbol.refs)))
                .collect();
            let body = if listed.is_empty() { "No symbol among the most referenced.".to_string() } else { listed.join("\n") };
            format!("\n## `{}` ({} references)\n\n{body}\n", file.path, number_text(file.refs))
        })
        .collect();
    cap_blocks(&head, &blocks, cap_bytes, "code-nav listed no symbol in this tree.\n", "code-nav")
}

fn number_text(number: f64) -> String {
    model_gateway::json::number_text(number)
}

/// Files go in while the map fits under its cap; the rest are counted at its end.
fn cap_blocks(head: &str, blocks: &[String], cap_bytes: i64, empty: &str, source: &'static str) -> RenderedRepoMap {
    let mut size = bytes(head);
    let mut listed = 0;
    for block in blocks {
        // The footer is sized for the most files it could count, so the map never passes its cap.
        let footer = if listed + 1 == blocks.len() { String::new() } else { dropped_line(blocks.len(), cap_bytes) };
        if size + bytes(block) + bytes(&footer) > cap_bytes {
            break;
        }
        size += bytes(block);
        listed += 1;
    }
    let dropped = blocks.len() - listed;
    let tail = if dropped == 0 {
        if blocks.is_empty() {
            empty.to_string()
        } else {
            String::new()
        }
    } else {
        dropped_line(dropped, cap_bytes)
    };
    RenderedRepoMap { dropped, listed, source, text: format!("{head}{}{tail}", blocks[..listed].concat()) }
}

fn render_entry(entry: &RepoMapEntry) -> String {
    let symbols: Vec<String> =
        entry.symbols.iter().map(|symbol| format!("- `{}` ({}, line {}, {} refs): `{}`", symbol.name, symbol.kind, symbol.line, symbol.references, symbol.signature)).collect();
    let body = if symbols.is_empty() { "No top-level symbols.".to_string() } else { symbols.join("\n") };
    format!("\n## `{}` (referenced by {} files)\n\n{body}\n", entry.path, entry.references)
}

fn dropped_line(count: usize, cap: i64) -> String {
    format!("\nThe map's cap of {cap} bytes left out the {count} least referenced files.\n")
}
