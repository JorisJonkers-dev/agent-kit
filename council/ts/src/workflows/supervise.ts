export interface WorkerLivenessReport {
  readonly kind: 'done' | 'waiting' | 'stalled'
  readonly reason?: string
  readonly resumeCondition?: string
  readonly deadline?: string // ISO8601
  readonly monitor?: string
}

export function parseWorkerLiveness(output: string): WorkerLivenessReport {
  if (/STATUS:\s*DONE\s*$/u.test(output)) {
    return { kind: 'done' }
  }

  const waitingMatch =
    /STATUS:\s*WAITING\(reason=([^,)]+),\s*resume-condition=([^,)]+),\s*deadline=([^,)]+)(?:,\s*monitor=([^,)]+))?\)\s*$/u.exec(
      output,
    )

  if (waitingMatch !== null) {
    const reason = waitingMatch[1]?.trim()
    const resumeCondition = waitingMatch[2]?.trim()
    const deadline = waitingMatch[3]?.trim()
    const monitor = waitingMatch[4]?.trim()

    return {
      kind: 'waiting',
      ...(reason !== undefined ? { reason } : {}),
      ...(resumeCondition !== undefined ? { resumeCondition } : {}),
      ...(deadline !== undefined ? { deadline } : {}),
      ...(monitor !== undefined ? { monitor } : {}),
    }
  }

  return { kind: 'stalled' }
}

export interface WatchdogWorkerEntry {
  readonly taskId: string
  readonly lastActivityAt: string // ISO8601
  readonly livenessReport?: WorkerLivenessReport
  readonly monitorName?: string // if waiting, the monitor name being watched
}

export interface WatchdogDecision {
  readonly kind: 'fresh' | 'stale' | 'waiting-with-live-monitor' | 'stalled'
  readonly taskId: string
  readonly reason?: string
}

export function evaluateWatchdog(
  entry: WatchdogWorkerEntry,
  nowMs: number,
  stallWindowMs: number,
  monitorList: readonly { readonly name: string; readonly status: string; readonly dead: boolean }[],
): WatchdogDecision {
  const lastActivityMs = new Date(entry.lastActivityAt).getTime()
  const elapsedMs = nowMs - lastActivityMs

  if (elapsedMs < stallWindowMs) {
    return { kind: 'fresh', taskId: entry.taskId }
  }

  const report = entry.livenessReport
  const monitorName = entry.monitorName ?? report?.monitor

  if (report?.kind === 'waiting' && monitorName !== undefined) {
    if (report.deadline !== undefined && new Date(report.deadline).getTime() <= nowMs) {
      return { kind: 'stalled', taskId: entry.taskId, reason: `deadline expired: ${report.deadline}` }
    }

    const monitor = monitorList.find((m) => m.name === monitorName)

    if (monitor !== undefined && !monitor.dead && monitor.status === 'polling') {
      return { kind: 'waiting-with-live-monitor', taskId: entry.taskId, reason: monitorName }
    }

    return {
      kind: 'stale',
      taskId: entry.taskId,
      reason:
        monitor === undefined
          ? `monitor ${monitorName} not found`
          : `monitor ${monitorName} is ${monitor.dead ? 'dead' : monitor.status}`,
    }
  }

  return { kind: 'stalled', taskId: entry.taskId }
}

export interface AutoNudgeConfig {
  readonly stallWindowMs: number
  readonly maxNudges: number
}

export interface NudgeRecord {
  readonly taskId: string
  readonly nudgeCount: number
  readonly lastNudgeAt: string
}

export function shouldEscalate(record: NudgeRecord, config: AutoNudgeConfig): boolean {
  return record.nudgeCount >= config.maxNudges
}

export interface PollUntilGreenInput {
  readonly sha: string
  readonly repo: string
  readonly monitorName: string
  readonly execDir: string
  readonly intervalMs?: number
  readonly deadlineMs?: number
  readonly finalizer?: string
}

function validateSha(sha: string): void {
  if (!/^[0-9a-f]{7,40}$/iu.test(sha)) throw new Error(`invalid SHA: ${sha}`)
}

function validateRepo(repo: string): void {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/u.test(repo)) throw new Error(`invalid repo: ${repo}`)
}

function msToDurationString(ms: number): string {
  const totalSeconds = Math.round(ms / 1_000)
  const hours = Math.floor(totalSeconds / 3_600)
  const minutes = Math.floor((totalSeconds % 3_600) / 60)
  const seconds = totalSeconds % 60

  if (hours > 0) return `${hours}h`
  if (minutes > 0) return `${minutes}m`
  return `${seconds}s`
}

export function buildPollUntilGreenMonitorArgs(input: PollUntilGreenInput): string[] {
  validateSha(input.sha)
  validateRepo(input.repo)
  const interval = input.intervalMs !== undefined ? msToDurationString(input.intervalMs) : '30s'
  const deadline = input.deadlineMs !== undefined ? msToDurationString(input.deadlineMs) : '45m'

  return [
    'start',
    '--name',
    input.monitorName,
    '--interval',
    interval,
    '--deadline',
    deadline,
    '--cmd',
    `probe:actions-runs-for-sha --sha ${input.sha} --repo ${input.repo} --expected-status success`,
    '--until',
    '"conclusion": "success"',
    '--then',
    input.finalizer ?? '',
    '--exec-dir',
    input.execDir,
  ]
}
