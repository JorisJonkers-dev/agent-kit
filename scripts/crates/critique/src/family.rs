//! The families a critic, an orchestrator and a family allowlist may name.

/// The families a critic can come from.
pub const MODEL_FAMILIES: [&str; 4] = ["mistral", "zhipu", "deepseek", "google"];

/// The families an orchestrator can come from: every critic family, plus those that only orchestrate.
pub const ORCHESTRATOR_FAMILIES: [&str; 6] = ["anthropic", "openai", "mistral", "zhipu", "deepseek", "google"];

/// The families a family allowlist may name: every family a client serves. `anthropic` (Claude Code) and `openai`
/// (Codex) are families like any other, so a repository's entry lists them for a delegation to reach those clients.
pub const ALLOWLIST_FAMILIES: [&str; 6] = ["mistral", "zhipu", "deepseek", "google", "anthropic", "openai"];

/// The family a critic can have: every family a client serves.
pub const CRITIC_FAMILIES: &[&str] = &ALLOWLIST_FAMILIES;
