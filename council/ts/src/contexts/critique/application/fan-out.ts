import { join } from 'node:path'

import { budgetReached, quotaReached } from '../domain/budget.js'
import { checkNamed, declaredChecks, withDeclaredSeverity, type DeclaredCheck } from '../domain/checks.js'
import { criticIdentity, readCriticRun, type CriticRun } from '../domain/critic.js'
import { checkEvidence, indexPacket, type PacketIndex } from '../domain/evidence.js'
import { readRunFinding, type CriticFinding, type RunFinding } from '../domain/findings.js'
import { JsonFields } from '../domain/json.js'
import { listedFiles, parseManifest, type RunManifest } from '../domain/manifest.js'
import { mergeFindings } from '../domain/merge.js'
import { READ_ONLY_TOOLSET, type CriticToolset } from '../domain/critic-tools.js'
import { assemblePrompt, type CriticPrompt } from '../domain/prompt.js'
import type { RosterCritic } from '../domain/routing.js'
import { CRITIQUE_STAGES, type CritiqueStage } from '../domain/stage.js'
import { FlowBudget } from './budget.js'
import { FINDING_SCHEMA, promptFields, readFrozenPacket, runCritic } from './critic-run.js'
import { CriticTools } from './critic-tools.js'
import { readInput, readJsonFile } from './decode.js'
import { findingsPath, manifestPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
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

/**
 * A finding is judged against the prompt of the critic that raised it first,
 * whose check a merged finding keeps: that check's declared severity, if it
 * declares one, replaces the critic's. The stage's evidence check speaks
 * first; only a finding it leaves to the orchestrator is checked for its
 * check id.
 */
function withPreVerdict(finding: RunFinding, stage: CritiqueStage, packet: PacketIndex, critics: RaisingCritics): RunFinding {
  const raiser = critics.get(finding.raised_by[0] ?? '')
  const declared = raiser?.checks ?? []
  const rated = withDeclaredSeverity(finding, declared)
  const preVerdict = checkEvidence(rated, stage, packet, raiser?.role) ?? checkNamed(rated, declared)
  return preVerdict === undefined ? rated : { ...rated, pre_verdict: preVerdict }
}

/** Each critic's role and the checks its prompt declared, by critic id. */
type RaisingCritics = ReadonlyMap<string, { readonly role: string; readonly checks: readonly DeclaredCheck[] }>

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
    const packet = indexPacket(await readFrozenPacket(this.deps, manifest), manifest.ticket_check?.ticket_ids)
    const budget = new FlowBudget(this.deps)
    const spent = await budget.vibeSpend(manifest)
    const tools = new CriticTools(this.deps, manifest.limits.search, join(runDir, 'search-proof'))
    const first = await Promise.all(manifest.critics.map((critic) => this.attempt({ budget, manifest, spent, tools, tripwire }, critic)))
    const outcomes = [...first, ...(await this.retries({ budget, manifest, tools, tripwire }, first))]
    const merged = mergeFindings(outcomes.map((outcome) => ({ critic_id: outcome.run.critic_id, findings: outcome.findings })))
    const critics: RaisingCritics = new Map(outcomes.map(({ checks, run }) => [run.critic_id, { checks, role: run.role }]))
    const findings = merged.map((finding) => withPreVerdict(finding, manifest.stage, packet, critics))
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

  /**
   * Once the first wave has ended, retries every failed critic that has a
   * fallback left, with the flow's spend and today's agy count read again,
   * since the first wave (and any other session on this machine) has added
   * to them.
   */
  private async retries(wave: Omit<Wave, 'spent'>, first: readonly CriticAttempt[]): Promise<readonly CriticAttempt[]> {
    const { manifest } = wave
    const spent = await wave.budget.vibeSpend(manifest)
    const blocked: Blocked = {
      agy: quotaReached(await wave.budget.agyInvocationsToday(), manifest.limits.antigravity_quota, manifest.stage) !== undefined,
      vibe: budgetReached(spent, manifest.limits) !== undefined,
    }
    const retries = fallbackRetries(manifest, first, blocked)
    return Promise.all(retries.map((retry) => this.attempt({ ...wave, spent }, retry.critic, retry.retryOf)))
  }

  /**
   * Launches one critic, unless it is a Vibe critic and the flow has spent
   * its cap: then it is recorded as a budget skip and never launched.
   */
  private async attempt(wave: Wave, critic: RosterCritic, retryOf?: string): Promise<CriticAttempt> {
    const skip = critic.client_config.kind === 'vibe' ? budgetReached(wave.spent, wave.manifest.limits) : undefined
    const criticDir = join(wave.manifest.run_dir, 'critics', critic.critic_id)
    const { checks, prompt: promptFor } = await this.promptFor(wave.manifest, critic)
    const retry = retryOf === undefined ? {} : { retry_of: retryOf }
    if (skip !== undefined) {
      const prompt = promptFor(READ_ONLY_TOOLSET)
      await this.deps.files.writeText(promptFields(prompt, criticDir).prompt_path, prompt.text)
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
      return { checks, findings: [], run }
    }
    const attempt = await runCritic(this.deps, { critic, criticDir, prompt: promptFor, retry, tools: wave.tools, tripwire: wave.tripwire })
    await wave.budget.logInvocation(wave.manifest, attempt.run, critic.client_config.kind)
    return { ...attempt, checks }
  }

  /** The critic's prompt for the tools it gets, and the checks its role and lenses declare. */
  private async promptFor(
    manifest: RunManifest,
    critic: RosterCritic,
  ): Promise<{ readonly prompt: (toolset: CriticToolset) => CriticPrompt; readonly checks: readonly DeclaredCheck[] }> {
    const asset = (path: string) => this.deps.files.readText(join(this.deps.assetsRoot, path))
    const role = await asset(`roles/${critic.role}.md`)
    const lenses = await Promise.all(critic.lenses.map(async (name) => ({ name, text: await asset(`lenses/${name}.md`) })))
    const sources = {
      contract: await asset('prompts/output-contract.md'),
      lenses,
      preamble: await asset('prompts/preamble.md'),
      role,
      schema: await asset(FINDING_SCHEMA),
    }
    return {
      checks: declaredChecks([role, ...lenses.map((lens) => lens.text)]),
      prompt: (toolset) => assemblePrompt(sources, listedFiles(manifest), manifest.stage, manifest.code?.slices, toolset),
    }
  }
}

