//! Routing's `gate`: the branches a push may update without a feature review, and how many commits a merge check
//! reads one by one.

use crate::error::ShapeError;
use crate::fields::Fields;

/// The most commits a merge may bring in and still be checked commit by commit, when routing leaves it out.
pub const DEFAULT_MERGE_COMMITS: f64 = 50.0;

/// Routing's `gate`.
#[derive(Clone, Debug, PartialEq)]
pub struct GateRouting {
    /// Globs over branch names; a push that updates only such branches is not gated.
    pub feature_branches: Vec<String>,
    /// The most commits a merge may bring in and still be checked commit by commit; a merge that brings in more is
    /// judged whole, so the check stays within the gate's deadline.
    pub merge_commits: f64,
}

/// Reads routing's `gate`.
pub fn read_gate_routing(fields: &Fields<'_>) -> Result<GateRouting, ShapeError> {
    Ok(GateRouting {
        feature_branches: fields.strings("feature_branches")?,
        merge_commits: if fields.has("merge_commits") { fields.positive_integer("merge_commits")? } else { DEFAULT_MERGE_COMMITS },
    })
}
