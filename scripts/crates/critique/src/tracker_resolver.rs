//! One repository's issue tracker: found from its issue-tracker doc, or from its remote when there is none. Prepare
//! reads the artifact under critique through it, and triage reads a ticket's labels, so every caller finds specs the
//! way the node runner's `application/tracker.ts` does.

use crate::error::CritiqueError;
use crate::gh::{GhIssue, IssueLocator, IssueRequest};
use crate::paths::{inside_repository, join};
use crate::repository::{refusal, Resolved};
use crate::runtime::Runtime;
use crate::tracker::{default_tracker, parse_issue_reference, render_issue, tracker_from_doc, IssueComment, IssueReference, IssueText, TrackerChoice, TRACKER_DOC};
use model_gateway::json::{number_text, Value};
use model_gateway::text::{basename, js_trim};

/// An artifact as its tracker holds it, read once so the packet can freeze it.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackedArtifact {
    pub tracker: &'static str,
    /// Where the artifact lives: `host/owner/repo#<number>` for an issue, its repository path for a file.
    pub reference: String,
    /// Its file name under the packet's `artifact/`.
    pub name: String,
    pub text: String,
    /// The issue's labels; a markdown file has none.
    pub labels: Vec<String>,
}

/// What reading an artifact came to: the artifact, or the refusal its tracker gives when it cannot be read.
pub enum ArtifactRead {
    Read(TrackedArtifact),
    Refused(Value),
}

/// One repository's issue tracker.
pub enum Tracker<'a> {
    Github(&'a Resolved),
    LocalMarkdown(&'a Resolved),
}

/// What resolving a tracker came to: the tracker, or the refusal for one the critique cannot read.
pub enum TrackerResolution<'a> {
    Resolved(Tracker<'a>),
    Refused(Value),
}

/// Finds a repository's tracker from its issue-tracker doc, or from its remote when there is none.
pub fn resolve_tracker<'a>(runtime: &Runtime, repository: &'a Resolved) -> Result<TrackerResolution<'a>, CritiqueError> {
    let doc = join(&[&repository.root, TRACKER_DOC]);
    let choice = if runtime.files.exists(&doc).unwrap_or(false) {
        tracker_from_doc(&runtime.files.read_text(&doc).map_err(CritiqueError::from_fs)?)
    } else {
        TrackerChoice::Known { kind: default_tracker(&repository.key) }
    };
    Ok(match choice {
        TrackerChoice::Unsupported { name } => TrackerResolution::Refused(refusal(
            "unsupported-tracker",
            &format!("{TRACKER_DOC} names {name}; the critique reads specs from GitHub issues or local markdown only"),
        )),
        TrackerChoice::Known { kind: "github" } => TrackerResolution::Resolved(Tracker::Github(repository)),
        TrackerChoice::Known { .. } => TrackerResolution::Resolved(Tracker::LocalMarkdown(repository)),
    })
}

impl Tracker<'_> {
    /// The artifact a reference names (`#167`, an issue URL, or a repository path), with an issue's comments frozen
    /// with its body when `comments` says so; or the refusal the tracker gives when it cannot be read.
    pub fn artifact(&self, runtime: &Runtime, reference: &str, comments: bool) -> Result<ArtifactRead, CritiqueError> {
        match self {
            Tracker::Github(repository) => github_artifact(runtime, repository, reference, comments),
            Tracker::LocalMarkdown(repository) => local_artifact(runtime, repository, reference).map(ArtifactRead::Read),
        }
    }
}

fn github_artifact(runtime: &Runtime, repository: &Resolved, reference: &str, comments: bool) -> Result<ArtifactRead, CritiqueError> {
    let issue = spec_issue(repository, reference)?;
    let named = issue_reference(&issue);
    let request = IssueRequest { comments, locator: IssueLocator { cwd: repository.root.clone(), number: issue.number, repo: issue.repository.clone() } };
    Ok(match runtime.gh.view_issue(&request) {
        Ok(read) => ArtifactRead::Read(TrackedArtifact {
            labels: read.labels.clone(),
            name: format!("issue-{}.md", number_text(issue.number)),
            reference: named,
            text: render_issue(&issue_text(&read)),
            tracker: "github",
        }),
        Err(error) => ArtifactRead::Refused(refusal("tracker-unreachable", &format!("cannot read issue {named} through gh: {}", first_line(&error)))),
    })
}

fn issue_text(read: &GhIssue) -> IssueText {
    IssueText {
        body: read.body.clone(),
        comments: read.comments.iter().map(issue_comment).collect(),
        title: read.title.clone(),
    }
}

/// One comment of an issue as gh read it, as the tracker renders it.
pub(crate) fn issue_comment(comment: &crate::gh::IssueComment) -> IssueComment {
    IssueComment { author: comment.author.clone(), body: comment.body.clone(), created_at: comment.created_at.clone() }
}

/// The issue a spec reference names, which must be one of this repository's.
fn spec_issue(repository: &Resolved, reference: &str) -> Result<IssueReference, CritiqueError> {
    let Some(issue) = parse_issue_reference(reference, &repository.key) else {
        return Err(CritiqueError::new(
            "invalid-input",
            format!("{reference} is not an issue reference; this repository tracks specs as GitHub issues, so name one as #<number> or by its URL"),
        ));
    };
    if issue.repository != repository.key {
        return Err(CritiqueError::new("outside-repository", format!("{reference} is an issue of {}, not of {}", issue.repository, repository.key)));
    }
    Ok(issue)
}

fn issue_reference(issue: &IssueReference) -> String {
    format!("{}#{}", issue.repository, number_text(issue.number))
}

fn local_artifact(runtime: &Runtime, repository: &Resolved, reference: &str) -> Result<TrackedArtifact, CritiqueError> {
    let path = inside_repository(&repository.root, reference)?;
    let absolute = join(&[&repository.root, &path]);
    if !runtime.files.exists(&absolute).unwrap_or(false) {
        return Err(CritiqueError::new(
            "invalid-input",
            format!("{reference} is not a file in the repository; this repository tracks specs as local markdown, so name one by its path"),
        ));
    }
    let text = runtime.files.read_text(&absolute).map_err(CritiqueError::from_fs)?;
    Ok(TrackedArtifact { labels: Vec::new(), name: basename(&path).to_string(), reference: path, text, tracker: "local-markdown" })
}

/// The first line of an error's message that holds something, trimmed; `gh failed` when none does.
fn first_line(message: &str) -> String {
    message.split('\n').map(js_trim).find(|line| !line.is_empty()).unwrap_or("gh failed").to_string()
}
