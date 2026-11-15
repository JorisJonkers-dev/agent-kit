//! Check every estate skill's recorded sources against GitHub.
//!
//! Each estate skill (a `skills/workstation/estate-*` folder) carries a
//! `sources.yaml` beside its `SKILL.md`, listing the docs it was checked
//! against: a `file` entry pins the git blob sha of one path on the
//! repository's default branch, a `release` entry pins the repository's latest
//! release tag. The default run asks GitHub for each source's current blob or
//! latest tag and lists the sources that moved: the blob changed, the path is
//! gone, or a newer release exists. A request that fails (an auth error, a
//! rate limit, a timeout after bounded retries) is a failure and never a moved
//! or gone source, and the run exits non-zero.
//!
//! Modes: `--validate` checks that every `sources.yaml` is well formed (CI runs
//! it); `--update SKILL` rewrites that skill's blobs, tags and dates from
//! GitHub after a person has rechecked the sources; `--sync-issue` keeps one
//! open issue, found by its title, listing the moved sources: it opens the
//! issue when drift appears, updates it while drift remains, and closes it only
//! after a successful run finds none. A failed run leaves the issue untouched.
//!
//! Only repository paths, blob shas and tags travel through `sources.yaml` and
//! the drift issue. File content is never asked for beyond GitHub's metadata
//! reply and never written anywhere.

use crate::{option_value, Io};
use serde_yaml::{Mapping, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SOURCES_FILE: &str = "sources.yaml";
const DEFAULT_ISSUE_TITLE: &str = "Skill sources drift";
/// A reply that suggests the request may succeed later; anything else fails at once.
const TRANSIENT: [&str; 5] = [
    "timeout",
    "timed out",
    "deadline exceeded",
    "connection",
    "http 5",
];

/// The GitHub CLI, as far as this check needs it.
pub trait Gh {
    fn gh(&mut self, args: &[&str]) -> Result<String, String>;
}

/// `gh` run as a subprocess, bounded-retrying transient failures.
pub struct Subprocess {
    pub program: String,
    pub timeout: Duration,
    pub retries: u32,
}

impl Default for Subprocess {
    fn default() -> Self {
        Subprocess {
            program: "gh".to_string(),
            timeout: Duration::from_secs(60),
            retries: 3,
        }
    }
}

impl Subprocess {
    /// One attempt: the output, or the reason it failed and whether it may be retried.
    fn attempt(&self, args: &[&str]) -> Result<String, (String, bool)> {
        let mut child = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| (error.to_string(), false))?;
        let readers: Vec<_> = [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn Read + Send>),
        ]
        .into_iter()
        .flatten()
        .map(|mut stream| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = stream.read_to_string(&mut text);
                text
            })
        })
        .collect();
        let started = Instant::now();
        let status = loop {
            if let Ok(Some(status)) = child.try_wait() {
                break Some(status);
            }
            if started.elapsed() >= self.timeout {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        // After a timeout the readers are left to finish by themselves: a
        // grandchild may still hold the pipes open.
        let Some(status) = status else {
            return Err(("timeout waiting for gh".to_string(), true));
        };
        let mut texts = readers
            .into_iter()
            .map(|reader| reader.join().unwrap_or_default());
        let (stdout, stderr) = (
            texts.next().unwrap_or_default(),
            texts.next().unwrap_or_default(),
        );
        if status.success() {
            return Ok(stdout);
        }
        let error = if stderr.trim().is_empty() {
            format!("gh exited {}", status.code().unwrap_or(-1))
        } else {
            stderr.trim().to_string()
        };
        Err((error.clone(), is_transient(&error)))
    }
}

impl Gh for Subprocess {
    fn gh(&mut self, args: &[&str]) -> Result<String, String> {
        let mut error = "gh made no attempt".to_string();
        for _ in 0..self.retries {
            match self.attempt(args) {
                Ok(stdout) => return Ok(stdout),
                Err((reason, retry)) => {
                    error = reason;
                    if !retry {
                        break;
                    }
                }
            }
        }
        Err(error)
    }
}

