//! The ticket graph file (docs/specs/gold-band/tickets.yaml) and the spec's
//! user stories, which the branch issue bodies quote.

use serde::Deserialize;
use std::collections::BTreeMap;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Deserialize)]
pub struct Branch {
    pub title: String,
    #[serde(default)]
    pub issue: Option<u64>,
    pub what: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ticket {
    pub key: String,
    #[serde(default)]
    pub issue: Option<u64>,
    pub branch: String,
    #[serde(default)]
    pub stories: Vec<u64>,
    pub title: String,
    #[serde(default)]
    pub blocked_by: Vec<String>,
    #[serde(default)]
    pub supersedes: Vec<u64>,
    pub lanes: Vec<String>,
    pub hosts: Vec<String>,
    pub what: String,
    pub ac: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub spec: u64,
    pub labels: Vec<String>,
    pub branches: BTreeMap<String, Branch>,
    pub tickets: Vec<Ticket>,
}

/// The key the spec issue goes by in plans; it is never created or edited.
pub const SPEC_KEY: &str = "SPEC";

impl Model {
    /// Parse and validate the graph: every ticket names a known branch and
    /// only known blockers, and keys are unique.
    pub fn parse(text: &str) -> Result<Model> {
        let model: Model = serde_yaml::from_str(text).map_err(|e| format!("tickets file: {e}"))?;
        let mut seen = std::collections::BTreeSet::new();
        for t in &model.tickets {
            if !seen.insert(t.key.as_str()) {
                return Err(format!("duplicate ticket key {}", t.key));
            }
            if !model.branches.contains_key(&t.branch) {
                return Err(format!("{}: unknown branch {}", t.key, t.branch));
            }
        }
        for t in &model.tickets {
            for b in &t.blocked_by {
                if !seen.contains(b.as_str()) {
                    return Err(format!("{}: blocked by unknown ticket {b}", t.key));
                }
            }
        }
        Ok(model)
    }

    pub fn ticket(&self, key: &str) -> Option<&Ticket> {
        self.tickets.iter().find(|t| t.key == key)
    }

    /// The published issue number of a key (spec, branch or ticket), if any.
    pub fn number(&self, key: &str) -> Option<u64> {
        if key == SPEC_KEY {
            Some(self.spec)
        } else if let Some(b) = self.branches.get(key) {
            b.issue
        } else {
            self.ticket(key).and_then(|t| t.issue)
        }
    }

    pub fn set_number(&mut self, key: &str, number: u64) {
        if let Some(b) = self.branches.get_mut(key) {
            b.issue = Some(number);
        } else if let Some(t) = self.tickets.iter_mut().find(|t| t.key == key) {
            t.issue = Some(number);
        }
    }

    /// Branch keys then ticket keys, in file order.
    pub fn keys(&self) -> Vec<String> {
        self.branches.keys().cloned().chain(self.tickets.iter().map(|t| t.key.clone())).collect()
    }

    /// The parent in the sub-issue chain: spec, then branch, then ticket.
    pub fn parent(&self, key: &str) -> String {
        match self.ticket(key) {
            Some(t) => t.branch.clone(),
            None => SPEC_KEY.to_string(),
        }
    }

    /// The sorted, de-duplicated stories the tickets of a branch deliver.
    pub fn stories_of_branch(&self, branch: &str) -> Vec<u64> {
        let mut all: Vec<u64> = self.tickets.iter().filter(|t| t.branch == branch).flat_map(|t| t.stories.clone()).collect();
        all.sort_unstable();
        all.dedup();
        all
    }
}

/// The spec's numbered user stories: `N. As ...` lines, keyed by number.
pub fn stories(spec_text: &str) -> BTreeMap<u64, String> {
    let mut out = BTreeMap::new();
    for line in spec_text.lines() {
        let digits: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && line[digits.len()..].starts_with(". As ") {
            if let Ok(n) = digits.parse() {
                out.insert(n, line.to_string());
            }
        }
    }
    out
}
