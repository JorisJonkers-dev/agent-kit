import type { EnvPort, ProcessResult } from '../../../../ports/index.js'
import { vibeConfigPath } from '../../application/locations.js'
import type { ModelGatewayDependencies, ModelGatewayFilesPort, ModelGatewayProcessPort, ModelSourcePort } from '../../application/ports.js'
import {
  hermesDefaultModel,
  parseAntigravityModels,
  parseOpenCodeModels,
  parseVibeAvailable,
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
 * The model id the gateway probes Vibe with: reserved for the gateway, never
 * a model, so Vibe refuses it before it makes any request and its refusal
 * answers with the aliases it accepts.
 */
export const VIBE_PROBE_MODEL = 'model-gateway-probe'

/** How long the probe waits for Vibe's refusal, which arrives within seconds. */
const PROBE_TIMEOUT_MS = 30_000

/**
 * Vibe's own answer, the only measure of what it accepts: the gateway runs
 * `vibe --model <probe id> -p ping`, which Vibe refuses before any request,
 * and reads the aliases from the `Available:` line of its refusal. Vibe gets
 * those aliases from its built-in catalogue too, so the home's
 * `config.toml` never decides what is offered: it is read only to join each
 * accepted alias to the model name the configuration gives it, which the
 * catalogue may know the model by. A probe that cannot run, times out or
 * says nothing the gateway can read leaves Vibe out with the reason — it
 * offers no model rather than one a run would be refused on.
 */
export class VibeModelSource implements ModelSourcePort {
  constructor(
    private readonly files: ModelGatewayFilesPort,
    private readonly env: EnvPort,
    private readonly process: ModelGatewayProcessPort,
  ) {}

  async list(): Promise<ClientListing> {
    const command = `vibe --model ${VIBE_PROBE_MODEL} -p ping`
    let result: ProcessResult
    try {
      result = await this.process.exec({
        args: ['--model', VIBE_PROBE_MODEL, '-p', 'ping'],
        command: 'vibe',
        timeoutMs: PROBE_TIMEOUT_MS,
      })
    } catch (error) {
      return { reason: `listing failed: ${command}: ${messageOf(error)}` }
    }
    if (result.timedOut === true) {
      return { reason: `listing failed: ${command} did not answer within ${String(PROBE_TIMEOUT_MS / 1000)}s` }
    }
    const accepted = parseVibeAvailable(`${result.stderr}\n${result.stdout}`)
    if (accepted === undefined) return { reason: failureReason(command, result) }
    const names = await this.configNames()
    return { models: accepted.map((alias) => ({ model: alias, names: names.get(alias) ?? [] })) }
  }

  /**
   * The name the home's configuration gives each alias it declares, which
   * joins an accepted alias to the catalogue entry that knows the model by
   * that name. Nothing about the configuration can fail the listing — it
   * never decides what is offered — so a home the environment does not name,
   * a file that is not there and one that is not TOML read no names.
   */
  private async configNames(): Promise<ReadonlyMap<string, readonly string[]>> {
    try {
      const path = vibeConfigPath(this.env)
      if (!(await this.files.exists(path))) return new Map()
      return new Map(vibeModels(await this.files.readText(path)).map(({ model, names = [] }) => [model, names]))
    } catch {
      return new Map()
    }
  }
}

/** The clients whose models the gateway reads from the client itself or from its configuration. */
export function modelSources(deps: Pick<ModelGatewayDependencies, 'env' | 'files' | 'process'>): Readonly<Record<string, ModelSourcePort>> {
  return {
    agy: new AntigravityModelSource(deps.process),
    hermes: new HermesModelSource(deps.process),
    opencode: new OpenCodeModelSource(deps.process),
    vibe: new VibeModelSource(deps.files, deps.env, deps.process),
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
