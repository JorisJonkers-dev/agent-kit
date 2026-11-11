//! The coverage gate (#449, spec #392): the gate command Workflow B runs
//! through the gate shim to demand 100 percent line coverage on the files a
//! change touches. Gold Band has no command node, so a gate node runs
//! `gold-band-gate run ... -- coverage-gate --repo-dir DIR` and a copier
//! verifies the report that wrapper wrote.
//!
//! ```text
//! coverage-gate --repo-dir DIR [--base REF] [--ignore TEXT]...
//! ```
//!
//! The files a change touches are the added, copied, modified and renamed
//! files, and the untracked ones, between the merge base of `--base` (default
//! `origin/main`) and the working tree. The repository picks its own coverage
//! tool, looked for in `DIR` and in each of its directories one level down:
//!
//! * `Cargo.toml` is Rust, measured by `cargo llvm-cov`;
//! * a `package.json` that names `vitest` is TypeScript, measured by
//!   `npx vitest run --coverage`;
//! * a `pyproject.toml`, `pytest.ini`, `tox.ini` or `setup.cfg` that names
//!   `pytest-cov` is Python, measured by `pytest --cov`.
//!
//! Each tool is asked for an lcov report, and only for the projects with a
//! changed file of their language. A changed file passes only when every
//! instrumented line in it ran; an uncovered file that was not changed never
//! fails the gate. A changed file the report does not list is not measured
//! (a test, a document, a file `--ignore` names) and is printed as such; a
//! file `--ignore` names (a substring of its repository-relative path) is
//! left out of the measuring.
//!
//! Exit 0 is full coverage (or nothing to measure), 1 is an uncovered line or
//! a coverage tool that failed, and 2 is bad usage, a repository with no
//! coverage tool configured, or a change that cannot be listed. Each failure
//! prints one `coverage-gate: ...` line on stderr. The tools and the clock of
//! the process sit behind the [`Host`] trait: tests fake it and
//! [`RealHost`] is the real one.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The crate's name, which prefixes every message line.
pub const NAME: &str = "coverage-gate";
/// The base a change is measured against when `--base` is not given.
pub const DEFAULT_BASE: &str = "origin/main";
/// The most uncovered lines named per file before the list is cut short.
const MAX_LISTED: usize = 20;

/// What a finished command printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    /// Whether it exited zero.
    pub ok: bool,
    /// Its standard output.
    pub stdout: String,
    /// Its standard error.
    pub stderr: String,
}

/// The outside world: commands, files and the standard streams.
pub trait Host {
    /// Runs `argv` with `dir` as its working directory.
    fn run(&self, dir: &Path, argv: &[String]) -> Result<Output, String>;
    /// The text of a file, or `None` when it cannot be read.
    fn read(&self, path: &Path) -> Option<String>;
    /// The names of the directories directly inside `dir`.
    fn subdirs(&self, dir: &Path) -> Vec<String>;
    /// A new, empty directory for one coverage report.
    fn scratch(&self) -> Result<PathBuf, String>;
    /// The absolute form of `path`, symlinks resolved.
    fn absolute(&self, path: &Path) -> Result<PathBuf, String>;
    /// Prints text on stdout, or on stderr when `stderr` is set.
    fn print(&self, stderr: bool, text: &str);
}

/// The real host: child processes, the file system and the standard streams.
pub struct RealHost;

impl Host for RealHost {
    fn run(&self, dir: &Path, argv: &[String]) -> Result<Output, String> {
        let out = Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("cannot run {}: {e}", argv[0]))?;
        Ok(Output {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn read(&self, path: &Path) -> Option<String> {
        fs::read_to_string(path).ok()
    }

    fn subdirs(&self, dir: &Path) -> Vec<String> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect()
    }

    fn scratch(&self) -> Result<PathBuf, String> {
        fresh_dir(&std::env::temp_dir().join(format!("coverage-gate-{}", std::process::id())))
    }

    fn absolute(&self, path: &Path) -> Result<PathBuf, String> {
        fs::canonicalize(path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))
    }

    fn print(&self, stderr: bool, text: &str) {
        if stderr {
            eprint!("{text}");
        } else {
            print!("{text}");
        }
    }
}

/// An empty directory at `dir`, whatever was there before.
fn fresh_dir(dir: &Path) -> Result<PathBuf, String> {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    Ok(dir.to_path_buf())
}

