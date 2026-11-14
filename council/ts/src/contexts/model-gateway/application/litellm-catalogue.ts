import { ModelGatewayError, type GatewayConfig } from '../domain/config.js'
import {
  catalogueStaleness,
  namesOf,
  notUsableReason,
  parseLiteLLMCatalogue,
  probeFor,
  type LiteLLMCatalogue,
} from '../domain/litellm-catalogue.js'
import { liteLLMCataloguePath } from './locations.js'
import type { ModelGatewayDependencies } from './ports.js'

/** The catalogue this machine holds, if it holds a readable one, and what is wrong with it, if anything. */
export interface CatalogueReading {
  readonly catalogue?: LiteLLMCatalogue
  readonly warning?: string
}

/**
 * Reads the live LiteLLM catalogue setup fetched. Nothing about it ever fails
 * a command: a missing or unreadable copy is a warning and no catalogue, so
 * the client models are listed unchecked; a stale one is a warning and still
 * the catalogue, because it is the newest there is.
 */
export async function readLiteLLMCatalogue(deps: ModelGatewayDependencies): Promise<CatalogueReading> {
  let path: string
  try {
    path = liteLLMCataloguePath(deps.env)
  } catch {
    return {}
  }
  const unchecked = 'so the models are listed unchecked'
  let text: string
  try {
    if (!(await deps.files.exists(path))) {
      return { warning: `no LiteLLM catalogue at ${path}: setup fetches it, ${unchecked}` }
    }
    text = await deps.files.readText(path)
  } catch {
    return { warning: `the LiteLLM catalogue at ${path} cannot be read, ${unchecked}` }
  }
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch {
    return { warning: `the LiteLLM catalogue at ${path} is not JSON, ${unchecked}` }
  }
  const parsed = parseLiteLLMCatalogue(raw)
  if ('problem' in parsed) return { warning: `the LiteLLM catalogue at ${path} is not a catalogue (${parsed.problem}), ${unchecked}` }
  const stale = catalogueStaleness(parsed, path, deps.clock.now().getTime())
  return stale === undefined ? { catalogue: parsed } : { catalogue: parsed, warning: stale }
}

/**
 * Refuses a run on a client model the LiteLLM catalogue says cannot be used
 * on its route, with the provider's reason; a model it does not know, a
 * client with no route, and a machine with no readable catalogue all pass.
 */
export async function requireUsable(deps: ModelGatewayDependencies, config: GatewayConfig, client: string, model: string): Promise<void> {
  const route = config.clients[client]?.litellm_route
  if (route === undefined) return
  const { catalogue } = await readLiteLLMCatalogue(deps)
  if (catalogue === undefined) return
  const probe = probeFor(catalogue, route, namesOf(config, client, model))
  if (probe === undefined || probe.status === 'usable') return
  throw new ModelGatewayError('model-not-usable', `${client}:${model} cannot run: ${notUsableReason(route, probe)}`)
}
