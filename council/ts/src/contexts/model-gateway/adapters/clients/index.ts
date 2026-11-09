import type { ClockPort, EnvPort, ProcessResult } from '../../../../ports/index.js'
import { vibeAvailableCachePath, vibeConfigPath } from '../../application/locations.js'
import type { ModelGatewayDependencies, ModelGatewayFilesPort, ModelGatewayProcessPort, ModelSourcePort } from '../../application/ports.js'
import {
  hermesDefaultModel,
  parseAntigravityModels,
  parseOpenCodeModels,
  parseVibeAvailable,
  VIBE_MODEL_ID,
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
 * The probe's own command, probed with the strictest caps Vibe takes, so
 * even a Vibe that somehow accepted the probe id as a model could buy
 * nothing with it: one turn, one token, no price. The outer timeout already
 * stops a Vibe that never answers, so the probe takes no `--time-limit` of
 * its own — a small one races the refusal the answer is read from.
 */
const PROBE_ARGS: readonly string[] = [
  '--model',
  VIBE_PROBE_MODEL,
  '--max-turns',
  '1',
  '--max-tokens',
  '1',
  '--max-price',
  '0',
  '-p',
  'ping',
]

/** The probe as its reasons name it. */
const PROBE_COMMAND = `vibe --model ${VIBE_PROBE_MODEL} --max-turns 1 --max-tokens 1 --max-price 0 -p ping`

/** How long one probe's answer holds, so the `models` calls inside it cost nothing. */
export const PROBE_CACHE_TTL_MS = 10 * 60 * 1000

/**
 * What one probe's answer is keyed by: the Vibe that gave it, by the file its
 * name on PATH resolves to — through every symlink to the file itself — with
 * what makes that file the one it is: its change time, its size and its
 * inode; and the home's `config.toml` by its path and the SHA-256 of its
 * content, so a retargeted symlink or a rewrite within the same modification
 * time is still another configuration. A file that is not there, or whose
 * stamp cannot be read, has no key but its own: null, which no stamp equals.
 */
export interface VibeProbeKey {
  readonly binary: string | null
  readonly binaryCtimeMs: number | null
  readonly binarySize: number | null
  readonly binaryIno: number | null
  readonly configPath: string | null
  readonly configSha256: string | null
}

/** The shape of the cache file, as it is written and read back. */
interface VibeProbeCacheEntry {
  readonly version: number
  readonly binary: string | null
  readonly binary_ctime_ms: number | null
  readonly binary_size: number | null
  readonly binary_ino: number | null
  readonly config_path: string | null
  readonly config_sha256: string | null
  readonly written_at_ms: number
  readonly models: readonly string[]
}

/** The one cache file shape the gateway reads; a later one re-probes. */
const CACHE_VERSION = 2

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
 *
 * A configuration that declares the probe id as a model of its own would let
 * the probe run a model instead of being refused, so it fails closed before
 * anything is launched: the listing is left unreadable with the reason, and
 * no probe runs at all.
 *
 * The probe costs a Vibe start every time it runs, so its answer is kept for
 * ten minutes beside the Vibe that gave it: before probing, the gateway looks
 * for an answer the same Vibe binary and the same `config.toml` gave within
 * the window, and only probes when there is none. A probe that failed is
 * never kept, so a bad moment does not stick for ten minutes; an answer that
 * does not read — a corrupt file, a key that no longer matches — is a plain
 * miss, and the probe runs.
 */
export class VibeModelSource implements ModelSourcePort {
  constructor(
    private readonly files: ModelGatewayFilesPort,
    private readonly env: EnvPort,
    private readonly process: ModelGatewayProcessPort,
    private readonly clock: ClockPort,
  ) {}

  async list(): Promise<ClientListing> {
    const declared = await this.declaredModels()
    if (declared.clash !== undefined) return { reason: declared.clash }
    const key = await this.probeKey()
    const cache = new VibeAvailableCache(this.files, this.env)
    const cached = await cache.read(key, this.clock.now().getTime())
    if (cached !== undefined) return this.listing(cached, declared.names)
    let result: ProcessResult
    try {
      result = await this.process.exec({
        args: [...PROBE_ARGS],
        command: 'vibe',
        timeoutMs: PROBE_TIMEOUT_MS,
      })
    } catch (error) {
      return { reason: `listing failed: ${PROBE_COMMAND}: ${messageOf(error)}` }
    }
    if (result.timedOut === true) {
      return { reason: `listing failed: ${PROBE_COMMAND} did not answer within ${String(PROBE_TIMEOUT_MS / 1000)}s` }
    }
    const accepted = parseVibeAvailable(`${result.stderr}\n${result.stdout}`)
    if (accepted === undefined) return { reason: failureReason(PROBE_COMMAND, result) }
    // Only an answer worth keeping is kept: a failed probe is never cached.
    await cache.write(key, accepted, this.clock.now().getTime())
    return this.listing(accepted, declared.names)
  }

  private listing(accepted: readonly string[], names: ReadonlyMap<string, readonly string[]>): ClientListing {
    return { models: accepted.map((alias) => ({ model: alias, names: names.get(alias) ?? [] })) }
  }

  /** The probe's cache key, built from the Vibe it would run and the home's configuration. */
  private async probeKey(): Promise<VibeProbeKey> {
    try {
      const configPath = vibeConfigPath(this.env)
      const resolved = await this.process.resolve('vibe')
      const binary = resolved === undefined ? undefined : await this.files.stamp(resolved)
      const config = await this.files.stamp(configPath)
      return {
        binary: binary?.realpath ?? null,
        binaryCtimeMs: binary?.ctimeMs ?? null,
        binarySize: binary?.size ?? null,
        binaryIno: binary?.ino ?? null,
        configPath: config?.realpath ?? configPath,
        configSha256: config?.sha256 ?? null,
      }
    } catch {
      // A machine the environment cannot name a home for has no answer it can
      // keep either: nothing matches, so the probe runs and nothing is written.
      return { binary: null, binaryCtimeMs: null, binarySize: null, binaryIno: null, configPath: null, configSha256: null }
    }
  }

  /**
   * The name the home's configuration gives each alias it declares, which
   * joins an accepted alias to the catalogue entry that knows the model by
   * that name — and the clash, when the configuration declares the probe id
   * itself as a model. Nothing about the configuration can fail the listing —
   * it never decides what is offered — so a home the environment does not
   * name, a file that is not there and one that is not TOML read no names.
   * The one exception is the clash: a configuration that makes the probe id a
   * model has made the probe's refusal impossible to rely on.
   */
  private async declaredModels(): Promise<{ readonly names: ReadonlyMap<string, readonly string[]>; readonly clash: undefined | string }> {
    try {
      const path = vibeConfigPath(this.env)
      if (!(await this.files.exists(path))) return { names: new Map(), clash: undefined }
      const declared = vibeModels(await this.files.readText(path))
      if (declared.some(({ model }) => model === VIBE_PROBE_MODEL)) {
        return {
          clash: `listing failed: ${path} declares the gateway's probe id '${VIBE_PROBE_MODEL}' as a model, so the probe could run it instead of being refused: rename or remove the alias`,
          names: new Map(),
        }
      }
      return {
        names: new Map(declared.map(({ model, names = [] }) => [model, names])),
        clash: undefined,
      }
    } catch {
      return { names: new Map(), clash: undefined }
    }
  }
}

/**
 * What one probe's answer looks like on disk: a file only its owner can read,
 * written in one step so a reader never sees half of it, read before a probe
 * runs and written after one that read. Every failure reads as a miss, so the
 * cache can never make a listing fail: only a probe can do that, and its
 * failures are never written. An answer written in the future — a clock that
 * stepped back — is outside the window too, so it is a miss.
 */
class VibeAvailableCache {
  constructor(
    private readonly files: ModelGatewayFilesPort,
    private readonly env: EnvPort,
  ) {}

  async read(key: VibeProbeKey, now: number): Promise<readonly string[] | undefined> {
    let entry: unknown
    try {
      const path = vibeAvailableCachePath(this.env)
      if (!(await this.files.exists(path))) return undefined
      entry = JSON.parse(await this.files.readText(path))
    } catch {
      return undefined
    }
    if (!isCacheEntry(entry, key, now)) return undefined
    return entry.models
  }

  /** Best effort, because a listing that worked never fails on one that could not be written. */
  async write(key: VibeProbeKey, models: readonly string[], now: number): Promise<void> {
    const entry: VibeProbeCacheEntry = {
      binary: key.binary,
      binary_ctime_ms: key.binaryCtimeMs,
      binary_ino: key.binaryIno,
      binary_size: key.binarySize,
      config_path: key.configPath,
      config_sha256: key.configSha256,
      models,
      version: CACHE_VERSION,
      written_at_ms: now,
    }
    try {
      await this.files.writeTextPrivate(vibeAvailableCachePath(this.env), JSON.stringify(entry))
    } catch {
      // The answer could not be kept; the next call probes again.
    }
  }
}

/** Whether the entry read is the answer of this Vibe, this configuration, inside its window. */
function isCacheEntry(entry: unknown, key: VibeProbeKey, now: number): entry is VibeProbeCacheEntry {
  if (typeof entry !== 'object' || entry === null) return false
  const record = entry as Readonly<Record<string, unknown>>
  if (record.version !== CACHE_VERSION) return false
  if (record.binary !== key.binary) return false
  if (record.binary_ctime_ms !== key.binaryCtimeMs) return false
  if (record.binary_size !== key.binarySize) return false
  if (record.binary_ino !== key.binaryIno) return false
  if (record.config_path !== key.configPath) return false
  if (record.config_sha256 !== key.configSha256) return false
  const writtenAt = record.written_at_ms
  if (typeof writtenAt !== 'number' || !Number.isFinite(writtenAt)) return false
  const age = now - writtenAt
  if (!(age >= 0 && age < PROBE_CACHE_TTL_MS)) return false
  const models = record.models
  return Array.isArray(models) && models.length > 0 && models.every((model) => typeof model === 'string' && VIBE_MODEL_ID.test(model))
}

/** The clients whose models the gateway reads from the client itself or from its configuration. */
export function modelSources(deps: Pick<ModelGatewayDependencies, 'clock' | 'env' | 'files' | 'process'>): Readonly<Record<string, ModelSourcePort>> {
  return {
    agy: new AntigravityModelSource(deps.process),
    hermes: new HermesModelSource(deps.process),
    opencode: new OpenCodeModelSource(deps.process),
    vibe: new VibeModelSource(deps.files, deps.env, deps.process, deps.clock),
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
