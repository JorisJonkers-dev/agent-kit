//! What every confined launch sees of git (`GitView`, ADR-0016): the git directories a worktree's `.git` file
//! points at, read-only, and the install of the git it runs; and the grants every launch starts with.

use crate::adapters::fs::SystemFiles;
use crate::client_confine::Confinement;
use crate::launchers::{base_grants, git_install, git_view, pi_grants, GitView, LaunchPaths, LaunchRequest, PiConfinement};
use crate::ports::{Git, GitRemote};
use crate::run::RunMode;

/// A git that names a worktree's git directory and common directory, and maybe its exec path.
struct Worktree {
    git_dir: Result<String, String>,
    common: Result<String, String>,
    exec_path: Option<String>,
}

impl Git for Worktree {
    fn root(&self, _path: &str) -> Result<Option<String>, String> {
        unreachable!("not asked")
    }
    fn remotes(&self, _path: &str) -> Result<Vec<GitRemote>, String> {
        unreachable!("not asked")
    }
    fn remove_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!("not asked")
    }
    fn delete_branch(&self, _repo: &str, _branch: &str) -> Result<(), String> {
        unreachable!("not asked")
    }
    fn common_dir(&self, _path: &str) -> Result<String, String> {
        self.common.clone()
    }
    fn add_worktree(&self, _repo: &str, _path: &str) -> Result<(), String> {
        unreachable!("not asked")
    }
    fn git_dir(&self, _path: &str) -> Result<String, String> {
        self.git_dir.clone()
    }
    fn create_worktree(&self, _repo: &str, _path: &str, _branch: &str, _base: &str) -> Result<(), String> {
        unreachable!("not asked")
    }
    fn stage_all(&self, _path: &str) -> Result<(), String> {
        unreachable!("not asked")
    }
    fn staged_files(&self, _path: &str, _base: Option<&str>) -> Result<Vec<String>, String> {
        unreachable!("not asked")
    }
    fn list_worktrees(&self, _repo: &str) -> Result<Vec<String>, String> {
        unreachable!("not asked")
    }
    fn status(&self, _path: &str) -> Result<String, String> {
        unreachable!("not asked")
    }
    fn exec_path(&self) -> Result<String, String> {
        self.exec_path.clone().ok_or_else(|| "no git".to_string())
    }
}

