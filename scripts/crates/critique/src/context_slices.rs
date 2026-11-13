//! The slices a code packet sends around a change, by rule and never by a model. The node runner's
//! `domain/context-slices.ts`.

use crate::recovery::line_count;
use crate::repo_map::SourceGraph;
use crate::source_scan::{mention_lines, SourceSymbol};
use crate::tree_sample::{merge_ranges, LineRange};
use std::collections::HashSet;

/// Some lines of a file of the tree that a reviewer needs beside the diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSlice {
    /// The repository path.
    pub path: String,
    pub first: usize,
    pub last: usize,
    /// Why it is in a code packet: `changed`, `callee` or `caller`.
    pub reason: &'static str,
    /// The symbol the slice holds, when it holds one.
    pub symbol: Option<String>,
    /// For a callee or a caller: the changed symbol it is called by or calls.
    pub of: Option<String>,
}

impl ContextSlice {
    /// The fields the node runner's slice object holds beyond `first` and `last`, in its own key order, which the
    /// range a slice makes keeps when the sample takes it as it is.
    pub fn extra_fields(&self) -> Vec<(&'static str, String)> {
        let mut fields = Vec::new();
        let (symbol, of) = (self.symbol.clone(), self.of.clone());
        if self.reason == "changed" {
            fields.extend(symbol.map(|symbol| ("symbol", symbol)));
            fields.push(("path", self.path.clone()));
            fields.push(("reason", self.reason.to_string()));
        } else {
            fields.extend(of.map(|of| ("of", of)));
            fields.push(("path", self.path.clone()));
            fields.push(("reason", self.reason.to_string()));
            fields.extend(symbol.map(|symbol| ("symbol", symbol)));
        }
        fields
    }
}

/// A file the change touched that is still in the tree, with its text before the change (none for an added file).
pub struct ChangedFile {
    pub path: String,
    pub text: String,
    pub before: Option<String>,
}

/// How many lines around a change a file the scan cannot read sends.
const CONTEXT_LINES: usize = 10;
/// How many lines around a change outside every symbol a source file sends.
const STATEMENT_CONTEXT_LINES: usize = 3;
/// How many callers and how many callees of each changed symbol are sent.
const RELATED_PER_SYMBOL: usize = 8;
/// Past this many cells, the line diff takes the whole changed middle of a file as changed.
const DIFF_CELLS: usize = 4_000_000;

/// The slices a code packet sends around a change: each changed hunk's enclosing top-level symbol (or, in a file the
/// scan cannot read, the hunk with ten lines either side), then each changed symbol's callees, then its callers. Each
/// slice once, in that order.
pub fn context_slices(graph: &SourceGraph, changed: &[ChangedFile]) -> Vec<ContextSlice> {
    let mut own: Vec<ContextSlice> = Vec::new();
    let mut symbols: Vec<(&str, SourceSymbol)> = Vec::new();
    for file in changed {
        let lines = changed_lines(file.before.as_deref(), &file.text);
        let scanned = graph.file(&file.path).is_some();
        let mut loose: Vec<LineRange> = Vec::new();
        let mut enclosing: Vec<SourceSymbol> = Vec::new();
        for line in lines {
            match graph.enclosing(&file.path, line) {
                Some(symbol) => match enclosing.iter().position(|held| held.first == symbol.first) {
                    Some(at) => enclosing[at] = symbol.clone(),
                    None => enclosing.push(symbol.clone()),
                },
                None => loose.push(around(line, if scanned { STATEMENT_CONTEXT_LINES } else { CONTEXT_LINES }, line_count(&file.text))),
            }
        }
        symbols.extend(enclosing.iter().map(|symbol| (file.path.as_str(), symbol.clone())));
        let mut ranges: Vec<(usize, usize, Option<String>)> = enclosing.iter().map(|symbol| (symbol.first, symbol.last, Some(symbol.name.clone()))).collect();
        ranges.extend(merge_ranges(&loose).into_iter().map(|range| (range.first, range.last, None)));
        ranges.sort_by_key(|range| range.0);
        own.extend(ranges.into_iter().map(|(first, last, symbol)| ContextSlice { path: file.path.clone(), first, last, reason: "changed", symbol, of: None }));
    }
    let callees = symbols.iter().flat_map(|(path, symbol)| callees_of(graph, path, symbol));
    let callers = symbols.iter().flat_map(|(path, symbol)| callers_of(graph, path, symbol));
    let mut seen: HashSet<String> = HashSet::new();
    own.iter().cloned().chain(callees.collect::<Vec<_>>()).chain(callers.collect::<Vec<_>>()).filter(|slice| seen.insert(format!("{}:{}-{}", slice.path, slice.first, slice.last))).collect()
}

