import { describe, expect, it } from 'vitest'

import {
  classifyRetryDetection,
  createRetryPolicyState,
  decideRetry,
  type RetryPolicyDetectionKind,
} from './retry-policy.js'

describe('retry detection classification', () => {
  it.each<RetryPolicyDetectionKind>([
    'output-heartbeat-stall',
    'action-heartbeat-stall',
    'progress-stall',
    'attempt-timeout',
    'loop-repeat',
    'loop-cycle',
  ])('classifies %s as retryable', (kind) => {
    expect(classifyRetryDetection({ kind })).toEqual({
      kind: 'retryable',
      detectionKind: kind,
    })
  })

  it.each([
    ['wall-clock-cap', 'budget-cap'],
    ['output-cap', 'budget-cap'],
    ['disk-cap', 'disk-cap'],
  ] as const)('classifies %s as non-retryable', (kind, reason) => {
    expect(classifyRetryDetection({ kind })).toEqual({
      kind: 'non-retryable',
      detectionKind: kind,
      reason,
    })
  })
})

describe('retry policy decisions', () => {
  it('retries retryable detections with deterministic capped jittered backoff', () => {
    const result = decideRetry({
      config: {
        baseBackoffMs: 2_000,
        jitterRatio: 0.25,
        maxAttempts: 4,
        maxBackoffMs: 1_500,
      },
      detection: { kind: 'progress-stall', idleMs: 9_000 },
      fingerprint: 'progress:last=1000',
      random: 1,
      state: createRetryPolicyState(),
    })

    expect(result).toEqual({
      kind: 'retry',
      attempt: 2,
      classification: {
        kind: 'retryable',
        detectionKind: 'progress-stall',
      },
      delayMs: 1_875,
      fingerprint: 'progress:last=1000',
      state: {
        attempts: 1,
        failureFingerprints: ['progress:last=1000'],
      },
    })
  })

  it('uses deterministic defaults when jitter and random values are omitted', () => {
    expect(
      decideRetry({
        config: { baseBackoffMs: 250, maxAttempts: 3 },
        detection: { kind: 'attempt-timeout', elapsedMs: 1_000 },
        fingerprint: 'attempt-timeout:1',
        state: createRetryPolicyState(),
      }),
    ).toEqual({
      kind: 'retry',
      attempt: 2,
      classification: {
        kind: 'retryable',
        detectionKind: 'attempt-timeout',
      },
      delayMs: 250,
      fingerprint: 'attempt-timeout:1',
      state: {
        attempts: 1,
        failureFingerprints: ['attempt-timeout:1'],
      },
    })
  })

  it('fails fast when a retryable fingerprint repeats', () => {
    const first = decideRetry({
      config: { baseBackoffMs: 100, maxAttempts: 4 },
      detection: { kind: 'loop-repeat', count: 3 },
      fingerprint: 'loop:npm-test',
      state: createRetryPolicyState(),
    })

    expect(
      decideRetry({
        config: { baseBackoffMs: 100, maxAttempts: 4 },
        detection: { kind: 'loop-repeat', count: 3 },
        fingerprint: 'loop:npm-test',
        state: first.state,
      }),
    ).toEqual({
      kind: 'fail-fast',
      classification: {
        kind: 'retryable',
        detectionKind: 'loop-repeat',
      },
      fingerprint: 'loop:npm-test',
      reason: 'repeated-fingerprint',
      state: {
        attempts: 2,
        failureFingerprints: ['loop:npm-test'],
      },
    })
  })

  it('does not retry terminal disk cap detections', () => {
    expect(
      decideRetry({
        config: { baseBackoffMs: 100, maxAttempts: 4 },
        detection: { kind: 'disk-cap', duBytes: 12, capBytes: 10 },
        fingerprint: 'disk-cap:12>10',
        state: createRetryPolicyState(),
      }),
    ).toEqual({
      kind: 'terminal',
      classification: {
        kind: 'non-retryable',
        detectionKind: 'disk-cap',
        reason: 'disk-cap',
      },
      fingerprint: 'disk-cap:12>10',
      reason: 'non-retryable',
      state: {
        attempts: 1,
        failureFingerprints: ['disk-cap:12>10'],
      },
    })
  })

  it('stops when max attempts are exhausted', () => {
    expect(
      decideRetry({
        config: { baseBackoffMs: 100, maxAttempts: 1 },
        detection: { kind: 'output-heartbeat-stall', idleMs: 5_000 },
        fingerprint: 'output:last=0',
        state: createRetryPolicyState(),
      }),
    ).toEqual({
      kind: 'terminal',
      classification: {
        kind: 'retryable',
        detectionKind: 'output-heartbeat-stall',
      },
      fingerprint: 'output:last=0',
      reason: 'max-attempts-exhausted',
      state: {
        attempts: 1,
        failureFingerprints: ['output:last=0'],
      },
    })
  })
})
