//! How much one search answer of the runner's search server may hold (ADR-0006).

use crate::error::ShapeError;
use crate::fields::Fields;
use model_gateway::json::{self, Value};

/// How much one search answer may hold: at most `max_matches` matches and `max_bytes` bytes of text. Both live in
/// routing, under `search`, so the live checks can tune them.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchLimits {
    pub max_matches: f64,
    pub max_bytes: f64,
}

/// The limits of a routing file that states none.
pub fn default_search_limits() -> SearchLimits {
    SearchLimits { max_bytes: 8192.0, max_matches: 50.0 }
}

/// Reads routing's `search`.
pub fn read_search_limits(fields: &Fields<'_>) -> Result<SearchLimits, ShapeError> {
    Ok(SearchLimits { max_bytes: fields.positive_integer("max_bytes")?, max_matches: fields.positive_integer("max_matches")? })
}

/// The three tools of the runner's search server.
pub const SEARCH_TOOLS: [&str; 3] = ["grep", "list", "symbol"];

/// One tool of the runner's search server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchToolName {
    Grep,
    List,
    Symbol,
}

impl SearchToolName {
    /// The tool's name, as the server and a critic's prompt write it.
    pub fn name(self) -> &'static str {
        match self {
            SearchToolName::Grep => "grep",
            SearchToolName::List => "list",
            SearchToolName::Symbol => "symbol",
        }
    }
}

/// What setup and fan-out ask a search server to prove that it cannot see past its packet: a packet with one file
/// inside, a file outside it, and two symlinks out of it, one to the outside directory and one to the file.
pub struct ReachProof {
    pub inside: (&'static str, &'static str),
    pub marker: &'static str,
    /// The outside directory, the file in it and the file's text.
    pub outside: (&'static str, &'static str, &'static str),
    pub outside_marker: &'static str,
    /// Each link's path in the packet and the target it points at.
    pub symlinks: [(&'static str, &'static str); 2],
}

pub const REACH_PROOF: ReachProof = ReachProof {
    inside: ("inside.md", "reach proof: inside the packet\n"),
    marker: "reach proof",
    outside: ("outside", "outside/secret.md", "reach proof: OUTSIDE-THE-PACKET\n"),
    outside_marker: "OUTSIDE-THE-PACKET",
    symlinks: [("escape", "../outside"), ("escape.md", "../outside/secret.md")],
};

/// One call a client makes to a search server.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchCall {
    pub tool: SearchToolName,
    pub arguments: Value,
}

/// The calls of the reach proof: every way out a critic could try (`..`, an absolute path, a symlink to a directory and
/// one to a file, a glob that climbs) and one search that must find the inside file. `outside_dir` is the outside
/// directory's absolute path.
pub fn reach_proof_calls(outside_dir: &str) -> Vec<SearchCall> {
    let grep = |extra: Option<(&str, &str)>| {
        let mut pairs = vec![("pattern", Value::str(REACH_PROOF.marker))];
        if let Some((key, value)) = extra {
            pairs.push((key, Value::str(value)));
        }
        SearchCall { tool: SearchToolName::Grep, arguments: json::object(pairs) }
    };
    let list = |glob: &str| SearchCall { tool: SearchToolName::List, arguments: json::object(vec![("glob", Value::str(glob))]) };
    vec![
        grep(None),
        grep(Some(("path", "../outside"))),
        grep(Some(("path", outside_dir))),
        grep(Some(("path", "escape"))),
        grep(Some(("path", "escape.md"))),
        grep(Some(("glob", "../**"))),
        list("**"),
        list("../**"),
        list(&format!("{outside_dir}/**")),
    ]
}

/// Why the proof's answers show a server that reaches past its packet, or cannot search it: none when the first answer
/// finds the inside file and no answer names the outside file or quotes it.
pub fn reach_problems(answers: &[String]) -> Vec<String> {
    let mut problems = Vec::new();
    if !answers.first().is_some_and(|answer| answer.contains(REACH_PROOF.inside.0)) {
        problems.push("a search of the whole packet did not find the file inside it".to_string());
    }
    for (index, answer) in answers.iter().enumerate() {
        if answer.contains(REACH_PROOF.outside_marker) || answer.contains("secret.md") {
            problems.push(format!("call {} returned text from outside the packet", index + 1));
        }
    }
    problems
}