/// The names of a text: `body.match(/[A-Za-z_$][\w$]*/g)`, each once, in the order they first appear.
fn names_in(body: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in body.chars().chain(std::iter::once(' ')) {
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            if !current.is_empty() || !c.is_ascii_digit() {
                current.push(c);
            }
        } else if !current.is_empty() {
            let name = std::mem::take(&mut current);
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// The symbols a changed symbol's body names, declared in its own file or in a file it imports.
fn callees_of(graph: &SourceGraph, path: &str, symbol: &SourceSymbol) -> Vec<ContextSlice> {
    let text = graph.file(path).map_or("", |file| file.text.as_str());
    let body = text.split('\n').skip(symbol.first - 1).take(symbol.last + 1 - symbol.first).collect::<Vec<_>>().join("\n");
    let mut found: Vec<ContextSlice> = Vec::new();
    for name in names_in(&body) {
        let Some((defined, declared)) = graph.definition(path, &name) else { continue };
        if defined == path && declared.first == symbol.first {
            continue;
        }
        found.push(ContextSlice { path: defined.to_string(), first: declared.first, last: declared.last, reason: "callee", symbol: Some(name), of: Some(symbol.name.clone()) });
    }
    found.truncate(RELATED_PER_SYMBOL);
    found
}

/// The symbols, in a changed symbol's own file and in the files that import it, that name it.
fn callers_of(graph: &SourceGraph, path: &str, symbol: &SourceSymbol) -> Vec<ContextSlice> {
    let mut found: Vec<ContextSlice> = Vec::new();
    for caller in std::iter::once(path.to_string()).chain(graph.references(path, &symbol.name)) {
        for line in mention_lines(graph.file(&caller).map_or("", |file| file.text.as_str()), &symbol.name) {
            let Some(enclosing) = graph.enclosing(&caller, line) else { continue };
            if caller == path && enclosing.first == symbol.first {
                continue;
            }
            if !found.iter().any(|other| other.path == caller && other.first == enclosing.first) {
                found.push(ContextSlice {
                    path: caller.clone(),
                    first: enclosing.first,
                    last: enclosing.last,
                    reason: "caller",
                    symbol: Some(enclosing.name.clone()),
                    of: Some(symbol.name.clone()),
                });
            }
        }
    }
    found.truncate(RELATED_PER_SYMBOL);
    found
}

fn around(line: usize, context: usize, lines: usize) -> LineRange {
    LineRange::new(line.saturating_sub(context).max(1), lines.min(line + context).max(1))
}

/// The lines of a file's new text the change added or rewrote, from 1. A removal with nothing added in its place
/// touches the line after it (or the last line). An added file is changed throughout.
pub fn changed_lines(before: Option<&str>, after: &str) -> Vec<usize> {
    let next = lines_of(after);
    let Some(before) = before else { return (1..=next.len()).collect() };
    let old = lines_of(before);
    let mut start = 0;
    while start < old.len() && start < next.len() && old[start] == next[start] {
        start += 1;
    }
    let mut end = 0;
    while end < old.len() - start && end < next.len() - start && old[old.len() - 1 - end] == next[next.len() - 1 - end] {
        end += 1;
    }
    let old_middle = &old[start..old.len() - end];
    let new_middle = &next[start..next.len() - end];
    if new_middle.is_empty() {
        return if old_middle.is_empty() || next.is_empty() { Vec::new() } else { vec![(start + 1).min(next.len())] };
    }
    if old_middle.len() * new_middle.len() > DIFF_CELLS {
        return (0..new_middle.len()).map(|i| start + i + 1).collect();
    }
    let kept = common_lines(old_middle, new_middle);
    let added: Vec<usize> = (0..new_middle.len()).filter(|i| !kept.contains(i)).map(|i| start + i + 1).collect();
    if added.is_empty() {
        vec![(start + 1).min(next.len())]
    } else {
        added
    }
}

/// A text's lines: a final newline ends the last line rather than starting another.
fn lines_of(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.strip_suffix('\n').unwrap_or(text).split('\n').collect()
}

/// The indexes of `b` on a longest common subsequence of `a` and `b`.
fn common_lines(a: &[&str], b: &[&str]) -> HashSet<usize> {
    let width = b.len() + 1;
    let mut table = vec![0u32; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i * width + j] = if a[i] == b[j] { table[(i + 1) * width + j + 1] + 1 } else { table[(i + 1) * width + j].max(table[i * width + j + 1]) };
        }
    }
    let mut kept = HashSet::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            kept.insert(j);
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    kept
}
