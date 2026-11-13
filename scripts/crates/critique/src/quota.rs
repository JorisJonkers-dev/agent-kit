//! Whether the day's Antigravity launches leave a stage room to use agy. The node runner's `quotaReached` of
//! `domain/budget.ts`.

use crate::limits::AntigravityQuota;
use model_gateway::json::number_text;

/// Why a stage may not launch agy today, or `None` while it may: at the daily cap no stage may; past the reserve
/// threshold only the reserved stages may.
pub fn quota_reached(count: usize, quota: &AntigravityQuota, stage: &str) -> Option<String> {
    let used = count as f64;
    if used >= quota.daily_cap {
        return Some(format!("antigravity: {count} invocations today have reached the daily cap of {}", number_text(quota.daily_cap)));
    }
    if used >= quota.reserve_threshold && !quota.reserved_stages.iter().any(|reserved| reserved == stage) {
        return Some(format!(
            "antigravity: {count} invocations today have reached the reserve threshold of {}; the rest is kept for {}",
            number_text(quota.reserve_threshold),
            quota.reserved_stages.join(" and ")
        ));
    }
    None
}
