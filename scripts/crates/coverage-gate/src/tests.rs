use super::*;
use std::cell::RefCell;

// ---- the fake host -----------------------------------------------------

/// A host whose commands, files and directories are scripted.
#[derive(Default)]
struct Fake {
    /// Answers for `git` calls, keyed by the arguments after `git`.
    git: RefCell<BTreeMap<String, Result<Output, String>>>,
    /// What the coverage tool answers; `None` is a clean run.
    tool: RefCell<Option<Result<Output, String>>>,
    /// The lcov text the coverage tool writes where its own command points.
    lcov: RefCell<Option<String>>,
    /// The files `read` can see.
    files: RefCell<BTreeMap<PathBuf, String>>,
    /// The directories `subdirs` reports.
    dirs: RefCell<BTreeMap<PathBuf, Vec<String>>>,
    /// The scratch directory, or the reason there is none.
    scratch: RefCell<Option<Result<PathBuf, String>>>,
    /// The answer of `absolute`; `None` is the identity.
    absolute: RefCell<Option<Result<PathBuf, String>>>,
    /// Every command run, with its working directory.
    ran: RefCell<Vec<(PathBuf, Vec<String>)>>,
    /// What was printed on stdout and on stderr.
    out: RefCell<String>,
    err: RefCell<String>,
}

fn output(ok: bool, stdout: &str, stderr: &str) -> Output {
    Output {
        ok,
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
    }
}

impl Fake {
    fn new() -> Fake {
        Fake::default()
    }

    /// A file `read` can see.
    fn file(&self, path: &str, text: &str) {
        self.files
            .borrow_mut()
            .insert(PathBuf::from(path), text.to_string());
    }

    /// The directories `subdirs` reports for `path`.
    fn subdirs_of(&self, path: &str, names: &[&str]) {
        self.dirs.borrow_mut().insert(
            PathBuf::from(path),
            names.iter().map(|name| name.to_string()).collect(),
        );
    }

    /// A `git` call that succeeds with `stdout`.
    fn git(&self, args: &str, stdout: &str) {
        self.git
            .borrow_mut()
            .insert(args.to_string(), Ok(output(true, stdout, "")));
    }

    /// A `git` call that exits non-zero.
    fn git_fails(&self, args: &str, stderr: &str) {
        self.git
            .borrow_mut()
            .insert(args.to_string(), Ok(output(false, "", stderr)));
    }

    /// A `git` call that cannot start.
    fn git_cannot_start(&self, args: &str, why: &str) {
        self.git
            .borrow_mut()
            .insert(args.to_string(), Err(why.to_string()));
    }

    /// The lcov text the coverage tool writes.
    fn lcov(&self, text: &str) {
        *self.lcov.borrow_mut() = Some(text.to_string());
    }

    /// The coverage tool exits non-zero with `stderr`.
    fn tool_fails(&self, stderr: &str) {
        *self.tool.borrow_mut() = Some(Ok(output(false, "", stderr)));
    }

    /// The coverage tool cannot start.
    fn tool_cannot_start(&self, why: &str) {
        *self.tool.borrow_mut() = Some(Err(why.to_string()));
    }

    /// The scratch directory cannot be made.
    fn no_scratch(&self, why: &str) {
        *self.scratch.borrow_mut() = Some(Err(why.to_string()));
    }

    /// `absolute` fails.
    fn no_absolute(&self, why: &str) {
        *self.absolute.borrow_mut() = Some(Err(why.to_string()));
    }

    /// Every command run, in order.
    fn commands(&self) -> Vec<Vec<String>> {
        self.ran
            .borrow()
            .iter()
            .map(|(_, argv)| argv.clone())
            .collect()
    }

    /// The directory the last command ran in.
    fn last_dir(&self) -> PathBuf {
        self.ran.borrow().last().expect("no command ran").0.clone()
    }
}

