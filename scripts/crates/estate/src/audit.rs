//! `estate audit REPO_DIR [--latest-sha SHA] [--json]`: offline mechanical
//! checks of one estate repository (or template) directory.
//!
//! Prints one line per gap as `<file>: <gap> (source: <doc>)`, or `no gaps`.
//! Exit 0 when the audit ran, gaps or not; 2 when REPO_DIR is not a directory
//! or `--latest-sha` is not a 40-hex SHA.
//!
//! The source names are labels for live documents named in SKILL.md. The audit
//! does not read them and copies nothing from them.

use crate::args::{self, Spec};
use crate::text::{expand_user, json_string, splitlines};
use crate::Ctx;
use std::path::{Path, PathBuf};

const SPEC: Spec = Spec {
    cmd: "audit",
    usage: "usage: estate audit REPO_DIR [--latest-sha SHA] [--json]",
    values: &["--latest-sha"],
    required: &[],
    flags: &["--json"],
    min_positional: 1,
    max_positional: 1,
};

const PIN_PREFIX: &str = "JorisJonkers-dev/github-workflows/";
const MAX_TEXT_BYTES: u64 = 1_000_000;
const RENOVATE_PRESET: &str = "github>JorisJonkers-dev/renovate-config";
const PRUNE: [&str; 6] = [".git", "node_modules", "vendor", ".venv", "target", "build"];

const SRC_PIN: &str = "github-workflows README.md";
const SRC_RELEASE: &str = "github-workflows latest release";
const SRC_CI: &str = "workspace docs/decisions/ADR-0018-repository-templates.md";
const SRC_RENOVATE: &str = "renovate-config README.md";
const SRC_DEPLOY: &str = "workspace CLAUDE.md (Flux reads deploy/production)";
const SRC_NONE: &str = "none (unverified)";
const SRC_PROXY: &str = "github-workflows README.md (packages-proxy-token is a deprecated alias)";
const SRC_ORG: &str = "workspace CLAUDE.md (old-org references are guarded)";

const PROXY_TOKEN: (&str, &str) = ("packages-proxy-token", SRC_PROXY);
const SCOPED_TEXT: [(&str, &str); 2] = [
    (
        "deploy-config-schema",
        "deploy-kit README.md (deploy-config-schema is archived)",
    ),
    ("ExtraToast/", SRC_ORG),
];
const SCOPED_ROOTS: [&str; 4] = [".github", "deploy", "Taskfile.yml", "renovate.json"];
const SCOPED_SUFFIXES: [&str; 6] = [".yml", ".yaml", ".json", ".tf", ".sh", ".mjs"];

pub struct Gap {
    pub file: String,
    pub gap: String,
    pub source: &'static str,
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn rel(path: &Path, root: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.join("/")
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn workflow_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root.join(".github").join("workflows")) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file() && matches!(p.extension().and_then(|e| e.to_str()), Some("yml" | "yaml"))
        })
        .collect();
    found.sort_by_key(|p| file_name(p));
    found
}

