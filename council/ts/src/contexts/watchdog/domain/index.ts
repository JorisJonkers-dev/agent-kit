export interface StallDetectorConfig {
  readonly actionHeartbeatAfterMs?: number
  readonly attemptTimeoutMs?: number
  readonly outputCapBytes?: number
  readonly outputHeartbeatAfterMs?: number
  readonly progressAfterMs?: number
  readonly stallAfterS?: number
  readonly wallClockCapMs?: number
}

export interface StallDetectorInput extends StallDetectorConfig {
  readonly heartbeat?: WatchdogHeartbeatSnapshot
  readonly logBytes?: number
  readonly nowMs: number
  readonly outputBytes?: number
}

export interface WatchdogProgressState {
  readonly attemptStartedAtMs: number
  readonly lastActionAtMs: number
  readonly lastOutputAtMs: number
  readonly lastProgressAtMs: number
  readonly outputBytes: number
  readonly startedAtMs: number
}

export type StallDetectorState = WatchdogProgressState

export interface WatchdogHeartbeatSnapshot {
  readonly lastActionAtMs?: number
  readonly lastOutputAtMs?: number
  readonly lastProgressAtMs?: number
  readonly outputBytes?: number
}

export type StallDetection =
  | {
      readonly kind: 'output-heartbeat-stall'
      readonly idleMs: number
      readonly lastOutputAtMs: number
    }
  | {
      readonly kind: 'action-heartbeat-stall'
      readonly idleMs: number
      readonly lastActionAtMs: number
    }
  | {
      readonly kind: 'progress-stall'
      readonly idleMs: number
      readonly lastProgressAtMs: number
    }

export type WatchdogBudgetDetection =
  | {
      readonly kind: 'wall-clock-cap'
      readonly elapsedMs: number
      readonly capMs: number
    }
  | {
      readonly kind: 'output-cap'
      readonly outputBytes: number
      readonly capBytes: number
    }
  | {
      readonly kind: 'attempt-timeout'
      readonly elapsedMs: number
      readonly timeoutMs: number
    }

export type WatchdogProgressDetection = StallDetection | WatchdogBudgetDetection

export type WatchdogProgressInput = StallDetectorInput

export interface StallDetectorResult {
  readonly state: StallDetectorState
  readonly detection: StallDetection | null
}

export interface WatchdogProgressResult {
  readonly state: WatchdogProgressState
  readonly detection: WatchdogProgressDetection | null
}

export interface ActionLine {
  readonly verbatim: string
  readonly normalized: string
}

export interface LoopDetectorConfig {
  readonly windowSize: number
  readonly repeatLimit: number
  readonly maxCycleGram?: number
}

export interface LoopDetectorState {
  readonly actions: readonly ActionLine[]
}

export type LoopDetection =
  | {
      readonly kind: 'loop-repeat'
      readonly normalized: string
      readonly count: number
    }
  | {
      readonly kind: 'loop-cycle'
      readonly gramSize: number
      readonly sequence: readonly string[]
    }

export interface LoopDetectorResult {
  readonly state: LoopDetectorState
  readonly detection: LoopDetection | null
}

export interface DiskUsageCapInput {
  readonly duBytes: number
  readonly capBytes: number
}

export interface DiskUsageCapDetection {
  readonly kind: 'disk-cap'
  readonly duBytes: number
  readonly capBytes: number
}

export type EscalationPhase =
  | 'ready'
  | 'terminated'
  | 'retry-with-preamble'
  | 'tier-escalated'
  | 'stalled'

export type EscalationAction =
  | 'terminate'
  | 'retry-with-preamble'
  | 'escalate-tier'
  | 'stalled'

export interface EscalationState {
  readonly phase: EscalationPhase
}

export interface EscalationConfig {
  readonly enableTierEscalation: boolean
}

export interface EscalationResult {
  readonly state: EscalationState
  readonly action: EscalationAction
}

