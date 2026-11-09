import { readToml } from '../../../shared-kernel/toml.js'
import { isObject, type ClientConfig, type CostClass, type GatewayConfig, type ModelEntry, type ModelSource, type Tier } from './config.js'
import type { LedgerRecord } from './ledger.js'
import { isProviderAllowed, openCodeProvider } from './run.js'

/** One model the gateway can list now, with where its list came from. */
export interface ListedModel {
  readonly client: string
  readonly model: string
  readonly family: string
  readonly cost_class: CostClass
  /** The highest tier the catalogue says it handles; null when the catalogue does not know it. */
  readonly tier: Tier | null
  readonly notes: string
  readonly source: ModelSource
}

/** A client the gateway knows but cannot use on this machine. */
export interface MissingClient {
  readonly client: string
  readonly reason: string
}

/** A model a client listed that the gateway cannot offer, and why. */
export interface SkippedModel {
  readonly client: string
  readonly model: string
  readonly reason: string
}

export interface ModelListing {
  readonly models: readonly ListedModel[]
  readonly missing: readonly MissingClient[]
  readonly skipped: readonly SkippedModel[]
}

/** One model a client listed, and the provider it runs on where the client names one. */
export interface ClientModel {
  /** The id the client accepts, which is what the gateway lists and a delegation names. */
  readonly model: string
  /** Other names the client's own listing gives the model, which the catalogue may know it by. */
  readonly names?: readonly string[]
  readonly provider?: string
}

/** What a client's own listing gave: the models it offers, or why it could not be read. */
export type ClientListing = { readonly models: readonly ClientModel[] } | { readonly reason: string }

/** The key the override's `disabled` list names a model by. */
export function modelKey(client: string, model: string): string {
  return `${client}:${model}`
}

/**
 * The model family a model id names, from its prefix; undefined when the
 * gateway cannot tell, which leaves the model out: the family allowlist
 * decides where a delegation may go, so a model of no known family has no
 * place to go.
 */
const FAMILY_PREFIXES: readonly (readonly [string, string])[] = [
  ['gemini', 'google'],
  ['claude', 'anthropic'],
  ['gpt', 'openai'],
  ['deepseek', 'deepseek'],
  ['glm', 'zhipu'],
  ['zai', 'zhipu'],
  ['kimi', 'moonshot'],
  ['minimax', 'minimax'],
  ['grok', 'xai'],
  ['qwen', 'alibaba'],
  ['mistral', 'mistral'],
  ['codestral', 'mistral'],
]

export function familyOf(model: string): string | undefined {
  return FAMILY_PREFIXES.find(([prefix]) => model.startsWith(prefix))?.[1]
}

/**
 * The family of a model id as a client writes it, which may name its provider
 * first (`anthropic/claude-sonnet-4.6`): the family is the model part's, never
 * the provider's.
 */
export function familyOfModelId(model: string): string | undefined {
  return familyOf(modelPart(model))
}

/**
 * The model ids an `agy models` listing names, in the order it printed them:
 * a `Fetching available models...` line, then one `<id>\t<display name>` per
 * model. A line that names no model is skipped.
 */
export function parseAntigravityModels(stdout: string): readonly string[] {
  const ids: string[] = []
  for (const line of stdout.split('\n')) {
    const [id = '', display] = line.split('\t')
    if (display === undefined) continue
    const trimmed = id.trim()
    if (trimmed !== '') ids.push(trimmed)
  }
  return ids
}

/** The models an `opencode models` listing names, one `<provider>/<model>` per line. */
export function parseOpenCodeModels(stdout: string): readonly ClientModel[] {
  const models: ClientModel[] = []
  for (const line of stdout.split('\n')) {
    const id = line.trim()
    const slash = id.indexOf('/')
    if (slash <= 0 || slash === id.length - 1) continue
    models.push({ model: id, provider: id.slice(0, slash) })
  }
  return models
}

/**
 * The model Hermes' own configuration names as its default, from
 * `hermes config get model --json`: an object with `default` and `provider`,
 * or the model id alone. Hermes has no non-interactive model listing — its
 * `hermes model` is an interactive picker — so its configuration is the only
 * reliable source, and it names one model rather than a list.
 */
export function hermesDefaultModel(stdout: string): ClientModel | undefined {
  let parsed: unknown
  try {
    parsed = JSON.parse(stdout)
  } catch {
    return undefined
  }
  if (typeof parsed === 'string') return parsed === '' ? undefined : { model: parsed }
  if (!isObject(parsed)) return undefined
  const model = typeof parsed.default === 'string' && parsed.default !== '' ? parsed.default : undefined
  if (model === undefined) return undefined
  const provider = typeof parsed.provider === 'string' && parsed.provider !== '' ? parsed.provider : undefined
  return { model, ...(provider === undefined ? {} : { provider }) }
}

