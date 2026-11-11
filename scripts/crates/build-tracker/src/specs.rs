//! Finds the spec a `--spec` argument names: a GitHub issue, or a local
//! markdown file in a git repository, and reads its tickets.

use crate::local_tickets::{local_spec_title, local_tickets, markdown_names, LocalTicketFile};
use crate::ports::{IssueRecord, RepositoryLocation, Runtime};
use crate::seed::{IssueTrackerSnapshot, SpecTicket};
use crate::spec::{
    issue_reference_text, local_repository_name, parse_spec_argument, same_repository, spec_text, state_base, state_location, ticket_files, RepositoryName, ResolvedSpec, SpecArgument, StateLocation, TicketFiles,
};
use crate::state::{ExternalBlocker, Failure};
use crate::text::{basename, dirname, first_line, join, resolve};

/// A resolved spec and how to read its tickets from its issue tracker.
pub struct SpecSource {
    pub spec: ResolvedSpec,
    /// Where the page links a local spec's ticket files; none for an issue spec, whose tickets link to their issues.
    pub files: Option<TicketFiles>,
    kind: Kind,
}

enum Kind {
    Issue { repository: RepositoryName, number: u64 },
    Local { dir: String, prefix: String, path: String, text: String },
}

/// A `--spec` argument resolved, with where its state file and page live.
pub struct LocatedSpec {
    pub source: SpecSource,
    pub location: StateLocation,
    /// How the state and the command output name the spec.
    pub spec: String,
}

/// The repository a local spec in this git repository belongs to, and where its ticket files are shown.
pub fn local_spec_place(location: &RepositoryLocation) -> (RepositoryName, TicketFiles) {
    (location.remote.clone().unwrap_or_else(|| local_repository_name(&location.main_root)), ticket_files(location.remote.as_ref()))
}

fn unexpected(message: String) -> Failure {
    Failure::new("unexpected", message)
}

pub fn locate_spec(argument: &str, runtime: &Runtime) -> Result<LocatedSpec, Failure> {
    let source = resolve_spec(argument, runtime)?;
    let Some(base) = state_base(&runtime.env) else {
        return Err(Failure::new(
            "no-state-home",
            "neither XDG_STATE_HOME nor HOME is an absolute path, so there is no place outside the repository for the state; set one of them",
        ));
    };
    Ok(LocatedSpec { location: state_location(&base, &source.spec), spec: spec_text(&source.spec), source })
}

fn resolve_spec(argument: &str, runtime: &Runtime) -> Result<SpecSource, Failure> {
    match parse_spec_argument(argument) {
        SpecArgument::Issue { number, repository } => {
            let repository = match repository {
                Some(repository) => repository,
                None => current_repository(argument, runtime)?,
            };
            Ok(SpecSource { files: None, kind: Kind::Issue { repository: repository.clone(), number }, spec: ResolvedSpec::Github { number, repository } })
        }
        SpecArgument::Path(path) => resolve_local_spec(&path, runtime),
    }
}

fn current_repository(argument: &str, runtime: &Runtime) -> Result<RepositoryName, Failure> {
    let location = runtime.repositories.locate(&runtime.cwd).map_err(unexpected)?;
    match location.and_then(|location| location.remote) {
        Some(remote) => Ok(remote),
        None => Err(Failure::new(
            "invalid-spec",
            format!("{argument} names an issue of the current repository, but {} is in no repository with a remote; name it as owner/repo#<number>", runtime.cwd),
        )),
    }
}