impl Host for Fake {
    fn run(&self, dir: &Path, argv: &[String]) -> Result<Output, String> {
        self.ran
            .borrow_mut()
            .push((dir.to_path_buf(), argv.to_vec()));
        if argv.first().map(String::as_str) == Some("git") {
            assert_eq!(argv[1..3], ["-c", "core.quotepath=off"]);
            let key = argv[3..].join(" ");
            return match self.git.borrow().get(&key).cloned() {
                Some(answer) => answer,
                None => panic!("unscripted git call: {key}"),
            };
        }
        // The coverage tool: write the report where its own command points.
        let target = argv
            .iter()
            .find_map(|arg| {
                let path = arg.strip_prefix("--cov-report=lcov:").unwrap_or(arg);
                path.ends_with(".lcov").then(|| PathBuf::from(path))
            })
            .or_else(|| {
                argv.iter().find_map(|arg| {
                    arg.strip_prefix("--coverage.reportsDirectory=")
                        .map(|dir| Path::new(dir).join("lcov.info"))
                })
            });
        if let (Some(text), Some(target)) = (self.lcov.borrow().clone(), target) {
            self.files.borrow_mut().insert(target, text);
        }
        match self.tool.borrow().clone() {
            Some(answer) => answer,
            None => Ok(output(true, "", "")),
        }
    }

    fn read(&self, path: &Path) -> Option<String> {
        self.files.borrow().get(path).cloned()
    }

    fn subdirs(&self, dir: &Path) -> Vec<String> {
        self.dirs.borrow().get(dir).cloned().unwrap_or_default()
    }

    fn scratch(&self) -> Result<PathBuf, String> {
        match self.scratch.borrow().clone() {
            Some(answer) => answer,
            None => Ok(PathBuf::from("/scratch")),
        }
    }

    fn absolute(&self, path: &Path) -> Result<PathBuf, String> {
        match self.absolute.borrow().clone() {
            Some(answer) => answer,
            None => Ok(path.to_path_buf()),
        }
    }

    fn print(&self, stderr: bool, text: &str) {
        let target = if stderr { &self.err } else { &self.out };
        target.borrow_mut().push_str(text);
    }
}

// ---- helpers -----------------------------------------------------------

fn call(host: &dyn Host, args: &[&str]) -> i32 {
    run_with(&args.iter().map(OsString::from).collect::<Vec<_>>(), host)
}

/// The gate on `/repo`, with `extra` after `--repo-dir`.
fn gate_at(host: &Fake, extra: &[&str]) -> i32 {
    let mut args = vec!["--repo-dir", "/repo"];
    args.extend_from_slice(extra);
    call(host, &args)
}

/// `git` answering that `files` changed against `base`.
fn changed_against(host: &Fake, base: &str, files: &[&str]) {
    host.git(&format!("merge-base {base} HEAD"), "base-sha\n");
    host.git(
        "diff --name-only --relative --diff-filter=ACMR base-sha",
        &files.join("\n"),
    );
    host.git("ls-files --others --exclude-standard", "");
}

/// `git` answering that `files` changed against the default base.
fn changed(host: &Fake, files: &[&str]) {
    changed_against(host, DEFAULT_BASE, files);
}

/// A repository at `/repo` with a `Cargo.toml` and `files` changed.
fn rust_repo(files: &[&str]) -> Fake {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\nresolver = \"2\"\n");
    changed(&host, files);
    host
}

// ---- a rust project ----------------------------------------------------

#[test]
fn a_rust_change_at_full_coverage_passes() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nDA:2,3\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage\n"
    );
    assert_eq!(*host.err.borrow(), "");
    let commands = host.commands();
    assert_eq!(commands.len(), 4);
    assert_eq!(
        commands[0],
        words(&[
            "git",
            "-c",
            "core.quotepath=off",
            "merge-base",
            DEFAULT_BASE,
            "HEAD"
        ])
    );
    assert_eq!(
        commands[1],
        words(&[
            "git",
            "-c",
            "core.quotepath=off",
            "diff",
            "--name-only",
            "--relative",
            "--diff-filter=ACMR",
            "base-sha"
        ])
    );
    assert_eq!(
        commands[2],
        words(&[
            "git",
            "-c",
            "core.quotepath=off",
            "ls-files",
            "--others",
            "--exclude-standard"
        ])
    );
    assert_eq!(
        commands[3],
        words(&[
            "cargo",
            "llvm-cov",
            "--lcov",
            "--output-path",
            "/scratch/rust.lcov"
        ])
    );
    assert_eq!(host.last_dir(), PathBuf::from("/repo"));
}

