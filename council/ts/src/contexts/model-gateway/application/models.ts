import { isObject, ModelGatewayError } from '../domain/config.js'
import { listModels, sortedClients, type ClientListing } from '../domain/models.js'
import { decidePolicy, permittedModels, type PolicyListing } from '../domain/policy.js'
import { loadConfig } from './config.js'
import { overridePath } from './locations.js'
import { resolveScope } from './policy.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** `models` takes the path of the repository the caller works in, when it has one. */
export interface ModelsInput {
  readonly repo?: string
}

export interface ModelsResult extends PolicyListing {
  readonly status: 'ok'
  /** One line saying why the data policy listed what it did. */
  readonly policy: string
}

/**
 * Lists every client model this machine can use now for the repository the
 * caller works in, and the clients it cannot. The data policy decides which
 * of them a delegation may reach and what it may carry.
 */
export class ModelsCommand implements ModelGatewayCommand<ModelsInput, ModelsResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(raw: ModelsInput): Promise<ModelsResult> {
    const input = readModelsInput(raw)
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const installed = new Set<string>()
    for (const [id, client] of sortedClients(config)) {
      if (await this.deps.process.which(client.binary)) installed.add(id)
    }
    const listings = new Map<string, ClientListing>()
    for (const [id, client] of sortedClients(config)) {
      const source = this.deps.sources[id]
      if (source === undefined || !installed.has(id) || client.model_source === 'declared') continue
      listings.set(id, await source.list())
    }
    const listing = listModels(config, installed, listings)
    const decision = decidePolicy(config, await resolveScope(this.deps, input.repo), listing.models)
    return { policy: decision.reason, status: 'ok', ...permittedModels(listing, decision) }
  }
}

function readModelsInput(raw: unknown): ModelsInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  const repo = raw.repo
  if (repo === undefined) return {}
  if (typeof repo !== 'string' || repo === '') throw new ModelGatewayError('invalid-input', 'repo must be a non-empty string')
  return { repo }
}
