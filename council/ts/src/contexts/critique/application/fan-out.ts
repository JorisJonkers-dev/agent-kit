import { join } from 'node:path'

import { budgetReached } from '../domain/budget.js'
import { criticIdentity, readCriticRun, type CriticOutcome, type CriticRun } from '../domain/critic.js'
import { checkEvidence, type PacketText } from '../domain/evidence.js'
import { readRunFinding, type CriticFinding, type RunFinding } from '../domain/findings.js'
import { JsonFields } from '../domain/json.js'
import { parseManifest, type RunManifest } from '../domain/manifest.js'
import { mergeFindings } from '../domain/merge.js'
import { assemblePrompt, type CriticPrompt } from '../domain/prompt.js'
import type { RosterCritic } from '../domain/routing.js'
import { CRITIQUE_STAGES, type CritiqueStage } from '../domain/stage.js'
import { FlowBudget } from './budget.js'
import { readInput, readJsonFile } from './decode.js'
import { findingsPath, manifestPath } from './locations.js'
import { reviewWith, type CritiqueCommand, type CritiqueDeps } from './ports.js'
import { ReadOnlyTripwire } from './tripwire.js'

export interface FanOutResult {
  readonly status: 'reviewed'
  readonly run_id: string
  readonly stage: CritiqueStage
  /** True when no critic of the stage produced a usable review; a critic that violated read-only produced none. */
  readonly degraded: boolean
  readonly findings_path: string
  readonly critics: readonly CriticRun[]
  readonly findings: readonly RunFinding[]
}

function withPreVerdict(finding: RunFinding, stage: CritiqueStage, packet: readonly PacketText[]): RunFinding {
  const preVerdict = checkEvidence(finding, stage, packet)
  return preVerdict === undefined ? finding : { ...finding, pre_verdict: preVerdict }
}

/** Reads `findings.json`, the merged findings fan-out wrote, back for record. */
export function parseFanOutResult(text: string): FanOutResult {
  const file = JsonFields.parse(text)
  return {
    critics: file.list('critics', readCriticRun),
    degraded: file.boolean('degraded'),
    findings: file.list('findings', readRunFinding),
    findings_path: file.string('findings_path'),
    run_id: file.string('run_id'),
    stage: file.oneOf('stage', CRITIQUE_STAGES),
    status: file.literal('status', 'reviewed'),
  }
}

const SCHEMA = 'schemas/finding.schema.json'

/**
 * Runs every critic of a prepared run at once, so a stage takes as long as
 * its slowest critic. Each critic is blind: its prompt is built from the
 * shipped prompt files and the packet's file list alone, and its client
 * starts a fresh session, so it never sees another critic's output, the
 * conversation or an earlier session. Duplicate findings are then merged.
 */
export class FanOutCommand implements CritiqueCommand<FanOutResult> {
  constructor(private readonly deps: CritiqueDeps) {}

  async run(raw: unknown): Promise<FanOutResult> {
    const runDir = readInput(raw, 'fan-out', (fields) => fields.string('run_dir'))
    const manifest = await readJsonFile(this.deps.files, manifestPath(runDir), 'invalid-run', parseManifest)
    const tripwire = await ReadOnlyTripwire.load(this.deps, manifest)
    const packet = await this.readPacket(manifest)
    const budget = new FlowBudget(this.deps)
    const spent = await budget.vibeSpend(manifest)
    const first = await Promise.all(manifest.critics.map((critic) => this.attempt({ budget, manifest, spent, tripwire }, critic)))
    const outcomes = [...first, ...(await this.retries({ budget, manifest, tripwire }, first))]
    const merged = mergeFindings(outcomes.map((outcome) => ({ critic_id: outcome.run.critic_id, findings: outcome.findings })))
    const findings = merged.map((finding) => withPreVerdict(finding, manifest.stage, packet))
    const result: FanOutResult = {
      critics: outcomes.map((outcome) => outcome.run),
      degraded: outcomes.every((outcome) => outcome.run.status !== 'ok'),
      findings,
      findings_path: findingsPath(runDir),
      run_id: manifest.run_id,
      stage: manifest.stage,
      status: 'reviewed',
    }
    await this.deps.files.writeText(result.findings_path, `${JSON.stringify(result, null, 2)}\n`)
    return result
  }