#[test]
fn a_rust_change_with_an_uncovered_line_fails() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nDA:7,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: src/lib.rs: uncovered lines 7\n"
    );
    assert_eq!(*host.out.borrow(), "");
}

#[test]
fn an_uncovered_file_the_change_does_not_touch_never_fails() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\nSF:src/other.rs\nDA:1,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage\n"
    );
}

#[test]
fn the_uncovered_lines_are_collapsed_into_ranges() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,0\nDA:3,0\nDA:4,0\nDA:5,0\nDA:9,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: src/lib.rs: uncovered lines 1, 3-5, 9\n"
    );
}

#[test]
fn the_base_ref_is_the_one_git_is_asked_about() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    changed_against(&host, "HEAD", &["src/lib.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &["--base", "HEAD"]), 0);
    assert_eq!(
        host.commands()[0],
        words(&[
            "git",
            "-c",
            "core.quotepath=off",
            "merge-base",
            "HEAD",
            "HEAD"
        ])
    );
}

// ---- a vitest project --------------------------------------------------

#[test]
fn a_vitest_project_in_a_subdirectory_measures_its_own_files() {
    let host = Fake::new();
    host.file(
        "/repo/council/package.json",
        "{\"devDependencies\": {\"vitest\": \"^1.0.0\"}}\n",
    );
    host.subdirs_of("/repo", &["council"]);
    changed(&host, &["council/ts/src/a.ts", "council/ts/src/b.tsx"]);
    host.lcov("SF:ts/src/a.ts\nDA:1,1\nend_of_record\nSF:ts/src/b.tsx\nDA:2,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 2 changed files at 100% line coverage\n"
    );
    let commands = host.commands();
    assert_eq!(commands.len(), 4);
    assert_eq!(
        commands[3],
        words(&[
            "npx",
            "vitest",
            "run",
            "--coverage",
            "--coverage.reporter=lcov",
            "--coverage.reportsDirectory=/scratch"
        ])
    );
    assert_eq!(host.last_dir(), PathBuf::from("/repo/council"));
}

#[test]
fn a_vitest_change_with_an_uncovered_line_fails() {
    let host = Fake::new();
    host.file(
        "/repo/council/package.json",
        "{\"devDependencies\": {\"vitest\": \"^1.0.0\"}}\n",
    );
    host.subdirs_of("/repo", &["council"]);
    changed(&host, &["council/ts/src/a.ts"]);
    host.lcov("SF:ts/src/a.ts\nDA:4,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: council/ts/src/a.ts: uncovered lines 4\n"
    );
}

// ---- a pytest-cov project ----------------------------------------------

#[test]
fn a_pytest_cov_project_measures_its_python_files() {
    let host = Fake::new();
    host.file(
        "/repo/pyproject.toml",
        "[project]\nname = \"app\"\n[project.optional-dependencies]\ntest = [\"pytest-cov\"]\n",
    );
    changed(&host, &["app/main.py"]);
    host.lcov("SF:app/main.py\nDA:3,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage\n"
    );
    assert_eq!(
        host.commands()[3],
        words(&[
            "python3",
            "-m",
            "pytest",
            "--cov",
            "--cov-report=lcov:/scratch/python.lcov"
        ])
    );
    assert_eq!(host.last_dir(), PathBuf::from("/repo"));
}

#[test]
fn a_pytest_cov_change_with_an_uncovered_line_fails() {
    let host = Fake::new();
    host.file("/repo/pyproject.toml", "requires = [\"pytest-cov\"]\n");
    changed(&host, &["app/main.py"]);
    host.lcov("SF:app/main.py\nDA:3,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: app/main.py: uncovered lines 3\n"
    );
}

#[test]
fn a_setup_cfg_naming_pytest_cov_is_a_python_project() {
    let host = Fake::new();
    host.file(
        "/repo/svc/setup.cfg",
        "[tool:pytest]\naddopts = -p pytest_cov\n",
    );
    host.subdirs_of("/repo", &["svc"]);
    changed(&host, &["svc/app.py"]);
    host.lcov("SF:app.py\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(host.last_dir(), PathBuf::from("/repo/svc"));
}

// ---- usage and configuration -------------------------------------------

#[test]
fn a_repository_with_no_coverage_tool_is_exit_2() {
    let host = Fake::new();
    host.file("/repo/package.json", "{\"dependencies\": {}}\n");
    assert_eq!(gate_at(&host, &[]), 2);
    let err = host.err.borrow();
    assert!(
        err.starts_with("coverage-gate: no coverage tool is configured in /repo: "),
        "{err}"
    );
    assert!(err.contains("cargo llvm-cov"), "{err}");
    assert!(err.contains("vitest"), "{err}");
    assert!(err.contains("pytest-cov"), "{err}");
    assert_eq!(*host.out.borrow(), "");
}

/// The gate refuses `args` with exit 2 and `why` above the usage line.
fn usage_error(args: &[&str], why: &str) {
    let host = Fake::new();
    assert_eq!(call(&host, args), 2, "{args:?}");
    let err = host.err.borrow();
    assert!(
        err.starts_with(&format!("coverage-gate: {why}\n")),
        "{args:?}: {err}"
    );
    assert!(
        err.contains("usage: coverage-gate --repo-dir DIR [--base REF] [--ignore TEXT]..."),
        "{args:?}: {err}"
    );
    assert_eq!(*host.out.borrow(), "", "{args:?}");
}

#[test]
fn bad_usage_is_exit_2_with_the_usage_line() {
    usage_error(&["--nope"], "unexpected argument --nope");
    usage_error(&["--repo-dir"], "--repo-dir needs a value");
    usage_error(&["--repo-dir", ""], "--repo-dir needs a value");
    usage_error(
        &["--repo-dir", "/repo", "--repo-dir", "/repo"],
        "--repo-dir given twice",
    );
    usage_error(
        &["--repo-dir", "/repo", "--base", "a", "--base", "b"],
        "--base given twice",
    );
    usage_error(&["--base", "main"], "--repo-dir is required");
}

// ---- the coverage tool ------------------------------------------------

#[test]
fn a_failing_coverage_tool_is_exit_1_with_the_last_of_its_stderr() {
    let host = rust_repo(&["src/lib.rs"]);
    host.tool_fails("one\ntwo\nthree\nfour\nfive\nsix\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: cargo llvm-cov failed in .: two | three | four | five | six\n"
    );
    assert_eq!(*host.out.borrow(), "");
}

#[test]
fn a_failing_tool_in_a_subdirectory_names_that_directory() {
    let host = Fake::new();
    host.file(
        "/repo/council/package.json",
        "{\"devDependencies\": {\"vitest\": \"^1.0.0\"}}\n",
    );
    host.subdirs_of("/repo", &["council"]);
    changed(&host, &["council/src/a.ts"]);
    host.tool_fails("boom\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: vitest coverage failed in council: boom\n"
    );
}

#[test]
fn a_failing_pytest_cov_run_is_named_in_the_message() {
    let host = Fake::new();
    host.file("/repo/pyproject.toml", "requires = [\"pytest-cov\"]\n");
    changed(&host, &["app/main.py"]);
    host.tool_fails("no tests ran\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: pytest-cov failed in .: no tests ran\n"
    );
}

#[test]
fn a_tool_that_writes_no_report_is_exit_1() {
    let host = rust_repo(&["src/lib.rs"]);
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: cargo llvm-cov wrote no lcov report\n"
    );
}

#[test]
fn a_tool_that_cannot_start_is_exit_1() {
    let host = rust_repo(&["src/lib.rs"]);
    host.tool_cannot_start("cannot run cargo: No such file or directory");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: cannot run cargo: No such file or directory\n"
    );
}

