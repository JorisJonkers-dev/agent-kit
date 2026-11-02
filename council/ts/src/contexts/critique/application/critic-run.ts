import { join } from 'node:path'

import { criticIdentity, type CriticOutcome, type CriticRun } from '../domain/critic.js'
import type { PacketText } from '../domain/evidence.js'
import type { CriticFinding } from '../domain/findings.js'
import type { RunManifest } from '../domain/manifest.js'
import type { CriticPrompt } from '../domain/prompt.js'
import type { RosterCritic } from '../domain/routing.js'
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
    })),
  )
}

export function promptFields(prompt: CriticPrompt, criticDir: string): Pick<CriticRun, 'prompt_path' | 'prompt_hashes' | 'lens_hashes'> {
  return { lens_hashes: prompt.lens_hashes, prompt_hashes: prompt.prompt_hashes, prompt_path: join(criticDir, 'prompt.md') }
}

/**
 * Runs one critic in its own copy of the packet, under the tripwire, through
 * the client of its kind, and keeps its raw output in `criticDir`. The prompt
 * must already be written at its prompt path.
 */
export async function runCritic(
  deps: Pick<CritiqueDeps, 'assetsRoot' | 'clients' | 'clock' | 'files'>,
  launch: {
    readonly tripwire: ReadOnlyTripwire
    readonly critic: RosterCritic
    readonly criticDir: string
    readonly prompt: CriticPrompt
    readonly retry?: Pick<CriticRun, 'retry_of'>
  },
): Promise<CriticLaunchResult> {
  const { critic, criticDir, prompt, tripwire } = launch
  const { prompt_path: promptPath } = promptFields(prompt, criticDir)
  const stdoutPath = join(criticDir, 'stdout.json')

  const packetDir = join(criticDir, 'packet')
  const armed = await tripwire.arm(packetDir)
  const started = deps.clock.monotonicMs()
  const report = await reviewWith(deps.clients, {
    criticDir,
    critic,
    packetDir,
    promptPath,
    schemaPath: join(deps.assetsRoot, FINDING_SCHEMA),
  })
  const durationMs = deps.clock.monotonicMs() - started
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
