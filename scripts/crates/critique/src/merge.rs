//! The merge of what several critics found: the same defect raised twice is read once. The node runner's
//! `domain/merge.ts`.

use crate::findings::{CriticFinding, Evidence, FindingBody, RunFinding, SEVERITIES};
use model_gateway::text::{is_js_whitespace, js_trim};

/// The findings one critic of a run reported.
pub struct CriticFindings {
    pub critic_id: String,
    pub findings: Vec<CriticFinding>,
}

/// A merged finding, and whether its suggestion was taken from a later critic, which puts the key after `raised_by`
/// where the node runner's object has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Merged {
    pub finding: RunFinding,
    pub late_suggestion: bool,
}

/// Lower case, single spaces, no surrounding quotes or trailing punctuation.
fn normal(text: &str) -> String {
    let mut folded = String::new();
    let mut in_space = false;
    for c in text.to_lowercase().chars() {
        if is_js_whitespace(c) {
            in_space = true;
            continue;
        }
        if in_space {
            folded.push(' ');
            in_space = false;
        }
        folded.push(c);
    }
    if in_space {
        folded.push(' ');
    }
    let trimmed = js_trim(&folded);
    let opening = ['"', '\'', '`', '\u{201c}', '\u{2018}'];
    let closing = ['"', '\'', '`', '\u{201d}', '\u{2019}', '.', ',', ';', ':'];
    trimmed.trim_start_matches(opening).trim_end_matches(closing).to_string()
}

fn quotes(evidence: &[Evidence]) -> Vec<String> {
    evidence.iter().filter(|item| item.kind == "quote").map(|item| normal(&item.value)).collect()
}

fn same_defect(a: &FindingBody, b: &FindingBody) -> bool {
    if a.category != b.category || normal(&a.location) != normal(&b.location) {
        return false;
    }
    let (quotes_a, quotes_b) = (quotes(&a.evidence), quotes(&b.evidence));
    if quotes_a.is_empty() && quotes_b.is_empty() {
        return normal(&a.claim) == normal(&b.claim);
    }
    quotes_a.iter().any(|qa| quotes_b.iter().any(|qb| qa.contains(qb.as_str()) || qb.contains(qa.as_str())))
}

fn rank(severity: &str) -> Option<usize> {
    SEVERITIES.iter().position(|level| *level == severity)
}

/// Merges findings that point at the same location and claim the same defect, across critics and within one, so the
/// orchestrator reads each problem once. A merged finding keeps every critic that raised it, so agreement between
/// families stays visible, and takes the highest severity any of them gave. Two findings are the same defect when their
/// locations match, ignoring case, spacing and quotes, their categories match, and they share a quote (one quote
/// containing the other counts, since critics cut quotes differently). Findings without quotes are the same defect only
/// when their claims match. Merged findings are numbered `F1`, `F2`, ... in the order first raised.
pub fn merge_findings(per_critic: &[CriticFindings]) -> Vec<Merged> {
    let mut merged: Vec<Merged> = Vec::new();
    for CriticFindings { critic_id, findings } in per_critic {
        for finding in findings {
            let Some(same) = merged.iter_mut().find(|candidate| same_defect(&candidate.finding.body, &finding.body)) else {
                let raised = RunFinding {
                    id: String::new(),
                    body: finding.body.clone(),
                    critic_finding_id: finding.id.clone(),
                    raised_by: vec![critic_id.clone()],
                    pre_verdict: None,
                    declared_severity: None,
                };
                merged.push(Merged { finding: raised, late_suggestion: false });
                continue;
            };
            let known = &mut same.finding;
            if !known.raised_by.contains(critic_id) {
                known.raised_by.push(critic_id.clone());
            }
            // What is new is judged against the evidence as it stood, so a repeat within the newcomer's own list stays.
            let added: Vec<Evidence> = finding.body.evidence.iter().filter(|item| !known.body.evidence.contains(item)).cloned().collect();
            known.body.evidence.extend(added);
            if rank(&finding.body.severity) < rank(&known.body.severity) {
                known.body.severity = finding.body.severity.clone();
            }
            if known.body.suggestion.is_none() && finding.body.suggestion.is_some() {
                known.body.suggestion = finding.body.suggestion.clone();
                same.late_suggestion = true;
            }
        }
    }
    for (index, entry) in merged.iter_mut().enumerate() {
        entry.finding.id = format!("F{}", index + 1);
    }
    merged
}