#[test]
fn a_scratch_directory_that_cannot_be_made_is_exit_1() {
    let host = rust_repo(&["src/lib.rs"]);
    host.no_scratch("cannot make /scratch: Permission denied");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: cannot make /scratch: Permission denied\n"
    );
}

// ---- what is measured --------------------------------------------------

#[test]
fn a_changed_source_file_the_report_does_not_list_fails() {
    let host = rust_repo(&["src/lib.rs", "src/new.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: src/new.rs: not in the coverage report, so no line of it is known to run\n"
    );
}

#[test]
fn a_main_rs_is_never_measured() {
    let host = rust_repo(&["src/lib.rs", "crates/x/src/main.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage (1 changed files not measured)\n"
    );
}

#[test]
fn ignore_leaves_a_matching_file_out_of_the_measuring() {
    let host = rust_repo(&["src/lib.rs", "src/generated.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\nSF:src/generated.rs\nDA:1,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: src/generated.rs: uncovered lines 1\n"
    );

    let host = rust_repo(&["src/lib.rs", "src/generated.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\nSF:src/generated.rs\nDA:1,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &["--ignore", "generated"]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage (1 changed files not measured)\n"
    );
}

#[test]
fn a_changed_file_of_a_language_with_no_project_is_skipped() {
    let host = rust_repo(&["src/lib.rs", "README.md", "app/main.py"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage (2 changed files not measured)\n"
    );
}

#[test]
fn a_file_listed_twice_counts_a_line_as_run_when_any_record_ran_it() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov(
        "SF:src/lib.rs\nDA:1,0\nDA:2,1\nDA:3,0\nDA:3,1\nend_of_record\n\
         SF:src/lib.rs\nDA:1,2\nend_of_record\n",
    );
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage\n"
    );
}

#[test]
fn malformed_da_lines_are_ignored() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("DA:1,1\nSF:src/lib.rs\nDA:oops,1\nDA:2,not-a-number\nDA:3\nDA:4,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage\n"
    );
}

#[test]
fn an_absolute_sf_path_inside_the_repository_is_made_relative() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:/repo/src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage\n"
    );
}