type WatchdogStatus = 'running' | 'stalled' | 'looping' | 'restarting' | 'escalated'

interface WatchdogState {
  readonly phase: EscalationPhase
  readonly status: WatchdogStatus
  advance(policy: TierEscalationPolicy): WatchdogTransition
  toEscalationState(): EscalationState
}

interface WatchdogTransition {
  readonly state: WatchdogState
  readonly action: EscalationAction
}

interface TierEscalationPolicy {
  afterRetryWithPreamble(): WatchdogTransition
}

interface LoopDetectionPolicy {
  detect(actions: readonly ActionLine[], config: LoopDetectorConfig): LoopDetection | null
}

interface LoopEvaluationState {
  readonly status: 'running' | 'looping'
  readonly detection: LoopDetection | null
}

export function createWatchdogProgressState(
  nowMs: number,
  outputBytes = 0,
): WatchdogProgressState {
  return {
    attemptStartedAtMs: nowMs,
    lastActionAtMs: nowMs,
    lastOutputAtMs: nowMs,
    lastProgressAtMs: nowMs,
    outputBytes,
    startedAtMs: nowMs,
  }
}

export function createStallDetectorState(nowMs: number, logBytes = 0): StallDetectorState {
  return createWatchdogProgressState(nowMs, logBytes)
}

export function evaluateWatchdogProgress(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): WatchdogProgressResult {
  const nextState = applyProgressHeartbeat(state, input)

  return {
    state: nextState,
    detection:
      detectWallClockCap(nextState, input) ??
      detectOutputCap(nextState, input) ??
      detectAttemptTimeout(nextState, input) ??
      detectOutputHeartbeatStall(nextState, input) ??
      detectActionHeartbeatStall(nextState, input) ??
      detectProgressStall(nextState, input),
  }
}

export function evaluateStall(
  state: StallDetectorState,
  input: StallDetectorInput,
): StallDetectorResult {
  const outputBytes = input.outputBytes ?? input.logBytes ?? state.outputBytes
  const outputGrew = outputBytes > state.outputBytes
  const heartbeat = input.heartbeat ?? (outputGrew ? { lastProgressAtMs: input.nowMs } : undefined)
  const progressAfterMs = input.progressAfterMs ?? secondsToMs(input.stallAfterS)

  const stallInput = {
    ...defined('actionHeartbeatAfterMs', input.actionHeartbeatAfterMs),
    ...defined('heartbeat', heartbeat),
    ...defined('outputHeartbeatAfterMs', input.outputHeartbeatAfterMs),
    ...defined('progressAfterMs', progressAfterMs),
    nowMs: input.nowMs,
    outputBytes,
  }
  const nextState = applyProgressHeartbeat(state, stallInput)

  return {
    state: nextState,
    detection:
      detectOutputHeartbeatStall(nextState, stallInput) ??
      detectActionHeartbeatStall(nextState, stallInput) ??
      detectProgressStall(nextState, stallInput),
  }
}

export function createLoopDetectorState(): LoopDetectorState {
  return { actions: [] }
}

export function appendLoopLine(
  state: LoopDetectorState,
  line: string,
  config: LoopDetectorConfig,
): LoopDetectorResult {
  const actions = extractActionLines(line)

  if (actions.length === 0) {
    return { state, detection: null }
  }

  const nextActions = [...state.actions, ...actions].slice(-config.windowSize)
  const nextState = { actions: nextActions }

  return {
    state: nextState,
    detection: classifyLoopDetection(detectLoop(nextActions, config)).detection,
  }
}

export function extractActionLines(line: string): readonly ActionLine[] {
  const trimmed = line.trim()

  if (trimmed.length === 0) {
    return []
  }

  const jsonActions = extractJsonActionLines(trimmed)

  if (jsonActions.length > 0) {
    return jsonActions
  }

  const codexCommand = extractCodexCommand(trimmed)

  return codexCommand === null ? [] : [toActionLine(codexCommand)]
}