/// The languages the gate measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Lang {
    Rust,
    TypeScript,
    Python,
}

impl Lang {
    /// The language of a source file, by extension.
    fn of(path: &str) -> Option<Lang> {
        match path.rsplit('.').next()? {
            "rs" => Some(Lang::Rust),
            "ts" | "tsx" | "mts" | "cts" => Some(Lang::TypeScript),
            "py" => Some(Lang::Python),
            _ => None,
        }
    }

    fn tool(self) -> &'static str {
        match self {
            Lang::Rust => "cargo llvm-cov",
            Lang::TypeScript => "vitest coverage",
            Lang::Python => "pytest-cov",
        }
    }

    /// The command that writes an lcov report at `lcov`.
    fn command(self, lcov: &Path) -> Vec<String> {
        let lcov = lcov.display().to_string();
        let dir = Path::new(&lcov).parent().unwrap_or(Path::new("."));
        let argv = match self {
            Lang::Rust => vec![
                "cargo".to_string(),
                "llvm-cov".to_string(),
                "--lcov".to_string(),
                "--output-path".to_string(),
                lcov.clone(),
            ],
            Lang::TypeScript => vec![
                "npx".to_string(),
                "vitest".to_string(),
                "run".to_string(),
                "--coverage".to_string(),
                "--coverage.reporter=lcov".to_string(),
                format!("--coverage.reportsDirectory={}", dir.display()),
            ],
            Lang::Python => vec![
                "python3".to_string(),
                "-m".to_string(),
                "pytest".to_string(),
                "--cov".to_string(),
                format!("--cov-report=lcov:{lcov}"),
            ],
        };
        argv
    }
}

/// One project the repository measures: a directory and its language.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Project {
    lang: Lang,
    /// Relative to the repository root; empty for the root itself.
    dir: String,
}

/// Whether a text names `pytest-cov`.
fn names_pytest_cov(text: &str) -> bool {
    text.contains("pytest-cov") || text.contains("pytest_cov")
}

/// The projects of one directory (`rel` is relative to the repository).
fn projects_in(host: &dyn Host, repo: &Path, rel: &str) -> Vec<Project> {
    let dir = repo.join(rel);
    let read = |name: &str| host.read(&dir.join(name));
    let mut found = Vec::new();
    let mut add = |lang| {
        found.push(Project {
            lang,
            dir: rel.to_string(),
        })
    };
    if read("Cargo.toml").is_some() {
        add(Lang::Rust);
    }
    if read("package.json").is_some_and(|text| text.contains("vitest")) {
        add(Lang::TypeScript);
    }
    let python = ["pyproject.toml", "pytest.ini", "tox.ini", "setup.cfg"];
    if python
        .iter()
        .any(|name| read(name).is_some_and(|text| names_pytest_cov(&text)))
    {
        add(Lang::Python);
    }
    found
}

/// Every project in the repository root and one level below it.
fn detect(host: &dyn Host, repo: &Path) -> Vec<Project> {
    let mut projects = projects_in(host, repo, "");
    let mut names = host.subdirs(repo);
    names.sort();
    for name in names {
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        projects.extend(projects_in(host, repo, &name));
    }
    projects
}

/// What the command line asked for.
struct Args {
    repo: PathBuf,
    base: String,
    ignore: Vec<String>,
}

fn usage() -> String {
    format!("usage: {NAME} --repo-dir DIR [--base REF] [--ignore TEXT]...")
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut repo = None;
    let mut base = None;
    let mut ignore = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let name = match arg.as_str() {
            "--repo-dir" | "--base" | "--ignore" => arg.as_str(),
            _ => return Err(format!("unexpected argument {arg}")),
        };
        let value = rest
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{name} needs a value"))?
            .clone();
        match name {
            "--repo-dir" if repo.is_none() => repo = Some(PathBuf::from(value)),
            "--base" if base.is_none() => base = Some(value),
            "--ignore" => ignore.push(value),
            _ => return Err(format!("{name} given twice")),
        }
    }
    Ok(Args {
        repo: repo.ok_or_else(|| "--repo-dir is required".to_string())?,
        base: base.unwrap_or_else(|| DEFAULT_BASE.to_string()),
        ignore,
    })
}

