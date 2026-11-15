//! Fail a marketplace pin bump that adds anything able to run code.
//!
//! For every marketplace whose pinned commit changed, each plugin the registry
//! installs from it is read at the old and the new commit. A hook, an MCP or LSP
//! server, or a script that only the new commit has fails the check, and so does
//! a plugin that can no longer be read. Edits to what was already there pass, so
//! a bump that adds nothing automerges and one that adds something waits for a
//! person. The upstream code is only read, never run.

use crate::json::{self, Json};
use crate::{option_value, Io};
use serde_yaml::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const MANIFEST: &str = ".claude-plugin/marketplace.json";
const PLUGIN_MANIFEST: &str = ".claude-plugin/plugin.json";
const SCRIPT_SUFFIXES: [&str; 20] = [
    ".sh", ".bash", ".zsh", ".fish", ".py", ".js", ".mjs", ".cjs", ".ts", ".mts", ".cts", ".rb",
    ".pl", ".php", ".ps1", ".bat", ".cmd", ".exe", ".jar", ".wasm",
];
/// Manifest fields known to carry nothing that runs, or to be read below. Any other is a finding.
const KNOWN_FIELDS: [&str; 25] = [
    "$schema",
    "name",
    "version",
    "description",
    "author",
    "homepage",
    "repository",
    "license",
    "keywords",
    "category",
    "tags",
    "strict",
    "source",
    "hooks",
    "mcpServers",
    "lspServers",
    "commands",
    "agents",
    "skills",
    "outputStyles",
    "displayName",
    "documentationUrl",
    "supportUrl",
    "privacyPolicyUrl",
    "termsOfServiceUrl",
];
const OFFICIAL: &str = "claude-plugins-official";
const COSMETIC_HOOK_FIELDS: [&str; 2] = ["timeout", "statusMessage"];
const SCRIPTS_SHOWN: usize = 10;
const GITHUB: &str = "https://github.com/";

type Checked<T> = Result<T, String>;

fn trunc(text: &str, len: usize) -> String {
    text.chars().take(len).collect()
}

/// What a plugin can run, keyed by identity, with a label for the report.
#[derive(Default)]
pub struct Surface {
    pub items: Vec<(String, String)>,
}

impl Surface {
    fn add(&mut self, key: String, label: String) {
        match self.items.iter_mut().find(|(seen, _)| *seen == key) {
            Some(item) => item.1 = label,
            None => self.items.push((key, label)),
        }
    }

    fn has(&self, key: &str) -> bool {
        self.items.iter().any(|(seen, _)| seen == key)
    }
}

fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `[A-Za-z0-9][\w.-]*/[\w.-]+`
fn is_slug(text: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || "_.-".contains(c);
    text.split_once('/').is_some_and(|(owner, name)| {
        owner
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && owner.chars().all(word)
            && !name.is_empty()
            && name.chars().all(word)
    })
}

/// `https://[^\s'"\\]+`
fn is_https(text: &str) -> bool {
    text.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty()
            && !rest
                .chars()
                .any(|c| c.is_whitespace() || "'\"\\".contains(c))
    })
}

/// Fetches upstream repositories at pinned commits, once each.
pub struct Fetcher {
    git_base: String,
    root: PathBuf,
    cache: Vec<((String, String), PathBuf)>,
}

impl Fetcher {
    pub fn new(git_base: &str, root: &Path) -> Fetcher {
        Fetcher {
            git_base: git_base.trim_end_matches('/').to_string(),
            root: root.to_path_buf(),
            cache: Vec::new(),
        }
    }

    /// Only an https URL or an owner/repo slug is fetched: upstream writes these.
    pub(crate) fn url(&self, repo_or_url: &str) -> Checked<String> {
        let slug = repo_or_url.strip_prefix(GITHUB).unwrap_or(repo_or_url);
        if is_slug(slug) {
            return Ok(format!(
                "{}/{}",
                self.git_base,
                slug.strip_suffix(".git").unwrap_or(slug)
            ));
        }
        if is_https(repo_or_url) {
            return Ok(repo_or_url.to_string());
        }
        Err(format!(
            "refuses to fetch {repo_or_url:?}: only https URLs and owner/repo are read"
        ))
    }