export function evaluateDiskUsageCap(input: DiskUsageCapInput): DiskUsageCapDetection | null {
  return input.duBytes > input.capBytes
    ? { kind: 'disk-cap', duBytes: input.duBytes, capBytes: input.capBytes }
    : null
}

function applyProgressHeartbeat(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): WatchdogProgressState {
  const outputBytes = input.heartbeat?.outputBytes ?? input.outputBytes ?? state.outputBytes
  const outputGrew = outputBytes > state.outputBytes

  return {
    attemptStartedAtMs: state.attemptStartedAtMs,
    lastActionAtMs: input.heartbeat?.lastActionAtMs ?? state.lastActionAtMs,
    lastOutputAtMs: input.heartbeat?.lastOutputAtMs ?? (outputGrew ? input.nowMs : state.lastOutputAtMs),
    lastProgressAtMs: input.heartbeat?.lastProgressAtMs ?? state.lastProgressAtMs,
    outputBytes,
    startedAtMs: state.startedAtMs,
  }
}

function detectWallClockCap(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): WatchdogBudgetDetection | null {
  if (input.wallClockCapMs === undefined) {
    return null
  }

  const elapsedMs = input.nowMs - state.startedAtMs

  return elapsedMs >= input.wallClockCapMs
    ? { kind: 'wall-clock-cap', elapsedMs, capMs: input.wallClockCapMs }
    : null
}

function detectOutputCap(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): WatchdogBudgetDetection | null {
  if (input.outputCapBytes === undefined) {
    return null
  }

  return state.outputBytes > input.outputCapBytes
    ? { kind: 'output-cap', outputBytes: state.outputBytes, capBytes: input.outputCapBytes }
    : null
}

function detectAttemptTimeout(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): WatchdogBudgetDetection | null {
  if (input.attemptTimeoutMs === undefined) {
    return null
  }

  const elapsedMs = input.nowMs - state.attemptStartedAtMs

  return elapsedMs >= input.attemptTimeoutMs
    ? { kind: 'attempt-timeout', elapsedMs, timeoutMs: input.attemptTimeoutMs }
    : null
}

function detectOutputHeartbeatStall(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): StallDetection | null {
  if (input.outputHeartbeatAfterMs === undefined) {
    return null
  }

  const idleMs = input.nowMs - state.lastOutputAtMs

  return idleMs >= input.outputHeartbeatAfterMs
    ? { kind: 'output-heartbeat-stall', idleMs, lastOutputAtMs: state.lastOutputAtMs }
    : null
}

function detectActionHeartbeatStall(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): StallDetection | null {
  if (input.actionHeartbeatAfterMs === undefined) {
    return null
  }

  const idleMs = input.nowMs - state.lastActionAtMs

  return idleMs >= input.actionHeartbeatAfterMs
    ? { kind: 'action-heartbeat-stall', idleMs, lastActionAtMs: state.lastActionAtMs }
    : null
}

function detectProgressStall(
  state: WatchdogProgressState,
  input: WatchdogProgressInput,
): StallDetection | null {
  if (input.progressAfterMs === undefined) {
    return null
  }

  const idleMs = input.nowMs - state.lastProgressAtMs

  return idleMs >= input.progressAfterMs
    ? { kind: 'progress-stall', idleMs, lastProgressAtMs: state.lastProgressAtMs }
    : null
}

export function createEscalationState(): EscalationState {
  return runningWatchdogState.toEscalationState()
}

export function advanceEscalation(
  state: EscalationState,
  config: EscalationConfig,
): EscalationResult {
  const transition = watchdogStates[state.phase].advance(tierEscalationPolicy(config))

  return {
    state: transition.state.toEscalationState(),
    action: transition.action,
  }
}

class RunningWatchdogState implements WatchdogState {
  readonly phase = 'ready'
  readonly status = 'running'

  advance(): WatchdogTransition {
    return transition(terminatedRestartingWatchdogState, 'terminate')
  }

