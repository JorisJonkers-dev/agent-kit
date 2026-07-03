import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { EventEmitter } from 'node:events'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PassThrough, Writable } from 'node:stream'
import type { ChildProcess, SpawnOptions } from 'node:child_process'

import { describe, expect, it } from 'vitest'

import {
  parseDuBytes,
  readDuBytes,
  WorkerSupervisorAdapter,
} from './index.js'
import type { WorkerSupervisorEvent } from './index.js'

class FakeChild extends EventEmitter {
  readonly stdout = new PassThrough()
  readonly stderr = new PassThrough()
  readonly stdin: Writable | null
  readonly writes: string[] = []
  readonly pid: number | undefined

  constructor(pid?: number, hasStdin = true) {
    super()
    this.pid = pid
    this.stdin = hasStdin
      ? new Writable({
          write: (chunk: Buffer | string, _encoding, callback) => {
            this.writes.push(String(chunk))
            callback()
          },
        })
      : null
  }

  exit(exitCode: number | null, signal: NodeJS.Signals | null = null): void {
    this.emit('exit', exitCode, signal)
  }
}

interface SpawnRecord {
  readonly command: string
  readonly args: readonly string[]
  readonly options: SpawnOptions
  readonly child: FakeChild
}

function fakeSpawn(children: FakeChild[], records: SpawnRecord[]) {
  return (command: string, args: readonly string[], options: SpawnOptions): ChildProcess => {
    const child = children.shift()
    if (child === undefined) {
      throw new Error('missing fake child')
    }

    records.push({ args, child, command, options })
    return child as unknown as ChildProcess
  }
}

function deferredSleep() {
  const waiters: (() => void)[] = []

  return {
    sleep: () =>
      new Promise<void>((resolve) => {
        waiters.push(resolve)
      }),
    tick: async () => {
      const waiter = waiters.shift()
      if (waiter === undefined) {
        await Promise.resolve()
        return
      }

      waiter()
      await Promise.resolve()
    },
  }
}

