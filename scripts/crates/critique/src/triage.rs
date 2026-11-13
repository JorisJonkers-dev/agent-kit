//! What decides, without any model, whether a ticket's change is risky.

use crate::error::ShapeError;
use crate::fields::Fields;

/// The cheap model asked, only when no rule fired, whether the change warrants a review.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageModel {
    /// A Vibe client of routing's `clients`.
    pub client: String,
    pub max_price_usd: f64,
    pub time_limit_s: f64,
}

/// Routing's `triage` as written: the risk rules and the triage model. The rules are repository paths that hold
/// risky code, a size above which any change is risky, and tracker labels that mark a ticket risky.
#[derive(Clone, Debug, PartialEq)]
pub struct TriageRules {
    /// Globs over repository paths, as git prints them.
    pub risk_paths: Vec<String>,
    /// Changed lines (added plus removed) above which a change is risky.
    pub changed_lines_threshold: f64,
    /// Tracker labels, in any case, that make a ticket risky.
    pub labels: Vec<String>,
    pub model: TriageModel,
}

/// Reads routing's `triage`.
pub fn read_triage_rules(fields: &Fields<'_>) -> Result<TriageRules, ShapeError> {
    let model = fields.object("model")?;
    Ok(TriageRules {
        changed_lines_threshold: fields.number("changed_lines_threshold")?,
        labels: fields.strings("labels")?,
        model: TriageModel {
            client: model.string("client")?,
            max_price_usd: model.number("max_price_usd")?,
            time_limit_s: model.number("time_limit_s")?,
        },
        risk_paths: fields.strings("risk_paths")?,
    })
}
