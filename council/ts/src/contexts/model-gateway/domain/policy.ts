import type { AllowlistEntry } from './allowlist.js'
import type { GatewayConfig } from './config.js'
import { modelKey, type ListedModel, type MissingClient, type ModelListing, type SkippedModel } from './models.js'

/** What a delegation to one client model may carry. */
export interface SendPermission {
  readonly code: boolean
  readonly history: boolean
}

/** What the data policy says about one client model. */
export interface ModelPermission {
  readonly client: string
  readonly model: string
  /** Whether a delegation may reach this client model at all. */
  readonly permitted: boolean
  /** True when the delegation carries only what the caller declares, never a repository. */
  readonly inline_only: boolean
  readonly may_send: SendPermission
}

/** The data policy's decision for every client model, and one line saying why. */
export interface PolicyDecision {
  readonly reason: string
  readonly permissions: readonly ModelPermission[]
}

/**
 * What a delegation would carry: a repository's content, or only what the
 * caller declares. A repository with no remote, and one owned by `nedap`,
 * are decided by their own rule rather than by an allowlist entry.
 */
export type DataScope =
  | { readonly kind: 'outside' }
  | { readonly kind: 'no-remote' }
  | { readonly kind: 'nedap' }
  | { readonly kind: 'repository'; readonly key: string; readonly entry: AllowlistEntry | undefined }

/**
 * The two clients a `nedap` repository may reach, whatever an allowlist entry
 * says. It is the client that counts, not the family: a Claude-family model
 * served by another client is not Claude Code.
 */
const NEDAP_CLIENTS = ['claude', 'codex']

/** The cost classes a delegation outside any repository may reach. */
const INLINE_COST_CLASSES = ['subsidised', 'metered-personal']

const NOTHING: SendPermission = { code: false, history: false }
const EVERYTHING: SendPermission = { code: true, history: true }

/**
 * One client model the policy decides about: the client that serves it, its
 * id, and the family it belongs to. A catalogue entry is one, and so is a
 * model a client listed itself.
 */
export interface PolicySubject {
  readonly client: string
  readonly model: string
  readonly family: string
}

/**
 * The data policy for one scope: per client model, whether a delegation may
 * reach it and what it may carry. A model the owner disabled is left out.
 *
 * It decides about `subjects`, which default to the catalogue: the caller
 * passes the models it actually lists, so a model a client lists itself is
 * decided by the same rules as a declared one.
 */
export function decidePolicy(
  config: GatewayConfig,
  scope: DataScope,
  subjects: readonly PolicySubject[] = config.models,
): PolicyDecision {
  const permissions = subjects
    .filter((subject) => !config.disabled.includes(modelKey(subject.client, subject.model)))
    .map((subject) => permission(config, subject, scope))
  return { permissions, reason: reasonFor(scope) }
}

function permission(config: GatewayConfig, subject: PolicySubject, scope: DataScope): ModelPermission {
  const base = { client: subject.client, inline_only: false, model: subject.model }
  switch (scope.kind) {
    case 'outside':
      return {
        ...base,
        inline_only: true,
        may_send: NOTHING,
        permitted: INLINE_COST_CLASSES.includes(config.clients[subject.client]?.cost_class ?? ''),
      }
    case 'no-remote':
      return { ...base, may_send: NOTHING, permitted: false }
    case 'nedap':
      return { ...base, may_send: EVERYTHING, permitted: NEDAP_CLIENTS.includes(subject.client) }
    case 'repository':
      return {
        ...base,
        may_send: scope.entry === undefined ? NOTHING : { code: scope.entry.code, history: scope.entry.history },
        permitted: scope.entry?.families.includes(subject.family) === true,
      }
  }
}

/** One line saying why the policy decided what it did. */
function reasonFor(scope: DataScope): string {
  switch (scope.kind) {
    case 'outside':
      return 'no repository: only inline content, to subsidised and metered-personal clients'
    case 'no-remote':
      return 'the repository has no remote, so nothing is sent'
    case 'nedap':
      return 'a nedap repository: only Codex and Claude Code, with code and history'
    case 'repository':
      return scope.entry === undefined
        ? `${scope.key} has no allowlist entry, so nothing is sent`
        : `${scope.key}: the entry allows ${scope.entry.families.length === 0 ? 'no family' : scope.entry.families.join(', ')}`
  }
}

/** One listed model with what the policy lets a delegation to it carry. */
export interface PermittedModel extends ListedModel {
  readonly inline_only: boolean
  readonly may_send: SendPermission
}

export interface PolicyListing {
  readonly models: readonly PermittedModel[]
  readonly missing: readonly MissingClient[]
  readonly skipped: readonly SkippedModel[]
}

/** The listed models the policy permits, each with what it may carry; the missing and skipped lists stay as they were. */
export function permittedModels(listing: ModelListing, decision: PolicyDecision): PolicyListing {
  const byKey = new Map(decision.permissions.map((permission) => [modelKey(permission.client, permission.model), permission]))
  const models = listing.models.flatMap((model) => {
    const permission = byKey.get(modelKey(model.client, model.model))
    if (permission?.permitted !== true) return []
    return [{ ...model, inline_only: permission.inline_only, may_send: permission.may_send }]
  })
  return { missing: listing.missing, models, skipped: listing.skipped }
}