    pub fn checkout(&mut self, repo_or_url: &str, commit: &str) -> Checked<PathBuf> {
        let url = self.url(repo_or_url)?;
        if !is_sha(commit) {
            return Err(format!(
                "refuses to fetch {url} at {commit:?}: not a full commit sha"
            ));
        }
        let key = (url.clone(), commit.to_string());
        if let Some((_, dest)) = self.cache.iter().find(|(seen, _)| *seen == key) {
            return Ok(dest.clone());
        }
        let dest = self.root.join(self.cache.len().to_string());
        let place = dest.to_string_lossy().into_owned();
        let failed = format!("could not fetch {url} at {}", trunc(commit, 12));
        fs::create_dir_all(&dest).map_err(|error| format!("{failed}: {error}"))?;
        let git = [
            "-c",
            "core.symlinks=false",
            "-c",
            "advice.detachedHead=false",
            "-C",
            &place,
        ];
        let steps: [Vec<&str>; 3] = [
            vec!["init", "-q", &place],
            [
                &git[..],
                &["fetch", "-q", "--depth", "1", "--", &url, commit],
            ]
            .concat(),
            [&git[..], &["checkout", "-q", "FETCH_HEAD"]].concat(),
        ];
        for step in steps {
            let output = Command::new("git")
                .args(step)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output();
            if !output.is_ok_and(|output| output.status.success()) {
                return Err(failed);
            }
        }
        self.cache.push((key, dest.clone()));
        Ok(dest)
    }
}

/// The path with `.` and `..` folded away, links followed when it exists.
fn resolve(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| {
        path.components().fold(PathBuf::new(), |mut folded, part| {
            match part {
                Component::ParentDir => {
                    folded.pop();
                }
                other => folded.push(other),
            }
            folded
        })
    })
}

/// Parse strictly: a file Claude Code might read differently is a finding, not an empty config.
fn json_file(path: &Path, label: &str) -> Checked<Json> {
    let read = fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|text| json::parse(&text));
    read.map_err(|error| format!("{label} cannot be read: {error}"))
}

fn inside(root: &Path, rel: &str, label: &str) -> Checked<PathBuf> {
    let path = resolve(&root.join(rel));
    path.starts_with(resolve(root))
        .then_some(path)
        .ok_or(format!("{label}: {rel} leaves the plugin"))
}

/// A manifest field that is inline, a path, or a list of paths; plus its default file.
fn configs(
    root: &Path,
    value: Option<&Json>,
    default: Option<&str>,
    label: &str,
) -> Checked<Vec<Json>> {
    let items: Vec<&Json> = match value {
        None | Some(Json::Null) => vec![],
        Some(Json::Arr(items)) => items.iter().collect(),
        Some(single) => vec![single],
    };
    let mut found = Vec::new();
    for item in items {
        found.push(match item {
            Json::Str(rel) => json_file(&inside(root, rel, label)?, &format!("{label} {rel}"))?,
            inline => inline.clone(),
        });
    }
    if let Some(file) = default.filter(|file| root.join(file).is_file()) {
        found.push(json_file(
            &inside(root, file, label)?,
            &format!("{label} {file}"),
        )?);
    }
    if found.iter().all(|item| matches!(item, Json::Obj(_))) {
        Ok(found)
    } else {
        Err(format!("{label} cannot be read: not an object"))
    }
}

fn objects(value: &Json) -> Option<&[(String, Json)]> {
    match value {
        Json::Obj(entries) => Some(entries),
        _ => None,
    }
}

fn list_of_objects(value: &Json) -> Option<&[Json]> {
    match value {
        Json::Arr(items) if items.iter().all(|item| matches!(item, Json::Obj(_))) => Some(items),
        _ => None,
    }
}