pub fn is_transient(error: &str) -> bool {
    let lower = error.to_lowercase();
    TRANSIENT.iter().any(|word| lower.contains(word))
}

fn json(reply: &str, label: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(reply).map_err(|_| format!("{label}: GitHub's reply is not JSON"))
}

/// Whether the repository itself is visible: a 404 on a path means little when it is not.
fn repo_reachable(gh: &mut dyn Gh, repo: &str) -> Result<(), String> {
    match gh.gh(&["api", &format!("repos/{repo}")]) {
        Err(error) if error.contains("HTTP 404") => Err(format!(
            "{repo}: not found or not visible to this token (HTTP 404)"
        )),
        other => other.map(|_| ()),
    }
}

fn percent_encode(path: &str) -> String {
    path.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"/_.-~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
        out
    })
}

/// The blob sha of one file on the default branch, or None when it is gone.
pub fn current_blob(gh: &mut dyn Gh, repo: &str, path: &str) -> Result<Option<String>, String> {
    let url = format!("repos/{repo}/contents/{}", percent_encode(path));
    match gh.gh(&[
        "api",
        "-H",
        "Accept: application/vnd.github.object+json",
        &url,
    ]) {
        Err(error) if error.contains("HTTP 404") => repo_reachable(gh, repo).map(|()| None),
        Err(error) => Err(error),
        Ok(reply) => match json(&reply, &format!("{repo} {path}"))?
            .get("sha")
            .and_then(|sha| sha.as_str())
        {
            Some(sha) => Ok(Some(sha.to_string())),
            None => Err(format!("{repo} {path}: GitHub's reply names no blob")),
        },
    }
}

/// The repository's latest release tag, or None when it has no release.
pub fn latest_tag(gh: &mut dyn Gh, repo: &str) -> Result<Option<String>, String> {
    match gh.gh(&["api", &format!("repos/{repo}/releases/latest")]) {
        Err(error) if error.contains("HTTP 404") => repo_reachable(gh, repo).map(|()| None),
        Err(error) => Err(error),
        Ok(reply) => match json(&reply, &format!("{repo} release"))?
            .get("tag_name")
            .and_then(|tag| tag.as_str())
        {
            Some(tag) => Ok(Some(tag.to_string())),
            None => Err(format!("{repo}: GitHub's reply names no release tag")),
        },
    }
}

fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let [year, month, day] = parts[..] else {
        return false;
    };
    if !(digits(year, 4) && digits(month, 2) && digits(day, 2)) {
        return false;
    }
    let (year, month, day): (u32, u32, u32) = (
        year.parse().unwrap_or(0),
        month.parse().unwrap_or(0),
        day.parse().unwrap_or(0),
    );
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let longest = match month {
        2 => 28 + u32::from(leap),
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    year >= 1 && (1..=12).contains(&month) && (1..=longest).contains(&day)
}

fn is_repo(text: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || "_.-".contains(c);
    text.split_once('/').is_some_and(|(owner, name)| {
        owner.chars().next().is_some_and(char::is_alphanumeric)
            && owner.chars().all(word)
            && !name.is_empty()
            && name.chars().all(word)
    })
}

/// One well-formed sources.yaml entry, keeping its raw mapping for rewrites.
pub struct Entry {
    pub kind: String,
    pub repo: String,
    pub path: String,
    pub blob: String,
    pub tag: String,
    pub raw: Mapping,
}

fn text<'a>(raw: &'a Mapping, key: &str) -> Option<&'a str> {
    raw.get(key).and_then(Value::as_str)
}

