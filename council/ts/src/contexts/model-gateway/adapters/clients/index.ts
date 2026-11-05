import type { EnvPort, ProcessResult } from '../../../../ports/index.js'
import { vibeConfigPath } from '../../application/locations.js'
import type { ModelGatewayDependencies, ModelGatewayFilesPort, ModelGatewayProcessPort, ModelSourcePort } from '../../application/ports.js'
import {
  hermesDefaultModel,
  parseAntigravityModels,
  parseOpenCodeModels,
  vibeModels,
  type ClientListing,
} from '../../domain/models.js'

/**
 * Antigravity's own listing: `agy models` prints a `Fetching available
 * models...` line, then one `<id>\t<display name>` per model. The gateway
 * keeps the ids; which families it may offer is the domain's decision.
 */
export class AntigravityModelSource implements ModelSourcePort {
  constructor(private readonly process: ModelGatewayProcessPort) {}

  async list(): Promise<ClientListing> {
    const command = 'agy models'
    let result: ProcessResult
    try {
      result = await this.process.exec({ args: ['models'], command: 'agy' })
    } catch (error) {
      return { reason: `listing failed: ${command}: ${messageOf(error)}` }
    }
    if (result.exitCode !== 0) return { reason: failureReason(command, result) }
    return { models: parseAntigravityModels(result.stdout).map((model) => ({ model })) }
  }
}

/**
 * OpenCode's own listing: `opencode models` prints one `<provider>/<model>`
 * per line, and the provider is what its cost class is set by.
 */
export class OpenCodeModelSource implements ModelSourcePort {
  constructor(private readonly process: ModelGatewayProcessPort) {}

  async list(): Promise<ClientListing> {
    const command = 'opencode models'
    let result: ProcessResult
    try {
      result = await this.process.exec({ args: ['models'], command: 'opencode' })
    } catch (error) {
      return { reason: `listing failed: ${command}: ${messageOf(error)}` }
    }
    if (result.exitCode !== 0) return { reason: failureReason(command, result) }
    return { models: parseOpenCodeModels(result.stdout) }
  }
}

/**
 * Hermes' own configuration: `hermes model` is an interactive picker, so the
 * only non-interactive source is the configuration itself, read through
 * Hermes' own reader — `hermes config get model --json` prints the default
 * model and the provider it runs on. That names one model, not the many
 * providers Hermes can reach; a listing that fails or names no model leaves
 * Hermes out with the reason rather than taking the whole listing down.
 */
export class HermesModelSource implements ModelSourcePort {
  constructor(private readonly process: ModelGatewayProcessPort) {}

  async list(): Promise<ClientListing> {
    const command = 'hermes config get model --json'
    let result: ProcessResult
    try {
      result = await this.process.exec({ args: ['config', 'get', 'model', '--json'], command: 'hermes' })
    } catch (error) {
      return { reason: `listing failed: ${command}: ${messageOf(error)}` }
    }
    if (result.exitCode !== 0) return { reason: failureReason(command, result) }
    const model = hermesDefaultModel(result.stdout)
    if (model === undefined) return { reason: `listing failed: ${command} named no model` }
    return { models: [model] }
  }
}

/**
 * Vibe's own configuration: the models it declares in `config.toml`, listed by
 * the alias `vibe --model` accepts, with the name the configuration gives
 * each. A file that is not there, that cannot be read as TOML, or whose home
 * the environment does not name, leaves Vibe out with the reason rather than
 * taking the whole listing down.
 */
export class VibeModelSource implements ModelSourcePort {
  constructor(
    private readonly files: ModelGatewayFilesPort,
    private readonly env: EnvPort,
  ) {}

  async list(): Promise<ClientListing> {
    let path = '~/.vibe/config.toml'
    try {
      path = vibeConfigPath(this.env)
      if (!(await this.files.exists(path))) return { reason: `no configuration: ${path} is not there` }
      return { models: vibeModels(await this.files.readText(path)) }
    } catch (error) {
      return { reason: `configuration unreadable: ${path}: ${messageOf(error)}` }
    }
  }
}

/** The clients whose models the gateway reads from the client itself or from its configuration. */
export function modelSources(deps: Pick<ModelGatewayDependencies, 'env' | 'files' | 'process'>): Readonly<Record<string, ModelSourcePort>> {
  return {
    agy: new AntigravityModelSource(deps.process),
    hermes: new HermesModelSource(deps.process),
    opencode: new OpenCodeModelSource(deps.process),
    vibe: new VibeModelSource(deps.files, deps.env),
  }
}

/** Why a listing command the gateway ran gave no models, with the client's own first line of complaint. */
function failureReason(command: string, result: ProcessResult): string {
  const detail = result.stderr.trim().split('\n')[0] ?? ''
  const exit = `${command} exited ${String(result.exitCode)}`
  return `listing failed: ${detail === '' ? exit : `${exit}: ${detail}`}`
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}