  toEscalationState(): EscalationState {
    return { phase: this.phase }
  }
}

class TerminatedRestartingWatchdogState implements WatchdogState {
  readonly phase = 'terminated'
  readonly status = 'restarting'

  advance(): WatchdogTransition {
    return transition(preambleRestartingWatchdogState, 'retry-with-preamble')
  }

  toEscalationState(): EscalationState {
    return { phase: this.phase }
  }
}

class PreambleRestartingWatchdogState implements WatchdogState {
  readonly phase = 'retry-with-preamble'
  readonly status = 'restarting'

  advance(policy: TierEscalationPolicy): WatchdogTransition {
    return policy.afterRetryWithPreamble()
  }

  toEscalationState(): EscalationState {
    return { phase: this.phase }
  }
}

class EscalatedWatchdogState implements WatchdogState {
  readonly phase = 'tier-escalated'
  readonly status = 'escalated'

  advance(): WatchdogTransition {
    return transition(stalledWatchdogState, 'stalled')
  }

  toEscalationState(): EscalationState {
    return { phase: this.phase }
  }
}

class StalledWatchdogState implements WatchdogState {
  readonly phase = 'stalled'
  readonly status = 'stalled'

  advance(): WatchdogTransition {
    return transition(stalledWatchdogState, 'stalled')
  }

  toEscalationState(): EscalationState {
    return { phase: this.phase }
  }
}

class EnabledTierEscalationPolicy implements TierEscalationPolicy {
  afterRetryWithPreamble(): WatchdogTransition {
    return transition(escalatedWatchdogState, 'escalate-tier')
  }
}

class DisabledTierEscalationPolicy implements TierEscalationPolicy {
  afterRetryWithPreamble(): WatchdogTransition {
    return transition(stalledWatchdogState, 'stalled')
  }
}

class RunningLoopEvaluationState implements LoopEvaluationState {
  readonly status = 'running'
  readonly detection = null
}

class LoopingWatchdogState implements LoopEvaluationState {
  readonly status = 'looping'

  constructor(readonly detection: LoopDetection) {}
}

class RepeatLoopDetectionPolicy implements LoopDetectionPolicy {
  detect(actions: readonly ActionLine[], config: LoopDetectorConfig): LoopDetection | null {
    const counts = new Map<string, number>()

    for (const action of actions) {
      const count = (counts.get(action.normalized) ?? 0) + 1

      if (count >= config.repeatLimit) {
        return { kind: 'loop-repeat', normalized: action.normalized, count }
      }

      counts.set(action.normalized, count)
    }

    return null
  }
}

class CycleLoopDetectionPolicy implements LoopDetectionPolicy {
  detect(actions: readonly ActionLine[], config: LoopDetectorConfig): LoopDetection | null {
    const maxGram = Math.min(config.maxCycleGram ?? 5, 5, Math.floor(actions.length / 3))

    for (let gramSize = 1; gramSize <= maxGram; gramSize += 1) {
      const offset = actions.length - gramSize
      const sequence = actions.slice(offset).map((action) => action.verbatim)

      if (matchesCycle(actions, sequence, gramSize)) {
        return { kind: 'loop-cycle', gramSize, sequence }
      }
    }

    return null
  }
}

const runningWatchdogState = new RunningWatchdogState()
const terminatedRestartingWatchdogState = new TerminatedRestartingWatchdogState()
const preambleRestartingWatchdogState = new PreambleRestartingWatchdogState()
const escalatedWatchdogState = new EscalatedWatchdogState()
const stalledWatchdogState = new StalledWatchdogState()
const enabledTierEscalationPolicy = new EnabledTierEscalationPolicy()
const disabledTierEscalationPolicy = new DisabledTierEscalationPolicy()
const runningLoopEvaluationState = new RunningLoopEvaluationState()

