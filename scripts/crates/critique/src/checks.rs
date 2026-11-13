//! The checks a critic's role and lens files declare, and what a finding's check says about it. The node runner's
//! `domain/checks.ts`.

use crate::findings::{PreVerdict, RunFinding, SEVERITIES};
use crate::js_regex::Regex;
use std::sync::OnceLock;

/// A check a critic's role or lens declares, the severity it fixes, if it fixes one, and whether it needs code under
/// `tree/`.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredCheck {
    pub id: String,
    pub severity: Option<String>,
    pub needs_tree: bool,
}

/// A check id opens a lens bullet or a role rule: a backticked lower-case slug at the start of a top-level bullet,
/// followed by a colon, with an optional fixed severity in brackets and an optional "Needs `tree/`." mark.
fn check_bullet() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    CELL.get_or_init(|| {
        Regex::new(r"^- `([a-z0-9]+(?:-[a-z0-9]+)*)`(?: \((critical|major|minor)\))?:( Needs `tree\/`\.)?", true).expect("the check bullet pattern compiles")
    })
}

/// Whether a check a finding names has the shape of a check id, rather than free text.
pub fn is_check_id(check: Option<&str>) -> bool {
    static CELL: OnceLock<Regex> = OnceLock::new();
    let regex = CELL.get_or_init(|| Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$", false).expect("the check id pattern compiles"));
    check.is_some_and(|check| regex.find_at(&check.chars().collect::<Vec<_>>(), 0).is_some())
}

/// The checks a critic's role and lens files declare, in the order they appear; an id declared twice counts once, as
/// first declared.
pub fn declared_checks(texts: &[&str]) -> Vec<DeclaredCheck> {
    let mut checks: Vec<DeclaredCheck> = Vec::new();
    for text in texts {
        let chars: Vec<char> = text.chars().collect();
        for hit in check_bullet().find_all(&chars) {
            let id = hit.group(&chars, 1).unwrap_or_default();
            let named = hit.group(&chars, 2);
            let severity = SEVERITIES.iter().find(|level| Some(**level) == named.as_deref()).map(|level| level.to_string());
            if !checks.iter().any(|check| check.id == id) {
                checks.push(DeclaredCheck { id, severity, needs_tree: hit.group(&chars, 3).is_some() });
            }
        }
    }
    checks
}

/// A finding whose check declares a severity takes that severity, whatever its critic wrote, and keeps it as
/// `declared_severity`, so record can tell an override from the critic's own rating.
pub fn with_declared_severity(finding: &RunFinding, declared: &[DeclaredCheck]) -> RunFinding {
    let severity = declared.iter().find(|check| Some(&check.id) == finding.body.check.as_ref()).and_then(|check| check.severity.clone());
    let mut result = finding.clone();
    if let Some(severity) = severity {
        result.declared_severity = Some(severity.clone());
        result.body.severity = severity;
    }
    result
}

/// Whether a finding names a check its critic's prompt declares: a hint when it names none, or one that no lens or
/// role of that prompt declares; nothing when the check is one of `declared`.
pub fn check_named(check: Option<&str>, declared: &[DeclaredCheck]) -> Option<PreVerdict> {
    let Some(check) = check else {
        return Some(PreVerdict { reason: "names no check".to_string(), reason_code: "no-check".to_string(), verdict: "hint".to_string() });
    };
    if declared.iter().any(|entry| entry.id == check) {
        return None;
    }
    Some(PreVerdict {
        reason: format!("names the check {check}, which no lens or role in its critic's prompt declares"),
        reason_code: "unknown-check".to_string(),
        verdict: "hint".to_string(),
    })
}

/// The hint a finding gets when what it checks needs code under `tree/` and the packet holds none.
pub fn needs_tree() -> PreVerdict {
    PreVerdict {
        reason: NEEDS_TREE_REASON.to_string(),
        reason_code: "needs-tree".to_string(),
        verdict: "hint".to_string(),
    }
}

pub const NEEDS_TREE_REASON: &str = "the check needs code under tree/, which this packet does not hold";

/// A finding of a check that needs code under `tree/` cannot be checked in a packet that holds none, so it is a
/// hint: the check was skipped.
pub fn check_runnable(check: Option<&str>, declared: &[DeclaredCheck], tree: bool) -> Option<PreVerdict> {
    let needs = declared.iter().find(|entry| Some(entry.id.as_str()) == check).is_some_and(|entry| entry.needs_tree);
    if tree || !needs {
        None
    } else {
        Some(needs_tree())
    }
}