/**
 * The models a Vibe `config.toml` declares, in the order it declares them: the
 * alias is what `vibe --model` accepts, and the model's own name is kept
 * beside it when it differs, because the catalogue may know the model by
 * either. An entry that is not a table declares no model. Throws on a file
 * that is not TOML, so the caller never acts on half a file.
 */
export function vibeModels(toml: string): readonly ClientModel[] {
  const models = readToml(toml).models
  if (!isObject(models)) return []
  const declared: ClientModel[] = []
  for (const [key, value] of Object.entries(models)) {
    if (!isObject(value)) continue
    const alias = typeof value.alias === 'string' && value.alias !== '' ? value.alias : key
    const name = typeof value.name === 'string' && value.name !== '' && value.name !== alias ? value.name : undefined
    declared.push({ model: alias, names: name === undefined ? [] : [name] })
  }
  return declared
}

/**
 * Every model the gateway can list now, sorted by client then model id.
 *
 * A client whose program is not on PATH is left out and named in `missing`,
 * whatever its model source. A declared client lists its catalogue entries. A
 * client whose models come from itself or from its configuration lists what
 * `listings` holds for it, joined to the catalogue; a listing that failed is
 * named in `missing` with its reason (Vibe still lists its catalogue entries), and a client the gateway has no listing
 * for lists nothing and is not reported missing. A model `blocked` names — the
 * ledger says the account cannot use it — is left out and named in `skipped`
 * with the reason its last run gave.
 */
export function listModels(
  config: GatewayConfig,
  installed: ReadonlySet<string>,
  listings: ReadonlyMap<string, ClientListing>,
  blocked: readonly BlockedModel[] = [],
): ModelListing {
  const models: ListedModel[] = []
  const missing: MissingClient[] = []
  const skipped: SkippedModel[] = []
  for (const [id, client] of sortedClients(config)) {
    if (!installed.has(id)) {
      missing.push({ client: id, reason: `not installed: ${client.binary} is not on PATH` })
      continue
    }
    if (client.model_source === 'declared') {
      models.push(...catalogueModels(config, id, client))
      continue
    }
    const listing = listings.get(id)
    if (listing === undefined) continue
    if ('reason' in listing) {
      missing.push({ client: id, reason: listing.reason })
      if (BUILT_IN_MODEL_CLIENTS.includes(id)) models.push(...catalogueModels(config, id, client))
      continue
    }
    const listed = listedModels(config, id, client, listing.models)
    models.push(...listed.models)
    skipped.push(...listed.skipped)
    if (BUILT_IN_MODEL_CLIENTS.includes(id)) models.push(...catalogueModels(config, id, client, listed.matched))
  }
  const barred = new Map(blocked.map((entry) => [modelKey(entry.client, entry.model), entry.reason]))
  const offered: ListedModel[] = []
  for (const entry of models) {
    const reason = barred.get(modelKey(entry.client, entry.model))
    if (reason === undefined) offered.push(entry)
    else skipped.push({ client: entry.client, model: entry.model, reason })
  }
  offered.sort((left, right) => compare(left.client, right.client) || compare(left.model, right.model))
  missing.sort((left, right) => compare(left.client, right.client))
  skipped.sort((left, right) => compare(left.client, right.client) || compare(left.model, right.model))
  return { models: offered, missing, skipped }
}

/** One client model the ledger says the account cannot use, and why. */
export interface BlockedModel {
  readonly client: string
  readonly model: string
  readonly reason: string
}

/**
 * The client models the ledger says the account cannot use, as each model's
 * own runs said: a model whose last run answered with the client's quota
 * exhaustion is named until the moment its answer said its quota resets, or
 * until a later run of it succeeded when it named no moment. Anything else a
 * run of the model did — succeed, or fail another way — leaves it listed.
 */
export function blockedModels(records: readonly LedgerRecord[], now: Date): readonly BlockedModel[] {
  const blocked = new Map<string, BlockedModel>()
  for (const record of [...records].sort((left, right) => Date.parse(left.at) - Date.parse(right.at))) {
    const key = modelKey(record.client, record.model)
    blocked.delete(key)
    if (record.outcome === 'ok' || record.reason !== 'quota-exhausted') continue
    if (record.resets_at !== undefined && Date.parse(record.resets_at) <= now.getTime()) continue
    blocked.set(key, {
      client: record.client,
      model: record.model,
      reason: `quota-exhausted at ${record.at}${record.resets_at === undefined ? '' : `; the quota resets at ${record.resets_at}`}`,
    })
  }
  return [...blocked.values()].sort((left, right) => compare(left.client, right.client) || compare(left.model, right.model))
}