fn hooks(surface: &mut Surface, config: &Json, label: &str) -> Checked<()> {
    let bad = format!("{label} hooks cannot be read: unexpected shape");
    let events = objects(config.get("hooks").unwrap_or(config)).ok_or(bad.clone())?;
    for (event, groups) in events {
        for group in list_of_objects(groups).ok_or(bad.clone())? {
            let matcher = group
                .get("matcher")
                .filter(|m| m.truthy())
                .map(Json::show)
                .unwrap_or_default();
            let listed = group.get("hooks").filter(|h| h.truthy());
            for hook in listed
                .map_or(Some(&[][..]), list_of_objects)
                .ok_or(bad.clone())?
            {
                let mut action = vec![
                    ("event".to_string(), Json::Str(event.clone())),
                    ("matcher".to_string(), Json::Str(matcher.clone())),
                ];
                for (key, value) in objects(hook)
                    .unwrap_or_default()
                    .iter()
                    .filter(|(key, _)| !COSMETIC_HOOK_FIELDS.contains(&key.as_str()))
                {
                    match action.iter_mut().find(|(seen, _)| seen == key) {
                        Some(slot) => slot.1 = value.clone(),
                        None => action.push((key.clone(), value.clone())),
                    }
                }
                let what = ["command", "url", "prompt"]
                    .iter()
                    .filter_map(|key| hook.get(key))
                    .find(|v| v.truthy())
                    .map(Json::show)
                    .unwrap_or_default();
                let spaced = if matcher.is_empty() {
                    String::new()
                } else {
                    format!(" {matcher}")
                };
                surface.add(
                    format!("hook:{}", Json::Obj(action).canonical()),
                    format!("hook {event}{spaced}: {}", trunc(&what, 160)),
                );
            }
        }
    }
    Ok(())
}

fn servers(
    surface: &mut Surface,
    kind: &str,
    config: &Json,
    wrapper: &str,
    label: &str,
) -> Checked<()> {
    let listed = objects(config.get(wrapper).unwrap_or(config))
        .filter(|entries| entries.iter().all(|(_, spec)| matches!(spec, Json::Obj(_))));
    for (name, spec) in listed.ok_or(format!(
        "{label} {kind} servers cannot be read: unexpected shape"
    ))? {
        let truthy = |key: &str| spec.get(key).filter(|v| v.truthy());
        let args = match truthy("args") {
            Some(Json::Arr(items)) => items.iter().map(Json::show).collect(),
            _ => vec![],
        };
        let command = [truthy("command").map(Json::show).unwrap_or_default()]
            .into_iter()
            .chain(args)
            .collect::<Vec<_>>()
            .join(" ");
        let target = truthy("url").map_or(command, Json::show);
        surface.add(
            format!("{kind}:{name}:{}", spec.canonical()),
            format!("{kind} server {name}: {}", trunc(&target, 160)),
        );
    }
    Ok(())
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let kind = entry.file_type();
        if kind.as_ref().is_ok_and(|kind| kind.is_dir()) {
            if entry.file_name() != ".git" {
                walk(&path, found);
            }
        } else if path.is_file() {
            found.push(path);
        }
    }
}

/// `!`cmd`` lines in markdown, which run when a command or skill is used, in order, once each.
fn shell_lines(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let (mut found, mut i): (Vec<String>, usize) = (Vec::new(), 0);
    while i + 1 < chars.len() {
        let close = (chars[i] == '!' && chars[i + 1] == '`')
            .then(|| chars[i + 2..].iter().position(|c| *c == '`' || *c == '\n'))
            .flatten()
            .filter(|len| *len > 0 && chars[i + 2 + len] == '`');
        match close {
            Some(len) => {
                let line: String = chars[i + 2..i + 2 + len].iter().collect();
                if !found.contains(&line) {
                    found.push(line);
                }
                i += len + 3;
            }
            None => i += 1,
        }
    }
    found
}

/// Script files, and `!` shell lines in markdown, which run when a command or skill is used.
pub(crate) fn scripts_in(folder: &Path, root: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    walk(folder, &mut files);
    files.sort();
    let mut found = Vec::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap_or(&path);
        if rel.components().any(|part| part.as_os_str() == ".git") {
            continue;
        }
        let shown = rel.to_string_lossy().replace('\\', "/");
        let suffix = path
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        let executable =
            fs::metadata(&path).is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0);
        let in_bin = rel
            .parent()
            .is_some_and(|parent| parent.components().any(|part| part.as_os_str() == "bin"));
        if SCRIPT_SUFFIXES.contains(&suffix.as_str()) || executable || in_bin {
            found.push((format!("script:{shown}"), format!("script {shown}")));
        } else if suffix == ".md" {
            for line in shell_lines(&String::from_utf8_lossy(
                &fs::read(&path).unwrap_or_default(),
            )) {
                found.push((
                    format!("shell:{shown}:{line}"),
                    format!("shell line {shown}: {}", trunc(&line, 160)),
                ));
            }
        }
    }
    found
}