#[test]
fn an_absolute_sf_path_outside_the_repository_is_ignored() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:/elsewhere/src/lib.rs\nDA:1,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert!(host
        .err
        .borrow()
        .contains("src/lib.rs: not in the coverage report"));
}

#[test]
fn a_relative_sf_path_is_joined_to_the_project_and_normalised() {
    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:./src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);

    let host = rust_repo(&["src/lib.rs"]);
    host.lcov("SF:src\\lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage\n"
    );
}

// ---- which project owns a file -----------------------------------------

#[test]
fn the_longest_prefix_project_owns_a_file() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.file("/repo/crates/Cargo.toml", "[package]\nname = \"crates\"\n");
    host.subdirs_of("/repo", &["crates"]);
    changed(&host, &["crates/src/lib.rs"]);
    // Relative to the root project this would be src/lib.rs, which is not the
    // changed file: only the crates project can resolve it.
    host.lcov("SF:src/lib.rs\nDA:1,0\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 1);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: crates/src/lib.rs: uncovered lines 1\n"
    );
    assert_eq!(host.commands().len(), 4);
    assert_eq!(host.last_dir(), PathBuf::from("/repo/crates"));
}

#[test]
fn the_root_project_owns_a_file_no_sub_project_covers() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.file("/repo/crates/Cargo.toml", "[package]\nname = \"crates\"\n");
    host.subdirs_of("/repo", &["crates"]);
    changed(&host, &["src/lib.rs"]);
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(host.commands().len(), 4);
    assert_eq!(host.last_dir(), PathBuf::from("/repo"));
}

#[test]
fn a_project_directory_is_a_prefix_only_at_a_path_boundary() {
    let host = Fake::new();
    host.file(
        "/repo/council/Cargo.toml",
        "[package]\nname = \"council\"\n",
    );
    host.subdirs_of("/repo", &["council"]);
    changed(&host, &["councilx/src/lib.rs"]);
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 0 changed files at 100% line coverage (1 changed files not measured)\n"
    );
    // No project owns the file, so no coverage tool ran.
    assert_eq!(host.commands().len(), 3);
}

