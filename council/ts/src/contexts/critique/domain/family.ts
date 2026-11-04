/** The families a critic can come from. */
export const MODEL_FAMILIES = ['mistral', 'zhipu', 'deepseek', 'google'] as const
export type ModelFamily = (typeof MODEL_FAMILIES)[number]

/** The families an orchestrator can come from: every critic family, plus those that only orchestrate. */
export const ORCHESTRATOR_FAMILIES = ['anthropic', 'openai', ...MODEL_FAMILIES] as const
export type OrchestratorFamily = (typeof ORCHESTRATOR_FAMILIES)[number]

/**
 * The families a family allowlist may name: every family a client serves.
 * `anthropic` (Claude Code) and `openai` (Codex) are families like any other,
 * so a repository's entry lists them for a delegation to reach those clients.
 * A critic still never comes from the orchestrator's own family.
 */
export const ALLOWLIST_FAMILIES = [...MODEL_FAMILIES, 'anthropic', 'openai'] as const
export type AllowlistFamily = (typeof ALLOWLIST_FAMILIES)[number]