fn fields(surface: &mut Surface, manifest: &Json, label: &str) {
    for (key, value) in objects(manifest).unwrap_or_default() {
        if !KNOWN_FIELDS.contains(&key.as_str()) {
            surface.add(
                format!("field:{label}:{key}:{}", value.canonical()),
                format!("manifest field {key}"),
            );
        }
    }
}

pub fn surface_of(root: &Path, entry: &Json) -> Checked<Surface> {
    let mut surface = Surface::default();
    let name = entry.get("name").map_or("None".to_string(), Json::show);
    let manifest = match root.join(PLUGIN_MANIFEST).is_file() {
        true => json_file(
            &root.join(PLUGIN_MANIFEST),
            &format!("{name}: {PLUGIN_MANIFEST}"),
        )?,
        false => Json::Obj(vec![]),
    };
    if !matches!(manifest, Json::Obj(_)) {
        return Err(format!(
            "{name}: {PLUGIN_MANIFEST} cannot be read: not an object"
        ));
    }
    fields(&mut surface, &manifest, "plugin");
    fields(&mut surface, entry, "entry");
    for (where_, source, defaults) in [
        (format!("{name}: plugin.json"), &manifest, true),
        (format!("{name}: marketplace entry"), entry, false),
    ] {
        let default = |file: &'static str| defaults.then_some(file);
        for config in configs(
            root,
            source.get("hooks"),
            default("hooks/hooks.json"),
            &where_,
        )? {
            hooks(&mut surface, &config, &where_)?;
        }
        for config in configs(
            root,
            source.get("mcpServers"),
            default(".mcp.json"),
            &where_,
        )? {
            servers(&mut surface, "MCP", &config, "mcpServers", &where_)?;
        }
        for config in configs(
            root,
            source.get("lspServers"),
            default(".lsp.json"),
            &where_,
        )? {
            servers(&mut surface, "LSP", &config, "lspServers", &where_)?;
        }
    }
    for (key, label) in scripts_in(root, root) {
        surface.add(key, label);
    }
    Ok(surface)
}

fn plugin_root(
    fetcher: &mut Fetcher,
    checkout: &Path,
    manifest: &Json,
    name: &str,
) -> Checked<(PathBuf, Json)> {
    let listed = manifest
        .get("plugins")
        .map(|plugins| list_of_objects(plugins).unwrap_or_default())
        .unwrap_or_default();
    let entry = listed
        .iter()
        .find(|plugin| plugin.get("name").and_then(Json::as_str) == Some(name))
        .ok_or(format!("{name}: not listed in the marketplace"))?;
    let source = entry.get("source").unwrap_or(&Json::Null);
    let field = |key: &str| {
        source
            .get(key)
            .filter(|v| v.truthy())
            .map(Json::show)
            .unwrap_or_default()
    };
    if let Json::Str(source) = source {
        let metadata = manifest
            .get("metadata")
            .and_then(|m| m.get("pluginRoot"))
            .filter(|v| v.truthy())
            .map_or(".".to_string(), Json::show);
        let rel = if source.starts_with("./") || metadata == "." {
            source.clone()
        } else {
            format!("{metadata}/{source}")
        };
        let root = resolve(&checkout.join(rel));
        if !root.starts_with(resolve(checkout)) {
            return Err(format!(
                "{name}: its source {source} leaves the marketplace"
            ));
        }
        if !root.is_dir() {
            return Err(format!("{name}: its source {source} does not exist"));
        }
        return Ok((root, entry.clone()));
    }
    if !matches!(source, Json::Obj(_)) {
        return Err(format!("{name}: its source cannot be read"));
    }
    let (sha, url) = (
        field("sha"),
        [field("url"), field("repo")]
            .into_iter()
            .find(|v| !v.is_empty())
            .unwrap_or_default(),
    );
    if sha.is_empty() {
        return Err(format!("{name}: pins no commit, so it cannot be reviewed"));
    }
    if url.is_empty() {
        return Err(format!("{name}: its source names no repository"));
    }
    let external = resolve(&fetcher.checkout(&url, &sha)?);
    let sub = field("path");
    let root = resolve(&external.join(&sub));
    if !root.starts_with(&external) {
        return Err(format!("{name}: its path {sub} leaves its repository"));
    }
    Ok((root, entry.clone()))
}

