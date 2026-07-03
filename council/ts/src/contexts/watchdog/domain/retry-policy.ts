export type RetryableRetryPolicyDetectionKind =
  | 'output-heartbeat-stall'
  | 'action-heartbeat-stall'
  | 'progress-stall'
  | 'attempt-timeout'
  | 'loop-repeat'
  | 'loop-cycle'

export type NonRetryableRetryPolicyDetectionKind = 'wall-clock-cap' | 'output-cap' | 'disk-cap'

export type RetryPolicyDetectionKind =
  | RetryableRetryPolicyDetectionKind
  | NonRetryableRetryPolicyDetectionKind

export interface RetryPolicyDetection {
  readonly kind: RetryPolicyDetectionKind
  readonly capBytes?: number
  readonly count?: number
  readonly duBytes?: number
  readonly elapsedMs?: number
  readonly gramSize?: number
  readonly idleMs?: number
  readonly lastActionAtMs?: number
  readonly lastOutputAtMs?: number
  readonly lastProgressAtMs?: number
  readonly normalized?: string
  readonly outputBytes?: number
  readonly sequence?: readonly string[]
  readonly timeoutMs?: number
}

export interface RetryPolicyConfig {
  readonly maxAttempts: number
  readonly baseBackoffMs: number
  readonly maxBackoffMs?: number
  readonly jitterRatio?: number
}

export interface RetryPolicyState {
  readonly attempts: number
  readonly failureFingerprints: readonly string[]
}

export interface RetryPolicyInput {
  readonly state: RetryPolicyState
  readonly config: RetryPolicyConfig
  readonly detection: RetryPolicyDetection
  readonly fingerprint: string
  readonly random?: number
}

export type RetryDetectionClassification =
  | {
      readonly kind: 'retryable'
      readonly detectionKind: RetryableRetryPolicyDetectionKind
    }
  | {
      readonly kind: 'non-retryable'
      readonly detectionKind: NonRetryableRetryPolicyDetectionKind
      readonly reason: 'budget-cap' | 'disk-cap'
    }

export type RetryDecision =
  | {
      readonly kind: 'retry'
      readonly state: RetryPolicyState
      readonly classification: Extract<RetryDetectionClassification, { readonly kind: 'retryable' }>
      readonly attempt: number
      readonly delayMs: number
      readonly fingerprint: string
    }
  | {
      readonly kind: 'fail-fast'
      readonly state: RetryPolicyState
      readonly classification: Extract<RetryDetectionClassification, { readonly kind: 'retryable' }>
      readonly reason: 'repeated-fingerprint'
      readonly fingerprint: string
    }
  | {
      readonly kind: 'terminal'
      readonly state: RetryPolicyState
      readonly classification: RetryDetectionClassification
      readonly reason: 'max-attempts-exhausted' | 'non-retryable'
      readonly fingerprint: string
    }

export function createRetryPolicyState(): RetryPolicyState {
  return { attempts: 0, failureFingerprints: [] }
}

export function classifyRetryDetection(
  detection: RetryPolicyDetection,
): RetryDetectionClassification {
  switch (detection.kind) {
    case 'output-heartbeat-stall':
    case 'action-heartbeat-stall':
    case 'progress-stall':
    case 'attempt-timeout':
    case 'loop-repeat':
    case 'loop-cycle':
      return { kind: 'retryable', detectionKind: detection.kind }
    case 'wall-clock-cap':
    case 'output-cap':
      return { kind: 'non-retryable', detectionKind: detection.kind, reason: 'budget-cap' }
    case 'disk-cap':
      return { kind: 'non-retryable', detectionKind: detection.kind, reason: 'disk-cap' }
  }
}

export function decideRetry(input: RetryPolicyInput): RetryDecision {
  const classification = classifyRetryDetection(input.detection)
  const nextState = recordFailure(input.state, input.fingerprint)

  if (classification.kind === 'non-retryable') {
    return {
      kind: 'terminal',
      classification,
      fingerprint: input.fingerprint,
      reason: 'non-retryable',
      state: nextState,
    }
  }

  if (input.state.failureFingerprints.includes(input.fingerprint)) {
    return {
      kind: 'fail-fast',
      classification,
      fingerprint: input.fingerprint,
      reason: 'repeated-fingerprint',
      state: nextState,
    }
  }

  if (nextState.attempts >= input.config.maxAttempts) {
    return {
      kind: 'terminal',
      classification,
      fingerprint: input.fingerprint,
      reason: 'max-attempts-exhausted',
      state: nextState,
    }
  }

  return {
    kind: 'retry',
    attempt: nextState.attempts + 1,
    classification,
    delayMs: retryDelayMs(input.config, nextState.attempts, input.random ?? 0),
    fingerprint: input.fingerprint,
    state: nextState,
  }
}

function recordFailure(state: RetryPolicyState, fingerprint: string): RetryPolicyState {
  const failureFingerprints = state.failureFingerprints.includes(fingerprint)
    ? state.failureFingerprints
    : [...state.failureFingerprints, fingerprint]

  return {
    attempts: state.attempts + 1,
    failureFingerprints,
  }
}

function retryDelayMs(
  config: RetryPolicyConfig,
  failedAttempts: number,
  random: number,
): number {
  const exponentialBackoffMs = config.baseBackoffMs * 2 ** (failedAttempts - 1)
  const cappedBackoffMs =
    config.maxBackoffMs === undefined
      ? exponentialBackoffMs
      : Math.min(exponentialBackoffMs, config.maxBackoffMs)
  const jitterRatio = config.jitterRatio ?? 0
  const jitterMultiplier = 1 - jitterRatio + random * jitterRatio * 2

  return Math.round(cappedBackoffMs * jitterMultiplier)
}