const watchdogStates: Readonly<Record<EscalationPhase, WatchdogState>> = {
  ready: runningWatchdogState,
  terminated: terminatedRestartingWatchdogState,
  'retry-with-preamble': preambleRestartingWatchdogState,
  'tier-escalated': escalatedWatchdogState,
  stalled: stalledWatchdogState,
}

const loopDetectionPolicies: readonly LoopDetectionPolicy[] = [
  new RepeatLoopDetectionPolicy(),
  new CycleLoopDetectionPolicy(),
]

function transition(state: WatchdogState, action: EscalationAction): WatchdogTransition {
  return { state, action }
}

function defined<Key extends string, Value>(
  key: Key,
  value: Value | undefined,
): Partial<Record<Key, NonNullable<Value>>> {
  return (value === undefined ? {} : { [key]: value }) as Partial<Record<Key, NonNullable<Value>>>
}

function secondsToMs(seconds: number | undefined): number | undefined {
  return seconds === undefined ? undefined : seconds * 1000
}

function tierEscalationPolicy(config: EscalationConfig): TierEscalationPolicy {
  return config.enableTierEscalation ? enabledTierEscalationPolicy : disabledTierEscalationPolicy
}

function classifyLoopDetection(detection: LoopDetection | null): LoopEvaluationState {
  return detection === null ? runningLoopEvaluationState : new LoopingWatchdogState(detection)
}

function detectLoop(
  actions: readonly ActionLine[],
  config: LoopDetectorConfig,
): LoopDetection | null {
  for (const policy of loopDetectionPolicies) {
    const detection = policy.detect(actions, config)

    if (detection !== null) {
      return detection
    }
  }

  return null
}

function matchesCycle(
  actions: readonly ActionLine[],
  sequence: readonly string[],
  gramSize: number,
): boolean {
  for (let repeat = 2; repeat <= 3; repeat += 1) {
    const offset = actions.length - repeat * gramSize

    for (let index = 0; index < gramSize; index += 1) {
      if (actions[offset + index]?.verbatim !== sequence[index]) {
        return false
      }
    }
  }

  return true
}

function extractJsonActionLines(line: string): readonly ActionLine[] {
  try {
    const parsed: unknown = JSON.parse(line)

    return findToolUses(parsed)
  } catch {
    return []
  }
}

function findToolUses(value: unknown): readonly ActionLine[] {
  if (Array.isArray(value)) {
    return value.flatMap((item) => findToolUses(item))
  }

  if (!isRecord(value)) {
    return []
  }

  const ownToolUse = value.type === 'tool_use' ? [toolUseToActionLine(value)] : []
  const nestedToolUses = Object.values(value).flatMap((item) => findToolUses(item))

  return [...ownToolUse, ...nestedToolUses]
}

function toolUseToActionLine(toolUse: Record<string, unknown>): ActionLine {
  const name = typeof toolUse.name === 'string' ? toolUse.name : 'unknown'
  const input = isRecord(toolUse.input) ? toolUse.input : {}

  return toActionLine(`tool:${name}:${stableStringify(input)}`)
}

function extractCodexCommand(line: string): string | null {
  const match = /^(?:[$>]|exec(?:_command)?[: ]|command[: ])\s*(?<command>.+)$/iu.exec(line)

  return match?.groups?.command?.trim() ?? null
}

function toActionLine(verbatim: string): ActionLine {
  return {
    verbatim,
    normalized: normalizeActionLine(verbatim),
  }
}

function normalizeActionLine(line: string): string {
  return line
    .trim()
    .toLowerCase()
    .replace(/[0-9a-f]{8,}/giu, '<hex>')
    .replace(/\d+/gu, '<n>')
    .replace(/\s+/gu, ' ')
}

function stableStringify(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.map((item) => stableStringify(item)).join(',')}]`
  }

  if (isRecord(value)) {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
      .join(',')}}`
  }

  const serialized: unknown = JSON.stringify(value)
  return typeof serialized === 'string' ? serialized : 'undefined'
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
