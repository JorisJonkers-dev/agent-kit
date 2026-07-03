import type { RunTaskView, RunView, RunViewTaskState } from './status.js'

export interface RunStatusRenderOptions {
  readonly color?: boolean
}

type DisplayState = RunViewTaskState | 'ready' | 'skipped'

interface BadgeSpec {
  readonly label: string
  readonly color: AnsiColor
}

interface WaveGroup {
  readonly label: string
  readonly tasks: readonly RunTaskView[]
}

type AnsiColor = 'blue' | 'cyan' | 'gray' | 'green' | 'magenta' | 'red' | 'yellow'

const RESET = '\u001B[0m'

const ANSI_CODES: Readonly<Record<AnsiColor, string>> = {
  blue: '\u001B[34m',
  cyan: '\u001B[36m',
  gray: '\u001B[90m',
  green: '\u001B[32m',
  magenta: '\u001B[35m',
  red: '\u001B[31m',
  yellow: '\u001B[33m',
}

const BADGES: Readonly<Record<DisplayState, BadgeSpec>> = {
  blocked: { color: 'red', label: 'BLOCKED' },
  'budget-cap': { color: 'yellow', label: 'BUDGET' },
  'dead-snapshot': { color: 'red', label: 'DEAD' },
  detected: { color: 'yellow', label: 'DETECTED' },
  'disk-cap': { color: 'yellow', label: 'DISK' },
  exited: { color: 'yellow', label: 'EXITED' },
  failed: { color: 'red', label: 'FAILED' },
  pending: { color: 'gray', label: 'PENDING' },
  ready: { color: 'cyan', label: 'READY' },
  restarting: { color: 'magenta', label: 'RESTART' },
  running: { color: 'blue', label: 'RUNNING' },
  skipped: { color: 'gray', label: 'SKIPPED' },
  'stale-snapshot': { color: 'yellow', label: 'STALE' },
  stalled: { color: 'yellow', label: 'STALLED' },
  stopped: { color: 'yellow', label: 'STOPPED' },
  succeeded: { color: 'green', label: 'OK' },
}

export function renderRunStatusTable(view: RunView, options: RunStatusRenderOptions = {}): string {
  const rows = view.tasks.map((task) => rowFor(task, options, displayStateFor(task)))
  const widths = columnWidths(rows)
  return [
    headerLine(view),
    rollupLine(view),
    activeLine(view),
    ...waveGroups(view).flatMap((group) => renderWaveGroup(group, rows, widths)),
  ].join('\n')
}

export function renderRunStatusJson(view: RunView): string {
  return `${stableStringify(view, 0)}\n`.trimEnd()
}

interface Row {
  readonly badge: string
  readonly details: string
  readonly duration: string
  readonly taskId: string
}

interface Widths {
  readonly badge: number
  readonly duration: number
  readonly taskId: number
}

function headerLine(view: RunView): string {
  return [
    `run ${view.run}`,
    `stage=${stageName(view.state)}`,
    `elapsed=${formatDuration(view.rollup.elapsedMs)}`,
    `started=${view.rollup.startedAt ?? '-'}`,
    `updated=${view.rollup.updatedAt ?? '-'}`,
  ].join(' ')
}

function rollupLine(view: RunView): string {
  return [
    `rollup counts=${renderCounts(view.tasks)}`,
    `ready=${renderList(view.rollup.readySet)}`,
    `critical=${renderList(view.rollup.criticalPath, '>')}`,
  ].join(' ')
}

function activeLine(view: RunView): string {
  const active = ['detected', 'restarting', 'running'] as const
  const parts = active.flatMap((state) => {
    const tasks = view.tasks.filter((task) => task.state === state)
    return tasks.length === 0 ? [] : [`${state}=${tasks.map(activeTaskLabel).join(',')}`]
  })
  return `active ${parts.length === 0 ? '-' : parts.join(' ')}`
}

function renderWaveGroup(group: WaveGroup, rows: readonly Row[], widths: Widths): readonly string[] {
  const byTaskId = new Map(rows.map((row) => [row.taskId, row]))
  return [
    `wave ${group.label}`,
    tableHeader(widths),
    ...group.tasks.map((task) => {
      const row = byTaskId.get(task.taskId)
      return row === undefined ? '' : tableRow(row, widths)
    }),
  ]
}

function tableHeader(widths: Widths): string {
  return [
    pad('badge', widths.badge),
    pad('task', widths.taskId),
    pad('duration', widths.duration),
    'details',
  ].join('  ')
}

function tableRow(row: Row, widths: Widths): string {
  return [
    pad(row.badge, widths.badge),
    pad(row.taskId, widths.taskId),
    pad(row.duration, widths.duration),
    row.details,
  ].join('  ')
}

function rowFor(task: RunTaskView, options: RunStatusRenderOptions, state: DisplayState): Row {
  return {
    badge: badgeFor(state, options),
    details: detailsFor(task),
    duration: formatDuration(task.durationMs),
    taskId: task.taskId,
  }
}

function columnWidths(rows: readonly Row[]): Widths {
  const badgeWidth = Math.max('badge'.length, ...rows.map((row) => visibleLength(row.badge))) + 1
  return {
    badge: badgeWidth,
    duration: Math.max('duration'.length, ...rows.map((row) => row.duration.length)),
    taskId: Math.max('task'.length, ...rows.map((row) => row.taskId.length)),
  }
}

function badgeFor(state: DisplayState, options: RunStatusRenderOptions): string {
  const spec = BADGES[state]
  const label = `[${spec.label}]`
  return options.color === true ? `${ANSI_CODES[spec.color]}${label}${RESET}` : label
}