describe('WorkerSupervisorAdapter', () => {
  it('spawns detached in the worktree and completes when the child exits cleanly', async () => {
    const child = new FakeChild(101)
    const records: SpawnRecord[] = []
    const events: WorkerSupervisorEvent[] = []
    const sleeper = deferredSleep()
    const adapter = new WorkerSupervisorAdapter({
      onEvent: (event) => events.push(event),
      sleep: (ms) => (ms === 1 ? Promise.resolve() : sleeper.sleep()),
      spawn: fakeSpawn([child], records),
    })

    const session = adapter.start({
      args: ['run'],
      command: 'agent',
      env: { EXTRA: '1' },
      id: 'T1',
      modelTier: 'cheap',
      stdin: 'initial prompt',
      worktree: '/tmp/worktree',
    })
    child.stdout.write('$ npm test\n')
    child.stderr.write('warning\n')
    child.exit(0)

    await expect(session.result).resolves.toMatchObject({
      exitCode: 0,
      restarts: 0,
      status: 'completed',
      stderr: 'warning\n',
      stdout: '$ npm test\n',
    })
    expect(records).toHaveLength(1)
    expect(records[0]?.options).toMatchObject({
      cwd: '/tmp/worktree',
      detached: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    expect(records[0]?.options.env).toMatchObject({ COUNCIL_MODEL_TIER: 'cheap', EXTRA: '1' })
    expect(child.writes).toEqual(['initial prompt\n'])
    expect(events.some((event) => event.type === 'started' && event.pid === 101)).toBe(true)
    expect(events.some((event) => event.type === 'stdout')).toBe(true)
    expect(events.some((event) => event.type === 'stderr')).toBe(true)
    await flushPromises()
    await sleeper.tick()
    await flushPromises()
  })

  it('terminates the process group, retries with preamble, escalates tier, then stalls', async () => {
    const first = new FakeChild(201)
    const second = new FakeChild(202)
    const third = new FakeChild(203)
    const records: SpawnRecord[] = []
    const events: WorkerSupervisorEvent[] = []
    const kills: { readonly pid: number; readonly signal: NodeJS.Signals }[] = []
    const sleeper = deferredSleep()
    let now = 0
    const adapter = new WorkerSupervisorAdapter({
      kill: (pid, signal) => {
        kills.push({ pid, signal })
        if (pid === -201) {
          first.exit(null, signal)
        }
        if (pid === -202) {
          second.exit(null, signal)
        }
        if (pid === -203) {
          third.exit(null, signal)
        }
      },
      nowMs: () => now,
      onEvent: (event) => events.push(event),
      sleep: sleeper.sleep,
      spawn: fakeSpawn([first, second, third], records),
    })
    const session = adapter.start({
      command: 'agent',
      escalationModelTier: 'max',
      id: 'T2',
      killGraceMs: 1,
      modelTier: 'cheap',
      pollIntervalMs: 1,
      restartPreamble: 'Continue after watchdog.',
      watchdog: { maxRestarts: 2, stallAfterS: 1 },
      worktree: '/tmp/worktree',
    })

    now = 1_000
    await sleeper.tick()
    expect(records).toHaveLength(2)
    expect(records[1]?.child.writes[0]).toContain('Continue after watchdog.')
    expect(records[1]?.options.cwd).toBe('/tmp/worktree')

    now = 2_000
    await sleeper.tick()
    expect(records).toHaveLength(3)
    expect(records[2]?.options.env).toMatchObject({ COUNCIL_MODEL_TIER: 'max' })
    expect(events.some((event) => event.type === 'tier-escalated' && event.modelTier === 'max')).toBe(true)

    now = 3_000
    await flushPromises()
    await sleeper.tick()
    await flushPromises()

    await expect(session.result).resolves.toMatchObject({
      modelTier: 'max',
      restarts: 2,
      status: 'stalled',
    })
    expect(kills).toEqual([
      { pid: -201, signal: 'SIGTERM' },
      { pid: -202, signal: 'SIGTERM' },
      { pid: -203, signal: 'SIGTERM' },
    ])
  })

  it('sends SIGKILL after the grace period when SIGTERM does not exit', async () => {
    const child = new FakeChild(301)
    const records: SpawnRecord[] = []
    const kills: NodeJS.Signals[] = []
    const sleeper = deferredSleep()
    const adapter = new WorkerSupervisorAdapter({
      kill: (_pid, signal) => {
        kills.push(signal)
        if (signal === 'SIGKILL') {
          child.exit(null, signal)
        }
      },
      sleep: (ms) => (ms === 1 ? Promise.resolve() : sleeper.sleep()),
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T3',
      killGraceMs: 1,
      worktree: '/tmp/worktree',
    })

    const stop = session.stop()
    await sleeper.tick()
    await sleeper.tick()
    await stop

    await expect(session.result).resolves.toMatchObject({
      signal: null,
      status: 'stopped',
    })
    expect(kills).toEqual(['SIGTERM', 'SIGKILL'])
  })

  it('treats ESRCH during SIGKILL fallback as already exited', async () => {
    const child = new FakeChild(311)
    const records: SpawnRecord[] = []
    const sleeper = deferredSleep()
    const adapter = new WorkerSupervisorAdapter({
      kill: (_pid, signal) => {
        if (signal === 'SIGKILL') {
          const error = new Error('gone') as Error & { code: string }
          error.code = 'ESRCH'
          throw error
        }
      },
      sleep: sleeper.sleep,
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T3-esrch',
      killGraceMs: 1,
      worktree: '/tmp/worktree',
    })

    const stop = session.stop()
    await sleeper.tick()
    await sleeper.tick()
    await stop

    await expect(session.result).resolves.toMatchObject({ status: 'stopped' })
  })

  it('surfaces non-ESRCH kill failures from session termination', async () => {
    const child = new FakeChild(312)
    const adapter = new WorkerSupervisorAdapter({
      kill: () => {
        throw new Error('permission denied')
      },
      spawn: fakeSpawn([child], []),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T3-kill-error',
      worktree: '/tmp/worktree',
    })

    await expect(session.stop()).rejects.toThrow('permission denied')
    child.exit(0)
    await expect(session.result).resolves.toMatchObject({ status: 'completed' })
  })

  it('ignores ESRCH during group termination and treats child failure as failed', async () => {
    const child = new FakeChild(401)
    const records: SpawnRecord[] = []
    const adapter = new WorkerSupervisorAdapter({
      kill: () => {
        const error = new Error('gone') as Error & { code: string }
        error.code = 'ESRCH'
        throw error
      },
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T4',
      worktree: '/tmp/worktree',
    })

    await session.stop('cancelled')
    child.exit(1)

    await expect(session.result).resolves.toMatchObject({
      status: 'stopped',
    })

    const failed = new FakeChild(402)
    const failing = new WorkerSupervisorAdapter({
      spawn: fakeSpawn([failed], []),
    }).start({
      command: 'agent',
      id: 'T4-failed',
      worktree: '/tmp/worktree',
    })
    failed.exit(2)
    await expect(failing.result).resolves.toMatchObject({ exitCode: 2, status: 'failed' })
  })

  it('delivers injects over stdin when supported', async () => {
    const child = new FakeChild(501)
    const records: SpawnRecord[] = []
    const adapter = new WorkerSupervisorAdapter({
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T5',
      supportsStreamingStdin: true,
      worktree: '/tmp/worktree',
    })

    await expect(session.inject('please continue')).resolves.toEqual({
      mode: 'streaming-stdin',
      restarted: false,
    })
    expect(child.writes).toEqual(['please continue\n'])
    child.exit(0)
    await expect(session.result).resolves.toMatchObject({ status: 'completed' })
    await expect(session.inject('too late')).rejects.toThrow('already finished')
  })

  it('falls back to checkpoint-and-resume inject using the restart path', async () => {
    const first = new FakeChild(601)
    const second = new FakeChild(602)
    const records: SpawnRecord[] = []
    const kills: number[] = []
    const adapter = new WorkerSupervisorAdapter({
      kill: (pid, signal) => {
        kills.push(pid)
        first.exit(null, signal)
      },
      spawn: fakeSpawn([first, second], records),
    })
    const session = adapter.start({
      checkpointPreamble: 'Checkpoint now.',
      command: 'agent',
      id: 'T6',
      stdin: 'original task',
      worktree: '/tmp/worktree',
    })

    await expect(session.inject('new operator turn')).resolves.toEqual({
      mode: 'checkpoint-and-resume',
      restarted: true,
    })
    expect(kills).toEqual([-601])
    expect(second.writes.join('')).toContain('Checkpoint now.')
    expect(second.writes.join('')).toContain('new operator turn')
    expect(second.writes.join('')).toContain('original task')
    second.exit(0)
    await expect(session.result).resolves.toMatchObject({ restarts: 1, status: 'completed' })
  })

  it('stops immediately on disk cap detection', async () => {
    const child = new FakeChild(701)
    const records: SpawnRecord[] = []
    const sleeper = deferredSleep()
    const adapter = new WorkerSupervisorAdapter({
      duBytes: () => Promise.resolve(12),
      kill: (_pid, signal) => { child.exit(null, signal); },
      sleep: sleeper.sleep,
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T7',
      pollIntervalMs: 1,
      watchdog: { diskCapBytes: 10 },
      worktree: '/tmp/worktree',
    })

    await sleeper.tick()

    await expect(session.result).resolves.toMatchObject({
      detection: { capBytes: 10, duBytes: 12, kind: 'disk-cap' },
      status: 'disk-cap',
    })
  })

  it('awaits async disk cap handling before finishing', async () => {
    const child = new FakeChild(711)
    const records: SpawnRecord[] = []
    const kills: NodeJS.Signals[] = []
    const sleeper = deferredSleep()
    const adapter = new WorkerSupervisorAdapter({
      duBytes: () => Promise.resolve(12),
      kill: (_pid, signal) => {
        kills.push(signal)
        if (signal === 'SIGKILL') {
          child.exit(null, signal)
        }
      },
      sleep: (ms) => (ms === 1 ? Promise.resolve() : sleeper.sleep()),
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T7-async',
      killGraceMs: 1,
      pollIntervalMs: 2,
      watchdog: { diskCapBytes: 10 },
      worktree: '/tmp/worktree',
    })

    await flushPromises()
    await sleeper.tick()
    await flushPromises()

    await expect(session.result).resolves.toMatchObject({
      detection: { capBytes: 10, duBytes: 12, kind: 'disk-cap' },
      status: 'disk-cap',
    })
    expect(kills).toEqual(['SIGTERM', 'SIGKILL'])
  })

  it('resets termination state when the graceful wait rejects', async () => {
    const child = new FakeChild(712)
    const adapter = new WorkerSupervisorAdapter({
      kill: () => undefined,
      sleep: (ms) =>
        ms === 1
          ? Promise.reject(new Error('grace wait failed'))
          : new Promise(() => undefined),
      spawn: fakeSpawn([child], []),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T7-grace-reject',
      killGraceMs: 1,
      worktree: '/tmp/worktree',
    })

    await expect(session.stop()).rejects.toThrow('grace wait failed')
    child.exit(0)

    await expect(session.result).resolves.toMatchObject({ status: 'completed' })
  })

  it('uses loop detections from output and stalls without tier escalation when disabled', async () => {
    const first = new FakeChild(801)
    const second = new FakeChild(802)
    const records: SpawnRecord[] = []
    const sleeper = deferredSleep()
    const adapter = new WorkerSupervisorAdapter({
      kill: (pid, signal) => {
        if (pid === -801) {
          first.exit(null, signal)
        }
        if (pid === -802) {
          second.exit(null, signal)
        }
      },
      sleep: sleeper.sleep,
      spawn: fakeSpawn([first, second], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T8',
      pollIntervalMs: 1,
      watchdog: { enableTierEscalation: false, maxRestarts: 1, repeatLimit: 2, windowSize: 3 },
      worktree: '/tmp/worktree',
    })
    first.stdout.write('$ npm test\n$ npm test\n')

    await sleeper.tick()
    expect(records).toHaveLength(2)
    second.stdout.write('$ npm test\n$ npm test\n')
    await sleeper.tick()

    await expect(session.result).resolves.toMatchObject({
      status: 'stalled',
    })
  })

  it('marks stalled instead of restarting when the restart cap is exhausted', async () => {
    const child = new FakeChild(901)
    const records: SpawnRecord[] = []
    const sleeper = deferredSleep()
    let now = 0
    const adapter = new WorkerSupervisorAdapter({
      kill: (_pid, signal) => { child.exit(null, signal); },
      nowMs: () => now,
      sleep: sleeper.sleep,
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T9',
      pollIntervalMs: 1,
      watchdog: { maxRestarts: 0, stallAfterS: 1 },
      worktree: '/tmp/worktree',
    })

    now = 1_000
    await sleeper.tick()

    await expect(session.result).resolves.toMatchObject({
      restarts: 0,
      status: 'stalled',
    })
  })

  it('handles children without a pid or stdin', async () => {
    const child = new FakeChild(undefined, false)
    const records: SpawnRecord[] = []
    const adapter = new WorkerSupervisorAdapter({
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T10',
      stdin: 'ignored',
      worktree: '/tmp/worktree',
    })

    await session.stop()
    await expect(session.result).resolves.toMatchObject({ status: 'stopped' })
  })
})

describe('process adapter OS integration', () => {
  it('kills a detached process group with grandchildren', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'council-supervisor-'))
    const marker = join(dir, 'grandchild-alive')
    const script = join(dir, 'child.mjs')
    await writeFile(
      script,
      [
        "import { spawn } from 'node:child_process'",
        `spawn(process.execPath, ['-e', ${JSON.stringify(grandchildScript(marker))}], { stdio: 'ignore' })`,
        "setInterval(() => {}, 1_000)",
      ].join('\n'),
    )

    const adapter = new WorkerSupervisorAdapter()
    const session = adapter.start({
      args: [script],
      command: process.execPath,
      id: 'real-tree',
      killGraceMs: 100,
      pollIntervalMs: 10_000,
      worktree: dir,
    })
    await waitFor(async () => {
      await readFile(marker, 'utf8')
    })
    await session.stop('test complete')
    const before = await readFile(marker, 'utf8')
    await new Promise((resolve) => {
      setTimeout(resolve, 120)
    })
    const after = await readFile(marker, 'utf8')

    await expect(session.result).resolves.toMatchObject({ status: 'stopped' })
    expect(after).toBe(before)
    await rm(dir, { force: true, recursive: true })
  })

  it('reads du output in bytes', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'council-du-'))
    await writeFile(join(dir, 'payload'), 'x')

    await expect(readDuBytes(dir)).resolves.toBeGreaterThanOrEqual(1024)
    await rm(dir, { force: true, recursive: true })
  })

  it('rejects invalid du output', () => {
    expect(() => parseDuBytes('/tmp/worktree', 'not-a-number\t/tmp/worktree')).toThrow(
      'could not parse du output',
    )
  })

  it('runs the default spawn path to a failed exit', async () => {
    const adapter = new WorkerSupervisorAdapter()
    const session = adapter.start({
      args: ['-e', 'process.exit(7)'],
      command: process.execPath,
      id: 'real-failure',
      pollIntervalMs: 10_000,
      worktree: process.cwd(),
    })

    await expect(session.result).resolves.toMatchObject({ exitCode: 7, status: 'failed' })
  })

  it('supports direct process-group kill errors outside the supervisor session', () => {
    const adapter = new WorkerSupervisorAdapter({
      kill: () => {
        throw new Error('permission denied')
      },
    })

    expect(() => { adapter.killGroup(123, 'SIGTERM'); }).toThrow('permission denied')
  })
})

async function waitFor(action: () => Promise<void>): Promise<void> {
  const started = Date.now()

  while (Date.now() - started < 1_000) {
    try {
      await action()
      return
    } catch {
      await new Promise((resolve) => {
        setTimeout(resolve, 20)
      })
    }
  }

  await action()
}

async function flushPromises(): Promise<void> {
  for (let index = 0; index < 5; index += 1) {
    await Promise.resolve()
  }
}

function grandchildScript(marker: string): string {
  return [
    "const { writeFileSync } = require('node:fs')",
    `setInterval(() => writeFileSync(${JSON.stringify(marker)}, String(Date.now())), 20)`,
  ].join('\n')
}
