//! What a writing run adds to a run: the worktree it works in, the other checkouts of the repository it holds
//! to the state it found them in, the work the delegate left staged, and the caller's check run in the
//! worktree once the delegate has ended. A writing run never works in a throwaway directory and is never
//! released: the worktree and its branch are the caller's to take, and the kept list is what removes them.
//!
//! The delegate has run by the time the worktree is staged and checked, so a failure of the gateway's own (git
//! cannot stage or list, the check's output cannot be kept) is a failed run with its reason, never an error
//! that would lose the result, the output and the ledger line.

use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::kept_file::KeptStore;
use crate::launchers::DELEGATION_MARK;
use crate::ports::{ProcessCommand, ProcessResult};
use crate::run::{RunInput, RunMode, Worker};
use crate::run_command::{Delegated, RunCommand, Workplace};
use crate::text::{join, js_slice};
use crate::timestamps::to_iso;
use std::collections::BTreeMap;

/// The brief a repository keeps for a Codex writing run, in the worktree the run works in.
pub const CODEX_BRIEF_FILE: &str = "codex-brief.md";

/// How much of that brief a run carries: a longer one is cut here.
pub const CODEX_BRIEF_MAX_CHARS: f64 = 20_000.0;

/// A writing run's input, narrowed: its repository, base and check are all named.
pub struct Writing<'a> {
    pub input: &'a RunInput,
    pub repo: &'a str,
    pub base: &'a str,
    pub check: &'a str,
}

/// The input of a writing run, or `None` for a read-only one. The node runner narrows the same way: a run is
/// writing only when its mode says so and it names the repository, base and check a writing run needs.
pub fn writing(input: &RunInput) -> Option<Writing<'_>> {
    if input.mode != RunMode::Write {
        return None;
    }
    Some(Writing { input, repo: input.repo.as_deref()?, base: input.base.as_deref()?, check: input.check.as_deref()? })
}

/// One watched worktree: where it is, and what `git status --porcelain` said before the delegate ran.
pub struct WatchedWorktree {
    pub path: String,
    pub before: String,
}

/// One check command's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckReport {
    pub command: String,
    pub exit_code: i32,
    /// The file the check's output (stdout, then stderr) was written to.
    pub output_path: String,
}

impl CheckReport {
    fn to_json(&self) -> Value {
        json::object(vec![
            ("command", Value::str(&self.command)),
            ("exit_code", Value::Num(self.exit_code as f64)),
            ("output_path", Value::str(&self.output_path)),
        ])
    }
}

/// What a writing run adds to its result: the worktree it worked in, what is staged there, and how the check
/// went. The members are written in the order the node runner's result literal spreads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteReport {
    pub branch: String,
    pub worktree: String,
    /// The other checkout of the repository whose changes beside the run made it fail.
    pub foreign_worktree: Option<String>,
    /// The check the gateway ran after the delegate; absent when the delegate did not finish, so none was run.
    pub check: Option<CheckReport>,
    /// The files the delegate's work staged, before the check ran; none for a run whose delegate did not
    /// finish, and `None` (a JSON `null`) when git could not list them.
    pub staged_files: Option<Vec<String>>,
    /// What git or the file system said when the gateway could not stage the worktree or keep the check's output.
    pub worktree_error: Option<String>,
}

impl WriteReport {
    /// The report's members, in the order the node runner writes them: `branch`, `worktree`, then the
    /// `foreign_worktree` of a run a checkout beside it failed, the `check`, the `staged_files` and the
    /// `worktree_error`. A foreign change and a check never come together, so one order serves both.
    pub fn members(&self) -> Vec<(&'static str, Value)> {
        let mut members = vec![("branch", Value::str(&self.branch)), ("worktree", Value::str(&self.worktree))];
        if let Some(foreign) = &self.foreign_worktree {
            members.push(("foreign_worktree", Value::str(foreign)));
        }
        if let Some(check) = &self.check {
            members.push(("check", check.to_json()));
        }
        let staged = match &self.staged_files {
            Some(files) => Value::Arr(files.iter().map(|file| Value::str(file)).collect()),
            None => Value::Null,
        };
        members.push(("staged_files", staged));
        if let Some(error) = &self.worktree_error {
            members.push(("worktree_error", Value::str(error)));
        }
        members
    }
}