/// One entry and the problems that make it not well formed.
fn parse_entry(item: &Value) -> Result<Entry, Vec<String>> {
    let Some(raw) = item.as_mapping() else {
        return Err(vec!["entry is not a mapping".to_string()]);
    };
    let mut problems = Vec::new();
    let kind = text(raw, "kind").filter(|kind| ["file", "release"].contains(kind));
    if kind.is_none() {
        problems.push("kind must be file or release");
    }
    if !text(raw, "repo").is_some_and(is_repo) {
        problems.push("repo must be <owner>/<name>");
    }
    if kind == Some("file") {
        if !text(raw, "path").is_some_and(|path| !path.is_empty()) {
            problems.push("path must name a file in the repository");
        }
        if !text(raw, "blob").is_some_and(|blob| {
            blob.len() == 40 && blob.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        }) {
            problems.push("blob must be a 40-hex git blob sha");
        }
    }
    if kind == Some("release") && !text(raw, "tag").is_some_and(|tag| !tag.is_empty()) {
        problems.push("tag must name a release");
    }
    if !text(raw, "checked").is_some_and(is_date) {
        problems.push("checked must be an ISO date");
    }
    if !problems.is_empty() {
        return Err(problems.into_iter().map(String::from).collect());
    }
    let field = |key: &str| text(raw, key).unwrap_or_default().to_string();
    Ok(Entry {
        kind: field("kind"),
        repo: field("repo"),
        path: field("path"),
        blob: field("blob"),
        tag: field("tag"),
        raw: raw.clone(),
    })
}

fn name_of(skill: &Path) -> String {
    skill
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One skill's sources and the problems that make its sources.yaml not well formed.
pub fn parse(skill: &Path) -> (Vec<Entry>, Vec<String>) {
    let name = name_of(skill);
    let path = skill.join(SOURCES_FILE);
    if !path.is_file() {
        return (vec![], vec![format!("{name}: has no {SOURCES_FILE}")]);
    }
    let data = std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|body| serde_yaml::from_str::<Value>(&body).map_err(|error| error.to_string()));
    let data = match data {
        Ok(data) => data,
        Err(error) => {
            return (
                vec![],
                vec![format!("{name}: {SOURCES_FILE} cannot be read: {error}")],
            )
        }
    };
    let Some(sources) = data.get("sources").and_then(Value::as_sequence) else {
        return (
            vec![],
            vec![format!(
                "{name}: {SOURCES_FILE} does not hold a sources list"
            )],
        );
    };
    if sources.is_empty() {
        return (
            vec![],
            vec![format!("{name}: {SOURCES_FILE} lists no sources")],
        );
    }
    let (mut entries, mut problems) = (Vec::new(), Vec::new());
    for (number, item) in sources.iter().enumerate() {
        match parse_entry(item) {
            Ok(entry) => entries.push(entry),
            Err(found) => {
                problems.extend(found.into_iter().map(|problem| {
                    format!("{name}: {SOURCES_FILE} entry {}: {problem}", number + 1)
                }))
            }
        }
    }
    (entries, problems)
}

/// The estate skill folders under the skills root.
pub fn skills(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && name_of(path).starts_with("estate-"))
        .collect();
    found.sort();
    found
}

/// Every problem that keeps a sources.yaml from being well formed.
pub fn validate(root: &Path) -> Vec<String> {
    skills(root)
        .iter()
        .flat_map(|skill| parse(skill).1)
        .collect()
}

/// What one drift check found: moved sources and failed requests, kept apart.
#[derive(Default)]
pub struct Report {
    pub drift: Vec<String>,
    pub failures: Vec<String>,
}

/// List the sources that moved, and the requests that failed, never mixed.
pub fn check(root: &Path, gh: &mut dyn Gh) -> Report {
    let mut report = Report::default();
    for skill in skills(root) {
        for entry in parse(&skill).0 {
            let label = format!("{}: {}", name_of(&skill), entry.repo);
            if entry.kind == "file" {
                match current_blob(gh, &entry.repo, &entry.path) {
                    Err(error) => report
                        .failures
                        .push(format!("{label} {} check failed: {error}", entry.path)),
                    Ok(None) => report.drift.push(format!("{label} {} is gone", entry.path)),
                    Ok(Some(current)) if current != entry.blob => {
                        report.drift.push(format!(
                            "{label} {} moved (blob {} -> {current})",
                            entry.path, entry.blob
                        ));
                    }
                    Ok(Some(_)) => {}
                }
            } else {
                match latest_tag(gh, &entry.repo) {
                    Err(error) => report
                        .failures
                        .push(format!("{label} release check failed: {error}")),
                    Ok(None) => report
                        .drift
                        .push(format!("{label} release is gone (recorded {})", entry.tag)),
                    Ok(Some(current)) if current != entry.tag => {
                        report.drift.push(format!(
                            "{label} has a newer release {current} (recorded {})",
                            entry.tag
                        ));
                    }
                    Ok(Some(_)) => {}
                }
            }
        }
    }
    report
}

