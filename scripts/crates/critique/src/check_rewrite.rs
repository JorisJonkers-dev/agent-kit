//! `check-rewrite`: the rewrite check run after `/critique-adrs` applies its editorial edits.
//! It confirms each original decision sentence appears byte-identical in exactly one rewritten
//! file, that no rewritten file labels a decision sentence that is not an original one, and that a
//! split marks the original record superseded by the split files. It reads the working tree and
//! writes nothing; a repository whose remotes are refused is still refused.

use crate::adr_log::{marked_superseded_by, AdrSource};
use crate::error::CritiqueError;
use crate::fields::Fields;
use crate::repository::{resolve, Resolution};
use crate::runtime::Runtime;
use model_gateway::json::{Object, Value};
use model_gateway::text::{join, resolve as resolve_path};

struct Rewritten {
    path: String,
    decision_sentences: Vec<String>,
}

struct Split {
    files: Vec<String>,
    original: String,
}

struct Input {
    original_sentences: Vec<String>,
    rewritten: Vec<Rewritten>,
    repo: String,
    split: Option<Split>,
}

fn read_input(raw: &Value) -> Result<Input, crate::error::ShapeError> {
    let fields = Fields::of(raw, "")?;
    let split = if fields.has("split") { Some(fields.object("split")?) } else { None };
    let original_sentences = fields.strings("original_sentences")?;
    let rewritten = fields.list("rewritten", |item| Ok(Rewritten { decision_sentences: item.strings("decision_sentences")?, path: item.string("path")? }))?;
    let repo = fields.string("repo")?;
    let split = match split {
        None => None,
        Some(split) => Some(Split { files: split.strings("files")?, original: split.string("original")? }),
    };
    Ok(Input { original_sentences, rewritten, repo, split })
}

pub fn run(raw: &Value, runtime: &Runtime) -> Result<Value, CritiqueError> {
    let input = read_input(raw).map_err(|error| error.into_error("invalid-input", "check-rewrite input"))?;
    let named = input.original_sentences.iter().chain(input.rewritten.iter().flat_map(|file| file.decision_sentences.iter()));
    if named.into_iter().any(|sentence| sentence.is_empty()) {
        return Err(CritiqueError::new("invalid-input", "check-rewrite input: a decision sentence is empty"));
    }
    let root = match resolve(runtime, &input.repo)? {
        Resolution::Refused(refusal) => return Ok(refusal),
        Resolution::Resolved(repository) => repository.root,
    };
    // Every path is placed in the repository before any file is looked at, as the node runner's concurrent reads do.
    let mut places = Vec::new();
    for file in &input.rewritten {
        places.push(inside_repository(&root, &file.path)?);
    }
    let mut texts = Vec::new();
    for (file, place) in input.rewritten.iter().zip(&places) {
        texts.push(read(runtime, &root, &file.path, place)?);
    }
    let split = match &input.split {
        None => None,
        Some(split) => {
            let place = inside_repository(&root, &split.original)?;
            Some((split, read(runtime, &root, &split.original, &place)?))
        }
    };
    let mut failures = Vec::new();
    for sentence in &input.original_sentences {
        let found_in: Vec<&str> = input.rewritten.iter().zip(&texts).filter(|(_, text)| text.contains(sentence.as_str())).map(|(file, _)| file.path.as_str()).collect();
        // Occurrences, not files: the same sentence twice in one file is a duplication too.
        let occurrences: usize = texts.iter().map(|text| text.matches(sentence.as_str()).count()).sum();
        if occurrences != 1 {
            let mut item = Object::new();
            item.set("check", Value::str("sentence-preserved"));
            item.set("found_in", Value::Arr(found_in.into_iter().map(Value::str).collect()));
            item.set("problem", Value::str(if occurrences == 0 { "missing" } else { "duplicated" }));
            item.set("sentence", Value::str(sentence));
            failures.push(Value::Obj(item));
        }
    }
    for file in &input.rewritten {
        for sentence in &file.decision_sentences {
            if !input.original_sentences.contains(sentence) {
                let mut item = Object::new();
                item.set("check", Value::str("no-new-decision-sentence"));
                item.set("file", Value::str(&file.path));
                item.set("sentence", Value::str(sentence));
                failures.push(Value::Obj(item));
            }
        }
    }
    if let Some((split, text)) = split {
        let record = AdrSource { path: split.original.clone(), text };
        let unmarked: Vec<Value> = split.files.iter().filter(|target| !marked_superseded_by(&record, target)).map(|target| Value::str(target)).collect();
        if !unmarked.is_empty() {
            let mut item = Object::new();
            item.set("check", Value::str("split-superseded"));
            item.set("original", Value::str(&split.original));
            item.set("unmarked", Value::Arr(unmarked));
            failures.push(Value::Obj(item));
        }
    }
    let mut result = Object::new();
    let status = if failures.is_empty() { "passed" } else { "failed" };
    result.set("failures", Value::Arr(failures));
    result.set("status", Value::str(status));
    Ok(Value::Obj(result))
}

/// The repository-relative form of `path`; a path outside the repository is an error.
fn inside_repository(root: &str, path: &str) -> Result<String, CritiqueError> {
    let absolute = resolve_path(root, path);
    let relative = if absolute == root {
        Some("")
    } else {
        absolute.strip_prefix(&format!("{}/", root.trim_end_matches('/')))
    };
    match relative {
        Some(relative) if !relative.starts_with("..") => Ok(relative.to_string()),
        _ => Err(CritiqueError::new("outside-repository", format!("{path} is outside the repository"))),
    }
}

/// A rewritten file as it sits in the working tree; one missing there is input the command cannot check.
fn read(runtime: &Runtime, root: &str, path: &str, place: &str) -> Result<String, CritiqueError> {
    let full = join(&[root, place]);
    if !runtime.files.exists(&full).unwrap_or(false) {
        return Err(CritiqueError::new("invalid-input", format!("check-rewrite input: {path} does not exist in the repository")));
    }
    runtime.files.read_text(&full).map_err(CritiqueError::from_fs)
}