fn surface_at(fetcher: &mut Fetcher, repo: &str, commit: &str, plugin: &str) -> Checked<Surface> {
    let checkout = fetcher.checkout(repo, commit)?;
    let at = format!("{repo} at {}", trunc(commit, 12));
    let manifest = json_file(&checkout.join(MANIFEST), &format!("{at}: {MANIFEST}"))?;
    if !matches!(manifest.get("plugins"), Some(Json::Arr(_))) {
        return Err(format!("{at}: {MANIFEST} cannot be read: no plugin list"));
    }
    let (root, entry) = plugin_root(fetcher, &checkout, &manifest, plugin)?;
    surface_of(&root, &entry)
}

/// `fnmatch` for the `*` and `?` a skill selector may carry.
fn glob_match(pattern: &[char], name: &[char]) -> bool {
    match (pattern.first(), name.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            glob_match(&pattern[1..], name) || (!name.is_empty() && glob_match(pattern, &name[1..]))
        }
        (Some(p), Some(n)) => (*p == '?' || p == n) && glob_match(&pattern[1..], &name[1..]),
        _ => false,
    }
}

fn dirs_named(dir: &Path, pattern: &[char], found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            let name: Vec<char> = entry.file_name().to_string_lossy().chars().collect();
            if glob_match(pattern, &name) {
                found.push(path.clone());
            }
            dirs_named(&path, pattern, found);
        }
    }
}

/// The directories a skill source selector publishes from.
fn skill_dirs(root: &Path, selector: &str) -> Vec<PathBuf> {
    if selector == "*" {
        return vec![root.to_path_buf()];
    }
    if selector.contains('/') {
        return vec![root.join(selector.strip_suffix("/*").unwrap_or(selector))];
    }
    let mut found = Vec::new();
    dirs_named(root, &selector.chars().collect::<Vec<_>>(), &mut found);
    found.sort();
    found
}

fn skill_scripts(fetcher: &mut Fetcher, pin: &Pin, commit: &str) -> Checked<Surface> {
    let checkout = fetcher.checkout(&pin.repo, commit)?;
    let root = if checkout.join("skills").is_dir() {
        checkout.join("skills")
    } else {
        checkout
    };
    let mut surface = Surface::default();
    for folder in skill_dirs(&root, &pin.selector) {
        if resolve(&folder).starts_with(resolve(&root)) && folder.is_dir() {
            for (key, label) in scripts_in(&folder, &root) {
                surface.add(key, label);
            }
        }
    }
    Ok(surface)
}

/// Report lines for what a bump adds; past a few scripts, only their count.
fn additions(name: &str, others: &[String], scripts: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = others
        .iter()
        .chain(scripts.iter().take(SCRIPTS_SHOWN))
        .map(|label| format!("  {name}: adds {label}"))
        .collect();
    if scripts.len() > SCRIPTS_SHOWN {
        lines.push(format!(
            "  {name}: adds {} more scripts",
            scripts.len() - SCRIPTS_SHOWN
        ));
    }
    lines
}

#[derive(Debug)]
pub struct Pin {
    pub name: String,
    pub repo: String,
    pub commit: String,
    pub selector: String,
}

/// What the registry says about marketplaces, skill sources and what is installed.
#[derive(Debug)]
pub struct Registry {
    pub marketplaces: Vec<Pin>,
    pub skill_sources: Vec<Pin>,
    pub plugins: Vec<(String, String)>,
    pub language_servers: Vec<String>,
}

fn scalar(item: &Value, key: &str) -> Option<String> {
    match item.get(key)? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn section<'a>(data: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    data.get(key)
        .and_then(Value::as_sequence)
        .into_iter()
        .flatten()
}

impl Registry {
    pub fn parse(text: &str) -> Checked<Registry> {
        let data: Value = serde_yaml::from_str(text)
            .map_err(|error| format!("the registry cannot be read: {error}"))?;
        if !data.is_mapping() {
            return Err("the registry is not a mapping".to_string());
        }
        let pins = |key: &str| -> Vec<Pin> {
            section(&data, key)
                .filter_map(|item| {
                    Some(Pin {
                        name: scalar(item, "name")?,
                        repo: scalar(item, "repo").unwrap_or_default(),
                        commit: scalar(item, "commit").unwrap_or("None".to_string()),
                        selector: scalar(item, "selector")
                            .filter(|s| !s.is_empty())
                            .unwrap_or("*".to_string()),
                    })
                })
                .collect()
        };
        Ok(Registry {
            marketplaces: pins("marketplaces"),
            skill_sources: pins("skill_sources"),
            plugins: section(&data, "plugins")
                .filter_map(|p| {
                    Some((
                        scalar(p, "name")?,
                        scalar(p, "marketplace").unwrap_or_default(),
                    ))
                })
                .collect(),
            language_servers: section(&data, "language_servers")
                .filter_map(|s| scalar(s, "plugin"))
                .collect(),
        })
    }

