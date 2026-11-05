import { parseCritiqueConfig, type GateFailure } from '../domain/gate.js'
import { isJsonObject, JsonFields } from '../domain/json.js'
import { EMPTY_SETUP_RESULTS, parseSetupResults, setupChecklist } from '../domain/setup.js'
import { GATE_DEADLINE_MS } from './gate.js'
import { critiqueConfigPath, gateLogPath, setupResultsPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'
import { RoutingSource } from './routing.js'

/** Why the session-start hook printed nothing. */
export type SessionSilentReason = 'off' | 'disabled' | 'no-remote' | 'nedap' | 'not-opted-in' | 'setup-current' | 'failed-open'

/**
 * What the session-start hook decided. It prints `line` when there is one
 * and nothing otherwise, and exits 0 either way.
 */
export type SessionStartResult =
  | {
      readonly status: 'silent'
      readonly reason: SessionSilentReason
      readonly line: null
      /** Only when it failed open. */
      readonly failure?: GateFailure
      readonly detail?: string
    }
  | {
      readonly status: 'suggest'
      /** Some check never ran on this machine, or one failed the last time it ran. */
      readonly reason: 'setup-missing' | 'setup-failing'
      /** One line suggesting `/flow setup`. */
      readonly line: string
    }

function silent(reason: Exclude<SessionSilentReason, 'failed-open'>): SessionStartResult {
  return { line: null, reason, status: 'silent' }
}

/**
 * The session-start hook (ADR-0002): in a repository opted in to the
 * critique, one line suggesting `/flow setup` while this machine's setup is
 * missing or failing, by the results `/flow setup` recorded; silence
 * otherwise. It reads local files and git only, never launches a CLI, so
 * an input that changed since a check last ran is `/flow setup`'s to find.
 * Like the gates it is inert under the kill switch and outside an opted-in
 * repository, and anything that keeps it from deciding is silent and logged.
 */
export class SessionStartCommand implements CritiqueCommand<SessionStartResult> {
  private readonly repositories: RepositoryGate
  private readonly routing: RoutingSource

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.routing = new RoutingSource(deps)
  }

  async run(raw: unknown): Promise<SessionStartResult> {
    if (this.deps.env.get('CRITIQUE_OFF') === '1') return silent('off')
    const deadline = this.deps.clock
      .sleep(GATE_DEADLINE_MS)
      .then((): SessionStartResult => failedOpen('deadline', `the session-start hook did not decide within ${String(GATE_DEADLINE_MS)} ms`))
    const result = await Promise.race([this.decide(raw).catch((error: unknown) => failedOpen('error', error instanceof Error ? error.message : String(error))), deadline])
    if (result.status === 'silent' && result.reason === 'failed-open') await this.log(raw, result.failure, result.detail)
    return result
  }

  private async decide(raw: unknown): Promise<SessionStartResult> {
    const configPath = critiqueConfigPath(this.deps.env)
    if (await this.deps.files.exists(configPath)) {
      if (parseCritiqueConfig(await this.deps.files.readText(configPath)).disabled) return silent('disabled')
    }
    const cwd = JsonFields.of(raw, 'the session-start payload').string('cwd')
    const repository = await this.repositories.resolve(cwd)
    if (repository.status === 'refused') return silent(repository.reason === 'nedap' ? 'nedap' : 'no-remote')
    const checks = setupChecklist((await this.routing.load()).clients).filter((check) => check.kind !== 'read-only')
    const resultsPath = setupResultsPath(this.deps.env)
    const results = (await this.deps.files.exists(resultsPath)) ? parseSetupResults(await this.deps.files.readText(resultsPath)) : EMPTY_SETUP_RESULTS
    const failing = checks.filter((check) => results.checks[check.id]?.result === 'fail')
    const [first] = failing
    if (first !== undefined) {
      const more = failing.length > 1 ? `, and ${String(failing.length - 1)} more` : ''
      return { line: `Critique: /flow setup's ${first.id} check failed on this machine${more}. Run /flow setup.`, reason: 'setup-failing', status: 'suggest' }
    }
    const never = checks.filter((check) => results.checks[check.id] === undefined).length
    if (never === 0) return silent('setup-current')
    return {
      line: `Critique: /flow setup has not checked this machine yet (${String(never)} of ${String(checks.length)} checks never ran). Run /flow setup.`,
      reason: 'setup-missing',
      status: 'suggest',
    }
  }

  /** Appends one line to the gate log beside the gates' own; a log that cannot be written is left alone. */
  private async log(raw: unknown, failure: GateFailure | undefined, detail: string | undefined): Promise<void> {
    const cwd = isJsonObject(raw) && typeof raw.cwd === 'string' ? raw.cwd : null
    const line = { cwd, detail, failure, gate: 'session-start', recorded_at: this.deps.clock.now().toISOString() }
    try {
      await this.deps.files.appendText(gateLogPath(this.deps.env), `${JSON.stringify(line)}\n`)
    } catch {
      // Nowhere to log; the session starts all the same.
    }
  }
}

function failedOpen(failure: GateFailure, detail: string): SessionStartResult {
  return { detail, failure, line: null, reason: 'failed-open', status: 'silent' }
}
