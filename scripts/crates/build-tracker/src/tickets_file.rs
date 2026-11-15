//! A spec's ticket file (`docs/specs/<name>/tickets.yaml`): its branches and
//! tickets, and the state `render --tickets` draws from them.

use crate::spec::RepositoryName;
use crate::state::{BuildTrackerState, Failure, Group, SpecReference, Status, Ticket};
use crate::yaml_subset::{parse_yaml_subset, Yaml};
use std::collections::{HashMap, HashSet};

pub struct Branch {
    pub id: String,
    pub issue: u64,
    pub title: String,
}

pub struct FileTicket {
    pub blocked_by: Vec<String>,
    pub branch: String,
    pub issue: u64,
    pub key: String,
    pub title: String,
}

pub struct TicketsFile {
    pub spec: u64,
    pub branches: Vec<Branch>,
    pub tickets: Vec<FileTicket>,
}

fn invalid(reason: &str) -> Failure {
    Failure::new("invalid-tickets", format!("the ticket file is invalid: {reason}"))
}

fn positive_integer(value: Option<&Yaml>, field: &str) -> Result<u64, Failure> {
    match value {
        Some(Yaml::Num(number)) if number.fract() == 0.0 && *number > 0.0 && *number < 9_007_199_254_740_992.0 => Ok(*number as u64),
        _ => Err(invalid(&format!("{field} must be an issue number"))),
    }
}

fn required_text(value: Option<&Yaml>, field: &str) -> Result<String, Failure> {
    match value {
        Some(Yaml::Str(text)) if !text.is_empty() => Ok(text.clone()),
        _ => Err(invalid(&format!("{field} must be text"))),
    }
}

pub fn parse_tickets_file(text: &str) -> Result<TicketsFile, Failure> {
    let root = parse_yaml_subset(text)?;
    if !matches!(root, Yaml::Map(_)) {
        return Err(invalid("the file is not a mapping"));
    }
    let spec = positive_integer(root.get("spec"), "spec")?;
    let Some(Yaml::Map(listed)) = root.get("branches") else {
        return Err(invalid("branches must be a mapping of branch ids"));
    };
    let mut branches = Vec::new();
    for (id, value) in listed {
        if !matches!(value, Yaml::Map(_)) {
            return Err(invalid(&format!("branch {id} is not a mapping")));
        }
        branches.push(Branch {
            id: id.clone(),
            issue: positive_integer(value.get("issue"), &format!("branch {id}: issue"))?,
            title: required_text(value.get("title"), &format!("branch {id}: title"))?,
        });
    }
    let Some(Yaml::List(listed)) = root.get("tickets") else {
        return Err(invalid("tickets must be a list"));
    };
    let mut tickets = Vec::new();
    for (index, value) in listed.iter().enumerate() {
        if !matches!(value, Yaml::Map(_)) {
            return Err(invalid(&format!("ticket {} is not a mapping", index + 1)));
        }
        let key = required_text(value.get("key"), &format!("ticket {}: key", index + 1))?;
        let place = format!("ticket {key}");
        let blocked_by = match value.get("blocked_by") {
            Some(Yaml::List(items)) => items.iter().map(|item| if let Yaml::Str(text) = item { Some(text.clone()) } else { None }).collect::<Option<Vec<String>>>(),
            _ => None,
        };
        let Some(blocked_by) = blocked_by else {
            return Err(invalid(&format!("{place}: blocked_by must be a list of ticket keys")));
        };
        tickets.push(FileTicket {
            blocked_by,
            branch: required_text(value.get("branch"), &format!("{place}: branch"))?,
            issue: positive_integer(value.get("issue"), &format!("{place}: issue"))?,
            title: required_text(value.get("title"), &format!("{place}: title"))?,
            key,
        });
    }
    if tickets.is_empty() {
        return Err(invalid("the file has no tickets"));
    }
    check_graph(&branches, &tickets)?;
    Ok(TicketsFile { branches, spec, tickets })
}

fn check_graph(branches: &[Branch], tickets: &[FileTicket]) -> Result<(), Failure> {
    let mut keys = HashSet::new();
    let mut issues = HashSet::new();
    for ticket in tickets {
        if !keys.insert(ticket.key.as_str()) {
            return Err(invalid(&format!("the key {} appears twice", ticket.key)));
        }
        if !issues.insert(ticket.issue) {
            return Err(invalid(&format!("the issue #{} appears twice", ticket.issue)));
        }
    }
    for ticket in tickets {
        if !branches.iter().any(|branch| branch.id == ticket.branch) {
            return Err(invalid(&format!("ticket {} is on branch {}, which the file does not define", ticket.key, ticket.branch)));
        }
        if let Some(stray) = ticket.blocked_by.iter().find(|blocker| !keys.contains(blocker.as_str())) {
            return Err(invalid(&format!("ticket {} is blocked by {stray}, which is not a ticket of the file", ticket.key)));
        }
    }
    Ok(())
}

/// `/\(live\)/i`: JavaScript folds only ASCII case for these letters.
fn marks_live(title: &str) -> bool {
    let chars: Vec<char> = title.chars().collect();
    chars.windows(6).any(|window| window.iter().zip("(live)".chars()).all(|(left, right)| left.eq_ignore_ascii_case(&right)))
}

/// The state the page is drawn from: every ticket `merged` when its issue is closed, one section per branch.
pub fn state_from_tickets(file: &TicketsFile, repository: &RepositoryName, title: &str, closed: &HashMap<u64, bool>) -> BuildTrackerState {
    let issue_of: HashMap<&str, u64> = file.tickets.iter().map(|ticket| (ticket.key.as_str(), ticket.issue)).collect();
    let mut tickets: Vec<Ticket> = file
        .tickets
        .iter()
        .map(|entry| Ticket {
            blocked_by: entry.blocked_by.iter().map(|key| issue_of.get(key.as_str()).copied().unwrap_or(0)).collect(),
            external: Vec::new(),
            file: None,
            live: marks_live(&entry.title),
            note: String::new(),
            number: entry.issue,
            status: if closed.get(&entry.issue) == Some(&true) { Status::Merged } else { Status::Todo },
            title: format!("{} {}", entry.key, entry.title),
        })
        .collect();
    let groups: Vec<Group> = file
        .branches
        .iter()
        .map(|branch| Group { name: branch.title.clone(), tickets: file.tickets.iter().filter(|entry| entry.branch == branch.id).map(|entry| entry.issue).collect() })
        .collect();
    tickets.sort_by_key(|ticket| ticket.number);
    BuildTrackerState {
        subtitle: format!("{} tickets in {} branches", tickets.len(), groups.len()),
        groups: Some(groups),
        milestones: Vec::new(),
        spec: SpecReference::Github { number: file.spec, repository: repository.clone() },
        tickets,
        title: title.to_string(),
    }
}