    fn installed(&self, market: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .plugins
            .iter()
            .filter(|(_, from)| from == market)
            .map(|(name, _)| name.clone())
            .collect();
        if market == OFFICIAL {
            names.extend(self.language_servers.iter().cloned());
        }
        names
    }
}

/// The pins that moved between the two registries, with the pin they moved from.
fn bumped<'a>(base: &[Pin], head: &'a [Pin]) -> Vec<(&'a Pin, String)> {
    head.iter()
        .filter_map(|pin| {
            Some((
                pin,
                base.iter()
                    .rev()
                    .find(|old| old.name == pin.name)?
                    .commit
                    .clone(),
            ))
        })
        .filter(|(pin, old)| *old != pin.commit)
        .collect()
}

/// The report lines and how many findings block the bump.
pub fn check(base: &Registry, head: &Registry, fetcher: &mut Fetcher) -> (Vec<String>, usize) {
    let (mut lines, mut findings) = (Vec::new(), 0);
    let (markets, skills) = (
        bumped(&base.marketplaces, &head.marketplaces),
        bumped(&base.skill_sources, &head.skill_sources),
    );
    if markets.is_empty() && skills.is_empty() {
        return (
            vec!["no marketplace pin changed, and no skill source pin".to_string()],
            0,
        );
    }
    for (pin, old) in skills {
        lines.push(format!(
            "skill source {} ({}): {} -> {}",
            pin.name,
            pin.repo,
            trunc(&old, 12),
            trunc(&pin.commit, 12)
        ));
        let new_items = match skill_scripts(fetcher, pin, &pin.commit) {
            Ok(items) => items,
            Err(error) => {
                lines.push(format!("  {error}"));
                findings += 1;
                continue;
            }
        };
        let old_items = skill_scripts(fetcher, pin, &old).unwrap_or_default();
        let added: Vec<String> = new_items
            .items
            .iter()
            .filter(|(key, _)| !old_items.has(key))
            .map(|(_, label)| label.clone())
            .collect();
        let shown = additions(&pin.name, &[], &added);
        lines.extend(if shown.is_empty() {
            vec!["  adds nothing that runs code".to_string()]
        } else {
            shown
        });
        findings += added.len();
    }
    for (market, old) in markets {
        lines.push(format!(
            "marketplace {} ({}): {} -> {}",
            market.name,
            market.repo,
            trunc(&old, 12),
            trunc(&market.commit, 12)
        ));
        let mut count = 0;
        for plugin in head.installed(&market.name) {
            let new_surface = match surface_at(fetcher, &market.repo, &market.commit, &plugin) {
                Ok(surface) => surface,
                Err(error) => {
                    lines.push(format!("  {error}"));
                    count += 1;
                    continue;
                }
            };
            let old_surface = surface_at(fetcher, &market.repo, &old, &plugin).unwrap_or_default();
            let added: Vec<&(String, String)> = new_surface
                .items
                .iter()
                .filter(|(key, _)| !old_surface.has(key))
                .collect();
            let scripts: Vec<String> = added
                .iter()
                .filter(|(key, _)| key.starts_with("script:"))
                .map(|(_, label)| label.clone())
                .collect();
            let others: BTreeSet<&String> = added
                .iter()
                .filter(|(key, _)| !key.starts_with("script:"))
                .map(|(_, label)| label)
                .collect();
            let mut ordered: Vec<String> = Vec::new();
            for (key, label) in &added {
                if !key.starts_with("script:") && !ordered.contains(label) && others.contains(label)
                {
                    ordered.push(label.clone());
                }
            }
            lines.extend(additions(&plugin, &ordered, &scripts));
            count += added.len();
        }
        if count == 0 {
            lines.push("  adds nothing that runs code".to_string());
        }
        findings += count;
    }
    (lines, findings)
}

struct Args {
    base_ref: Option<String>,
    base_registry: Option<String>,
    registry: String,
    git_base: String,
}

