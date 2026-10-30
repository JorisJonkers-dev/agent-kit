/** The model families a critic can come from, so the ones a family allowlist may name. */
export const MODEL_FAMILIES = ['mistral', 'zhipu', 'deepseek', 'google'] as const
export type ModelFamily = (typeof MODEL_FAMILIES)[number]

/** The families an orchestrator can come from: every critic family, plus those that only orchestrate. */
export const ORCHESTRATOR_FAMILIES = ['anthropic', 'openai', ...MODEL_FAMILIES] as const
export type OrchestratorFamily = (typeof ORCHESTRATOR_FAMILIES)[number]
