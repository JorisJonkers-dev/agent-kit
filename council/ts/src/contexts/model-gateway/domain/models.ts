import { readToml } from '../../../shared-kernel/toml.js'
import { isObject, type ClientConfig, type CostClass, type GatewayConfig, type ModelSource, type Tier } from './config.js'

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
  readonly model: string
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
 * The model aliases a Vibe `config.toml` declares, in the order it declares
 * them: the alias is what `vibe --model` accepts. An entry that is not a table
 * declares no model. Throws on a file that is not TOML, so the caller never
 * acts on half a file.
 */
export function vibeModelAliases(toml: string): readonly string[] {
  const models = readToml(toml).models
  if (!isObject(models)) return []
  const aliases: string[] = []
  for (const [key, value] of Object.entries(models)) {
    if (!isObject(value)) continue
    aliases.push(typeof value.alias === 'string' && value.alias !== '' ? value.alias : key)
  }
  return aliases
}

/**
 * Every model the gateway can list now, sorted by client then model id.
 *
 * A client whose program is not on PATH is left out and named in `missing`,
 * whatever its model source. A declared client lists its catalogue entries. A
 * client whose models come from itself or from its configuration lists what
 * `listings` holds for it, joined to the catalogue; a listing that failed is
 * named in `missing` with its reason, and a client the gateway has no listing
 * for lists nothing and is not reported missing.
 */
export function listModels(
  config: GatewayConfig,
  installed: ReadonlySet<string>,
  listings: ReadonlyMap<string, ClientListing>,
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
      continue
    }
    const listed = listedModels(config, id, client, listing.models)
    models.push(...listed.models)
    skipped.push(...listed.skipped)
  }
  models.sort((left, right) => compare(left.client, right.client) || compare(left.model, right.model))
  missing.sort((left, right) => compare(left.client, right.client))
  skipped.sort((left, right) => compare(left.client, right.client) || compare(left.model, right.model))
  return { models, missing, skipped }
}

/** The models a declared client offers, as the catalogue describes them. */
function catalogueModels(config: GatewayConfig, id: string, client: ClientConfig): readonly ListedModel[] {
  return config.models
    .filter((entry) => entry.client === id && !config.disabled.includes(modelKey(id, entry.model)))
    .map((entry) => ({
      client: id,
      cost_class: client.cost_class,
      family: entry.family,
      model: entry.model,
      notes: entry.notes,
      source: client.model_source,
      tier: entry.tier,
    }))
}

/**
 * The models a client listed, joined to the catalogue on the listed id: a
 * model the catalogue knows takes its family, tier and notes from there, and
 * one it does not is still listed, with its family worked out from its id and
 * no tier. A model a listing names twice is listed once. A model of a family
 * the client may not reach is left out; one whose family the gateway cannot
 * work out is reported in `skipped`, so it never vanishes silently.
 */
function listedModels(
  config: GatewayConfig,
  id: string,
  client: ClientConfig,
  listed: readonly ClientModel[],
): { readonly models: readonly ListedModel[]; readonly skipped: readonly SkippedModel[] } {
  const models: ListedModel[] = []
  const skipped: SkippedModel[] = []
  const seen = new Set<string>()
  for (const { model, provider } of listed) {
    if (seen.has(model)) continue
    seen.add(model)
    if (config.disabled.includes(modelKey(id, model))) continue
    const entry = config.models.find((candidate) => candidate.client === id && candidate.model === model)
    const family = entry?.family ?? familyOf(modelPart(model))
    if (family === undefined) {
      skipped.push({ client: id, model, reason: FAMILY_UNKNOWN })
      continue
    }
    if (client.families !== undefined && !client.families.includes(family)) continue
    models.push({
      client: id,
      cost_class: costClassFor(client, provider),
      family,
      model,
      notes: entry?.notes ?? '',
      source: client.model_source,
      tier: entry?.tier ?? null,
    })
  }
  return { models, skipped }
}

/** Why a listed model is not offered: nobody has said which family it belongs to. */
const FAMILY_UNKNOWN = 'family unknown: add it to the catalogue in defaults.json or the machine override'

/** The model part of a `<provider>/<model>` id, which is what names its family. */
function modelPart(model: string): string {
  return model.slice(model.lastIndexOf('/') + 1)
}

/** The cost class of the provider a model runs on, else the client's own. */
function costClassFor(client: ClientConfig, provider: string | undefined): CostClass {
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
