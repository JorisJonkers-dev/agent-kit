//! Finds the decision docs a spec is checked against. The node runner's `application/decision-docs.ts`.
//!
//! With a domain doc, they are the files and directories it names, and for a `CONTEXT-MAP.md` each context's
//! `CONTEXT.md` and `docs/adr/` as well. Without one, they are the first of `GLOSSARY.md` and `CONTEXT.md` that
//! exists, and the ADRs in `docs/adr/`. A record whose status says it is proposed, rejected, superseded or deprecated
//! is left out; one with no status counts as accepted. Every path returned is repository-relative.

use crate::decision_docs::{in_force, named_paths, ADR_DIRECTORY, CONTEXT_MAP, DOMAIN_DOC, GLOSSARIES, NOT_RECORDS};
use crate::error::CritiqueError;
use crate::paths::join;
use crate::runtime::Runtime;
use crate::source_scan::posix_dirname;
use model_gateway::text::basename;

/// The decision docs of the repository at `root` that are in force.
pub fn resolve_decision_docs(runtime: &Runtime, root: &str) -> Result<Vec<String>, CritiqueError> {
    let domain_doc = join(&[root, DOMAIN_DOC]);
    let found = if runtime.files.exists(&domain_doc).map_err(CritiqueError::from_fs)? {
        from_domain_doc(runtime, root, &runtime.files.read_text(&domain_doc).map_err(CritiqueError::from_fs)?)?
    } else {
        fallback(runtime, root)
    };
    let mut seen: Vec<String> = Vec::new();
    let mut docs = Vec::new();
    for path in found {
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        if NOT_RECORDS.contains(&basename(&path)) || in_force(&runtime.files.read_text(&join(&[root, &path])).map_err(CritiqueError::from_fs)?) {
            docs.push(path);
        }
    }
    Ok(docs)
}

fn from_domain_doc(runtime: &Runtime, root: &str, text: &str) -> Result<Vec<String>, CritiqueError> {
    let mut found = Vec::new();
    for path in named_paths(text) {
        found.extend(expand(runtime, root, &path));
        if basename(&path) == CONTEXT_MAP && is_file(runtime, root, &path) {
            found.extend(contexts(runtime, root, &runtime.files.read_text(&join(&[root, &path])).map_err(CritiqueError::from_fs)?));
        }
    }
    Ok(found)
}

/// Each `CONTEXT.md` a context map names, with the ADRs beside it.
fn contexts(runtime: &Runtime, root: &str, map: &str) -> Vec<String> {
    let mut found = Vec::new();
    for path in named_paths(map).into_iter().filter(|named| basename(named) == "CONTEXT.md") {
        found.extend(expand(runtime, root, &path));
        found.extend(markdown_in(runtime, root, &join(&[&posix_dirname(&path), ADR_DIRECTORY])));
    }
    found
}

fn fallback(runtime: &Runtime, root: &str) -> Vec<String> {
    let glossary = GLOSSARIES.iter().find(|candidate| is_file(runtime, root, candidate));
    glossary.map(|name| name.to_string()).into_iter().chain(markdown_in(runtime, root, ADR_DIRECTORY)).collect()
}

/// A file as itself, a directory as the markdown files directly inside it, anything missing as nothing.
fn expand(runtime: &Runtime, root: &str, path: &str) -> Vec<String> {
    let inside = markdown_in(runtime, root, path);
    if !inside.is_empty() {
        return inside;
    }
    if is_file(runtime, root, path) {
        vec![path.to_string()]
    } else {
        Vec::new()
    }
}

fn markdown_in(runtime: &Runtime, root: &str, directory: &str) -> Vec<String> {
    runtime.dirs.list_files(&join(&[root, directory])).into_iter().filter(|name| name.ends_with(".md")).map(|name| join(&[directory, &name])).collect()
}

/// True only for a plain file, so an empty directory is never read as a doc.
fn is_file(runtime: &Runtime, root: &str, path: &str) -> bool {
    runtime.dirs.list_files(&join(&[root, &posix_dirname(path)])).iter().any(|name| name == basename(path))
}