fn parse_args(args: &[String], default_git_base: &str) -> Result<Args, String> {
    let mut parsed = Args {
        base_ref: None,
        base_registry: None,
        registry: "registry/estate-tooling.yaml".to_string(),
        git_base: default_git_base.to_string(),
    };
    let mut i = 0;
    while i < args.len() {
        let slots: [(&str, &mut Option<String>); 2] = [
            ("--base-ref", &mut parsed.base_ref),
            ("--base-registry", &mut parsed.base_registry),
        ];
        let mut used = None;
        for (name, slot) in slots {
            if let Some((value, taken)) = option_value(args, i, name) {
                *slot = Some(value.ok_or(format!("{name} needs a value"))?.to_string());
                used = Some(taken);
            }
        }
        for (name, slot) in [
            ("--registry", &mut parsed.registry),
            ("--git-base", &mut parsed.git_base),
        ] {
            if let Some((value, taken)) = option_value(args, i, name) {
                *slot = value.ok_or(format!("{name} needs a value"))?.to_string();
                used = Some(taken);
            }
        }
        i += used.ok_or(format!("unknown argument {}", args[i]))?;
    }
    if parsed.base_ref.is_some() == parsed.base_registry.is_some() {
        return Err("exactly one of --base-ref and --base-registry is required".to_string());
    }
    Ok(parsed)
}

fn base_text(args: &Args, cwd: &Path) -> Checked<String> {
    if let Some(reference) = &args.base_ref {
        let shown = Command::new("git")
            .current_dir(cwd)
            .args(["show", &format!("{reference}:{}", args.registry)])
            .output();
        let shown = shown.map_err(|error| error.to_string())?;
        if !shown.status.success() {
            return Err(String::from_utf8_lossy(&shown.stderr).trim().to_string());
        }
        return Ok(String::from_utf8_lossy(&shown.stdout).into_owned());
    }
    let path = args.base_registry.clone().unwrap_or_default();
    fs::read_to_string(&path).map_err(|error| format!("{path}: {error}"))
}

/// A fence longer than any backtick run in upstream text, which cannot then close it.
fn fence_for(report: &str) -> String {
    let longest = report.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat(3.max(longest + 1))
}

/// What the run takes from its surroundings, so a test can set it without touching the process.
pub struct Env {
    /// `$GITHUB_STEP_SUMMARY`: the file the report is appended to.
    pub summary: Option<String>,
    /// `$AGENT_KIT_GIT_BASE`: where owner/repo slugs are fetched from.
    pub git_base: String,
    /// Where `--base-ref` is read from.
    pub cwd: PathBuf,
}

pub fn run(args: &[String], io: &mut Io) -> i32 {
    let env = Env {
        summary: std::env::var("GITHUB_STEP_SUMMARY").ok(),
        git_base: std::env::var("AGENT_KIT_GIT_BASE").unwrap_or("https://github.com".to_string()),
        cwd: PathBuf::from("."),
    };
    run_env(args, io, &env)
}

pub fn run_env(args: &[String], io: &mut Io, env: &Env) -> i32 {
    match parse_args(args, &env.git_base).and_then(|parsed| review(&parsed, io, env)) {
        Ok(code) => code,
        Err(message) => {
            io.complain(&format!("marketplace-bump: {message}"));
            1
        }
    }
}

fn review(args: &Args, io: &mut Io, env: &Env) -> Checked<i32> {
    let base = Registry::parse(&base_text(args, &env.cwd)?)?;
    let head = Registry::parse(
        &fs::read_to_string(env.cwd.join(&args.registry))
            .map_err(|error| format!("{}: {error}", args.registry))?,
    )?;
    let tmp = std::env::temp_dir().join(format!(
        "ci-helpers-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let (mut lines, findings) = check(&base, &head, &mut Fetcher::new(&args.git_base, &tmp));
    let _ = fs::remove_dir_all(&tmp);
    if findings > 0 {
        lines.push(format!(
            "{findings} finding(s): review the upstream change before merging this bump"
        ));
    }
    let report = lines.join("\n");
    io.say(&report);
    if let Some(summary) = &env.summary {
        let fence = fence_for(&report);
        let mut handle = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(summary)
            .map_err(|error| format!("{summary}: {error}"))?;
        let _ = write!(
            handle,
            "## Marketplace pin review\n\n{fence}\n{report}\n{fence}\n"
        );
    }
    Ok(i32::from(findings > 0))
}