#[test]
fn hidden_node_modules_and_target_directories_are_not_scanned() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.file("/repo/crates/Cargo.toml", "[package]\nname = \"crates\"\n");
    for dir in [".hidden", "node_modules", "target"] {
        host.file(
            &format!("/repo/{dir}/package.json"),
            "{\"devDependencies\": {\"vitest\": \"^1.0.0\"}}\n",
        );
    }
    host.subdirs_of("/repo", &[".hidden", "node_modules", "target", "crates"]);
    changed(
        &host,
        &[
            "crates/src/lib.rs",
            "node_modules/pkg/a.ts",
            ".hidden/b.ts",
            "target/c.ts",
        ],
    );
    host.lcov("SF:src/lib.rs\nDA:1,1\nend_of_record\n");
    assert_eq!(gate_at(&host, &[]), 0);
    assert_eq!(
        *host.out.borrow(),
        "coverage-gate: 1 changed files at 100% line coverage (3 changed files not measured)\n"
    );
    // Three git calls and one tool run: the skipped directories have no project.
    assert_eq!(host.commands().len(), 4);
    assert_eq!(host.last_dir(), PathBuf::from("/repo/crates"));
}

// ---- a change that cannot be listed ------------------------------------

#[test]
fn a_merge_base_that_fails_is_exit_2() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.git_fails(
        "merge-base origin/main HEAD",
        "fatal: not a git repository\n",
    );
    assert_eq!(gate_at(&host, &[]), 2);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: git merge-base origin/main HEAD failed: fatal: not a git repository\n"
    );
}

#[test]
fn a_merge_base_that_prints_nothing_is_exit_2() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.git("merge-base origin/main HEAD", "\n");
    assert_eq!(gate_at(&host, &[]), 2);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: git merge-base origin/main HEAD printed nothing\n"
    );
}

#[test]
fn a_diff_that_fails_is_exit_2() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.git("merge-base origin/main HEAD", "base-sha\n");
    host.git_fails(
        "diff --name-only --relative --diff-filter=ACMR base-sha",
        "fatal: bad revision\n",
    );
    assert_eq!(gate_at(&host, &[]), 2);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: git diff --name-only --relative --diff-filter=ACMR base-sha failed: fatal: bad revision\n"
    );
}

#[test]
fn an_ls_files_that_fails_is_exit_2() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.git("merge-base origin/main HEAD", "base-sha\n");
    host.git(
        "diff --name-only --relative --diff-filter=ACMR base-sha",
        "",
    );
    host.git_fails(
        "ls-files --others --exclude-standard",
        "fatal: index unreadable\n",
    );
    assert_eq!(gate_at(&host, &[]), 2);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: git ls-files --others --exclude-standard failed: fatal: index unreadable\n"
    );
}

#[test]
fn a_git_that_cannot_start_is_exit_2() {
    let host = Fake::new();
    host.file("/repo/Cargo.toml", "[workspace]\n");
    host.git_cannot_start(
        "merge-base origin/main HEAD",
        "cannot run git: No such file",
    );
    assert_eq!(gate_at(&host, &[]), 2);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: cannot run git: No such file\n"
    );
}

#[test]
fn a_repository_that_cannot_be_resolved_is_exit_2() {
    let host = Fake::new();
    host.no_absolute("cannot resolve /repo: No such file or directory");
    assert_eq!(gate_at(&host, &[]), 2);
    assert_eq!(
        *host.err.borrow(),
        "coverage-gate: cannot resolve /repo: No such file or directory\n"
    );
}

// ---- the line list -----------------------------------------------------

#[test]
fn ranges_collapses_runs_and_cuts_the_list_at_twenty() {
    assert_eq!(ranges(&[]), "");
    assert_eq!(ranges(&[4]), "4");
    assert_eq!(ranges(&[1, 3, 4, 5, 9]), "1, 3-5, 9");
    let many: Vec<u32> = (0..25).map(|index| index * 2 + 1).collect();
    assert_eq!(
        ranges(&many),
        "1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31, 33, 35, 37, 39 and 5 more"
    );
}

// ---- the real host -----------------------------------------------------