/**
 * The clients that offer models of their own beside the ones their
 * configuration declares: Vibe's built-ins, which `config.toml` does not
 * name, are the catalogue's entries for it. Hermes has none; its catalogue
 * entries only describe the model its configuration names.
 */
const BUILT_IN_MODEL_CLIENTS: readonly string[] = ['vibe']

/**
 * The models a client offers as the catalogue describes them, marked
 * `declared`; a catalogue entry in `except` is already listed from the
 * client's own source, which wins, so it is not listed twice.
 */
function catalogueModels(
  config: GatewayConfig,
  id: string,
  client: ClientConfig,
  except: ReadonlySet<ModelEntry> = new Set(),
): readonly ListedModel[] {
  return config.models
    .filter(
      (entry) =>
        entry.client === id &&
        !except.has(entry) &&
        (client.families === undefined || client.families.includes(entry.family)) &&
        ![entry.model, ...(entry.names ?? [])].some((key) => config.disabled.includes(modelKey(id, key))),
    )
    .map((entry) => ({
      client: id,
      cost_class: client.cost_class,
      family: entry.family,
      model: entry.model,
      notes: entry.notes,
      source: 'declared',
      tier: entry.tier,
    }))
}

/**
 * The models a client listed, joined to the catalogue on the listed id or any
 * name the listing gives it (the catalogue knows a model by its id or by its
 * `names`): a model the catalogue knows takes its family, tier and notes from
 * there, and one it does not is still listed, with its family worked out from
 * its id or names and no tier. A model is listed by the id the client
 * accepts, never by a name. A model a listing names twice is listed once. A model of a family
 * the client may not reach is left out; one whose family the gateway cannot
 * work out is reported in `skipped`, so it never vanishes silently. OpenCode
 * models of a provider the client's `providers` allowlist (else `opencode-go`)
 * does not name, and its pay-per-token ones, are dropped without a trace: no
 * run could reach them.
 */
function listedModels(
  config: GatewayConfig,
  id: string,
  client: ClientConfig,
  listed: readonly ClientModel[],
): {
  readonly models: readonly ListedModel[]
  readonly skipped: readonly SkippedModel[]
  /** The catalogue entries a listed model joined to, whether or not it was then offered. */
  readonly matched: ReadonlySet<ModelEntry>
} {
  const models: ListedModel[] = []
  const skipped: SkippedModel[] = []
  const matched = new Set<ModelEntry>()
  const seen = new Set<string>()
  for (const { model, names = [], provider } of listed) {
    if (seen.has(model)) continue
    seen.add(model)
    const runsOn = id === 'opencode' ? (provider ?? openCodeProvider(model)) : provider
    if (id === 'opencode' && !isProviderAllowed(runsOn, client.providers)) continue
    const known = [model, ...names]
    const entry = config.models.find(
      (candidate) => candidate.client === id && [candidate.model, ...(candidate.names ?? [])].some((name) => known.includes(name)),
    )
    const family = entry?.family ?? known.map((name) => familyOf(modelPart(name))).find((found) => found !== undefined)
    const barred = family !== undefined && client.families !== undefined && !client.families.includes(family)
    // A model of a barred family leaves its catalogue entry to the family restriction, which drops it too.
    if (entry !== undefined && !barred) matched.add(entry)
    if ([model, entry?.model, ...(entry?.names ?? [])].some((key) => key !== undefined && config.disabled.includes(modelKey(id, key)))) continue
    if (family === undefined) {
      skipped.push({ client: id, model, reason: FAMILY_UNKNOWN })
      continue
    }
    if (barred) continue
    models.push({
      client: id,
      cost_class: costClassFor(client, runsOn),
      family,
      model,
      notes: entry?.notes ?? '',
      source: client.model_source,
      tier: entry?.tier ?? null,
    })
  }
  return { matched, models, skipped }
}

/** Why a listed model is not offered: nobody has said which family it belongs to. */
const FAMILY_UNKNOWN = 'family unknown: add it to the catalogue in defaults.json or the machine override'

/** The model part of a `<provider>/<model>` id, which is what names its family. */
export function modelPart(model: string): string {
  return model.slice(model.lastIndexOf('/') + 1)
}

/** The cost class of the provider a model runs on, else the client's own. */
export function costClassFor(client: ClientConfig, provider: string | undefined): CostClass {
  if (provider === undefined) return client.cost_class
  return client.provider_cost_classes?.[provider] ?? client.cost_class
}

/** Every client the gateway knows, in client order. */
export function sortedClients(config: GatewayConfig): readonly (readonly [string, ClientConfig])[] {
  return Object.entries(config.clients).sort(([left], [right]) => compare(left, right))
}

function compare(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0
}
