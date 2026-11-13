//! What routing caps beyond each critic's own price cap and time limit.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::packet_search::{default_search_limits, read_search_limits, SearchLimits};
use crate::stage::CRITIQUE_STAGES;

/// Antigravity reports no quota headless, so the runner counts its own agy launches per day. Past the reserve
/// threshold only the reserved stages may still use agy; at the daily cap none may.
#[derive(Clone, Debug, PartialEq)]
pub struct AntigravityQuota {
    pub daily_cap: f64,
    pub reserve_threshold: f64,
    pub reserved_stages: Vec<String>,
}

/// What routing caps beyond each critic's own price cap and time limit, under the keys routing gives them. Vibe
/// carries no dollar cap of its own: it is counted against its plan limit, which the model gateway reads.
#[derive(Clone, Debug, PartialEq)]
pub struct CritiqueLimits {
    pub antigravity_quota: AntigravityQuota,
    /// How much one answer of the runner's search server may hold (ADR-0006).
    pub search: SearchLimits,
}

/// #167's limits, for a routing file that does not state its own.
pub fn default_limits() -> CritiqueLimits {
    CritiqueLimits {
        antigravity_quota: AntigravityQuota {
            daily_cap: 20.0,
            reserve_threshold: 10.0,
            reserved_stages: vec!["spec".to_string(), "feature-review".to_string()],
        },
        search: default_search_limits(),
    }
}

/// Reads the limits from routing or a run manifest; a missing key keeps #167's value.
pub fn read_limits(file: &Fields<'_>) -> Result<CritiqueLimits, ShapeError> {
    let quota = if file.has("antigravity_quota") { Some(file.object("antigravity_quota")?) } else { None };
    Ok(CritiqueLimits {
        antigravity_quota: match quota {
            None => default_limits().antigravity_quota,
            Some(quota) => AntigravityQuota {
                daily_cap: quota.number("daily_cap")?,
                reserve_threshold: quota.number("reserve_threshold")?,
                reserved_stages: quota.one_of_list("reserved_stages", &CRITIQUE_STAGES)?,
            },
        },
        search: if file.has("search") { read_search_limits(&file.object("search")?)? } else { default_limits().search },
    })
}