/// Rewrite one skill's blobs, tags and dates from what GitHub holds now.
pub fn update(
    root: &Path,
    name: &str,
    today: &str,
    gh: &mut dyn Gh,
) -> Result<Vec<String>, String> {
    if name.contains('/') || !name.starts_with("estate-") {
        return Err(format!("{name}: not an estate skill folder name"));
    }
    let skill = root.join(name);
    if !skill.is_dir() {
        return Err(format!("{name}: no such skill under {}", root.display()));
    }
    let (mut entries, problems) = parse(&skill);
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let mut lines = vec![format!(
        "{name}: recording what GitHub holds today ({today})"
    )];
    for entry in &mut entries {
        if entry.kind == "file" {
            let blob = current_blob(gh, &entry.repo, &entry.path)?.ok_or(format!(
                "{name}: {} {} is gone; nothing to record",
                entry.repo, entry.path
            ))?;
            entry.raw.insert("blob".into(), blob.clone().into());
            lines.push(format!("{name}: {} {} blob {blob}", entry.repo, entry.path));
        } else {
            let tag = latest_tag(gh, &entry.repo)?.ok_or(format!(
                "{name}: {} has no release; nothing to record",
                entry.repo
            ))?;
            entry.raw.insert("tag".into(), tag.clone().into());
            lines.push(format!("{name}: {} release {tag}", entry.repo));
        }
        entry.raw.insert("checked".into(), today.into());
    }
    let mut document = Mapping::new();
    document.insert(
        "sources".into(),
        Value::Sequence(
            entries
                .into_iter()
                .map(|entry| Value::Mapping(entry.raw))
                .collect(),
        ),
    );
    let body = serde_yaml::to_string(&document).unwrap_or_default();
    std::fs::write(skill.join(SOURCES_FILE), body).map_err(|error| error.to_string())?;
    Ok(lines)
}

/// The number of the one open issue with this title, if there is one.
fn find_issue(gh: &mut dyn Gh, title: &str) -> Result<Option<u64>, String> {
    let reply = gh.gh(&[
        "issue",
        "list",
        "--state",
        "open",
        "--limit",
        "1000",
        "--json",
        "number,title",
    ])?;
    let data = json(&reply, "gh issue list")?;
    let issues = data.as_array().map(Vec::as_slice).unwrap_or_default();
    Ok(issues
        .iter()
        .find(|issue| issue.get("title").and_then(|t| t.as_str()) == Some(title))
        .and_then(|issue| issue.get("number")?.as_u64()))
}

fn issue_body(drift: &[String]) -> String {
    let listing: Vec<String> = drift.iter().map(|line| format!("- {line}")).collect();
    format!(
        "Sources that moved since {SOURCES_FILE} was last updated:\n\n{}\n\nAfter rechecking a source, run `ci-helpers skill-sources --update <skill>`.\n",
        listing.join("\n")
    )
}

/// Keep one open issue in step with the drift report; say what was done.
pub fn sync_issue(gh: &mut dyn Gh, title: &str, drift: &[String]) -> Result<String, String> {
    let number = find_issue(gh, title)?;
    match (drift.is_empty(), number) {
        (false, None) => {
            gh.gh(&[
                "issue",
                "create",
                "--title",
                title,
                "--body",
                &issue_body(drift),
            ])?;
            Ok(format!(
                "opened {title:?} listing {} moved source(s)",
                drift.len()
            ))
        }
        (false, Some(number)) => {
            gh.gh(&[
                "issue",
                "edit",
                &number.to_string(),
                "--body",
                &issue_body(drift),
            ])?;
            Ok(format!(
                "updated {title:?} (#{number}) listing {} moved source(s)",
                drift.len()
            ))
        }
        (true, Some(number)) => {
            gh.gh(&["issue", "close", &number.to_string()])?;
            Ok(format!("closed {title:?} (#{number})"))
        }
        (true, None) => Ok(format!("{title:?}: no drift and no issue open")),
    }
}

