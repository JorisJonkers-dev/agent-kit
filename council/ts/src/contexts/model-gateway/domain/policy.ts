import type { AllowlistEntry } from './allowlist.js'
import { openCodeProvider } from './run.js'
import type { GatewayConfig } from './config.js'
import { costClassFor, modelKey, type ListedModel, type MissingClient, type ModelListing, type SkippedModel } from './models.js'

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
 * caller declares. A repository with no remote, and one owned by `nedap`, are
 * decided by their own rule rather than by an allowlist entry. Every other
 * repository is decided by its entry — a personal repository without one by
 * the entry derived from the catalogue, with code and without history.
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

/** The cost classes the owner, not an employer, is paying for. */
const PERSONAL_COST_CLASSES = ['subsidised', 'metered-personal']

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
 *
 * A personal repository — every one whose remote its owner controls — that
 * has no recorded entry is given one derived from the catalogue as it stands:
 * every family a subsidised or metered-personal client model serves, with
 * code but without history. The derived entry is never written to the
 * allowlist; an entry the owner records always wins over it.
 */
export function decidePolicy(
  config: GatewayConfig,
  scope: DataScope,
  subjects: readonly PolicySubject[] = config.models,
): PolicyDecision {
  const defaulted = scope.kind === 'repository' && scope.entry === undefined
  const resolved = defaulted ? { ...scope, entry: defaultPersonalEntry(config) } : scope
  const permissions = subjects
    .filter((subject) => !config.disabled.includes(modelKey(subject.client, subject.model)))
    .map((subject) => permission(config, subject, resolved))
  return { permissions, reason: reasonFor(resolved, defaulted) }
}

/**
 * The personal default an unrecorded repository is delegated under: every
 * family the catalogue's own entries serve from a subsidised or
 * metered-personal client model, in catalogue order, without repeats. A model
 * the catalogue prices nowhere has no tier, so the delegate skill will not
 * route to it: the default reaches as far as the catalogue does.
 */
function defaultPersonalEntry(config: GatewayConfig): AllowlistEntry {
  const families: string[] = []
  for (const entry of config.models) {
    const client = config.clients[entry.client]
    if (client === undefined) continue
    const provider = entry.client === 'opencode' ? openCodeProvider(entry.model) : undefined
    if (!PERSONAL_COST_CLASSES.includes(costClassFor(client, provider))) continue
    if (!families.includes(entry.family)) families.push(entry.family)
  }
  return { code: true, families, history: false }
}

function permission(config: GatewayConfig, subject: PolicySubject, scope: DataScope): ModelPermission {
  const base = { client: subject.client, inline_only: false, model: subject.model }
  switch (scope.kind) {
    case 'outside':
      return {
        ...base,
        inline_only: true,
        may_send: NOTHING,
        permitted: PERSONAL_COST_CLASSES.includes(config.clients[subject.client]?.cost_class ?? ''),
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
/** One line saying why the policy decided what it did; `defaulted` marks an entry derived from the catalogue rather than recorded. */
function reasonFor(scope: DataScope, defaulted: boolean): string {
  switch (scope.kind) {
    case 'outside':
      return 'no repository: only inline content, to subsidised and metered-personal clients'
    case 'no-remote':
      return 'the repository has no remote, so nothing is sent'
    case 'nedap':
      return 'a nedap repository: only Codex and Claude Code, with code and history'
    case 'repository':
      return defaulted
        ? `${scope.key} has no allowlist entry: the personal default applies, every subsidised and metered-personal family, with code but no history`
        : `${scope.key}: the entry allows ${
            scope.entry === undefined || scope.entry.families.length === 0 ? 'no family' : scope.entry.families.join(', ')
          }`
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
