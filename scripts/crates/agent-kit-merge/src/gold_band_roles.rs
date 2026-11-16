//! `agent-kit-merge converge gold-band-roles`: the `context/profiles` directory of the personal Gold Band instance,
//! one Markdown profile per role the registry declares, rendered from the critique skill's own role, prompt, schema
//! and lens files (the Rust port of `scripts/converge.py gold-band-roles`, #810).
//!
//! A profile the registry does not declare is removed unless the keep file names its stem; a stem or id in
//! `gold_band.retired_roles` goes even when kept, and the app's own `pf-builtin-*` profiles are never touched.
//! `--check` prints the changes and writes nothing.

use crate::gold_band::{self, Processes};
use crate::{Env, Failure, Io};
use regex::Regex;
use serde_yaml::{Mapping, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The stamp every generated profile's front matter carries, so a second run rewrites nothing.
const STAMP: &str = "2025-01-01T00:00:00Z";

/// The id prefix of the app's own profiles, which are never touched.
const BUILT_IN: &str = "pf-builtin-";

/// The fields a role may declare.
const ROLE_FIELDS: [&str; 6] = ["file", "id", "name", "summary", "source", "lenses"];

const TOOL: &str = "gold-band-roles";

/// A file stem or agent id: lower-case letters and digits in dash-separated groups.
const ID_PATTERN: &str = r"^[a-z0-9]+(-[a-z0-9]+)*$";

/// A front matter value that stays one plain YAML scalar.
const PLAIN_PATTERN: &str = r"^[A-Za-z0-9$(][^\n#]*$";

/// Why the directory cannot be converged at all.
struct Cannot(String);

/// The flags of `converge gold-band-roles`.
pub struct RolesArgs {
    pub keep: PathBuf,
    pub registry: PathBuf,
    pub profiles: Option<PathBuf>,
    pub critique: Option<PathBuf>,
    pub closing: Option<PathBuf>,
    pub check: bool,
}

/// One declared role, held to the fields render reads once its problems are none.
struct Role {
    file: String,
    id: String,
    name: String,
    summary: String,
    source: String,
    lenses: Vec<String>,
}

/// The roles source, the declared roles and the retired stems and ids.
fn registry(path: &Path) -> Result<(Option<String>, Vec<Mapping>, BTreeSet<String>), Cannot> {
    let text = fs::read_to_string(path).map_err(|error| Cannot(format!("{} cannot be read: {error}", path.display())))?;
    let data: Value = serde_yaml::from_str(&text).map_err(|error| Cannot(format!("{} cannot be read: {error}", path.display())))?;
    let Some(data) = data.as_mapping() else { return Err(Cannot(format!("{} is not a mapping", path.display()))) };
    let Some(section) = data.get("gold_band").and_then(Value::as_mapping) else {
        return Err(Cannot(format!("{} has no gold_band mapping", path.display())));
    };
    let source = match section.get("roles_source") {
        None | Some(Value::Null) => None,
        Some(Value::String(source)) => Some(source.clone()),
        Some(_) => return Err(Cannot(format!("{}: gold_band.roles_source must be a path", path.display()))),
    };
    let roles: Option<Vec<Mapping>> = section.get("roles").and_then(Value::as_sequence).and_then(|items| items.iter().map(|item| item.as_mapping().cloned()).collect());
    let Some(roles) = roles else { return Err(Cannot(format!("{}: gold_band.roles must be a list of mappings", path.display()))) };
    let retired: Option<BTreeSet<String>> = match section.get("retired_roles") {
        None | Some(Value::Null) => Some(BTreeSet::new()),
        Some(value) => value.as_sequence().and_then(|items| items.iter().map(|item| item.as_str().map(str::to_string)).collect()),
    };
    let Some(retired) = retired else { return Err(Cannot(format!("{}: gold_band.retired_roles must be a list of stems or ids", path.display()))) };
    Ok((source, roles, retired))
}

fn text_of<'a>(role: &'a Mapping, key: &str) -> Option<&'a str> {
    role.get(key).and_then(Value::as_str)
}

