import { isObject, ModelGatewayError } from '../domain/config.js'
import { runRecords } from '../domain/ledger.js'
import { listModels, sortedClients, type ClientListing, type SkippedModel } from '../domain/models.js'
import { judgeListing } from '../domain/litellm-catalogue.js'
import { decidePolicy, permittedModels, type PolicyListing } from '../domain/policy.js'
import { listProfiles, type ProfileListing } from '../domain/profiles.js'
import { runClientOf } from '../domain/run.js'
import { standingRefusals } from '../domain/usage.js'
import { loadConfig } from './config.js'
import { readLiteLLMCatalogue } from './litellm-catalogue.js'
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
  /** The delegation profiles, each with the models it leaves for this repository, or why it leaves none. */
  readonly profiles: readonly ProfileListing[]
  /**
   * What is wrong with the LiteLLM catalogue, when a client it judges is
   * installed: missing, unreadable or stale. Absent when nothing is.
   */
  readonly warnings?: readonly string[]
}

/**
 * Lists every client model this machine can use now for the repository the
 * caller works in, and the clients it cannot. The data policy decides which
 * of them a delegation may reach and what it may carry. A client model whose
 * own answer said it cannot run — which the ledger remembers, until the reset
 * the answer named — is not offered: `models` names it in `skipped`.
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
      const source = this.deps.sources[id] ?? (runClientOf(id) === 'pi' ? this.deps.sources.pi : undefined)
      if (source === undefined || !installed.has(id) || client.model_source === 'declared') continue
      listings.set(id, await source.list(id))
    }
    const judged = sortedClients(config).some(([id, client]) => installed.has(id) && (client.litellm_route !== undefined || runClientOf(id) === 'pi'))
    const reading = judged ? await readLiteLLMCatalogue(this.deps) : {}
    const listing = judgeListing(config, listModels(config, installed, listings, await this.unavailable()), reading.catalogue)
    const decision = decidePolicy(config, await resolveScope(this.deps, input.repo), listing.models)
    const permitted = permittedModels(listing, decision)
    return {
      policy: decision.reason,
      status: 'ok',
      ...permitted,
      profiles: listProfiles(config, permitted.models, decision.reason),
      ...(reading.warning === undefined ? {} : { warnings: [reading.warning] }),
    }
  }

  /**
   * The client models this machine cannot run now, as their own answers said:
   * read from the machine-wide ledger. A ledger the gateway cannot read knows
   * nothing, so it lists none and the listing still works.
   */
  private async unavailable(): Promise<readonly SkippedModel[]> {
    try {
      return standingRefusals(runRecords(await this.deps.ledger.read()), this.deps.clock.now())
    } catch {
      return []
    }
  }
}

function readModelsInput(raw: unknown): ModelsInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  const repo = raw.repo
  if (repo === undefined) return {}
  if (typeof repo !== 'string' || repo === '') throw new ModelGatewayError('invalid-input', 'repo must be a non-empty string')
  return { repo }
}
