import { join } from 'node:path'

import type { ModelFamily } from '../domain/data-policy.js'
import type { Category, Evidence, Severity } from '../domain/findings.js'
import type { RunManifest } from '../domain/manifest.js'
import { assemblePrompt, type PromptHashes } from '../domain/prompt.js'
import type { RosterCritic } from '../domain/routing.js'
import { classifyVibeRun, vibeLaunch, type CriticFailure } from '../domain/vibe.js'
import { asObject, requireString } from './input.js'
import { findingsPath, manifestPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

export interface CriticRun {
  readonly critic_id: string
  readonly client: string
  readonly family: ModelFamily
  readonly model: string
  readonly role: string
  readonly lenses: readonly string[]
  readonly status: 'ok' | 'failed'
  readonly failure?: CriticFailure
  readonly detail?: string
  readonly no_findings_reason?: string
  readonly exit_code: number
  readonly cost_usd: number | null
  readonly duration_ms: number
  readonly finding_count: number
  readonly prompt_hashes: PromptHashes
  readonly lens_hashes: Readonly<Record<string, string>>
  readonly prompt_path: string
  readonly stdout_path: string
  readonly export_path: string
}

/** A finding as the orchestrator receives it: the critic's fields plus who raised it. */
export interface RunFinding {
  readonly id: string
  readonly critic_finding_id: string
  readonly raised_by: readonly string[]
  readonly severity: Severity
  readonly category: Category
  readonly location: string
  readonly claim: string
  readonly evidence: readonly Evidence[]
  readonly suggestion?: string
}

export interface FanOutResult {
  readonly run_id: string
  readonly stage: string
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
    client: critic.client,
    cost_usd: outcome.cost_usd,
    critic_id: critic.critic_id,
    duration_ms: durationMs,
    exit_code: result.exitCode,
    export_path: exportPath,
    family: critic.client_config.family,
    finding_count: findings.length,
    lens_hashes: prompt.lens_hashes,
    lenses: critic.lenses,
    model: critic.client_config.model,
    prompt_hashes: prompt.prompt_hashes,
    prompt_path: paths.promptPath,
    role: critic.role,
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