  /** The packet as prepare froze it, read before any critic runs, so no critic can plant a quote. */
  private async readPacket(manifest: RunManifest): Promise<readonly PacketText[]> {
    return Promise.all(
      manifest.packet_files.map(async (file) => ({
        kind: file.kind,
        path: file.path,
        text: await this.deps.files.readText(join(manifest.packet_dir, file.path)),
      })),
    )
  }

  /**
   * Once the first wave has ended, retries every failed critic that
   * has a fallback left, with the flow's spend read again, since the first
   * wave has added to it.
   */
  private async retries(wave: Omit<Wave, 'spent'>, first: readonly CriticAttempt[]): Promise<readonly CriticAttempt[]> {
    const spent = await wave.budget.vibeSpend(wave.manifest)
    const vibeBlocked = budgetReached(spent, wave.manifest.limits) !== undefined
    const retries = fallbackRetries(wave.manifest, first, vibeBlocked)
    return Promise.all(retries.map((retry) => this.attempt({ ...wave, spent }, retry.critic, retry.retryOf)))
  }

  /**
   * Launches one critic, unless it is a Vibe critic and the flow has spent
   * its cap: then it is recorded as a budget skip and never launched.
   */
  private async attempt(wave: Wave, critic: RosterCritic, retryOf?: string): Promise<CriticAttempt> {
    const skip = critic.client_config.kind === 'vibe' ? budgetReached(wave.spent, wave.manifest.limits) : undefined
    const criticDir = join(wave.manifest.run_dir, 'critics', critic.critic_id)
    const prompt = await this.writePrompt(wave.manifest, critic, criticDir)
    const retry = retryOf === undefined ? {} : { retry_of: retryOf }
    if (skip !== undefined) {
      const run: CriticRun = {
        ...criticIdentity(critic),
        ...promptFields(prompt, criticDir),
        ...retry,
        charged_usd: 0,
        cost_usd: null,
        detail: skip,
        duration_ms: 0,
        exit_code: null,
        finding_count: 0,
        status: 'budget-skip',
      }
      return { findings: [], run }
    }
    const attempt = await this.runCritic(wave.tripwire, critic, criticDir, prompt, retry)
    await wave.budget.logInvocation(wave.manifest, attempt.run, critic.client_config.kind)
    return attempt
  }

  private async writePrompt(manifest: RunManifest, critic: RosterCritic, criticDir: string): Promise<CriticPrompt> {
    const asset = (path: string) => this.deps.files.readText(join(this.deps.assetsRoot, path))
    const prompt = assemblePrompt(
      {
        contract: await asset('prompts/output-contract.md'),
        lenses: await Promise.all(critic.lenses.map(async (name) => ({ name, text: await asset(`lenses/${name}.md`) }))),
        preamble: await asset('prompts/preamble.md'),
        role: await asset(`roles/${critic.role}.md`),
        schema: await asset(SCHEMA),
      },
      manifest.packet_files,
    )
    await this.deps.files.writeText(promptFields(prompt, criticDir).prompt_path, prompt.text)
    return prompt
  }

