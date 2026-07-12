import type { ProcessPort } from '../ports/process.js'
import { evaluatePredicate, isMonitorDead, parseDuration } from '../contexts/monitor/index.js'
import type { MonitorFsAdapter } from '../contexts/monitor/index.js'
import type { MonitorState, MonitorStatus } from '../contexts/monitor/index.js'

export interface MonitorStartInput {
  readonly name: string
  readonly interval: string
  readonly deadline: string
  readonly cmd: string
  readonly until: string
  readonly then: string
  readonly execDir: string
}

export interface MonitorStatusInput {
  readonly name: string
  readonly execDir: string
}

export interface MonitorListInput {
  readonly execDir: string
}

export interface MonitorStartResult {
  readonly status: MonitorStatus
  readonly lastOutput: string
}

export interface MonitorStatusResult {
  readonly state: MonitorState
}

export interface MonitorListResult {
  readonly monitors: readonly MonitorListEntry[]
}

export interface MonitorListEntry {
  readonly name: string
  readonly status: MonitorStatus
  readonly dead: boolean
  readonly lastTickAt: string
}

export interface MonitorWorkflowDeps {
  readonly process: ProcessPort
  readonly fs: MonitorFsAdapter
  readonly nowIso: () => string
  readonly nowMs: () => number
  readonly sleep: (ms: number) => Promise<void>
  readonly env?: Readonly<Record<string, string>>
}

export async function startMonitor(
  input: MonitorStartInput,
  deps: MonitorWorkflowDeps,
): Promise<MonitorStartResult> {
  const intervalMs = parseDuration(input.interval)
  const deadlineMs = parseDuration(input.deadline)
  const startedAt = deps.nowIso()
  const deadlineIso = new Date(new Date(startedAt).getTime() + deadlineMs).toISOString()

  let state: MonitorState = {
    name: input.name,
    status: 'polling',
    startedAt,
    deadline: deadlineIso,
    lastTickAt: startedAt,
    lastOutput: '',
    intervalMs,
    cmd: input.cmd,
    until: input.until,
    then: input.then,
  }

  await deps.fs.writeState(input.execDir, state)

  for (;;) {
    const nowMs = deps.nowMs()
    const nowIso = deps.nowIso()

    if (nowMs >= new Date(deadlineIso).getTime()) {
      const timedOut: MonitorState = { ...state, status: 'timed-out', lastTickAt: nowIso }
      await deps.fs.writeState(input.execDir, timedOut)
      throw new Error(
        `monitor ${input.name} timed out after ${input.deadline}; last output: ${state.lastOutput}`,
      )
    }

    const probeOutput = await runProbe(input.cmd, input.execDir, deps)
    state = { ...state, lastTickAt: nowIso, lastOutput: probeOutput }
    await deps.fs.writeState(input.execDir, state)

    if (evaluatePredicate(input.until, probeOutput)) {
      const finalizerOutput = await runFinalizer(input.then, input.execDir, deps)
      const passed: MonitorState = {
        ...state,
        status: 'passed',
        lastOutput: finalizerOutput.length > 0 ? finalizerOutput : probeOutput,
      }
      await deps.fs.writeState(input.execDir, passed)
      return { status: 'passed', lastOutput: passed.lastOutput }
    }

    await deps.sleep(intervalMs)
  }
}

export async function monitorStatus(
  input: MonitorStatusInput,
  deps: MonitorWorkflowDeps,
): Promise<MonitorStatusResult> {
  const state = await deps.fs.readState(input.execDir, input.name)
  return { state }
}

export async function monitorList(
  input: MonitorListInput,
  deps: MonitorWorkflowDeps,
): Promise<MonitorListResult> {
  const states = await deps.fs.listStates(input.execDir)
  const nowMs = deps.nowMs()
  const monitors: MonitorListEntry[] = states.map((state) => ({
    name: state.name,
    status: state.status,
    dead: isMonitorDead(state, nowMs),
    lastTickAt: state.lastTickAt,
  }))
  return { monitors }
}

async function runProbe(
  cmd: string,
  execDir: string,
  deps: MonitorWorkflowDeps,
): Promise<string> {
  if (cmd.startsWith('probe:')) {
    return runBuiltinProbe(cmd, deps)
  }
  const result = await deps.process.exec({
    command: 'sh',
    args: ['-c', cmd],
    cwd: execDir,
    ...(deps.env !== undefined ? { env: deps.env } : {}),
  })
  return result.stdout + result.stderr
}

