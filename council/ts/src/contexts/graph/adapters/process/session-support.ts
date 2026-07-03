import type { MaybePromise, SpawnInput, WatchdogConfig, WorkerSupervisorWatchdogConfig } from './types.js'
import { optional } from './types.js'

export function joinPrompt(...parts: readonly (string | undefined)[]): string | undefined {
  const joined = parts
    .filter((part): part is string => part !== undefined && part.length > 0)
    .join('\n\n')
  return joined.length === 0 ? undefined : joined
}

export function normalizeWatchdogConfig(config: WorkerSupervisorWatchdogConfig = {}): WatchdogConfig {
  return {
    ...optional('diskCapBytes', config.diskCapBytes),
    enableTierEscalation: config.enableTierEscalation ?? true,
    loop: {
      ...optional('maxCycleGram', config.maxCycleGram),
      repeatLimit: config.repeatLimit ?? 3,
      windowSize: config.windowSize ?? 20,
    },
    maxRestarts: config.maxRestarts ?? 2,
    stallAfterS: config.stallAfterS ?? 300,
  }
}

export function spawnInput(preamble: string, modelTier: string | undefined): SpawnInput {
  return {
    preamble,
    ...optional('modelTier', modelTier),
  }
}

export function isPromiseLike<T>(value: MaybePromise<T>): value is Promise<T> {
  return (
    typeof value === 'object' &&
    value !== null &&
    'then' in value &&
    typeof (value as { readonly then?: unknown }).then === 'function'
  )
}

export function thenMaybe<T, Next>(
  value: MaybePromise<T>,
  next: () => MaybePromise<Next>,
): MaybePromise<Next> {
  return isPromiseLike(value) ? value.then(next) : next()
}
