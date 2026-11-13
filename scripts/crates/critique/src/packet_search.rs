//! How much one search answer of the runner's search server may hold (ADR-0006).

use crate::error::ShapeError;
use crate::fields::Fields;

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
