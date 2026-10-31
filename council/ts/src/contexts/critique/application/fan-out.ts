import { join } from 'node:path'

import { criticIdentity, readCriticRun, type CriticRun } from '../domain/critic.js'
import { readRunFinding, type RunFinding } from '../domain/findings.js'
import { JsonFields } from '../domain/json.js'
import { parseManifest, type RunManifest } from '../domain/manifest.js'
import { assemblePrompt } from '../domain/prompt.js'
import type { RosterCritic } from '../domain/routing.js'
import { CRITIQUE_STAGES, type CritiqueStage } from '../domain/stage.js'
import { readInput, readJsonFile } from './decode.js'
import { findingsPath, manifestPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'

export interface FanOutResult {
  readonly status: 'reviewed'
  readonly run_id: string
  readonly stage: CritiqueStage
  /** True when no critic of the stage produced a usable review. */
  readonly degraded: boolean
  readonly findings_path: string
  readonly critics: readonly CriticRun[]
  readonly findings: readonly RunFinding[]
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

/** Runs every critic of a prepared run and collects its findings. */
export class FanOutCommand implements CritiqueCommand<FanOutResult> {
  constructor(private readonly deps: CritiqueDeps) {}

  async run(raw: unknown): Promise<FanOutResult> {
    const runDir = readInput(raw, 'fan-out', (fields) => fields.string('run_dir'))
    const manifest = await readJsonFile(this.deps.files, manifestPath(runDir), 'invalid-run', parseManifest)
    const outcomes = await Promise.all(manifest.critics.map((critic) => this.runCritic(manifest, critic)))
    const findings: RunFinding[] = []
    for (const outcome of outcomes) {
      for (const finding of outcome.findings) {
        const { id, ...fields } = finding
        findings.push({ ...fields, critic_finding_id: id, id: `F${String(findings.length + 1)}`, raised_by: [outcome.run.critic_id] })
      }
    }
    const result: FanOutResult = {
      critics: outcomes.map((outcome) => outcome.run),
      degraded: outcomes.every((outcome) => outcome.run.status === 'failed'),
      findings,
      findings_path: findingsPath(runDir),
      run_id: manifest.run_id,
      stage: manifest.stage,
      status: 'reviewed',
    }
    await this.deps.files.writeText(result.findings_path, `${JSON.stringify(result, null, 2)}\n`)
    return result
  }

  private async runCritic(manifest: RunManifest, critic: RosterCritic) {
    const criticDir = join(manifest.run_dir, 'critics', critic.critic_id)
    const asset = (path: string) => this.deps.files.readText(join(this.deps.assetsRoot, path))
    const prompt = assemblePrompt(
      {
        contract: await asset('prompts/output-contract.md'),
        lenses: await Promise.all(critic.lenses.map(async (name) => ({ name, text: await asset(`lenses/${name}.md`) }))),
        preamble: await asset('prompts/preamble.md'),
        role: await asset(`roles/${critic.role}.md`),
        schema: await asset('schemas/finding.schema.json'),
      },
      manifest.packet_files,
    )
    const promptPath = join(criticDir, 'prompt.md')
    const stdoutPath = join(criticDir, 'stdout.json')
    await this.deps.files.writeText(promptPath, prompt.text)

    const started = this.deps.clock.monotonicMs()
    const report = await this.deps.clients[critic.client_config.kind].review({
      criticDir,
      critic,
      packetDir: manifest.packet_dir,
      promptPath,
    })
    const durationMs = this.deps.clock.monotonicMs() - started
    await this.deps.files.writeText(stdoutPath, report.stdout)
    await this.deps.files.writeText(join(criticDir, 'stderr.txt'), report.stderr)
    const { outcome } = report

    const findings = outcome.status === 'ok' ? outcome.findings : []
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
      status: outcome.status,
      stdout_path: stdoutPath,
      ...(outcome.status === 'failed' ? { detail: outcome.detail, failure: outcome.failure } : {}),
      ...(outcome.status === 'ok' && outcome.no_findings_reason !== undefined
        ? { no_findings_reason: outcome.no_findings_reason }
        : {}),
    }
    return { findings, run }
  }
}
