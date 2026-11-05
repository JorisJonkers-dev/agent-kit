import { KNOWN_FAMILIES } from '../../../shared-kernel/families.js'

/** The families a critic can come from. */
export const MODEL_FAMILIES = ['mistral', 'zhipu', 'deepseek', 'google'] as const
export type ModelFamily = (typeof MODEL_FAMILIES)[number]

/** The families an orchestrator can come from: every critic family, plus those that only orchestrate. */
export const ORCHESTRATOR_FAMILIES = ['anthropic', 'openai', ...MODEL_FAMILIES] as const
export type OrchestratorFamily = (typeof ORCHESTRATOR_FAMILIES)[number]

/**
 * The families a family allowlist may name: every known family, from the one
 * shared list the model gateway also reads, so an entry the gateway would act
 * on is never one the critique runner refuses to read. `anthropic` (Claude
 * Code) and `openai` (Codex) are families like any other, so a repository's
 * entry lists them for a delegation to reach those clients. A critic still
 * never comes from the orchestrator's own family.
 */
export const ALLOWLIST_FAMILIES = KNOWN_FAMILIES as readonly string[]
export type AllowlistFamily = string

/**
 * The family a critic can have: every family a client serves. Only a `nedap`
 * repository's work roster has critics from `anthropic` or `openai`; every
 * other roster draws on the four model families.
 */
export const CRITIC_FAMILIES = [...MODEL_FAMILIES, 'anthropic', 'openai'] as const
export type CriticFamily = (typeof CRITIC_FAMILIES)[number]
