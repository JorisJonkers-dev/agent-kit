//! The deterministic facts of ADR-0004: a file's history as git reports it, and what a set of
//! modules imports and how it depends on itself. Ported from the node runner's `code-facts.ts`.

use crate::git::LastChange;
use crate::import_scan::{language_of, resolve_import, scan_imports, ScanLanguage};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};

/// JavaScript's default string order: by UTF-16 code unit, so the output never depends on the machine's locale.
pub fn code_unit_order(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// One file's history as the runner reads it from git: commit ids and committer dates, never an author or a message.
pub struct FileHistory {
    pub path: String,
    /// Newest first.
    pub commits: Vec<LastChange>,
}

pub struct HistoryFact {
    pub path: String,
    pub last_commit: Option<String>,
    /// The committer date of the last commit, as git prints it (ISO 8601); never today's.
    pub last_date: Option<String>,
    /// How many commits changed the path.
    pub churn: usize,
}

pub fn history_facts(histories: &[FileHistory]) -> Vec<HistoryFact> {
    histories
        .iter()
        .map(|history| HistoryFact {
            churn: history.commits.len(),
            last_commit: history.commits.first().map(|change| change.commit.clone()),
            last_date: history.commits.first().map(|change| change.date.clone()),
            path: history.path.clone(),
        })
        .collect()
}

/// A module's source at the commit; `None` when the commit has no such file.
pub struct ModuleSource {
    pub path: String,
    pub text: Option<String>,
}

pub struct ImportFact {
    pub specifier: String,
    /// The tree file the specifier names; `None` for a package, a standard-library module, or a Go package directory.
    pub resolved: Option<String>,
}

pub struct ModuleFact {
    pub path: String,
    /// The scan used; `None` when the language is not known, and then no import facts were computed.
    pub language: Option<ScanLanguage>,
    pub imports: Vec<ImportFact>,
    /// How many named modules import this one.
    pub fan_in: usize,
    /// How many named modules this one imports.
    pub fan_out: usize,
    /// fan_out / (fan_in + fan_out) over the named modules; `None` for a module with neither.
    pub instability: Option<f64>,
}

pub struct CodeFacts {
    pub modules: Vec<ModuleFact>,
    /// Each import cycle among the named modules, its members sorted.
    pub cycles: Vec<Vec<String>>,
    /// What was not computed, and why.
    pub notes: Vec<String>,
}

pub fn code_facts(sources: &[ModuleSource], tree: &HashSet<String>) -> CodeFacts {
    let mut notes = Vec::new();
    let scanned: Vec<(String, Option<ScanLanguage>, Vec<ImportFact>)> = sources
        .iter()
        .map(|source| {
            let path = &source.path;
            let language = language_of(path);
            let imports = match (language, &source.text) {
                (None, _) => {
                    notes.push(format!("{path}: no import facts were computed (the import scan does not know this language)"));
                    Vec::new()
                }
                (Some(_), None) => {
                    notes.push(format!("{path}: no import facts were computed (the file is not in the commit)"));
                    Vec::new()
                }
                (Some(language), Some(text)) => scan_imports(language, text)
                    .into_iter()
                    .map(|found| ImportFact { resolved: resolve_import(language, path, &found, tree), specifier: found.specifier })
                    .collect(),
            };
            (path.clone(), language, imports)
        })
        .collect();
    let named: HashSet<&str> = sources.iter().map(|source| source.path.as_str()).collect();
    let mut edges: Vec<(String, Vec<String>)> = Vec::new();
    for (path, _, imports) in &scanned {
        let targets: BTreeSet<Vec<u16>> = imports
            .iter()
            .filter_map(|import| import.resolved.as_deref())
            .filter(|resolved| *resolved != path.as_str() && named.contains(resolved))
            .map(|resolved| resolved.encode_utf16().collect())
            .collect();
        let targets = targets.into_iter().map(|units| String::from_utf16_lossy(&units)).collect();
        // A path named twice is one node, the later entry replacing the earlier one in the earlier one's place, as a Map does.
        match edges.iter_mut().find(|(known, _)| known == path) {
            Some(entry) => entry.1 = targets,
            None => edges.push((path.clone(), targets)),
        }
    }
    let modules = scanned
        .into_iter()
        .map(|(path, language, imports)| {
            let fan_out = edges.iter().find(|(known, _)| *known == path).map_or(0, |(_, targets)| targets.len());
            let fan_in = edges.iter().filter(|(_, targets)| targets.contains(&path)).count();
            let instability = (fan_in + fan_out != 0).then(|| fan_out as f64 / (fan_in + fan_out) as f64);
            ModuleFact { path, language, imports, fan_in, fan_out, instability }
        })
        .collect();
    CodeFacts { cycles: cycles(&edges), modules, notes }
}

struct Tarjan<'a> {
    edges: &'a HashMap<&'a str, &'a Vec<String>>,
    index: HashMap<&'a str, usize>,
    low: HashMap<&'a str, usize>,
    stack: Vec<&'a str>,
    found: Vec<Vec<String>>,
}

impl<'a> Tarjan<'a> {
    fn visit(&mut self, node: &'a str) {
        self.index.insert(node, self.index.len());
        self.low.insert(node, self.index[node]);
        self.stack.push(node);
        let targets: &'a Vec<String> = self.edges[node];
        for next in targets {
            if !self.index.contains_key(next.as_str()) {
                self.visit(next);
                self.low.insert(node, self.low[node].min(self.low[next.as_str()]));
            } else if self.stack.contains(&next.as_str()) {
                self.low.insert(node, self.low[node].min(self.index[next.as_str()]));
            }
        }
        if self.low[node] == self.index[node] {
            let at = self.stack.iter().position(|member| *member == node).unwrap_or_default();
            let mut component: Vec<String> = self.stack.split_off(at).into_iter().map(str::to_string).collect();
            if component.len() > 1 {
                component.sort_by(|a, b| code_unit_order(a, b));
                self.found.push(component);
            }
        }
    }
}

/// The strongly connected components of more than one module (Tarjan), each sorted, and in order of their first member.
fn cycles(edges: &[(String, Vec<String>)]) -> Vec<Vec<String>> {
    let by_path: HashMap<&str, &Vec<String>> = edges.iter().map(|(path, targets)| (path.as_str(), targets)).collect();
    let mut tarjan = Tarjan { edges: &by_path, index: HashMap::new(), low: HashMap::new(), stack: Vec::new(), found: Vec::new() };
    let mut nodes: Vec<&str> = edges.iter().map(|(path, _)| path.as_str()).collect();
    nodes.sort_by(|a, b| code_unit_order(a, b));
    for node in nodes {
        if !tarjan.index.contains_key(node) {
            tarjan.visit(node);
        }
    }
    let mut found = tarjan.found;
    found.sort_by(|a, b| code_unit_order(&a[0], &b[0]));
    found
}
