import { performance } from 'node:perf_hooks'

import type { ClockPort, EnvPort } from '../../ports/index.js'

export class SystemClockAdapter implements ClockPort {
  now(): Date {
    return new Date()
  }

  monotonicMs(): number {
    return performance.now()
  }

  async sleep(ms: number): Promise<void> {
    await new Promise<void>((resolve) => {
      setTimeout(resolve, ms)
    })
  }
}

export class ProcessEnvAdapter implements EnvPort {
  private readonly source: NodeJS.ProcessEnv

  constructor(source: NodeJS.ProcessEnv = process.env) {
    this.source = source
  }

  get(name: string): string | undefined {
    return this.source[name]
  }

  require(name: string): string {
    const value = this.get(name)
    if (value === undefined) {
      throw new Error(`required environment variable is missing: ${name}`)
    }
    return value
  }

  all(): Readonly<Record<string, string>> {
    const entries = Object.entries(this.source).filter(
      (entry): entry is [string, string] => entry[1] !== undefined,
    )
    return Object.fromEntries(entries)
  }
}