fn words(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

/// One git call in the repository; its non-empty stdout lines.
fn git(host: &dyn Host, repo: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let mut argv = words(&["git"]);
    argv.extend(words(args));
    let out = host.run(repo, &argv)?;
    if !out.ok {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            out.stderr.trim()
        ));
    }
    Ok(out
        .stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect())
}

/// The files the change touches, relative to the repository root.
fn changed_files(host: &dyn Host, repo: &Path, base: &str) -> Result<BTreeSet<String>, String> {
    let merge_base = git(host, repo, &["merge-base", base, "HEAD"])?
        .into_iter()
        .next()
        .ok_or_else(|| format!("git merge-base {base} HEAD printed nothing"))?;
    let mut files: BTreeSet<String> = git(
        host,
        repo,
        &[
            "diff",
            "--name-only",
            "--relative",
            "--diff-filter=ACMR",
            &merge_base,
        ],
    )?
    .into_iter()
    .collect();
    files.extend(git(
        host,
        repo,
        &["ls-files", "--others", "--exclude-standard"],
    )?);
    Ok(files)
}

/// The instrumented lines of each file in an lcov report, with whether each
/// ran. A file listed twice (one record per instantiation) counts a line as
/// run when any record ran it.
fn parse_lcov(text: &str) -> BTreeMap<String, BTreeMap<u32, bool>> {
    let mut files: BTreeMap<String, BTreeMap<u32, bool>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            current = Some(path.to_string());
        } else if line == "end_of_record" {
            current = None;
        } else if let (Some(file), Some(data)) = (&current, line.strip_prefix("DA:")) {
            let mut parts = data.split(',');
            let number = parts.next().and_then(|n| n.parse::<u32>().ok());
            let count = parts.next().and_then(|n| n.parse::<f64>().ok());
            if let (Some(number), Some(count)) = (number, count) {
                let ran = files
                    .entry(file.clone())
                    .or_default()
                    .entry(number)
                    .or_insert(false);
                *ran |= count > 0.0;
            }
        }
    }
    files
}

/// The path a report names, relative to the repository root; `None` for a
/// file outside the repository.
fn repo_relative(repo: &Path, project_dir: &str, path: &str) -> Option<String> {
    let relative = if Path::new(path).is_absolute() {
        Path::new(path).strip_prefix(repo).ok()?.to_path_buf()
    } else {
        Path::new(project_dir).join(path)
    };
    let text = relative.to_string_lossy().replace('\\', "/");
    Some(text.trim_start_matches("./").to_string())
}

/// `1, 3-5, 9`: sorted line numbers with runs collapsed, at most
/// [`MAX_LISTED`] entries.
fn ranges(lines: &[u32]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0;
    for index in 0..lines.len() {
        let last = index + 1 == lines.len() || lines[index + 1] != lines[index] + 1;
        if last {
            parts.push(if start == index {
                lines[index].to_string()
            } else {
                format!("{}-{}", lines[start], lines[index])
            });
            start = index + 1;
        }
    }
    let more = parts.len().saturating_sub(MAX_LISTED);
    parts.truncate(MAX_LISTED);
    let mut text = parts.join(", ");
    if more > 0 {
        text.push_str(&format!(" and {more} more"));
    }
    text
}

/// The result of measuring the changed files of one project.
#[derive(Default)]
struct Measured {
    /// Changed files that ran every instrumented line.
    covered: Vec<String>,
    /// Changed files with lines that did not run, and which lines.
    uncovered: Vec<(String, Vec<u32>)>,
    /// Changed files the report does not list.
    unlisted: Vec<String>,
}