function detailsFor(task: RunTaskView): string {
  return [
    task.title,
    ...optionalDetail('worker', task.workerId),
    ...optionalDetail('pid', task.pid),
    ...optionalAttempt(task.attempt),
    ...optionalRestarts(task.restarts),
    ...optionalDetail('model', task.modelTier),
    ...optionalDetail('detection', task.lastDetection),
    ...optionalDetail('terminal', task.terminalStatus),
    ...blockedByDetail(task.blockedBy),
  ].join('; ')
}

function optionalDetail(name: string, value: number | string | null): readonly string[] {
  return value === null ? [] : [`${name}=${String(value)}`]
}

function optionalAttempt(attempt: number): readonly string[] {
  return attempt === 0 ? [] : [`attempt=${String(attempt)}`]
}

function optionalRestarts(restarts: number): readonly string[] {
  return restarts === 0 ? [] : [`restarts=${String(restarts)}`]
}

function blockedByDetail(blockedBy: readonly string[]): readonly string[] {
  return blockedBy.length === 0 ? [] : [`blocked-by=${blockedBy.join(',')}`]
}

function waveGroups(view: RunView): readonly WaveGroup[] {
  const byTaskId = new Map<string, RunTaskView>(view.tasks.map((task) => [task.taskId, task]))
  if (view.waves.length === 0) {
    return groupTasks(view.tasks)
  }
  const groupedIds = new Set(view.waves.flatMap((wave) => wave))
  const waveGroupsFromView = view.waves.map((wave, index) => ({
    label: String(index),
    tasks: wave.flatMap((taskId) => {
      const task = byTaskId.get(taskId)
      return task === undefined ? [] : [task]
    }),
  }))
  const ungrouped = view.tasks.filter((task) => !groupedIds.has(task.taskId))
  return ungrouped.length === 0 ? waveGroupsFromView : [...waveGroupsFromView, ...groupTasks(ungrouped)]
}

function groupTasks(tasks: readonly RunTaskView[]): readonly WaveGroup[] {
  const labels = [...new Set(tasks.map((task) => (task.wave === null ? '?' : String(task.wave))))]
  return labels.map((label) => ({
    label,
    tasks: tasks.filter((task) => (task.wave === null ? '?' : String(task.wave)) === label),
  }))
}

function renderCounts(tasks: readonly RunTaskView[]): string {
  const counts = tasks.reduce<Record<string, number>>((accumulator, task) => {
    const state = displayStateFor(task)
    accumulator[state] = (accumulator[state] ?? 0) + 1
    return accumulator
  }, {})
  return Object.keys(counts)
    .sort()
    .map((key) => `${key}:${String(counts[key])}`)
    .join(' ')
}

function displayStateFor(task: RunTaskView): DisplayState {
  if (task.terminalStatus === 'no-op') {
    return 'skipped'
  }
  if (task.state === 'pending' && task.dependenciesSatisfied) {
    return 'ready'
  }
  return task.state
}

function activeTaskLabel(task: RunTaskView): string {
  const marker = task.lastDetection ?? (task.pid === null ? null : `pid=${String(task.pid)}`)
  return marker === null ? task.taskId : `${task.taskId}(${marker})`
}

function renderList(values: readonly string[], separator = ','): string {
  return values.length === 0 ? '-' : values.join(separator)
}

function stageName(value: RunView['state']): string {
  return typeof value.stage === 'string' ? value.stage : JSON.stringify(value)
}

function formatDuration(milliseconds: number): string {
  const totalSeconds = Math.floor(milliseconds / 1000)
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  if (hours > 0) {
    return `${String(hours)}h${padTime(minutes)}m${padTime(seconds)}s`
  }
  if (minutes > 0) {
    return `${String(minutes)}m${padTime(seconds)}s`
  }
  return `${String(seconds)}s`
}

function padTime(value: number): string {
  return value.toString().padStart(2, '0')
}

function pad(value: string, width: number): string {
  return `${value}${' '.repeat(Math.max(0, width - visibleLength(value)))}`
}

function visibleLength(value: string): number {
  const withoutColors = Object.values(ANSI_CODES).reduce((text, code) => text.split(code).join(''), value)
  return withoutColors.split(RESET).join('').length
}

function stableStringify(value: unknown, depth: number): string {
  if (Array.isArray(value)) {
    return stableArray(value, depth)
  }
  if (value !== null && typeof value === 'object') {
    return stableObject(value as Readonly<Record<string, unknown>>, depth)
  }
  return JSON.stringify(value)
}

function stableArray(values: readonly unknown[], depth: number): string {
  if (values.length === 0) {
    return '[]'
  }
  const indent = indentation(depth + 1)
  const closeIndent = indentation(depth)
  return `[\n${values.map((value) => `${indent}${stableStringify(value, depth + 1)}`).join(',\n')}\n${closeIndent}]`
}

function stableObject(record: Readonly<Record<string, unknown>>, depth: number): string {
  const keys = Object.keys(record).sort()
  if (keys.length === 0) {
    return '{}'
  }
  const indent = indentation(depth + 1)
  const closeIndent = indentation(depth)
  return `{\n${keys
    .map((key) => `${indent}${JSON.stringify(key)}: ${stableStringify(record[key] ?? null, depth + 1)}`)
    .join(',\n')}\n${closeIndent}}`
}

function indentation(depth: number): string {
  return '  '.repeat(depth)
}