/// One line per problem with the declared roles; empty when they may be written.
fn problems(declared: &[Mapping], retired: &BTreeSet<String>) -> Vec<String> {
    let id = Regex::new(ID_PATTERN).expect("pattern");
    let plain = Regex::new(PLAIN_PATTERN).expect("pattern");
    let mut problems = Vec::new();
    let (mut files, mut ids) = (BTreeSet::new(), BTreeSet::new());
    for role in declared {
        let file = text_of(role, "file");
        let stem = file.unwrap_or("?");
        match file {
            Some(file) if !id.is_match(file) => problems.push(format!("{stem}: file must match {ID_PATTERN}")),
            None => problems.push(format!("{stem}: file must match {ID_PATTERN}")),
            Some(file) if file.starts_with(BUILT_IN) => problems.push(format!("{stem}: file must not name a built-in profile")),
            Some(file) if !files.insert(file.to_string()) => problems.push(format!("{stem}: duplicate file")),
            Some(_) => {}
        }
        let role_id = text_of(role, "id");
        let name = role_id.unwrap_or(stem);
        match role_id {
            Some(role_id) if !role_id.starts_with("pf-") => problems.push(format!("{name}: id must start with pf-")),
            None => problems.push(format!("{name}: id must start with pf-")),
            Some(role_id) if role_id.starts_with(BUILT_IN) => problems.push(format!("{name}: id must not name a built-in profile")),
            Some(role_id) if !ids.insert(role_id.to_string()) => problems.push(format!("{name}: duplicate id")),
            Some(_) => {}
        }
        for key in ["name", "summary", "source"] {
            if text_of(role, key).is_none() {
                problems.push(format!("{name}: {key} must be a string"));
            }
        }
        // name and summary go into the front matter unquoted, so each must stay one plain YAML scalar.
        for key in ["name", "summary"] {
            if text_of(role, key).is_some_and(|value| !plain.is_match(value)) {
                problems.push(format!("{name}: {key} must be a plain single-line scalar"));
            }
        }
        let lenses = role.get("lenses").and_then(Value::as_sequence);
        if !lenses.is_some_and(|lenses| lenses.iter().all(Value::is_string)) {
            problems.push(format!("{name}: lenses must be a list of names"));
        }
        if file.is_some_and(|file| retired.contains(file)) || role_id.is_some_and(|role_id| retired.contains(role_id)) {
            problems.push(format!("{name}: declared and retired at once"));
        }
        for key in role.keys() {
            let key = key.as_str().map_or_else(|| serde_yaml::to_string(key).unwrap_or_default().trim().to_string(), str::to_string);
            if !ROLE_FIELDS.contains(&key.as_str()) {
                problems.push(format!("{name}: unknown field {key}"));
            }
        }
    }
    problems
}

