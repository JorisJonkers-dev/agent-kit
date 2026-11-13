//! A stage's depth signals as routing configures them: the thresholds, risk terms and labels that select deep.

use crate::error::ShapeError;
use crate::fields::Fields;

/// One stage's depth signals as routing configures them. Any one that fires selects deep.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthSignalsConfig {
    /// Per measure, the value above which the stage goes deep.
    pub thresholds: Vec<(String, f64)>,
    /// Terms whose presence in the artifact, as a whole word and in any case, selects deep.
    pub risk_terms: Vec<String>,
    /// Tracker labels, in any case, that select deep.
    pub labels: Vec<String>,
}

/// The signals of a stage whose routing states none.
pub fn no_depth_signals() -> DepthSignalsConfig {
    DepthSignalsConfig { labels: Vec::new(), risk_terms: Vec::new(), thresholds: Vec::new() }
}

/// Reads a stage's `depth_signals`. Routing that names a measure its stage does not know is rejected.
pub fn read_depth_signals_config(fields: &Fields<'_>, stage: &str) -> Result<DepthSignalsConfig, ShapeError> {
    let known = measure_names(stage);
    let thresholds = if fields.has("thresholds") { fields.number_map("thresholds")? } else { Vec::new() };
    for (measure, _) in &thresholds {
        if !known.contains(&measure.as_str()) {
            let names = if known.is_empty() { "none".to_string() } else { known.join(", ") };
            return Err(ShapeError(format!(
                "stages.{stage}.depth_signals.thresholds.{measure} is not a measure of stage {stage}; it knows {names}"
            )));
        }
    }
    Ok(DepthSignalsConfig {
        labels: if fields.has("labels") { fields.strings("labels")? } else { Vec::new() },
        risk_terms: if fields.has("risk_terms") { fields.strings("risk_terms")? } else { Vec::new() },
        thresholds,
    })
}

/// The size measures a stage knows, by the name routing gives their thresholds, in the order the node runner's
/// table holds them. A stage gains depth signals by adding a measure there; the routing shape stays the same.
fn measure_names(stage: &str) -> &'static [&'static str] {
    match stage {
        "architecture-review" | "decision-recovery" => &["context_maps", "source_lines"],
        "adr-review" => &["adrs"],
        "plan" => &["steps", "stored_data_steps"],
        "decisions" => &["decisions", "words"],
        "feature-review" => &[],
        "spec" => &["user_stories", "words"],
        "ticket-review" => &["changed_lines", "risk_path_hits"],
        "tickets" => &["tickets"],
        _ => &[],
    }
}