/// A scratch tree: a repository's common directory with some of what git reads, and a linked worktree.
fn scratch(name: &str) -> (String, String, String) {
    let root = std::env::temp_dir().join(format!("git-view-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let common = root.join("repo/.git");
    for dir in ["objects", "refs", "info", "worktrees/wt"] {
        std::fs::create_dir_all(common.join(dir)).expect("a git directory");
    }
    for file in ["HEAD", "config"] {
        std::fs::write(common.join(file), "x\n").expect("a git file");
    }
    let work = root.join("wt");
    std::fs::create_dir_all(&work).expect("the worktree");
    let text = |path: std::path::PathBuf| path.to_string_lossy().into_owned();
    std::fs::write(work.join(".git"), format!("gitdir: {}\n", text(common.join("worktrees/wt")))).expect("the .git file");
    (text(root), text(common), text(work))
}

#[test]
fn a_git_install_is_its_prefix_unless_the_system_runtime_holds_it() {
    assert_eq!(git_install("/Library/Developer/CommandLineTools/usr/libexec/git-core\n"), Some("/Library/Developer/CommandLineTools".to_string()));
    assert_eq!(git_install("/Applications/Xcode.app/Contents/Developer/usr/libexec/git-core"), Some("/Applications/Xcode.app/Contents/Developer".to_string()));
    assert_eq!(git_install("/opt/homebrew/Cellar/git/2.47.0/libexec/git-core"), Some("/opt/homebrew/Cellar/git/2.47.0".to_string()));
    for held in ["/usr/libexec/git-core", "/usr/local/libexec/git-core", "/System/x/libexec/git-core", "/usr/lib/git-core", "relative/libexec/git-core", "/libexec/git-core"] {
        assert_eq!(git_install(held), None, "{held}");
    }
}

#[test]
fn a_linked_worktree_sees_its_git_directory_and_what_git_reads_of_the_common_one() {
    let (root, common, work) = scratch("linked");
    let files = SystemFiles { cwd: root.clone() };
    let git = Worktree { git_dir: Ok(format!("{common}/worktrees/wt")), common: Ok(common.clone()), exec_path: Some("/Library/Developer/CommandLineTools/usr/libexec/git-core".to_string()) };
    let view = git_view(&git, &files, &work, &[]);
    let expected: Vec<String> = ["worktrees/wt", "HEAD", "config", "objects", "refs", "info"].iter().map(|part| format!("{common}/{part}")).collect();
    assert_eq!(view, GitView { reads: expected, run: Some("/Library/Developer/CommandLineTools".to_string()) });
    // What a writing run may write already is not granted twice, and the common directory itself never is.
    let writable = vec![format!("{common}/worktrees/wt"), format!("{common}/objects"), format!("{common}/refs")];
    let narrowed = git_view(&git, &files, &work, &writable);
    assert_eq!(narrowed.reads, ["HEAD", "config", "info"].iter().map(|part| format!("{common}/{part}")).collect::<Vec<_>>());
    assert!(!narrowed.reads.contains(&common));
    std::fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn a_repository_an_empty_directory_or_a_git_that_cannot_say_adds_no_read() {
    let (root, common, work) = scratch("plain");
    let files = SystemFiles { cwd: root.clone() };
    let silent = Worktree { git_dir: Err("not asked".into()), common: Err("not asked".into()), exec_path: None };
    // The repository's own `.git` is a directory inside the working directory: git is not even asked.
    assert_eq!(git_view(&silent, &files, &format!("{root}/repo"), &[]), GitView::default());
    // An empty directory has no `.git`.
    assert_eq!(git_view(&silent, &files, &root, &[]), GitView::default());
    // A worktree git cannot place, or whose git directory is inside it.
    assert_eq!(git_view(&silent, &files, &work, &[]), GitView::default());
    let inside = Worktree { git_dir: Ok(format!("{work}/.git")), common: Ok(common.clone()), exec_path: Some("/usr/libexec/git-core".into()) };
    assert_eq!(git_view(&inside, &files, &work, &[]), GitView::default());
    std::fs::write(format!("{work}/.git"), "not a link\n").expect("the .git file");
    let linked = Worktree { git_dir: Ok(format!("{common}/worktrees/wt")), common: Ok(common), exec_path: None };
    assert_eq!(git_view(&linked, &files, &work, &[]), GitView::default());
    std::fs::remove_dir_all(&root).expect("cleanup");
}

fn request(mode: Option<RunMode>) -> LaunchRequest {
    LaunchRequest { model: "m".into(), time_limit_s: 1.0, max_price_usd: None, mode, provider: None }
}

#[test]
fn every_confined_launch_starts_from_the_same_grants_with_the_git_it_sees() {
    let view = GitView { reads: vec!["/repo/.git/worktrees/w".into(), "/repo/.git/HEAD".into()], run: Some("/clt".into()) };
    let paths = LaunchPaths { export_dir: "/e".into(), prompt_path: "/p".into(), run_dir: None, work_dir: "/w".into(), git_writable: None, git_view: view };
    let base = base_grants(&request(Some(RunMode::ReadOnly)), &paths, "tool");
    assert_eq!(base, ["--read", "/w", "--read", "/repo/.git/worktrees/w", "--read", "/repo/.git/HEAD", "--run", "/clt", "--run-from", "tool"]);
    // Pi and every other client begin with exactly these.
    let confine = PiConfinement { launcher: "l".into(), agent_dir: "/a".into(), bin: None, port: 8, path: None, skills: Vec::new(), extensions: Vec::new() };
    assert_eq!(pi_grants(&request(None), &paths, "tool", &confine)[..base.len()], base[..]);
    let client = Confinement { launcher: "l".into(), ..Confinement::default() }.args(&request(None), &paths, "tool", &[]);
    assert_eq!(client[1..=base.len()], base[..]);
    let writing = base_grants(&request(Some(RunMode::Write)), &LaunchPaths { git_writable: Some(vec!["/g".into()]), ..paths.clone() }, "tool");
    assert_eq!(writing[..4], ["--write", "/w", "--write", "/g"]);
}