/// A role whose problems are none, as render reads it.
fn role(declared: &Mapping) -> Role {
    let text = |key: &str| text_of(declared, key).unwrap_or_default().to_string();
    let lenses = declared.get("lenses").and_then(Value::as_sequence).map(|lenses| lenses.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    Role { file: text("file"), id: text("id"), name: text("name"), summary: text("summary"), source: text("source"), lenses }
}

/// A source file's text; a missing or undecodable one cannot be converged.
fn source_text(path: &Path) -> Result<String, Cannot> {
    fs::read_to_string(path).map_err(|error| Cannot(format!("{}: {error}", path.display())))
}

/// One section: the text, a closing newline, then the blank line that separates it.
fn section(text: &str) -> String {
    let close = if text.ends_with('\n') { "" } else { "\n" };
    format!("{text}{close}\n")
}

/// One profile: the front matter, then the role, the shared prompts, the schema, its lenses and the closing, each
/// verbatim, never templated.
fn render(critique: &Path, role: &Role, closing: &str) -> Result<String, Cannot> {
    let header = format!(
        "---\nid: {}\nname: {}\nsummary: {}\ncreatedAt: {STAMP}\nupdatedAt: {STAMP}\ndynamicTemplate: false\n---\n",
        role.id, role.name, role.summary
    );
    let schema = source_text(&critique.join("schemas").join("finding.schema.json"))?;
    let mut sections = vec![
        source_text(&critique.join("roles").join(&role.source))?,
        source_text(&critique.join("prompts").join("preamble.md"))?,
        source_text(&critique.join("prompts").join("output-contract.md"))?,
        format!("```json\n{}\n```", schema.trim_end()),
    ];
    for lens in &role.lenses {
        sections.push(source_text(&critique.join("lenses").join(format!("{lens}.md")))?);
    }
    sections.push(closing.to_string());
    Ok(header + "\n" + &sections.iter().map(|text| section(text)).collect::<String>())
}

/// The `id:` of a profile's first front matter block, or `None`.
fn profile_id(text: Option<&str>) -> Option<String> {
    let mut lines = text?.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            return None;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.trim() == "id" {
                return Some(value.trim().to_string()).filter(|value| !value.is_empty());
            }
        }
    }
    None
}

