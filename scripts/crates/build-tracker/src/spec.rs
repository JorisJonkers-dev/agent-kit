//! What a `--spec` argument names, and where its state file and page live.

use crate::text::{basename, dirname, encode_uri_component, fnv_hash, is_space, is_word, join, parse_number};
use std::collections::HashMap;

pub const GITHUB_HOST: &str = "github.com";

/// A repository on a git host. The host is kept in lower case; owner and repo keep the case they were written in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryName {
    pub host: String,
    pub owner: String,
    pub repo: String,
}

pub fn repository_name(host: &str, owner: &str, repo: &str) -> RepositoryName {
    RepositoryName { host: host.to_lowercase(), owner: owner.to_string(), repo: repo.to_string() }
}

/// Whether two names are one repository: a git host ignores the case of owner and repo names.
pub fn same_repository(left: &RepositoryName, right: &RepositoryName) -> bool {
    left.host == right.host && left.owner.to_lowercase() == right.owner.to_lowercase() && left.repo.to_lowercase() == right.repo.to_lowercase()
}

/// An issue in a known repository.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueReference {
    pub repository: RepositoryName,
    pub number: u64,
}

fn is_name(segment: &str) -> bool {
    !segment.is_empty() && segment.chars().all(|c| is_word(c) || c == '.' || c == '-')
}

/// `[1-9]\d*`.
fn is_issue_number(text: &str) -> bool {
    !text.starts_with('0') && !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// Reads `owner/repo#12` or `host/owner/repo#12`.
pub fn parse_issue_reference(text: &str) -> Option<IssueReference> {
    let (head, number) = text.split_once('#')?;
    if !is_issue_number(number) {
        return None;
    }
    let segments: Vec<&str> = head.split('/').collect();
    if !segments.iter().all(|segment| is_name(segment)) {
        return None;
    }
    let (host, owner, repo) = match segments[..] {
        [owner, repo] => (GITHUB_HOST, owner, repo),
        [host, owner, repo] => (host, owner, repo),
        _ => return None,
    };
    Some(IssueReference { number: parse_number(number)?, repository: repository_name(host, owner, repo) })
}

/// An issue's or pull request's page address, and which of the two it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueUrl {
    pub reference: IssueReference,
    pub pull: bool,
}

/// Reads `https://host/owner/repo/issues/12`, or `/pull/12`.
pub fn parse_issue_url(url: &str) -> Option<IssueUrl> {
    let lower = url.to_ascii_lowercase();
    let rest = if lower.starts_with("https://") {
        &url[8..]
    } else if lower.starts_with("http://") {
        &url[7..]
    } else {
        return None;
    };
    let segment = |text: &str| -> bool { !text.is_empty() && !text.chars().any(is_space) };
    let parts: Vec<&str> = rest.splitn(6, '/').collect();
    let Some(&[host, owner, repo, kind, tail]) = parts.get(..5) else {
        return None;
    };
    let kind = kind.to_ascii_lowercase();
    if !(segment(host) && segment(owner) && segment(repo) && (kind == "issues" || kind == "pull")) {
        return None;
    }
    // `tail` is the number and whatever follows: the rest of a path, a query or a fragment.
    let after = parts.get(5).map(|more| format!("/{more}")).unwrap_or_default();
    let tail = format!("{tail}{after}");
    let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
    let (number, extra) = tail.split_at(digits);
    let extra_ok = extra.is_empty() || (extra.starts_with(['/', '?', '#']) && !extra.chars().any(is_space));
    if !is_issue_number(number) || !extra_ok {
        return None;
    }
    Some(IssueUrl { pull: kind == "pull", reference: IssueReference { number: parse_number(number)?, repository: repository_name(host, owner, repo) } })
}

/// What a `--spec` argument names, before the repository it lives in is known.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecArgument {
    Issue { number: u64, repository: Option<RepositoryName> },
    Path(String),
}

/// Reads `12`, `#12`, `owner/repo#12`, `host/owner/repo#12` or an issue URL as an issue; anything else is a path.
pub fn parse_spec_argument(argument: &str) -> SpecArgument {
    let text = crate::text::trim(argument);
    let bare = text.strip_prefix('#').unwrap_or(text);
    if let Some(number) = Some(bare).filter(|bare| is_issue_number(bare)).and_then(parse_number) {
        return SpecArgument::Issue { number, repository: None };
    }
    let issue = parse_issue_reference(text).or_else(|| parse_issue_url(text).filter(|url| !url.pull).map(|url| url.reference));
    match issue {
        Some(issue) => SpecArgument::Issue { number: issue.number, repository: Some(issue.repository) },
        None => SpecArgument::Path(argument.to_string()),
    }
}

