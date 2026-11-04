import { declaredClients, listModels, type ModelListing } from '../domain/models.js'
import { loadConfig } from './config.js'
import { overridePath } from './locations.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/** `models` takes no input yet; a later ticket adds the repository it is asked about. */
export type ModelsInput = Readonly<Record<string, unknown>>

export interface ModelsResult extends ModelListing {
  readonly status: 'ok'
}

/** Lists every client model this machine can use now, and the clients it cannot. */
export class ModelsCommand implements ModelGatewayCommand<ModelsInput, ModelsResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(): Promise<ModelsResult> {
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const installed = new Set<string>()
    for (const [id, client] of declaredClients(config)) {
      if (await this.deps.process.which(client.binary)) installed.add(id)
    }
    return { status: 'ok', ...listModels(config, installed) }
  }
}