/// Written beside it and renamed, so a starting Gold Band never reads half a profile.
fn write_profile(path: &Path, text: &str) -> std::io::Result<()> {
    let staged = path.with_file_name(format!(".{}.agent-kit-tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    let written = fs::write(&staged, text).and_then(|()| fs::rename(&staged, path));
    if written.is_err() {
        let _ = fs::remove_file(&staged);
    }
    written
}

/// The profiles of the directory, by stem.
fn existing(profiles: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(profiles) else { return Vec::new() };
    let mut found: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| (path.file_stem().unwrap_or_default().to_string_lossy().into_owned(), path))
        .collect();
    found.sort();
    found
}

/// The roles rendered, by file, or why they cannot be.
fn rendered(args: &RolesArgs, source: Option<String>, declared: &[Mapping]) -> Result<Vec<(String, String)>, Cannot> {
    let kit = args.registry.parent().and_then(Path::parent).unwrap_or(Path::new(""));
    let critique = match (&args.critique, source) {
        (Some(critique), _) => critique.clone(),
        (None, Some(source)) => kit.join(source),
        (None, None) => return Err(Cannot(format!("{}: gold_band.roles_source must be a path", args.registry.display()))),
    };
    let closing = source_text(&args.closing.clone().unwrap_or_else(|| kit.join("scripts/gold-band/roles-closing.md")))?;
    // Every role is rendered before anything is touched: one unreadable source leaves the directory as it was.
    declared.iter().map(role).map(|role| Ok((role.file.clone(), render(&critique, &role, &closing)?))).collect()
}

/// `converge gold-band-roles`.
pub fn run(args: &RolesArgs, processes: &dyn Processes, io: &mut Io, env: Env) -> Result<i32, Failure> {
    let profiles = args.profiles.clone().unwrap_or_else(|| gold_band::settings_path(env).parent().unwrap_or(Path::new("")).join("context").join("profiles"));
    let data = profiles.parent().and_then(Path::parent).unwrap_or(Path::new("")).to_path_buf();
    let prepared = registry(&args.registry).and_then(|(source, declared, retired)| {
        let found = problems(&declared, &retired);
        Ok((source, declared, retired, found))
    });
    let (source, declared, retired) = match prepared {
        Err(Cannot(message)) => {
            writeln!(io.out, "{TOOL}: cannot converge: {message}")?;
            return Ok(3);
        }
        Ok((_, _, _, found)) if !found.is_empty() => {
            for problem in found {
                writeln!(io.out, "{TOOL}: {problem}")?;
            }
            return Ok(1);
        }
        Ok((source, declared, retired, _)) => (source, declared, retired),
    };
    if !args.check {
        if let Some(running) = gold_band::running(&data, env, processes) {
            writeln!(io.out, "{TOOL}: Gold Band instance {running} is running; quit it and re-run setup (nothing was written)")?;
            return Ok(gold_band::RUNNING_EXIT);
        }
    }
    let rendered = match rendered(args, source, &declared) {
        Ok(rendered) => rendered,
        Err(Cannot(message)) => {
            writeln!(io.out, "{TOOL}: cannot converge: {message}")?;
            return Ok(3);
        }
    };
    let keep = match crate::converge::keep(&args.keep, io)? {
        Ok(keep) => keep,
        Err(status) => return Ok(status),
    };
    let existing = existing(&profiles);
    let read = |stem: &str| existing.iter().find(|(known, _)| known == stem).and_then(|(_, path)| fs::read_to_string(path).ok());
    for (stem, _) in &rendered {
        if profile_id(read(stem).as_deref()).is_some_and(|held| held.starts_with(BUILT_IN)) {
            writeln!(io.out, "{TOOL}: {stem}: the existing profile is a built-in; nothing was written")?;
            return Ok(1);
        }
    }
    let is_existing = |stem: &str| existing.iter().any(|(known, _)| known == stem);
    let added: Vec<&String> = rendered.iter().filter(|(stem, _)| !is_existing(stem)).map(|(stem, _)| stem).collect();
    let changed: Vec<&String> = rendered.iter().filter(|(stem, text)| is_existing(stem) && read(stem).as_deref() != Some(text.as_str())).map(|(stem, _)| stem).collect();
    let mut removed: Vec<(String, &str)> = Vec::new();
    for (stem, path) in &existing {
        if rendered.iter().any(|(known, _)| known == stem) {
            continue;
        }
        let role_id = profile_id(fs::read_to_string(path).ok().as_deref());
        if role_id.as_deref().is_some_and(|role_id| role_id.starts_with(BUILT_IN)) {
            continue;
        }
        if retired.contains(stem) || role_id.as_ref().is_some_and(|role_id| retired.contains(role_id)) {
            removed.push((stem.clone(), "retired"));
        } else if !(keep.contains(stem) || keep.contains(&format!("{stem}.md"))) {
            removed.push((stem.clone(), "undeclared"));
        }
    }
    if added.is_empty() && changed.is_empty() && removed.is_empty() {
        writeln!(io.out, "profiles unchanged")?;
        return Ok(0);
    }
    for stem in &added {
        writeln!(io.out, "added {stem}")?;
    }
    for stem in &changed {
        writeln!(io.out, "changed {stem}")?;
    }
    for (stem, reason) in &removed {
        writeln!(io.out, "removed {stem} ({reason})")?;
    }
    if args.check {
        return Ok(0);
    }
    // Each profile is one write group, and the guard runs before every group.
    let groups: Vec<(String, bool)> = added.iter().chain(&changed).map(|stem| (stem.to_string(), true)).chain(removed.iter().map(|(stem, _)| (stem.clone(), false))).collect();
    let mut written: Vec<String> = Vec::new();
    for (index, (stem, write)) in groups.iter().enumerate() {
        if let Some(running) = gold_band::running(&data, env, processes) {
            let pending: Vec<String> = groups[index..].iter().map(|(each, _)| format!("{each}.md")).collect();
            return gold_band::stopped(TOOL, &running, &written, &pending, io);
        }
        let path = profiles.join(format!("{stem}.md"));
        let done = if *write {
            make_private_dir(&profiles).and_then(|()| write_profile(&path, &rendered.iter().find(|(known, _)| known == stem).expect("rendered").1))
        } else {
            fs::remove_file(&path)
        };
        if let Err(error) = done {
            writeln!(io.out, "{TOOL}: cannot converge: {error}")?;
            return Ok(3);
        }
        written.push(format!("{stem}.md"));
    }
    Ok(0)
}

/// The directory and its parents, the new ones readable by this user alone.
fn make_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
}

#[cfg(test)]
#[path = "gold_band_roles/tests.rs"]
mod tests;
