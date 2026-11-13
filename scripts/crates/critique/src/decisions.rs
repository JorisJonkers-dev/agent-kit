//! The decisions stage's two texts: the feature statement and the decision summary the orchestrator writes. The node
//! runner's `domain/decisions.ts`.

use crate::error::CritiqueError;
use model_gateway::text::js_trim;

/// The most lines the orchestrator's feature statement may have.
pub const FEATURE_MAX_LINES: usize = 5;

/// The most lines the orchestrator's decision summary may have.
pub const SUMMARY_MAX_LINES: usize = 40;

/// Where an artifact that is no tracker item came from.
pub const DECISION_DOCS: &str = "decision-docs";
pub const DECISION_SUMMARY: &str = "decision-summary";

/// The packet path of the feature statement.
pub const FEATURE_PATH: &str = "feature/feature.md";

/// The packet file name of the orchestrator's decision summary, under `artifact/`.
pub const SUMMARY_NAME: &str = "decision-summary.md";

/// The feature statement as the packet freezes it.
pub fn feature_statement(feature: &str) -> Result<String, CritiqueError> {
    Ok(format!("# Feature\n\n{}\n", limited_lines("feature", feature, FEATURE_MAX_LINES)?))
}

/// The decision summary as the packet freezes it.
pub fn decision_summary(summary: &str) -> Result<String, CritiqueError> {
    Ok(format!("{}\n", limited_lines("decision_summary", summary, SUMMARY_MAX_LINES)?))
}

fn limited_lines<'a>(field: &str, text: &'a str, max: usize) -> Result<&'a str, CritiqueError> {
    let trimmed = js_trim(text);
    // `split(/\r?\n/)`: a lone carriage return does not end a line.
    let lines = trimmed.split('\n').count();
    if lines > max {
        return Err(CritiqueError::new("invalid-input", format!("{field} must be at most {max} lines; it has {lines}")));
    }
    Ok(trimmed)
}
