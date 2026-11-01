import { join } from 'node:path'

import { criticIdentity, readCriticRun, type CriticOutcome, type CriticRun } from '../domain/critic.js'
import { checkEvidence, type PacketText } from '../domain/evidence.js'
import { readRunFinding, type RunFinding } from '../domain/findings.js'
import { JsonFields } from '../domain/json.js'
import { listedFiles, parseManifest, type RunManifest } from '../domain/manifest.js'
import { mergeFindings } from '../domain/merge.js'
import { assemblePrompt } from '../domain/prompt.js'
import type { RosterCritic } from '../domain/routing.js'
import { CRITIQUE_STAGES, type CritiqueStage } from '../domain/stage.js'
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
    const outcomes = await Promise.all(manifest.critics.map((critic) => this.runCritic(manifest, tripwire, critic)))
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

  private async runCritic(manifest: RunManifest, tripwire: ReadOnlyTripwire, critic: RosterCritic) {
    const criticDir = join(manifest.run_dir, 'critics', critic.critic_id)
    const asset = (path: string) => this.deps.files.readText(join(this.deps.assetsRoot, path))
    const prompt = assemblePrompt(
      {
        contract: await asset('prompts/output-contract.md'),
        lenses: await Promise.all(critic.lenses.map(async (name) => ({ name, text: await asset(`lenses/${name}.md`) }))),
        preamble: await asset('prompts/preamble.md'),
        role: await asset(`roles/${critic.role}.md`),
        schema: await asset(SCHEMA),
      },
      listedFiles(manifest),
      manifest.code !== undefined,
    )
    const promptPath = join(criticDir, 'prompt.md')
    const stdoutPath = join(criticDir, 'stdout.json')
    await this.deps.files.writeText(promptPath, prompt.text)

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
      cost_usd: outcome.cost_usd,
      duration_ms: durationMs,
      exit_code: report.exitCode,
      export_path: report.exportPath,
      finding_count: findings.length,
      lens_hashes: prompt.lens_hashes,
      prompt_hashes: prompt.prompt_hashes,
      prompt_path: promptPath,
      stdout_path: stdoutPath,
      ...criticStatus(outcome, violations),
    }
    return { findings, run }
  }
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