  private async runCritic(
    tripwire: ReadOnlyTripwire,
    critic: RosterCritic,
    criticDir: string,
    prompt: CriticPrompt,
    retry: Pick<CriticRun, 'retry_of'>,
  ): Promise<CriticAttempt> {
    const { prompt_path: promptPath } = promptFields(prompt, criticDir)
    const stdoutPath = join(criticDir, 'stdout.json')

    const packetDir = join(criticDir, 'packet')
    const armed = await tripwire.arm(packetDir)
    const started = this.deps.clock.monotonicMs()
    const report = await reviewWith(this.deps.clients, {
      criticDir,
      critic,
      packetDir,
      promptPath,
      schemaPath: join(this.deps.assetsRoot, SCHEMA),
    })
    const durationMs = this.deps.clock.monotonicMs() - started
    const violations = await armed.check()
    await this.deps.files.writeText(stdoutPath, report.stdout)
    await this.deps.files.writeText(join(criticDir, 'stderr.txt'), report.stderr)
    const { outcome } = report

    const findings = outcome.status === 'ok' && violations.length === 0 ? outcome.findings : []
    const run: CriticRun = {
      ...criticIdentity(critic),
      ...promptFields(prompt, criticDir),
      ...retry,
      charged_usd: report.chargedUsd,
      cost_usd: outcome.cost_usd,
      duration_ms: durationMs,
      exit_code: report.exitCode,
      export_path: report.exportPath,
      finding_count: findings.length,
      stdout_path: stdoutPath,
      ...criticStatus(outcome, violations),
    }
    return { findings, run }
  }
}

/** What one wave of critics shares: the run, its tripwire and budget, and the flow's spend before the wave. */
interface Wave {
  readonly manifest: RunManifest
  readonly tripwire: ReadOnlyTripwire
  readonly budget: FlowBudget
  readonly spent: number
}

function promptFields(prompt: CriticPrompt, criticDir: string): Pick<CriticRun, 'prompt_path' | 'prompt_hashes' | 'lens_hashes'> {
  return { lens_hashes: prompt.lens_hashes, prompt_hashes: prompt.prompt_hashes, prompt_path: join(criticDir, 'prompt.md') }
}

interface CriticAttempt {
  readonly findings: readonly CriticFinding[]
  readonly run: CriticRun
}

interface FallbackRetry {
  readonly critic: RosterCritic
  readonly retryOf: string
}

/**
 * One retry per failed critic, through the first of its fallbacks whose
 * family no critic that reviewed, and no earlier retry, already has (when the
 * stage requires distinct families). A critic that violated read-only gets
 * none: a client that wrote once may write again. A retry runs with the
 * failed critic's role, lenses, cap and time limit, under its own critic id,
 * so it has its own packet copy and tripwire, and it has no fallbacks of its
 * own.
 */
function fallbackRetries(manifest: RunManifest, first: readonly CriticAttempt[], vibeBlocked: boolean): readonly FallbackRetry[] {
  const taken = new Set(first.filter((attempt) => attempt.run.status === 'ok').map((attempt) => attempt.run.family))
  const retries: FallbackRetry[] = []
  manifest.critics.forEach((critic, index) => {
    if (first[index]?.run.status !== 'failed') return
    const eligible = critic.fallbacks.filter((candidate) => !manifest.distinct_families || !taken.has(candidate.client_config.family))
    // Past the flow's Vibe cap a fallback that costs no money is preferred;
    // with none, the first fallback is still named, as a budget skip.
    const fallback = eligible.find((candidate) => !vibeBlocked || candidate.client_config.kind !== 'vibe') ?? eligible[0]
    if (fallback === undefined) return
    taken.add(fallback.client_config.family)
    retries.push({
      critic: {
        ...critic,
        client: fallback.client,
        client_config: fallback.client_config,
        critic_id: `${critic.critic_id}-retry-${fallback.client}`,
        fallbacks: [],
      },
      retryOf: critic.critic_id,
    })
  })
  return retries
}

/**
 * A critic that changed what it may only read gave no review, whatever its
 * output says, so the read-only violation outranks every other outcome.
 */
function criticStatus(outcome: CriticOutcome, violations: readonly string[]): Pick<CriticRun, 'status' | 'failure' | 'detail' | 'no_findings_reason'> {
  if (violations.length > 0) return { detail: `read-only violated: ${violations.join('; ')}`, status: 'violated-readonly' }
  if (outcome.status === 'failed') return { detail: outcome.detail, failure: outcome.failure, status: 'failed' }
  return { status: 'ok', ...(outcome.no_findings_reason === undefined ? {} : { no_findings_reason: outcome.no_findings_reason }) }
}