async function runFinalizer(
  cmd: string,
  execDir: string,
  deps: MonitorWorkflowDeps,
): Promise<string> {
  if (cmd.length === 0) return ''
  if (cmd.startsWith('probe:')) {
    return runBuiltinProbe(cmd, deps)
  }
  const result = await deps.process.exec({
    command: 'sh',
    args: ['-c', cmd],
    cwd: execDir,
    ...(deps.env !== undefined ? { env: deps.env } : {}),
  })
  return result.stdout + result.stderr
}

async function runBuiltinProbe(cmd: string, deps: MonitorWorkflowDeps): Promise<string> {
  const parts = cmd.split(/\s+/u)
  /* c8 ignore next -- split always produces at least one element */
  const probeName = parts[0] ?? ''

  if (probeName === 'probe:actions-runs-for-sha') {
    return runActionsRunsForShaProbe(parts, deps)
  }
  if (probeName === 'probe:pr-mergeable') {
    return runPrMergeableProbe(parts, deps)
  }
  if (probeName === 'probe:ghcr-package-visible') {
    return runGhcrPackageVisibleProbe(parts, deps)
  }

  throw new Error(`unknown built-in probe: ${probeName}`)
}

function parseBuiltinFlag(parts: readonly string[], flag: string): string | undefined {
  const idx = parts.indexOf(`--${flag}`)
  if (idx < 0) return undefined
  return parts[idx + 1]
}

async function runActionsRunsForShaProbe(
  parts: readonly string[],
  deps: MonitorWorkflowDeps,
): Promise<string> {
  const sha = parseBuiltinFlag(parts, 'sha')
  const repo = parseBuiltinFlag(parts, 'repo')
  if (sha === undefined) throw new Error('probe:actions-runs-for-sha requires --sha')
  if (repo === undefined) throw new Error('probe:actions-runs-for-sha requires --repo')
  const token = deps.env?.['GITHUB_TOKEN'] ?? process.env['GITHUB_TOKEN']
  if (token === undefined) throw new Error('GITHUB_TOKEN env var is required for probe:actions-runs-for-sha')
  const result = await deps.process.exec({
    command: 'sh',
    args: [
      '-c',
      `curl -sf -H "Authorization: Bearer ${token}" -H "Accept: application/vnd.github+json" "https://api.github.com/repos/${repo}/actions/runs?head_sha=${sha}"`,
    ],
  })
  return result.stdout
}

async function runPrMergeableProbe(
  parts: readonly string[],
  deps: MonitorWorkflowDeps,
): Promise<string> {
  const repo = parseBuiltinFlag(parts, 'repo')
  const pr = parseBuiltinFlag(parts, 'pr')
  if (repo === undefined) throw new Error('probe:pr-mergeable requires --repo')
  if (pr === undefined) throw new Error('probe:pr-mergeable requires --pr')
  const token = deps.env?.['GITHUB_TOKEN'] ?? process.env['GITHUB_TOKEN']
  if (token === undefined) throw new Error('GITHUB_TOKEN env var is required for probe:pr-mergeable')
  const result = await deps.process.exec({
    command: 'sh',
    args: [
      '-c',
      `curl -sf -H "Authorization: Bearer ${token}" -H "Accept: application/vnd.github+json" "https://api.github.com/repos/${repo}/pulls/${pr}"`,
    ],
  })
  return result.stdout
}

async function runGhcrPackageVisibleProbe(
  parts: readonly string[],
  deps: MonitorWorkflowDeps,
): Promise<string> {
  const pkg = parseBuiltinFlag(parts, 'package')
  const version = parseBuiltinFlag(parts, 'version')
  if (pkg === undefined) throw new Error('probe:ghcr-package-visible requires --package')
  if (version === undefined) throw new Error('probe:ghcr-package-visible requires --version')
  const token = deps.env?.['GITHUB_TOKEN'] ?? process.env['GITHUB_TOKEN']
  if (token === undefined) throw new Error('GITHUB_TOKEN env var is required for probe:ghcr-package-visible')
  const result = await deps.process.exec({
    command: 'sh',
    args: [
      '-c',
      `curl -sf -o /dev/null -w "%{http_code}" -H "Authorization: Bearer ${token}" "https://ghcr.io/v2/${pkg}/manifests/${version}"`,
    ],
  })
  return result.stdout.trim() === '200' ? '{"visible":true}' : '{"visible":false}'
}