/// Today's date, `YYYY-MM-DD`, in UTC.
pub fn today() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    civil((seconds / 86_400) as i64)
}

/// The proleptic Gregorian date `days` after 1970-01-01.
pub fn civil(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[derive(Default)]
struct Args {
    root: Option<String>,
    validate: bool,
    update: Option<String>,
    sync_issue: bool,
    issue_title: Option<String>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args::default();
    let mut i = 0;
    while i < args.len() {
        let used = if let Some((value, used)) = option_value(args, i, "--root") {
            parsed.root = Some(value.ok_or("--root needs a value")?.to_string());
            used
        } else if let Some((value, used)) = option_value(args, i, "--update") {
            parsed.update = Some(value.ok_or("--update needs a skill")?.to_string());
            used
        } else if let Some((value, used)) = option_value(args, i, "--issue-title") {
            parsed.issue_title = Some(value.ok_or("--issue-title needs a value")?.to_string());
            used
        } else if args[i] == "--validate" {
            parsed.validate = true;
            1
        } else if args[i] == "--sync-issue" {
            parsed.sync_issue = true;
            1
        } else {
            return Err(format!("unknown argument {}", args[i]));
        };
        i += used;
    }
    if usize::from(parsed.validate)
        + usize::from(parsed.update.is_some())
        + usize::from(parsed.sync_issue)
        > 1
    {
        return Err("--validate, --update and --sync-issue are mutually exclusive".to_string());
    }
    Ok(parsed)
}

pub fn run(args: &[String], io: &mut Io) -> i32 {
    run_with(args, io, &mut Subprocess::default(), &today())
}

pub fn run_with(args: &[String], io: &mut Io, gh: &mut dyn Gh, today: &str) -> i32 {
    match parse_args(args) {
        Err(message) => {
            io.complain(&format!("skill-sources: {message}"));
            2
        }
        Ok(parsed) => match drive(&parsed, io, gh, today) {
            Ok(code) => code,
            Err(error) => {
                io.say(&format!("check failed: {error}"));
                1
            }
        },
    }
}

fn drive(args: &Args, io: &mut Io, gh: &mut dyn Gh, today: &str) -> Result<i32, String> {
    let root = PathBuf::from(args.root.as_deref().unwrap_or("skills/workstation"));
    if !root.is_dir() {
        return Err(format!("{}: no such skills folder", root.display()));
    }
    if args.validate {
        let problems = validate(&root);
        problems.iter().for_each(|problem| io.say(problem));
        io.say(&if problems.is_empty() {
            "every sources.yaml is well formed".to_string()
        } else {
            format!("{} problem(s)", problems.len())
        });
        return Ok(i32::from(!problems.is_empty()));
    }
    if let Some(name) = &args.update {
        update(&root, name, today, gh)?
            .iter()
            .for_each(|line| io.say(line));
        return Ok(0);
    }
    let problems = validate(&root);
    if !problems.is_empty() {
        problems.iter().for_each(|problem| io.say(problem));
        return Ok(1);
    }
    let report = check(&root, gh);
    report
        .drift
        .iter()
        .chain(&report.failures)
        .for_each(|line| io.say(line));
    if !report.drift.is_empty() {
        io.say(&format!("{} source(s) moved", report.drift.len()));
    }
    if !report.failures.is_empty() {
        io.say(&format!("{} check(s) failed", report.failures.len()));
    }
    if report.drift.is_empty() && report.failures.is_empty() {
        io.say("no sources moved");
    }
    if args.sync_issue {
        if !report.failures.is_empty() {
            io.say("the drift issue is left untouched");
            return Ok(1);
        }
        io.say(&sync_issue(
            gh,
            args.issue_title.as_deref().unwrap_or(DEFAULT_ISSUE_TITLE),
            &report.drift,
        )?);
        return Ok(0);
    }
    Ok(i32::from(
        !report.drift.is_empty() || !report.failures.is_empty(),
    ))
}
