import type { ClientConfig, CostClass, GatewayConfig, ModelSource, Tier } from './config.js'

/** One model the gateway can list now, with where its list came from. */
export interface ListedModel {
  readonly client: string
  readonly model: string
  readonly family: string
  readonly cost_class: CostClass
  readonly tier: Tier
  readonly notes: string
  readonly source: ModelSource
}

/** A client the gateway knows but cannot use on this machine. */
export interface MissingClient {
  readonly client: string
  readonly reason: string
}

export interface ModelListing {
  readonly models: readonly ListedModel[]
  readonly missing: readonly MissingClient[]
}

/** The key the override's `disabled` list names a model by. */
export function modelKey(client: string, model: string): string {
  return `${client}:${model}`
}

/** The clients whose models the gateway declares itself, in client order. */
export function declaredClients(config: GatewayConfig): readonly (readonly [string, ClientConfig])[] {
  return Object.entries(config.clients)
    .filter(([, client]) => client.model_source === 'declared')
    .sort(([left], [right]) => compare(left, right))
}

/**
 * Every model the gateway can list now, sorted by client then model id.
 *
 * A declared client whose program is not on PATH is left out and named in
 * `missing`. A client whose models come from the client itself or from its
 * configuration lists nothing yet and is not reported missing: the gateway
 * does not know what it offers until it can read it.
 */
export function listModels(config: GatewayConfig, installed: ReadonlySet<string>): ModelListing {
  const models: ListedModel[] = []
  const missing: MissingClient[] = []
  for (const [id, client] of declaredClients(config)) {
    if (!installed.has(id)) {
      missing.push({ client: id, reason: `not installed: ${client.binary} is not on PATH` })
      continue
    }
    for (const entry of config.models) {
      if (entry.client !== id || config.disabled.includes(modelKey(id, entry.model))) continue
      models.push({
        client: id,
        model: entry.model,
        family: entry.family,
        cost_class: client.cost_class,
        tier: entry.tier,
        notes: entry.notes,
        source: client.model_source,
      })
    }
  }
  models.sort((left, right) => compare(left.client, right.client) || compare(left.model, right.model))
  return { models, missing }
}

function compare(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0
}