fn resolve_local_spec(path: &str, runtime: &Runtime) -> Result<SpecSource, Failure> {
    let file = resolve(&runtime.cwd, path);
    let Ok(text) = runtime.files.read_text(&file) else {
        return Err(Failure::new("invalid-spec", format!("{path} is neither an issue reference (#<number>, owner/repo#<number> or an issue URL) nor a readable spec file")));
    };
    let Some(location) = runtime.repositories.locate(dirname(&file)).map_err(unexpected)? else {
        return Err(Failure::new("invalid-spec", format!("{path} is in no git repository")));
    };
    let relative = format!("{}{}", location.prefix, basename(&file));
    let (repository, files) = local_spec_place(&location);
    Ok(SpecSource {
        files: Some(files),
        kind: Kind::Local { dir: dirname(&file).to_string(), path: relative.clone(), prefix: location.prefix, text },
        spec: ResolvedSpec::Local { path: relative, repository },
    })
}

impl SpecSource {
    /// The spec's title and tickets as the issue tracker holds them now.
    pub fn snapshot(&self, runtime: &Runtime) -> Result<IssueTrackerSnapshot, Failure> {
        match &self.kind {
            Kind::Issue { repository, number } => read_issue_spec(repository, *number, runtime),
            Kind::Local { dir, prefix, path, text } => Ok(IssueTrackerSnapshot { tickets: read_local_tickets(dir, prefix, path, runtime)?, title: local_spec_title(text, path) }),
        }
    }
}

/// The spec issue's title and its sub-issues, each with its native blocked-by links.
fn read_issue_spec(repository: &RepositoryName, number: u64, runtime: &Runtime) -> Result<IssueTrackerSnapshot, Failure> {
    let reference = issue_reference_text(repository, number);
    let tracker_unreachable = |message: String| {
        Failure::new("tracker-unreachable", format!("cannot read the tickets of {reference}: {}", first_line(&message).unwrap_or_else(|| "the tracker failed".to_string())))
    };
    let spec_issue = runtime.issues.issue(repository, number).map_err(tracker_unreachable)?;
    let subs = runtime.issues.sub_issues(repository, number).map_err(tracker_unreachable)?;
    if let Some(outside) = subs.iter().find(|sub| !same_repository(&sub.repository, repository)) {
        return Err(Failure::new(
            "unsupported-ticket",
            format!(
                "{} is a sub-issue of {reference} in another repository; the build tracker keys tickets by issue number in the spec's repository",
                issue_reference_text(&outside.repository, outside.number)
            ),
        ));
    }
    let inside = |blocker: &IssueRecord| same_repository(&blocker.repository, repository) && subs.iter().any(|sub| sub.number == blocker.number);
    let mut tickets = Vec::new();
    for sub in &subs {
        let blockers = runtime.issues.blocked_by(&sub.repository, sub.number).map_err(tracker_unreachable)?;
        tickets.push(SpecTicket {
            blocked_by: blockers.iter().filter(|blocker| inside(blocker)).map(|blocker| blocker.number).collect(),
            closed: sub.closed,
            external: blockers
                .iter()
                .filter(|blocker| !inside(blocker))
                .map(|blocker| ExternalBlocker { reference: issue_reference_text(&blocker.repository, blocker.number), state: Some(if blocker.closed { "closed" } else { "open" }.to_string()) })
                .collect(),
            file: None,
            number: sub.number,
            title: sub.title.clone(),
        });
    }
    Ok(IssueTrackerSnapshot { tickets, title: spec_issue.title })
}

/// The `.md` files in the `issues/` folder beside the spec, as `/to-tickets` writes them.
fn read_local_tickets(spec_dir: &str, prefix: &str, spec_path: &str, runtime: &Runtime) -> Result<Vec<SpecTicket>, Failure> {
    let folder = join(&[spec_dir, "issues"]);
    let Ok(names) = runtime.files.list_files(&folder) else {
        return Err(Failure::new("invalid-ticket-file", format!("there is no issues/ folder beside {spec_path}")));
    };
    let mut files = Vec::new();
    for name in markdown_names(&names) {
        let text = runtime.files.read_text(&join(&[&folder, &name])).map_err(|error| unexpected(error.message))?;
        files.push(LocalTicketFile { file: format!("{prefix}issues/{name}"), text });
    }
    local_tickets(&files)
}
