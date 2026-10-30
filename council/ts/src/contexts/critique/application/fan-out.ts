import { join } from 'node:path'

import type { CriticRun } from '../domain/critic.js'
import type { RunFinding } from '../domain/findings.js'
import type { RunManifest } from '../domain/manifest.js'
import { assemblePrompt } from '../domain/prompt.js'
import { criticIdentity, type RosterCritic } from '../domain/routing.js'
import type { CritiqueStage } from '../domain/stage.js'
import { classifyVibeRun, vibeLaunch } from '../domain/vibe.js'
import { asObject, requireString } from './input.js'
import { findingsPath, manifestPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

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

/** Runs every critic of a prepared run and collects its findings. */
export async function fanOut(deps: CritiqueDeps, raw: unknown): Promise<FanOutResult> {
  const runDir = requireString(asObject(raw, 'fan-out input'), 'run_dir')
  const manifest = JSON.parse(await deps.files.readText(manifestPath(runDir))) as RunManifest
  const outcomes = await Promise.all(manifest.critics.map((critic) => runCritic(deps, manifest, critic)))
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
  await deps.files.writeText(result.findings_path, `${JSON.stringify(result, null, 2)}\n`)
  return result
}

async function runCritic(deps: CritiqueDeps, manifest: RunManifest, critic: RosterCritic) {
  const criticDir = join(manifest.run_dir, 'critics', critic.critic_id)
  const asset = (path: string) => deps.files.readText(join(deps.assetsRoot, path))
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
  const paths = {
    exportDir: join(criticDir, 'export'),
    packetDir: manifest.packet_dir,
    promptPath: join(criticDir, 'prompt.md'),
  }
  const exportPath = join(paths.exportDir, 'export.json')
  const stdoutPath = join(criticDir, 'stdout.json')
  await deps.files.writeText(paths.promptPath, prompt.text)

  const started = deps.clock.monotonicMs()
  const result = await deps.process.exec(vibeLaunch(critic, paths))
  const durationMs = deps.clock.monotonicMs() - started
  await deps.files.writeText(stdoutPath, result.stdout)
  await deps.files.writeText(join(criticDir, 'stderr.txt'), result.stderr)
  const outcome = classifyVibeRun({
    exitCode: result.exitCode,
    exported: await readExport(deps, exportPath),
    stderr: result.stderr,
    stdout: result.stdout,
  })

  const findings = outcome.status === 'ok' ? outcome.findings : []
  const run: CriticRun = {
    ...criticIdentity(critic),
    cost_usd: outcome.cost_usd,
    duration_ms: durationMs,
    exit_code: result.exitCode,
    export_path: exportPath,
    finding_count: findings.length,
    lens_hashes: prompt.lens_hashes,
    prompt_hashes: prompt.prompt_hashes,
    prompt_path: paths.promptPath,
    status: outcome.status,
    stdout_path: stdoutPath,
    ...(outcome.status === 'failed' ? { detail: outcome.detail, failure: outcome.failure } : {}),
    ...(outcome.status === 'ok' && outcome.no_findings_reason !== undefined
      ? { no_findings_reason: outcome.no_findings_reason }
      : {}),
  }
  return { findings, run }
}

async function readExport(deps: CritiqueDeps, path: string): Promise<unknown> {
  if (!(await deps.files.exists(path))) return undefined
  try {
    return JSON.parse(await deps.files.readText(path)) as unknown
  } catch {
    return undefined
  }
}