fn dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("coverage-gate-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn the_real_host_runs_a_command_in_a_directory() {
    let d = dir("real-run");
    let out = RealHost
        .run(&d, &words(&["sh", "-c", "echo hi; echo oops >&2"]))
        .unwrap();
    assert!(out.ok);
    assert_eq!(out.stdout, "hi\n");
    assert_eq!(out.stderr, "oops\n");
    let out = RealHost.run(&d, &words(&["false"])).unwrap();
    assert!(!out.ok);
}

#[test]
fn the_real_host_reports_a_command_that_cannot_start() {
    let d = dir("real-nostart");
    let why = RealHost
        .run(&d, &words(&["/no/such/coverage-tool"]))
        .unwrap_err();
    assert!(
        why.starts_with("cannot run /no/such/coverage-tool: "),
        "{why}"
    );
}

#[test]
fn the_real_host_reads_a_file_and_reports_a_missing_one() {
    let d = dir("real-read");
    fs::write(d.join("Cargo.toml"), "[workspace]\n").unwrap();
    assert_eq!(
        RealHost.read(&d.join("Cargo.toml")).as_deref(),
        Some("[workspace]\n")
    );
    assert_eq!(RealHost.read(&d.join("absent")), None);
}

#[test]
fn the_real_host_lists_only_the_directories_inside_a_directory() {
    let d = dir("real-subdirs");
    fs::create_dir(d.join("crates")).unwrap();
    fs::write(d.join("Cargo.toml"), "").unwrap();
    assert_eq!(RealHost.subdirs(&d), vec!["crates".to_string()]);
    assert!(RealHost.subdirs(&d.join("absent")).is_empty());
}

#[test]
fn the_real_host_makes_a_scratch_directory() {
    let d = RealHost.scratch().unwrap();
    assert!(d.is_dir());
    assert!(d.starts_with(std::env::temp_dir()));
}

#[test]
fn the_real_host_resolves_a_path_and_reports_a_missing_one() {
    let d = dir("real-absolute");
    let resolved = RealHost.absolute(&d).unwrap();
    assert!(resolved.is_absolute());
    assert!(RealHost.absolute(&d.join("absent")).is_err());
}

#[test]
fn the_real_host_prints_to_both_streams() {
    RealHost.print(false, "");
    RealHost.print(true, "");
}

#[test]
fn the_public_run_uses_the_real_host() {
    assert_eq!(run(["--nope"].map(OsString::from)), 2);
}

#[test]
fn the_real_host_lists_the_files_a_change_touches() {
    let d = dir("real-git");
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&d)
            .output()
            .expect("git");
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "gate@example.invalid"]);
    git(&["config", "user.name", "Coverage Gate"]);
    fs::write(d.join("base.rs"), "fn base() {}\n").unwrap();
    git(&["add", "base.rs"]);
    git(&["-c", "commit.gpgsign=false", "commit", "-q", "-m", "base"]);
    fs::write(d.join("base.rs"), "fn base() { }\n").unwrap();
    fs::write(d.join("new.rs"), "fn new() {}\n").unwrap();
    let files = changed_files(&RealHost, &d, "HEAD").unwrap();
    assert_eq!(
        files,
        BTreeSet::from(["base.rs".to_string(), "new.rs".to_string()])
    );
}

#[test]
fn a_fresh_directory_that_cannot_be_made_is_an_error() {
    let d = dir("fresh");
    let blocker = d.join("file");
    fs::write(&blocker, "x").unwrap();
    let why = fresh_dir(&blocker.join("below")).unwrap_err();
    assert!(why.starts_with("cannot make "), "{why}");
    assert_eq!(fresh_dir(&d.join("new")).unwrap(), d.join("new"));
}

#[test]
fn projects_are_measured_rust_first_then_typescript_then_python() {
    let project = |lang, dir: &str| Project {
        lang,
        dir: dir.to_string(),
    };
    let mut projects = vec![
        project(Lang::Python, ""),
        project(Lang::TypeScript, "a"),
        project(Lang::Rust, "b"),
        project(Lang::Rust, ""),
    ];
    projects.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        projects,
        vec![
            project(Lang::Rust, ""),
            project(Lang::Rust, "b"),
            project(Lang::TypeScript, "a"),
            project(Lang::Python, ""),
        ]
    );
}
