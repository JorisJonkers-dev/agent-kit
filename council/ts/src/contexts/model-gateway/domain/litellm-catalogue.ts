import { isObject, type GatewayConfig } from './config.js'
import type { ListedModel, ModelListing, SkippedModel } from './models.js'

/**
 * The live LiteLLM model catalogue (#568): what the litellm repository's sync
 * found when it probed every model of every provider route with one tiny
 * request through that route's key. Setup fetches the published
 * `model-catalog.json` into the cache (scripts/litellm_catalogue.py); the
 * gateway reads that copy and never fetches it itself.
 *
 * A client that names a `litellm_route` in its configuration has its listed
 * models judged by that route's probes: only a usable one is offered, a
 * blocked one is skipped with the provider's own reason, and a free one is
 * marked. A model the catalogue does not know is offered as before, since the
 * client's own answer is still the measure of what it accepts.
 */

/** What one probe found for one model. */
export interface CatalogueProbe {
  /** `usable`, `blocked`, `unknown`, `budget-exhausted`, `error`, `not-chat`, `unprobed` or `pending`. */
  readonly status: string
  /** The provider's own text when the probe was refused; empty otherwise. */
  readonly reason: string
  readonly free: boolean
}

export interface LiteLLMCatalogue {
  /** When the sync last probed, as the catalogue says it. */
  readonly updated_at: string
  /** Each provider route's models, by id without the route prefix. */
  readonly routes: ReadonlyMap<string, ReadonlyMap<string, CatalogueProbe>>
}

/** A catalogue older than this means the sync, or setup's fetch, has stopped: two days, against an hourly sync. */
export const CATALOGUE_STALE_AFTER_MS = 48 * 60 * 60 * 1000

/**
 * The catalogue a parsed `model-catalog.json` holds, or why it holds none: an
 * object with an `updated_at` string and a `providers` object whose every
 * entry has a `models` list. A model without a string id is passed over; a
 * missing status is `unknown`, a missing reason is empty, and only `free:
 * true` marks a model free.
 */
export function parseLiteLLMCatalogue(raw: unknown): LiteLLMCatalogue | { readonly problem: string } {
  if (!isObject(raw)) return { problem: 'not a JSON object' }
  if (typeof raw.updated_at !== 'string') return { problem: 'no updated_at' }
  if (!isObject(raw.providers)) return { problem: 'no providers object' }
  const routes = new Map<string, ReadonlyMap<string, CatalogueProbe>>()
  for (const [route, provider] of Object.entries(raw.providers)) {
    if (!isObject(provider) || !Array.isArray(provider.models)) return { problem: `provider ${route} has no models list` }
    const models = new Map<string, CatalogueProbe>()
    for (const model of provider.models) {
      if (!isObject(model) || typeof model.id !== 'string') continue
      const meta = isObject(model.metadata) ? model.metadata : {}
      const id = model.id.startsWith(`${route}/`) ? model.id.slice(route.length + 1) : model.id
      models.set(id, {
        free: meta.free === true,
        reason: typeof meta.reason === 'string' ? meta.reason : '',
        status: typeof meta.status === 'string' ? meta.status : 'unknown',
      })
    }
    routes.set(route, models)
  }
  return { routes, updated_at: raw.updated_at }
}

/** A warning when the catalogue is older than two days, or names no instant; undefined when it is fresh. */
export function catalogueStaleness(catalogue: LiteLLMCatalogue, path: string, nowMs: number): string | undefined {
  const at = Date.parse(catalogue.updated_at)
  if (Number.isNaN(at)) return `the LiteLLM catalogue at ${path} names no updated_at instant, so its age is unknown`
  if (nowMs - at <= CATALOGUE_STALE_AFTER_MS) return undefined
  const hours = Math.floor((nowMs - at) / 3_600_000)
  return `the LiteLLM catalogue at ${path} is stale: updated ${catalogue.updated_at}, ${String(hours)} hours ago; setup refreshes it`
}

/**
 * The probe that judges a client model on its route: the model is looked up
 * by its id and by every name its defaults entry gives it, each without the
 * route prefix (`opencode-go/kimi-k2.7-code` on route `opencode-go` is
 * `kimi-k2.7-code`); an id that names another provider is not looked up. A
 * usable probe under any name wins; else the first probe found judges it;
 * undefined when the catalogue does not know the model at all.
 */
export function probeFor(
  catalogue: LiteLLMCatalogue,
  route: string,
  names: readonly string[],
): CatalogueProbe | undefined {
  const models = catalogue.routes.get(route)
  if (models === undefined) return undefined
  const probes = names.flatMap((name) => {
    const bare = name.startsWith(`${route}/`) ? name.slice(route.length + 1) : name.includes('/') ? undefined : name
    const probe = bare === undefined ? undefined : models.get(bare)
    return probe === undefined ? [] : [probe]
  })
  return probes.find((probe) => probe.status === 'usable') ?? probes[0]
}

/** Why a model the catalogue does not call usable is not offered. */
export function notUsableReason(route: string, probe: CatalogueProbe): string {
  if (probe.status === 'blocked') return `blocked by the provider on LiteLLM route ${route}: ${probe.reason}`
  const reason = probe.reason === '' ? '' : `: ${probe.reason}`
  return `the LiteLLM catalogue marks it ${probe.status} on route ${route}${reason}`
}

/** Every name a client model goes by: its id, and the id and names of the defaults entry it belongs to. */
export function namesOf(config: GatewayConfig, client: string, model: string): readonly string[] {
  const entry = config.models.find(
    (candidate) => candidate.client === client && [candidate.model, ...(candidate.names ?? [])].includes(model),
  )
  return entry === undefined ? [model] : [...new Set([model, entry.model, ...(entry.names ?? [])])]
}

/**
 * The listing with the catalogue applied: each model of a client with a
 * `litellm_route` that the catalogue knows is offered only when usable, then
 * marked `free` when its probe says so; one it calls anything else moves to
 * `skipped` with the reason, and a model already skipped that it refuses
 * takes that reason instead. Without a catalogue the listing is unchanged.
 */
export function judgeListing(config: GatewayConfig, listing: ModelListing, catalogue: LiteLLMCatalogue | undefined): ModelListing {
  if (catalogue === undefined) return listing
  const notUsable = (client: string, id: string): string | undefined => {
    const route = config.clients[client]?.litellm_route
    const probe = route === undefined ? undefined : probeFor(catalogue, route, namesOf(config, client, id))
    return route === undefined || probe === undefined || probe.status === 'usable' ? undefined : notUsableReason(route, probe)
  }
  const models: ListedModel[] = []
  // A model skipped for another reason that the catalogue also refuses is skipped for the provider's reason.
  const skipped: SkippedModel[] = listing.skipped.map((entry) => {
    const reason = notUsable(entry.client, entry.model)
    return reason === undefined ? entry : { ...entry, reason }
  })
  for (const model of listing.models) {
    const reason = notUsable(model.client, model.model)
    if (reason !== undefined) {
      skipped.push({ client: model.client, model: model.model, reason })
      continue
    }
    const route = config.clients[model.client]?.litellm_route
    const free = route !== undefined && probeFor(catalogue, route, namesOf(config, model.client, model.model))?.free === true
    models.push(free ? { ...model, free: true } : model)
  }
  skipped.sort((left, right) => compare(left.client, right.client) || compare(left.model, right.model))
  return { missing: listing.missing, models, skipped }
}

function compare(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0
}