/// `owner/repo#12`, with the host in front when it is not github.com.
pub fn issue_reference_text(repository: &RepositoryName, number: u64) -> String {
    let name = format!("{}/{}#{number}", repository.owner, repository.repo);
    if repository.host == GITHUB_HOST {
        name
    } else {
        format!("{}/{name}", repository.host)
    }
}

/// A spec whose repository is known: where its state lives and how the state names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedSpec {
    Github { repository: RepositoryName, number: u64 },
    /// `path` is the spec file's path under the repository root, with `/` separators.
    Local { repository: RepositoryName, path: String },
}

impl ResolvedSpec {
    pub fn repository(&self) -> &RepositoryName {
        match self {
            ResolvedSpec::Github { repository, .. } | ResolvedSpec::Local { repository, .. } => repository,
        }
    }
}

/// How the state file and the command output name the spec.
pub fn spec_text(spec: &ResolvedSpec) -> String {
    match spec {
        ResolvedSpec::Github { repository, number } => issue_reference_text(repository, *number),
        ResolvedSpec::Local { path, .. } => path.clone(),
    }
}

/// A repository with no remote is named after its main working tree: `local/<parent>/<name>`.
pub fn local_repository_name(main_root: &str) -> RepositoryName {
    let or_local = |name: &str| if name.is_empty() { "local".to_string() } else { name.to_string() };
    repository_name("local", &or_local(basename(dirname(main_root))), &or_local(basename(main_root)))
}

/// Where the page links a local ticket's file: the remote's file view, or the working tree on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TicketFiles {
    Remote(RepositoryName),
    Disk,
}

/// The remote's file view when the remote is on a host whose file view is known, else the working tree.
pub fn ticket_files(remote: Option<&RepositoryName>) -> TicketFiles {
    match remote {
        Some(remote) if remote.host == GITHUB_HOST => TicketFiles::Remote(remote.clone()),
        _ => TicketFiles::Disk,
    }
}

/// The address of a file under the repository root on the remote's file view; none for files only on disk,
/// since a published page must carry no local path.
pub fn file_link(files: &TicketFiles, path: &str) -> Option<String> {
    match files {
        TicketFiles::Disk => None,
        TicketFiles::Remote(repository) => {
            let encoded: Vec<String> = path.split('/').map(encode_uri_component).collect();
            Some(format!("https://{}/{}/{}/blob/HEAD/{}", repository.host, repository.owner, repository.repo, encoded.join("/")))
        }
    }
}

/// A local spec path as one directory name: lower case, every run of other characters one `-`, then a short hash of
/// the exact path, so two paths that read alike never share a state.
pub fn spec_slug(path: &str) -> String {
    let mut readable = String::new();
    for c in path.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            readable.push(c);
        } else if !readable.ends_with('-') {
            readable.push('-');
        }
    }
    format!("{}-{}", readable.trim_matches('-'), fnv_hash(path))
}

/// Where every state lives: `$XDG_STATE_HOME/agent-kit/build-tracker`, or `~/.local/state/agent-kit/build-tracker` when
/// XDG_STATE_HOME is unset or not absolute. Without an absolute HOME either there is none.
pub fn state_base(env: &HashMap<String, String>) -> Option<String> {
    let absolute = |name: &str| env.get(name).filter(|value| value.starts_with('/'));
    match (absolute("XDG_STATE_HOME"), absolute("HOME")) {
        (Some(xdg), _) => Some(join(&[xdg, "agent-kit", "build-tracker"])),
        (None, Some(home)) => Some(join(&[home, ".local", "state", "agent-kit", "build-tracker"])),
        (None, None) => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateLocation {
    pub state: String,
    pub page: String,
}

/// `<base>/<host>/<owner>/<repo>/<spec>/state.json`, with `page.html` beside it; owner and repo in lower case.
pub fn state_location(base: &str, spec: &ResolvedSpec) -> StateLocation {
    let repository = spec.repository();
    let name = match spec {
        ResolvedSpec::Github { number, .. } => number.to_string(),
        ResolvedSpec::Local { path, .. } => spec_slug(path),
    };
    let dir = join(&[base, &repository.host, &repository.owner.to_lowercase(), &repository.repo.to_lowercase(), &name]);
    StateLocation { page: format!("{dir}/page.html"), state: format!("{dir}/state.json") }
}