/// What a writing run's worktree came to: the reason the run carries, and the report beside it.
pub struct FinishedWrite {
    pub reason: Option<String>,
    pub report: WriteReport,
}

/// What a Codex writing run gets beyond the prompt: the git directories its sandbox may write, and the brief
/// the repository keeps for it. Every other run gets neither.
#[derive(Default)]
pub struct CodexWorker {
    pub git_writable: Option<Vec<String>>,
    pub worker: Option<Worker>,
}

/// What staging a worktree came to: the files it staged (`None` when git could not list them), and the reason
/// and words of a git that could not stage or list.
struct Staged {
    files: Option<Vec<String>>,
    reason: Option<String>,
    error: Option<String>,
}

/// What one check came to: its report, and what the file system said when the output could not be kept.
struct CheckOutcome {
    report: CheckReport,
    output_error: Option<String>,
}

impl RunCommand<'_> {
    /// Every worktree of the repository but nothing of the run's own: where each stands before the delegate
    /// runs, so the run can hold every other checkout of the repository to the state the delegate found it in.
    /// Worktrees the gateway itself keeps for its runs — everything under the repository's common directory's
    /// `model-gateway/worktrees` — are not watched: they are this gateway's own workplaces, whose changes are
    /// its own work and never a change beside the run.
    pub(crate) fn watch_others(&self, repo: &str) -> Result<Vec<WatchedWorktree>, GatewayError> {
        let worktrees = self
            .runtime
            .git
            .list_worktrees(repo)
            .map_err(|message| GatewayError::new("worktree-failed", format!("{repo}: cannot list the worktrees to watch: {message}")))?;
        let common = self.common_dir(repo)?;
        let kept = join(&[&common, "model-gateway", "worktrees"]);
        let mut watched = Vec::new();
        for path in worktrees {
            if path == kept || path.starts_with(&format!("{kept}/")) {
                continue;
            }
            let before = self
                .runtime
                .git
                .status(&path)
                .map_err(|message| GatewayError::new("worktree-failed", format!("{repo}: cannot read the status of {path}: {message}")))?;
            watched.push(WatchedWorktree { path, before });
        }
        Ok(watched)
    }

    /// A writing run's worktree: created off the caller's base, in the repository's gateway directory, on a
    /// branch of its own named after the run. It is entered in the kept list at once, so whatever the run comes
    /// to, the workflow's end can remove it; the run itself never does. A list that cannot take it leaves the
    /// worktree nobody would remove, so it and its branch are removed again, and the list's error is the run's.
    pub(crate) fn write_workplace(&self, write: &Writing, root: &str, id: &str, at: i64) -> Result<Workplace, GatewayError> {
        let dir = join(&[&self.common_dir(root)?, "model-gateway", "worktrees", id]);
        let branch = format!("gateway/{id}");
        if let Err(message) = self.runtime.git.create_worktree(root, &dir, &branch, write.base) {
            self.runtime.files.remove(&dir).map_err(GatewayError::from_fs)?;
            return Err(GatewayError::new("worktree-failed", format!("{root}: cannot create a worktree off {}: {message}", write.base)));
        }
        let entry = json::object(vec![
            ("at", Value::Str(to_iso(at))),
            ("branch", Value::str(&branch)),
            ("path", Value::str(&dir)),
            ("repo", Value::str(root)),
            ("run_id", Value::str(id)),
            ("workflow", Value::str(&write.input.workflow)),
        ]);
        if let Err(error) = (KeptStore { runtime: self.runtime }).add(&entry) {
            let _ = self.runtime.git.remove_worktree(root, &dir);
            let _ = self.runtime.git.delete_branch(root, &branch);
            return Err(error);
        }
        Ok(Workplace { dir, worktree_of: None, throwaway: false, branch: Some(branch) })
    }

    /// What a Codex writing run gets beyond the prompt: the git directories its sandbox may write, so it can
    /// commit in the worktree, and the brief the repository keeps for it. Every other run gets neither, and its
    /// prompt is the one it always got.
    pub(crate) fn codex_worker(&self, run: &Delegated) -> CodexWorker {
        let input = run.input;
        let Some(write) = writing(input) else {
            return CodexWorker::default();
        };
        if input.client != "codex" {
            return CodexWorker::default();
        }
        let Some(git_writable) = self.codex_git_writable(run.work_dir, run.repo.unwrap_or(write.repo)) else {
            return CodexWorker::default();
        };
        CodexWorker { git_writable: Some(git_writable), worker: Some(Worker { brief: self.codex_brief(run.work_dir) }) }
    }

    /// The git directories a Codex writing run may write: the worktree's own git directory, which sits outside
    /// the worktree, and the repository's shared object, ref and log directories. The common directory itself is
    /// never named, so `.git/hooks` and `.git/config` stay read-only. `None` when git cannot say where they are:
    /// the run then goes on as it did before, and the gateway stages whatever the delegate left uncommitted.
    fn codex_git_writable(&self, work_dir: &str, repo: &str) -> Option<Vec<String>> {
        let git_dir = self.runtime.git.git_dir(work_dir).ok()?;
        let common = self.common_dir(repo).ok()?;
        if git_dir == common {
            return None;
        }
        Some(vec![git_dir, join(&[&common, "objects"]), join(&[&common, "refs"]), join(&[&common, "logs"])])
    }

    /// The brief the worktree keeps for a Codex writing run, cut at the limit; `None` when it keeps none.
    fn codex_brief(&self, work_dir: &str) -> Option<String> {
        let path = join(&[work_dir, CODEX_BRIEF_FILE]);
        if !self.runtime.files.exists(&path).ok()? {
            return None;
        }
        let text = self.runtime.files.read_text(&path).ok()?;
        Some(js_slice(&text, 0.0, Some(CODEX_BRIEF_MAX_CHARS)))
    }

    /// What happens to a writing run's worktree once the delegate has ended. A delegate that finished has its
    /// work staged first, and listed, and only then gets the caller's check run in the worktree: whatever the
    /// check writes (coverage, a build, a rewritten lockfile) is not the delegate's diff, and stays an unstaged
    /// change in the worktree. A delegate that did not finish (a timeout, an exit code, output that cannot be
    /// read) leaves its worktree unstaged, with no check run. The gateway never commits, merges or pushes: the
    /// worktree and its branch are the caller's to take.
    ///
    /// A worktree of the repository the run watches is held to the state its snapshot holds: one a delegate
    /// changed, or one that cannot be read when the run ends, makes the run fail with `foreign-worktree-change`,
    /// and the name of the worktree goes out beside the result. A change of another checkout is checked before
    /// the delegate's own failure: whatever went wrong inside the run, a checkout of the repository beside it
    /// changed, and that is what the result says first.
    pub(crate) fn finish_write(&self, run: &Delegated, write: &Writing, delegate_reason: Option<&str>) -> FinishedWrite {
        let branch = run.branch.unwrap_or_default().to_string();
        let worktree = run.work_dir.to_string();
        let foreign = self.foreign_change(&run.watched);
        let staged = if delegate_reason.is_none() { Some(self.stage(run.work_dir, write.base)) } else { None };
        if let Some(foreign) = foreign {
            return FinishedWrite {
                reason: Some("foreign-worktree-change".to_string()),
                report: WriteReport {
                    branch,
                    worktree,
                    foreign_worktree: Some(foreign.path.clone()),
                    check: None,
                    staged_files: staged.as_ref().map_or(Some(Vec::new()), |staged| staged.files.clone()),
                    worktree_error: staged.and_then(|staged| staged.error),
                },
            };
        }
        let Some(staged) = staged else {
            return FinishedWrite {
                reason: delegate_reason.map(str::to_string),
                report: WriteReport { branch, worktree, foreign_worktree: None, check: None, staged_files: Some(Vec::new()), worktree_error: None },
            };
        };
        let Some(check) = self.run_check(run, write) else {
            return FinishedWrite {
                reason: Some("grant-expired".to_string()),
                report: WriteReport {
                    branch,
                    worktree,
                    foreign_worktree: None,
                    check: None,
                    staged_files: staged.files,
                    worktree_error: staged.error,
                },
            };
        };
        let failure = staged.error.clone().or_else(|| check.output_error.clone());
        let reason = staged.reason.clone().or_else(|| match (&check.output_error, check.report.exit_code) {
            (Some(_), _) => Some("check-output-failed".to_string()),
            (None, 0) => None,
            (None, _) => Some("check-failed".to_string()),
        });
        FinishedWrite {
            reason,
            report: WriteReport {
                branch,
                worktree,
                foreign_worktree: None,
                check: Some(check.report),
                staged_files: staged.files,
                worktree_error: failure,
            },
        }
    }

    /// Stages everything in the worktree and lists what that staged: the delegate's diff, taken before the check
    /// can add to it. The listing is taken against the run's base, so a change the delegate committed itself on
    /// the worktree's branch is listed too. A git that cannot stage leaves nothing staged; one that cannot list
    /// leaves the worktree staged but unlisted, which the result says with `null` rather than an empty list that
    /// would read as a delegate that changed nothing.
    fn stage(&self, work_dir: &str, base: &str) -> Staged {
        if let Err(message) = self.runtime.git.stage_all(work_dir) {
            return Staged { files: Some(Vec::new()), reason: Some("stage-failed".to_string()), error: Some(message) };
        }
        match self.runtime.git.staged_files(work_dir, Some(base)) {
            Ok(files) => Staged { files: Some(files), reason: None, error: None },
            Err(message) => Staged { files: None, reason: Some("stage-list-failed".to_string()), error: Some(message) },
        }
    }

    /// The watched worktree that is no longer as it was found: its listing changed, or it cannot be read any
    /// more. The delegate has run by now, so this never fails; a worktree that went away counts as changed.
    fn foreign_change<'a>(&self, watched: &'a [WatchedWorktree]) -> Option<&'a WatchedWorktree> {
        watched.iter().find(|one| self.runtime.git.status(&one.path).map_or(true, |now| now != one.before))
    }

    /// The caller's check, `sh -c <command>` in the worktree, held to the run's time limit and, for a run under
    /// a grant, to the time still left on its grant and workflow when the check starts (at least a second): the
    /// delegate has spent part of it. With none left the check is not started and this gives `None`. The check
    /// carries the delegation mark, like the delegate, so test code the delegate wrote cannot write a grant or
    /// delegate on. Its output is kept in the run directory, and a check whose output cannot be kept still
    /// reports its exit code; a check that cannot start has exit code 127 and the reason as its output.
    fn run_check(&self, run: &Delegated, write: &Writing) -> Option<CheckOutcome> {
        let mut timeout_ms = write.input.time_limit_s * 1000.0;
        if let Some(ends_at) = run.ends_at_ms {
            let left_ms = (ends_at - self.runtime.clock.now_ms()) as f64;
            if left_ms <= 0.0 {
                return None;
            }
            timeout_ms = timeout_ms.min(left_ms.max(1000.0));
        }
        let command = ProcessCommand {
            command: "sh".to_string(),
            args: vec!["-c".to_string(), write.check.to_string()],
            cwd: Some(run.work_dir.to_string()),
            env: BTreeMap::from([(DELEGATION_MARK.to_string(), "1".to_string())]),
            timeout_ms: Some(timeout_ms as u64),
            stdin: None,
        };
        let result = self.runtime.process.run(&command).unwrap_or_else(|message| ProcessResult {
            stdout: String::new(),
            stderr: message,
            exit_code: 127,
            timed_out: false,
        });
        let output_path = join(&[run.run_dir, "check-output.txt"]);
        let report = CheckReport { command: write.check.to_string(), exit_code: result.exit_code, output_path };
        match self.runtime.files.write_text(&report.output_path, &format!("{}{}", result.stdout, result.stderr)) {
            Ok(()) => Some(CheckOutcome { report, output_error: None }),
            Err(error) => Some(CheckOutcome { report, output_error: Some(error.message) }),
        }
    }
}
