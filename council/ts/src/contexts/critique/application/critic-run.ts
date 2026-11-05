import { join } from 'node:path'

import { criticIdentity, type CriticOutcome, type CriticRun } from '../domain/critic.js'
import type { PacketText } from '../domain/evidence.js'
import type { CriticFinding } from '../domain/findings.js'
import type { RunManifest } from '../domain/manifest.js'
import type { CriticPrompt } from '../domain/prompt.js'
import { toolNames, type CriticToolset } from '../domain/critic-tools.js'
import type { RosterCritic } from '../domain/routing.js'
import type { CriticTools } from './critic-tools.js'
import { reviewWith, type CritiqueDeps } from './ports.js'
import type { ReadOnlyTripwire } from './tripwire.js'

/** The finding schema every critic answers in, under the shipped assets. */
export const FINDING_SCHEMA = 'schemas/finding.schema.json'

/** What one critic launch gave: its findings (none unless its review was usable) and its run. */
export interface CriticLaunchResult {
  readonly findings: readonly CriticFinding[]
  readonly run: CriticRun
}

/** The packet as prepare froze it, read before any critic runs, so no critic can plant a quote. */
export async function readFrozenPacket(deps: Pick<CritiqueDeps, 'files'>, manifest: RunManifest): Promise<readonly PacketText[]> {
  return Promise.all(
    manifest.packet_files.map(async (file) => ({
      kind: file.kind,
      path: file.path,
      text: await deps.files.readText(join(manifest.packet_dir, file.path)),
      ...(file.window === undefined ? {} : { window: file.window }),
    })),
  )
}

export function promptFields(prompt: CriticPrompt, criticDir: string): Pick<CriticRun, 'prompt_path' | 'prompt_hashes' | 'lens_hashes'> {
  return { lens_hashes: prompt.lens_hashes, prompt_hashes: prompt.prompt_hashes, prompt_path: join(criticDir, 'prompt.md') }
}

/**
 * Runs one critic in its own copy of the packet, under the tripwire, through
 * the client of its kind, and keeps its raw output in `criticDir`. Its tools
 * are chosen once its copy exists, and its prompt, built for those tools,
 * is written before it starts. A search server started for it is stopped
 * once it ends, however it ended.
 */
export async function runCritic(
  deps: Pick<CritiqueDeps, 'assetsRoot' | 'clients' | 'clock' | 'files' | 'search'>,
  launch: {
    readonly tripwire: ReadOnlyTripwire
    readonly critic: RosterCritic
    readonly criticDir: string
    /** The critic's prompt, for the tools it has. */
    readonly prompt: (toolset: CriticToolset) => CriticPrompt
    readonly tools: CriticTools
    readonly retry?: Pick<CriticRun, 'retry_of'>
  },
): Promise<CriticLaunchResult> {
  const { critic, criticDir, tripwire } = launch
  const stdoutPath = join(criticDir, 'stdout.json')

  const packetDir = join(criticDir, 'packet')
  const armed = await tripwire.arm(packetDir)
  const choice = await launch.tools.choose(critic, packetDir)
  const { toolset } = choice
  const prompt = launch.prompt(toolset)
  const { prompt_path: promptPath } = promptFields(prompt, criticDir)
  await deps.files.writeText(promptPath, prompt.text)
  const started = deps.clock.monotonicMs()
  const report = await reviewWith(deps.clients, {
    criticDir,
    critic,
    packetDir,
    promptPath,
    schemaPath: join(deps.assetsRoot, FINDING_SCHEMA),
    toolset,
  }).finally(async () => {
    if (toolset.kind === 'vibe-search') await deps.search?.stop(packetDir)
  })
  const durationMs = deps.clock.monotonicMs() - started
  const searchFailure = choice.searchFailure ?? report.searchFailure
  const violations = await armed.check()
  await deps.files.writeText(stdoutPath, report.stdout)
  await deps.files.writeText(join(criticDir, 'stderr.txt'), report.stderr)
  const { outcome } = report

  const findings = outcome.status === 'ok' && violations.length === 0 ? outcome.findings : []
  const run: CriticRun = {
    ...criticIdentity(critic),
    ...promptFields(prompt, criticDir),
    ...launch.retry,
    charged_usd: report.chargedUsd,
    cost_usd: outcome.cost_usd,
    duration_ms: durationMs,
    exit_code: report.exitCode,
    export_path: report.exportPath,
    finding_count: findings.length,
    stdout_path: stdoutPath,
    ...(report.vibeCostUsd === undefined ? {} : { vibe_cost_usd: report.vibeCostUsd }),
    ...(report.tokens === undefined ? {} : { tokens: report.tokens }),
    tools: toolNames(toolset),
    ...(searchFailure === undefined ? {} : { search_failure: searchFailure }),
    ...criticStatus(outcome, violations),
  }
  return { findings, run }
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