/// Measures `files` (repository-relative) of one project; the tool's failure
/// is the error.
fn measure(
    host: &dyn Host,
    repo: &Path,
    project: &Project,
    files: &[String],
) -> Result<Measured, String> {
    let scratch = host.scratch()?;
    let lcov = scratch.join(format!("{:?}.lcov", project.lang).to_lowercase());
    let run = host.run(&repo.join(&project.dir), &project.lang.command(&lcov))?;
    if !run.ok {
        let tail: Vec<&str> = run.stderr.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "{} failed in {}: {}",
            project.lang.tool(),
            if project.dir.is_empty() {
                "."
            } else {
                &project.dir
            },
            tail.join(" | ")
        ));
    }
    // vitest names its report lcov.info inside the reports directory.
    let text = host
        .read(&lcov)
        .or_else(|| host.read(&scratch.join("lcov.info")))
        .ok_or_else(|| format!("{} wrote no lcov report", project.lang.tool()))?;
    let mut report: BTreeMap<String, BTreeMap<u32, bool>> = BTreeMap::new();
    for (path, lines) in parse_lcov(&text) {
        if let Some(path) = repo_relative(repo, &project.dir, &path) {
            for (number, ran) in lines {
                let seen = report.entry(path.clone()).or_default();
                *seen.entry(number).or_insert(false) |= ran;
            }
        }
    }
    let mut measured = Measured::default();
    for file in files {
        match report.get(file) {
            None => measured.unlisted.push(file.clone()),
            Some(lines) => {
                let missed: Vec<u32> = lines
                    .iter()
                    .filter(|(_, ran)| !**ran)
                    .map(|(number, _)| *number)
                    .collect();
                if missed.is_empty() {
                    measured.covered.push(file.clone());
                } else {
                    measured.uncovered.push((file.clone(), missed));
                }
            }
        }
    }
    Ok(measured)
}

/// The project a changed file belongs to: the one of its language whose
/// directory is the longest prefix of its path.
fn owner<'a>(projects: &'a [Project], file: &str) -> Option<&'a Project> {
    let lang = Lang::of(file)?;
    projects
        .iter()
        .filter(|p| {
            p.lang == lang
                && (p.dir.is_empty()
                    || file
                        .strip_prefix(&p.dir)
                        .is_some_and(|r| r.starts_with('/')))
        })
        .max_by_key(|p| p.dir.len())
}

/// Runs one command line against the real world and returns its exit code.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    run_with(&args.into_iter().collect::<Vec<_>>(), &RealHost)
}

/// [`run`] against any [`Host`].
pub fn run_with(args: &[OsString], host: &dyn Host) -> i32 {
    let args: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let parsed = match parse(&args) {
        Ok(parsed) => parsed,
        Err(why) => {
            host.print(true, &format!("{NAME}: {why}\n{}\n", usage()));
            return 2;
        }
    };
    match gate(host, &parsed) {
        Ok(code) => code,
        Err((code, why)) => {
            host.print(true, &format!("{NAME}: {why}\n"));
            code
        }
    }
}

/// The gate itself; the error carries the exit code and the message.
fn gate(host: &dyn Host, args: &Args) -> Result<i32, (i32, String)> {
    let repo = host.absolute(&args.repo).map_err(|why| (2, why))?;
    let projects = detect(host, &repo);
    if projects.is_empty() {
        return Err((
            2,
            format!(
                "no coverage tool is configured in {}: a Cargo.toml (cargo llvm-cov), a package.json naming vitest, or a pyproject.toml, pytest.ini, tox.ini or setup.cfg naming pytest-cov",
                repo.display()
            ),
        ));
    }
    let changed = changed_files(host, &repo, &args.base).map_err(|why| (2, why))?;
    let mut by_project: BTreeMap<Project, Vec<String>> = BTreeMap::new();
    let mut skipped: Vec<String> = Vec::new();
    for file in changed {
        let ignored = args.ignore.iter().any(|text| file.contains(text.as_str()));
        match owner(&projects, &file) {
            Some(project) if !ignored => by_project.entry(project.clone()).or_default().push(file),
            _ => skipped.push(file),
        }
    }
    let mut failed = false;
    let mut covered = 0;
    for (project, files) in &by_project {
        let measured = match measure(host, &repo, project, files) {
            Ok(measured) => measured,
            Err(why) => {
                host.print(true, &format!("{NAME}: {why}\n"));
                failed = true;
                continue;
            }
        };
        covered += measured.covered.len();
        skipped.extend(measured.unlisted);
        for (file, lines) in measured.uncovered {
            host.print(
                true,
                &format!("{NAME}: {file}: uncovered lines {}\n", ranges(&lines)),
            );
            failed = true;
        }
    }
    let note = if skipped.is_empty() {
        String::new()
    } else {
        format!(" ({} changed files not measured)", skipped.len())
    };
    if failed {
        return Ok(1);
    }
    host.print(
        false,
        &format!("{NAME}: {covered} changed files at 100% line coverage{note}\n"),
    );
    Ok(0)
}

#[cfg(test)]
mod tests;