/** What one wave of critics shares: the run, its tripwire and budget, and the flow's spend before the wave. */
interface Wave {
  readonly manifest: RunManifest
  readonly tripwire: ReadOnlyTripwire
  readonly budget: FlowBudget
  readonly spent: number
  readonly tools: CriticTools
}

interface CriticAttempt {
  readonly findings: readonly CriticFinding[]
  readonly run: CriticRun
  /** The checks the critic's prompt declared. */
  readonly checks: readonly DeclaredCheck[]
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
/** Which kinds of client may not be launched now: agy past its quota, Vibe past the flow's cap. */
type Blocked = Readonly<Record<RosterCritic['client_config']['kind'], boolean>>

function fallbackRetries(manifest: RunManifest, first: readonly CriticAttempt[], blocked: Blocked): readonly FallbackRetry[] {
  const taken = new Set(first.filter((attempt) => attempt.run.status === 'ok').map((attempt) => attempt.run.family))
  const retries: FallbackRetry[] = []
  manifest.critics.forEach((critic, index) => {
    if (first[index]?.run.status !== 'failed') return
    const eligible = critic.fallbacks.filter(
      (candidate) =>
        !(candidate.client_config.kind === 'agy' && blocked.agy) &&
        (!manifest.distinct_families || !taken.has(candidate.client_config.family)),
    )
    // Past the flow's Vibe cap a fallback that costs no money is preferred;
    // with none, the first fallback is still named, as a budget skip.
    const fallback = eligible.find((candidate) => !(candidate.client_config.kind === 'vibe' && blocked.vibe)) ?? eligible[0]
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