/// `os.walk` top-down without following links: a directory's files in name
/// order, then each subdirectory in name order, skipping pruned directories.
fn walk(base: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(base) else {
        return files;
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .collect();
    names.sort();
    let (dirs, plain): (Vec<_>, Vec<_>) = names.into_iter().partition(|(_, p)| p.is_dir());
    files.extend(plain.into_iter().map(|(_, p)| p));
    for (name, path) in dirs {
        if !PRUNE.contains(&name.as_str()) && !path.is_symlink() {
            files.extend(walk(&path));
        }
    }
    files
}

/// Workflows, composite actions, and the `.tmpl` workflow templates.
fn pin_files(root: &Path) -> Vec<PathBuf> {
    let mut found = workflow_files(root);
    let bases: [(PathBuf, &[&str]); 2] = [
        (root.join(".github").join("actions"), &[".yml", ".yaml"]),
        (root.join("templates"), &[".tmpl"]),
    ];
    for (base, suffixes) in bases {
        found.extend(
            walk(&base)
                .into_iter()
                .filter(|p| suffixes.iter().any(|s| file_name(p).ends_with(s))),
        );
    }
    found
}

/// Drops a `#` comment: from a `#` at the line start or after whitespace
/// (that whitespace character goes too).
fn strip_comment(line: &str) -> &str {
    let mut previous: Option<(usize, char)> = None;
    for (i, c) in line.char_indices() {
        if c == '#' {
            match previous {
                None => return "",
                Some((p, w)) if w.is_whitespace() => return &line[..p],
                Some(_) => {}
            }
        }
        previous = Some((i, c));
    }
    line
}

/// Every `JorisJonkers-dev/github-workflows/<path>@<ref>` in one line.
fn find_pins(line: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut pos = 0;
    while let Some(offset) = line[pos..].find(PIN_PREFIX) {
        let start = pos + offset + PIN_PREFIX.len();
        let rest = &line[start..];
        let path_len = rest
            .find(|c: char| c == '@' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(rest.len());
        let after = &rest[path_len..];
        let ref_len = after
            .get(1..)
            .map(|r| {
                r.find(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == '#')
                    .unwrap_or(r.len())
            })
            .unwrap_or(0);
        if path_len > 0 && after.starts_with('@') && ref_len > 0 {
            found.push((
                rest[..path_len].to_string(),
                after[1..1 + ref_len].to_string(),
            ));
            pos = start + path_len + 1 + ref_len;
        } else {
            pos += offset + 1;
        }
    }
    found
}

fn read_lossy(path: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).into_owned()
}

fn check_pins(root: &Path, latest: Option<&str>, gaps: &mut Vec<Gap>) {
    for path in pin_files(root) {
        for (index, raw) in splitlines(&read_lossy(&path)).into_iter().enumerate() {
            for (name, reference) in find_pins(strip_comment(raw)) {
                let place = format!("line {}: github-workflows {name}@{reference}", index + 1);
                let (message, source) = if reference == "main" {
                    (format!("{place} pinned to main"), SRC_PIN)
                } else if !is_sha(&reference) {
                    (format!("{place} not pinned by 40-hex SHA"), SRC_PIN)
                } else if latest.is_some_and(|l| l != reference) {
                    (
                        format!(
                            "{place} is not the latest release SHA {}",
                            latest.unwrap_or_default()
                        ),
                        SRC_RELEASE,
                    )
                } else {
                    continue;
                };
                gaps.push(Gap {
                    file: rel(&path, root),
                    gap: message,
                    source,
                });
            }
        }
    }
}

/// `name: Pipeline Complete` as a whole (already stripped) line.
fn is_pipeline_name(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("name:") else {
        return false;
    };
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(['\'', '"']).unwrap_or(rest);
    let Some(rest) = rest.strip_prefix("Pipeline Complete") else {
        return false;
    };
    rest.strip_prefix(['\'', '"'])
        .unwrap_or(rest)
        .trim()
        .is_empty()
}

/// Counts job-level `name: Pipeline Complete` lines under the top-level `jobs:` key.
fn count_pipeline_jobs(text: &str) -> usize {
    let mut hits = 0;
    let mut in_jobs = false;
    let mut job_indent: Option<usize> = None;
    let mut key_indent: Option<usize> = None;
    for raw in splitlines(text) {
        let line = strip_comment(raw).trim_end();
        let stripped = line.trim();
        if stripped.is_empty() {
            continue;
        }
        let indent = line.chars().count() - line.trim_start().chars().count();
        if indent == 0 {
            in_jobs = stripped == "jobs:";
            job_indent = None;
            key_indent = None;
        } else if in_jobs {
            let job = *job_indent.get_or_insert(indent);
            if indent == job {
                key_indent = None;
            } else if key_indent.is_none() {
                key_indent = Some(indent);
            }
            if Some(indent) == key_indent && is_pipeline_name(stripped) {
                hits += 1;
            }
        }
    }
    hits
}

fn check_pipeline(root: &Path, gaps: &mut Vec<Gap>) {
    let hits: usize = workflow_files(root)
        .iter()
        .map(|p| count_pipeline_jobs(&read_lossy(p)))
        .sum();
    let message = match hits {
        0 => "no job named Pipeline Complete (the one required check)".to_string(),
        1 => return,
        n => format!("{n} jobs named Pipeline Complete; expected one"),
    };
    gaps.push(Gap {
        file: ".github/workflows".into(),
        gap: message,
        source: SRC_CI,
    });
}

/// Removes `//` and `/* */` comments outside JSON strings, as renovate.json allows.
fn strip_json_comments(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let skip = match chars[i] {
            '"' => string_end(&chars, i).inspect(|&end| out.extend(&chars[i..end])),
            '/' if chars.get(i + 1) == Some(&'/') => Some(
                chars[i..]
                    .iter()
                    .position(|&c| c == '\n')
                    .map_or(chars.len(), |p| i + p),
            ),
            '/' if chars.get(i + 1) == Some(&'*') => (i + 2..chars.len().saturating_sub(1))
                .find(|&j| chars[j] == '*' && chars[j + 1] == '/')
                .map(|j| j + 2),
            _ => None,
        };
        match skip {
            Some(next) => i = next,
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// The index after the string that opens at `start`, or `None` when it never closes.
fn string_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => i += 2,
            '\\' => return None,
            '"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

fn check_renovate(root: &Path, gaps: &mut Vec<Gap>) {
    let candidates = [
        root.join("renovate.json"),
        root.join(".github").join("renovate.json"),
    ];
    let Some(path) = candidates.iter().find(|p| p.is_file()) else {
        gaps.push(Gap {
            file: "renovate.json".into(),
            gap: "no renovate.json at the root or in .github".into(),
            source: SRC_RENOVATE,
        });
        return;
    };
    let file = rel(path, root);
    let Ok(data) =
        serde_json::from_str::<serde_json::Value>(&strip_json_comments(&read_lossy(path)))
    else {
        gaps.push(Gap {
            file,
            gap: "could not confirm the preset (renovate.json could not be parsed)".into(),
            source: SRC_RENOVATE,
        });
        return;
    };
    let extends: Vec<&serde_json::Value> = match data.get("extends") {
        Some(serde_json::Value::Array(list)) => list.iter().collect(),
        Some(single @ serde_json::Value::String(_)) => vec![single],
        _ => Vec::new(),
    };
    let present = extends.iter().any(|e| {
        e.as_str()
            .is_some_and(|s| s == RENOVATE_PRESET || s.starts_with(&format!("{RENOVATE_PRESET}#")))
    });
    if !present {
        gaps.push(Gap {
            file,
            gap: format!("does not extend \"{RENOVATE_PRESET}\""),
            source: SRC_RENOVATE,
        });
    }
}

/// The text of a small regular text file, or `None` for binary, large or non-regular files.
fn read_text_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_TEXT_BYTES {
        return None;
    }
    let data = std::fs::read(path).ok()?;
    if data.iter().take(8192).any(|&b| b == 0) {
        return None;
    }
    Some(String::from_utf8_lossy(&data).into_owned())
}

fn check_retired(root: &Path, gaps: &mut Vec<Gap>) {
    if root.join("platform").is_dir() {
        gaps.push(Gap {
            file: "platform/".into(),
            gap: "top-level platform/ directory; deploy files belong in deploy/".into(),
            source: SRC_DEPLOY,
        });
    }
    for path in walk(root) {
        let name = file_name(&path);
        let relpath = rel(&path, root);
        if matches!(name.to_lowercase().as_str(), "fleet.yaml" | "fleet.yml") {
            gaps.push(Gap {
                file: relpath.clone(),
                gap: "fleet.yaml is flagged as retired (unverified: no live doc says so); projects are <name>.project.yml"
                    .into(),
                source: SRC_NONE,
            });
        }
        let Some(text) = read_text_file(&path) else {
            continue;
        };
        let mut tokens = vec![PROXY_TOKEN];
        let first = relpath.split('/').next().unwrap_or_default();
        if SCOPED_ROOTS.contains(&first) && SCOPED_SUFFIXES.iter().any(|s| name.ends_with(s)) {
            tokens.extend(SCOPED_TEXT);
        }
        for (index, line) in splitlines(&text).into_iter().enumerate() {
            for (token, source) in &tokens {
                if line.contains(token) {
                    gaps.push(Gap {
                        file: relpath.clone(),
                        gap: format!("line {}: retired reference '{token}'", index + 1),
                        source,
                    });
                }
            }
        }
    }
}

/// The gaps of one repository directory.
pub fn audit(root: &Path, latest: Option<&str>) -> Vec<Gap> {
    let mut gaps = Vec::new();
    check_pins(root, latest, &mut gaps);
    check_pipeline(root, &mut gaps);
    check_renovate(root, &mut gaps);
    check_retired(root, &mut gaps);
    gaps
}

fn json_list(gaps: &[Gap]) -> String {
    if gaps.is_empty() {
        return "[]".into();
    }
    let rows: Vec<String> = gaps
        .iter()
        .map(|g| {
            format!(
                "  {{\n    \"file\": {},\n    \"gap\": {},\n    \"source\": {}\n  }}",
                json_string(&g.file),
                json_string(&g.gap),
                json_string(g.source)
            )
        })
        .collect();
    format!("[\n{}\n]", rows.join(",\n"))
}

pub fn run(argv: &[String], ctx: &mut Ctx) -> i32 {
    let parsed = match args::parse(&SPEC, argv, ctx) {
        Ok(p) => p,
        Err(status) => return status,
    };
    let given = &parsed.positional[0];
    let root = PathBuf::from(expand_user(given, ctx.home.as_deref()));
    if !root.is_dir() {
        ctx.warn(&format!("estate audit: not a directory: {given}"));
        return 2;
    }
    let latest = parsed.value("--latest-sha");
    if latest.is_some_and(|l| !is_sha(l)) {
        ctx.warn("estate audit: --latest-sha must be 40 lowercase hex characters");
        return 2;
    }
    let gaps = audit(&root, latest);
    if parsed.flag("--json") {
        ctx.say(&json_list(&gaps));
    } else if gaps.is_empty() {
        ctx.say("no gaps");
    } else {
        for g in &gaps {
            ctx.say(&format!("{}: {} (source: {})", g.file, g.gap, g.source));
        }
    }
    0
}

#[cfg(test)]
mod tests;
