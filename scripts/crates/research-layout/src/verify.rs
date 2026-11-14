//! `verify`: Workflow A's verifier (issue #456). It only reads: the papers
//! directory the merge worker wrote, the fetch log and pages the fetch shim
//! stored, and git's record of what the run changed.
//!
//! It fails with every problem of [crate::layout::check] and, on top:
//! * `<id>: page-not-as-fetched` for a catalog entry whose `url` the fetch
//!   log fetched but none of whose files is byte for byte the stored page
//!   (so a page edited after the fetch, to fit a quote, is caught);
//! * for every note under `notes/` the run added or changed (against
//!   `--base`, plus untracked notes): `<note>: no-quote` when it quotes
//!   nothing, `<note>: quote-too-short` for a quote under 8 characters,
//!   and `<note>: quote-page-not-fetched: <page>` when the page a quote names
//!   is not one the fetch stored (a page the merge worker wrote itself);
//! * `notes: no-run-note` when the run added no note at all.
//!
//! A quote that is not on its page fails `check` itself
//! (`<page>: quote-not-found`), and since every quoted page must be a
//! fetched one, a quote that is not on its fetched page fails verification.

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::fetch::LOG;
use crate::layout::{check, collapse, escapes, note_quotes, read_catalog, split_front};

/// The shortest quote that counts (as for the finding check).
const MIN_QUOTE: usize = 8;

/// The fetch log's stored pages: url to SHA-256.
fn fetched_pages(fetched: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = fetched.join(LOG);
    let text =
        fs::read_to_string(&path).map_err(|e| format!("{}: cannot read: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("{}: not valid JSON: {e}", path.display()))?;
    let Some(sources) = value.get("sources").and_then(Value::as_array) else {
        return Err(format!("{}: no sources array", path.display()));
    };
    let mut pages = BTreeMap::new();
    for source in sources {
        let url = source.get("url").and_then(Value::as_str);
        let sha = source.get("sha256").and_then(Value::as_str);
        if let (Some(url), Some(sha)) = (url, sha) {
            pages.insert(url.to_string(), sha.to_string());
        }
    }
    Ok(pages)
}

/// The SHA-256 of a papers-relative file, or `None` when it escapes the
/// directory or cannot be read.
fn file_sha(papers: &Path, rel: &str) -> Option<String> {
    if escapes(rel) {
        return None;
    }
    let bytes = fs::read(papers.join(rel)).ok()?;
    Some(gold_band_report::sha256_hex(&bytes))
}

/// The notes the run added or changed: `notes/<name>.md` files (not the
/// README) that differ from `base` or are untracked, sorted.
pub(crate) fn changed_notes(git: &str, papers: &Path, base: &str) -> Result<Vec<String>, String> {
    let mut names = BTreeSet::new();
    let diff = ["diff", "--name-only", "--relative", base, "--", "notes"];
    let untracked = ["ls-files", "--others", "--exclude-standard", "--", "notes"];
    for args in [&diff[..], &untracked[..]] {
        let out = Command::new(git)
            .arg("-C")
            .arg(papers)
            .args(args)
            .output()
            .map_err(|e| format!("cannot start {git}: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git {}: {}",
                args[0],
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let rest = line.strip_prefix("notes/").unwrap_or_default();
            if rest.ends_with(".md") && !rest.contains('/') && rest != "README.md" {
                names.insert(line.to_string());
            }
        }
    }
    Ok(names.into_iter().collect())
}

/// The problems of the run's own notes against the stored pages.
fn note_problems(papers: &Path, notes: &[String], stored: &BTreeSet<&str>) -> Vec<String> {
    let mut problems = Vec::new();
    if notes.is_empty() {
        problems.push("notes: no-run-note".to_string());
    }
    for note in notes {
        let text = fs::read_to_string(papers.join(note)).unwrap_or_default();
        let (_, body) = split_front(&text);
        let quotes = note_quotes(&body);
        if quotes.is_empty() {
            problems.push(format!("{note}: no-quote"));
        }
        for (quote, page) in quotes {
            if collapse(&quote).chars().count() < MIN_QUOTE {
                problems.push(format!("{note}: quote-too-short"));
            }
            let fetched = file_sha(papers, &page).is_some_and(|sha| stored.contains(sha.as_str()));
            if !fetched {
                problems.push(format!("{note}: quote-page-not-fetched: {page}"));
            }
        }
    }
    problems
}

/// Every problem of the papers directory after a Workflow A run, or the
/// reason the verifier could not read its inputs (exit 2).
pub fn verify(git: &str, papers: &Path, fetched: &Path, base: &str) -> Result<Vec<String>, String> {
    let pages = fetched_pages(fetched)?;
    let notes = changed_notes(git, papers, base)?;
    let mut problems = check(papers);
    if let Ok((entries, _)) = read_catalog(papers) {
        for entry in &entries {
            let Some(sha) = entry.url.as_ref().and_then(|url| pages.get(url)) else {
                continue;
            };
            let as_fetched = entry
                .files
                .iter()
                .any(|file| file_sha(papers, file).as_ref() == Some(sha));
            if !as_fetched {
                problems.push(format!("{}: page-not-as-fetched", entry.id));
            }
        }
    }
    let stored: BTreeSet<&str> = pages.values().map(String::as_str).collect();
    problems.extend(note_problems(papers, &notes, &stored));
    Ok(problems)
}
