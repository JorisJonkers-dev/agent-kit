import { appendFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { EventEmitter } from 'node:events'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { PassThrough, Writable } from 'node:stream'
import { spawn as nodeSpawn } from 'node:child_process'
import type { ChildProcess, SpawnOptions } from 'node:child_process'

import { afterEach, describe, expect, it } from 'vitest'

import {
  parseDuBytes,
  readDuBytes,
  WorkerSupervisorAdapter,
} from './index.js'
import type { WorkerSupervisorEvent } from './index.js'
import type { WorkerSupervisorSnapshot } from './types.js'

const PROCESS_TAIL_MAX_CHARS = 4096
const tempRoots: string[] = []

afterEach(async () => {
  await Promise.all(tempRoots.splice(0).map((root) => rm(root, { force: true, recursive: true })))
})

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
    const worktree = await tempRoot()
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
      env: {
        AGENT_MCP_PROFILE: 'env-profile',
        COUNCIL_MODEL_TIER: 'explicit-env-tier',
        EXTRA: '1',
        KB_AUTO_MCP_DISABLED: '0',
      },
      id: 'T1',
      mcpProfile: 'code-intel',
      modelTier: 'cheap',
      stdin: 'initial prompt',
      worktree,
    })
    child.stdout.write('$ npm test\n')
    child.stdout.write('done\n')
    child.stderr.write('warning\n')
    child.exit(0)

    await expect(session.result).resolves.toMatchObject({
      exitCode: 0,
      restarts: 0,
      status: 'completed',
      stderr: 'warning\n',
      stderrBytes: Buffer.byteLength('warning\n'),
      stderrLogPath: 'workers/T1/logs/stderr.log',
      stdout: '$ npm test\ndone\n',
      stdoutBytes: Buffer.byteLength('$ npm test\ndone\n'),
      stdoutLogPath: 'workers/T1/logs/stdout.log',
    })
    expect(records).toHaveLength(1)
    expect(records[0]?.options).toMatchObject({
      cwd: worktree,
      detached: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    })
    expect(records[0]?.options.env).toMatchObject({
      AGENT_MCP_PROFILE: 'code-intel',
      COUNCIL_MODEL_TIER: 'cheap',
      EXTRA: '1',
      KB_AUTO_MCP_DISABLED: '1',
    })
    expect(child.writes).toEqual(['initial prompt\n'])
    expect(events).toContainEqual(expect.objectContaining({
      attemptId: 1,
      modelTier: 'cheap',
      pid: 101,
      restart: 1,
      restartCount: 0,
      taskId: 'T1',
      type: 'started',
    }))
    expect(events).toContainEqual(expect.objectContaining({
      attemptId: 1,
      byteCount: Buffer.byteLength('$ npm test\n'),
      logPath: 'workers/T1/logs/stdout.log',
      modelTier: 'cheap',
      offset: 0,
      pid: 101,
      restartCount: 0,
      taskId: 'T1',
      tail: '$ npm test\n',
      tailBytes: Buffer.byteLength('$ npm test\n'),
      type: 'stdout',
    }))
    expect(events).toContainEqual(expect.objectContaining({
      attemptId: 1,
      byteCount: Buffer.byteLength('done\n'),
      logPath: 'workers/T1/logs/stdout.log',
      modelTier: 'cheap',
      offset: Buffer.byteLength('$ npm test\n'),
      pid: 101,
      restartCount: 0,
      taskId: 'T1',
      tail: 'done\n',
      tailBytes: Buffer.byteLength('done\n'),
      type: 'stdout',
    }))
    expect(events).toContainEqual(expect.objectContaining({
      attemptId: 1,
      byteCount: Buffer.byteLength('warning\n'),
      logPath: 'workers/T1/logs/stderr.log',
      modelTier: 'cheap',
      offset: 0,
      pid: 101,
      restartCount: 0,
      taskId: 'T1',
      tail: 'warning\n',
      tailBytes: Buffer.byteLength('warning\n'),
      type: 'stderr',
    }))
    expect(events).toContainEqual(expect.objectContaining({
      attemptId: 1,
      exitCode: 0,
      modelTier: 'cheap',
      pid: 101,
      restartCount: 0,
      signal: null,
      taskId: 'T1',
      type: 'exited',
    }))
    await flushPromises()
    await sleeper.tick()
    await flushPromises()
    await expect(readFile(join(worktree, 'workers', 'T1', 'logs', 'stdout.log'), 'utf8')).resolves.toBe(
      '$ npm test\ndone\n',
    )
    await expect(readFile(join(worktree, 'workers', 'T1', 'logs', 'stderr.log'), 'utf8')).resolves.toBe(
      'warning\n',
    )
  })

  it('writes supervisor snapshots on start, output, detection, restart, and exit', async () => {
    const worktree = await tempRoot()
    const first = new FakeChild(131)
    const second = new FakeChild(132)
    const snapshots: WorkerSupervisorSnapshot[] = []
    const sleeper = deferredSleep()
    let now = 0
    const adapter = new WorkerSupervisorAdapter({
      kill: (pid, signal) => {
        if (pid === -131) {
          first.exit(null, signal)
        }
      },
      nowMs: () => now,
      onSnapshot: (snapshot) => { snapshots.push(snapshot); },
      sleep: sleeper.sleep,
      spawn: fakeSpawn([first, second], []),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T1-snapshot',
      modelTier: 'cheap',
      pollIntervalMs: 1,
      watchdog: { maxRestarts: 1, stallAfterS: 1 },
      worktree,
    })

    expect(snapshots).toContainEqual(expect.objectContaining({
      attempt_id: 1,
      model_tier: 'cheap',
      offsets: { stderr: 0, stdout: 0 },
      pid: 131,
      restart_count: 0,
      status: 'running',
      task_id: 'T1-snapshot',
    }))

    first.stdout.write('before stall\n')
    expect(snapshots).toContainEqual(expect.objectContaining({
      attempt_id: 1,
      offsets: { stderr: 0, stdout: Buffer.byteLength('before stall\n') },
      pid: 131,
      status: 'running',
    }))

    now = 1_000
    await sleeper.tick()
    await flushPromises()
    now = 2_000
    await sleeper.tick()
    await flushPromises()

    expect(snapshots.some((snapshot) =>
      snapshot.attempt_id === 1 &&
      snapshot.pid === 131 &&
      snapshot.status === 'detected' &&
      snapshot.watchdog.pending_detection?.kind === 'progress-stall',
    )).toBe(true)
    expect(snapshots).toContainEqual(expect.objectContaining({
      attempt_id: 2,
      pid: 132,
      restart_count: 1,
      status: 'restarting',
    }))

    second.exit(0)

    await expect(session.result).resolves.toMatchObject({ status: 'completed' })
    expect(snapshots).toContainEqual(expect.objectContaining({
      attempt_id: 2,
      exit_code: 0,
      pid: 132,
      signal: null,
      status: 'exited',
    }))
  })

  it('reattaches to a live pid from saved offsets without replaying prior output', async () => {
    const worktree = await tempRoot()
    const taskId = 'T1-reattach'
    const logDir = join(worktree, 'workers', taskId, 'logs')
    const priorStdout = 'old stdout\n'
    const priorStderr = 'old stderr\n'
    await mkdir(logDir, { recursive: true })
    await writeFile(join(logDir, 'stdout.log'), priorStdout, 'utf8')
    await writeFile(join(logDir, 'stderr.log'), priorStderr, 'utf8')

    const events: WorkerSupervisorEvent[] = []
    const snapshots: WorkerSupervisorSnapshot[] = []
    const sleeper = deferredSleep()
    let live = true
    const adapter = new WorkerSupervisorAdapter({
      isPidAlive: () => live,
      onEvent: (event) => events.push(event),
      onSnapshot: (snapshot) => { snapshots.push(snapshot); },
      sleep: sleeper.sleep,
    })
    const session = adapter.reattach({
      command: 'agent',
      id: taskId,
      pollIntervalMs: 1,
      worktree,
    }, supervisorSnapshot(taskId, {
      offsets: {
        stderr: Buffer.byteLength(priorStderr),
        stdout: Buffer.byteLength(priorStdout),
      },
      pid: 4242,
    }))

    await expect(session.inject('continue')).rejects.toThrow('does not have an attached stdin')
    await appendFile(join(logDir, 'stdout.log'), 'new stdout\n', 'utf8')
    await appendFile(join(logDir, 'stderr.log'), 'new stderr\n', 'utf8')
    await sleeper.tick()

    expect(events.filter((event) => event.type === 'started')).toEqual([])
    await waitFor(() => {
      expect(events).toContainEqual(expect.objectContaining({
        attemptId: 2,
        byteCount: Buffer.byteLength('new stdout\n'),
        offset: Buffer.byteLength(priorStdout),
        pid: 4242,
        restartCount: 1,
        tail: 'new stdout\n',
        type: 'stdout',
      }))
      return Promise.resolve()
    })
    expect(events).toContainEqual(expect.objectContaining({
      attemptId: 2,
      byteCount: Buffer.byteLength('new stderr\n'),
      offset: Buffer.byteLength(priorStderr),
      pid: 4242,
      restartCount: 1,
      tail: 'new stderr\n',
      type: 'stderr',
    }))
    expect(snapshots).toContainEqual(expect.objectContaining({
      offsets: {
        stderr: Buffer.byteLength(`${priorStderr}new stderr\n`),
        stdout: Buffer.byteLength(`${priorStdout}new stdout\n`),
      },
      pid: 4242,
      status: 'running',
    }))

    live = false
    await sleeper.tick()
    await flushPromises()

    await expect(session.result).resolves.toMatchObject({
      exitCode: null,
      status: 'dead-snapshot',
    })
  })

  it('classifies stale and dead snapshots as terminal without emitting lifecycle events', async () => {
    const events: WorkerSupervisorEvent[] = []
    const adapter = new WorkerSupervisorAdapter({
      isPidAlive: () => false,
      onEvent: (event) => events.push(event),
    })
    const request = {
      command: 'agent',
      id: 'T1-terminal-snapshot',
      worktree: '/tmp/worktree',
    }
    const stale = adapter.reattach(request, supervisorSnapshot(request.id, {
      pid: 5151,
      status: 'exited',
    }))
    await expect(stale.result).resolves.toMatchObject({ status: 'stale-snapshot' })
    await stale.stop('already terminal')
    await expect(stale.inject('continue')).rejects.toThrow('terminal snapshot')

    const dead = adapter.reattach(request, supervisorSnapshot(request.id, { pid: 6161 }))
    await expect(dead.result).resolves.toMatchObject({ status: 'dead-snapshot' })
    const defaultDead = new WorkerSupervisorAdapter().reattach(
      request,
      supervisorSnapshot(request.id, { pid: 99_999_999 }),
    )
    await expect(defaultDead.result).resolves.toMatchObject({ status: 'dead-snapshot' })

    expect(() => {
      adapter.reattach(request, supervisorSnapshot('other-task', { pid: 7171 }))
    }).toThrow('supervisor snapshot task_id must match request id: T1-terminal-snapshot')

    expect(events).toEqual([])
  })

  it('surfaces unexpected default pid liveness failures during reattach', () => {
    const originalKill: typeof process.kill = process.kill.bind(process)
    const error = new Error('permission denied') as NodeJS.ErrnoException
    error.code = 'EPERM'
    process.kill = ((pid: number, signal?: NodeJS.Signals | number) => {
      void pid
      void signal
      throw error
    })
    try {
      expect(() => {
        new WorkerSupervisorAdapter().reattach({
          command: 'agent',
          id: 'T1-liveness-error',
          worktree: '/tmp/worktree',
        }, supervisorSnapshot('T1-liveness-error', { pid: 7171 }))
      }).toThrow('permission denied')
    } finally {
      process.kill = originalKill
    }
  })

  it('stops a reattached watcher before the next poll resumes', async () => {
    const sleeper = deferredSleep()
    const kills: number[] = []
    const adapter = new WorkerSupervisorAdapter({
      isPidAlive: () => true,
      kill: (pid) => { kills.push(pid); },
      sleep: sleeper.sleep,
    })
    const session = adapter.reattach({
      command: 'agent',
      id: 'T1-stop-reattach',
      pollIntervalMs: 1,
      worktree: '/tmp/worktree',
    }, supervisorSnapshot('T1-stop-reattach', { pid: 8181 }))

    await session.stop('operator')
    await sleeper.tick()

    expect(kills).toEqual([-8181])
    await expect(session.result).resolves.toMatchObject({ status: 'stopped' })
  })

  it('classifies a reattached snapshot as stale when logs are shorter than saved offsets', async () => {
    const worktree = await tempRoot()
    const taskId = 'T1-truncated'
    const logDir = join(worktree, 'workers', taskId, 'logs')
    const sleeper = deferredSleep()
    await mkdir(logDir, { recursive: true })
    await writeFile(join(logDir, 'stdout.log'), 'short\n', 'utf8')
    await writeFile(join(logDir, 'stderr.log'), '', 'utf8')
    const session = new WorkerSupervisorAdapter({
      isPidAlive: () => true,
      sleep: sleeper.sleep,
    }).reattach({
      command: 'agent',
      id: taskId,
      pollIntervalMs: 1,
      worktree,
    }, supervisorSnapshot(taskId, {
      offsets: {
        stderr: 0,
        stdout: 99,
      },
      pid: 9191,
    }))

    await sleeper.tick()

    await expect(session.result).resolves.toMatchObject({ status: 'stale-snapshot' })
  })

  it('spools full logs while exposing only bounded tails and relative log paths', async () => {
    const worktree = await tempRoot()
    const child = new FakeChild(121)
    const events: WorkerSupervisorEvent[] = []
    const stdout = `${'o'.repeat(PROCESS_TAIL_MAX_CHARS + 20)}\nfinal stdout\n`
    const stderr = `${'e'.repeat(PROCESS_TAIL_MAX_CHARS + 10)}\nfinal stderr\n`
    const adapter = new WorkerSupervisorAdapter({
      onEvent: (event) => events.push(event),
      spawn: fakeSpawn([child], []),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T1-logs',
      worktree,
    })

    child.stdout.write(stdout)
    child.stderr.write(stderr)
    child.exit(0)

    await expect(session.result).resolves.toMatchObject({
      status: 'completed',
      stderr: tail(stderr),
      stderrBytes: Buffer.byteLength(stderr),
      stderrLogPath: 'workers/T1-logs/logs/stderr.log',
      stdout: tail(stdout),
      stdoutBytes: Buffer.byteLength(stdout),
      stdoutLogPath: 'workers/T1-logs/logs/stdout.log',
    })
    expect(events).toContainEqual(expect.objectContaining({
      byteCount: Buffer.byteLength(stdout),
      logPath: 'workers/T1-logs/logs/stdout.log',
      tail: tail(stdout),
      tailBytes: Buffer.byteLength(tail(stdout)),
      type: 'stdout',
    }))
    expect(events).toContainEqual(expect.objectContaining({
      byteCount: Buffer.byteLength(stderr),
      logPath: 'workers/T1-logs/logs/stderr.log',
      tail: tail(stderr),
      tailBytes: Buffer.byteLength(tail(stderr)),
      type: 'stderr',
    }))
    await expect(readFile(join(worktree, 'workers', 'T1-logs', 'logs', 'stdout.log'), 'utf8')).resolves.toBe(
      stdout,
    )
    await expect(readFile(join(worktree, 'workers', 'T1-logs', 'logs', 'stderr.log'), 'utf8')).resolves.toBe(
      stderr,
    )
  })

  it('preserves an explicit COUNCIL_MODEL_TIER env value when no model tier is requested', async () => {
    const child = new FakeChild(111)
    const records: SpawnRecord[] = []
    const adapter = new WorkerSupervisorAdapter({
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      env: { AGENT_MCP_PROFILE: 'env-profile', COUNCIL_MODEL_TIER: 'env-tier' },
      id: 'T1-env-tier',
      worktree: '/tmp/worktree',
    })

    child.exit(0)

    await expect(session.result).resolves.toMatchObject({ status: 'completed' })
    expect(records[0]?.options.env).toMatchObject({
      AGENT_MCP_PROFILE: 'env-profile',
      COUNCIL_MODEL_TIER: 'env-tier',
      KB_AUTO_MCP_DISABLED: '1',
    })
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
    expect(records[1]?.options.env).toMatchObject({
      COUNCIL_MODEL_TIER: 'cheap',
      KB_AUTO_MCP_DISABLED: '1',
    })
    expect(events.some((event) =>
      event.type === 'detected' &&
      event.attemptId === 1 &&
      event.detection.kind === 'progress-stall' &&
      event.modelTier === 'cheap' &&
      event.pid === 201 &&
      event.restartCount === 0 &&
      event.taskId === 'T2',
    )).toBe(true)
    expect(events.some((event) =>
      event.type === 'terminated' &&
      event.attemptId === 1 &&
      event.detection?.kind === 'progress-stall' &&
      event.modelTier === 'cheap' &&
      event.pid === 201 &&
      event.restartCount === 0 &&
      event.signal === 'SIGTERM' &&
      event.taskId === 'T2',
    )).toBe(true)
    expect(events.some((event) =>
      event.type === 'started' &&
      event.attemptId === 2 &&
      event.detection?.kind === 'progress-stall' &&
      event.modelTier === 'cheap' &&
      event.pid === 202 &&
      event.restart === 2 &&
      event.restartCount === 1 &&
      event.taskId === 'T2',
    )).toBe(true)
    expect(events.some((event) =>
      event.type === 'restarted' &&
      event.attemptId === 2 &&
      event.detection?.kind === 'progress-stall' &&
      event.modelTier === 'cheap' &&
      event.pid === 202 &&
      event.previousPid === 201 &&
      event.restart === 1 &&
      event.restartCount === 1 &&
      event.taskId === 'T2',
    )).toBe(true)

    now = 2_000
    await sleeper.tick()
    expect(records).toHaveLength(3)
    expect(records[2]?.options.env).toMatchObject({
      COUNCIL_MODEL_TIER: 'max',
      KB_AUTO_MCP_DISABLED: '1',
    })
    expect(events.some((event) =>
      event.type === 'tier-escalated' &&
      event.attemptId === 2 &&
      event.detection?.kind === 'progress-stall' &&
      event.modelTier === 'max' &&
      event.pid === 202 &&
      event.restartCount === 1 &&
      event.taskId === 'T2',
    )).toBe(true)

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

  it('stalls checkpoint-and-resume inject when the restart cap is exhausted', async () => {
    const child = new FakeChild(611)
    const records: SpawnRecord[] = []
    const adapter = new WorkerSupervisorAdapter({
      kill: (_pid, signal) => { child.exit(null, signal); },
      spawn: fakeSpawn([child], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T6-restart-cap',
      watchdog: { maxRestarts: 0 },
      worktree: '/tmp/worktree',
    })

    await expect(session.inject('new operator turn')).resolves.toEqual({
      mode: 'checkpoint-and-resume',
      restarted: true,
    })

    expect(records).toHaveLength(1)
    await expect(session.result).resolves.toMatchObject({ restarts: 0, status: 'stalled' })
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
    const worktree = await tempRoot()
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
      worktree,
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

  it('retries loop-cycle detections with a stable multi-field fingerprint', async () => {
    const worktree = await tempRoot()
    const first = new FakeChild(851)
    const second = new FakeChild(852)
    const records: SpawnRecord[] = []
    const sleeper = deferredSleep()
    const adapter = new WorkerSupervisorAdapter({
      kill: (pid, signal) => {
        if (pid === -851) {
          first.exit(null, signal)
        }
      },
      sleep: sleeper.sleep,
      spawn: fakeSpawn([first, second], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T8-cycle',
      pollIntervalMs: 1,
      watchdog: { maxCycleGram: 2, maxRestarts: 1, repeatLimit: 10, windowSize: 6 },
      worktree,
    })
    first.stdout.write('$ a\n$ b\n$ a\n$ b\n$ a\n$ b\n')

    await sleeper.tick()

    expect(records).toHaveLength(2)
    expect(second.writes.join('')).toContain('Watchdog detection: loop-cycle.')
    second.exit(0)
    await expect(session.result).resolves.toMatchObject({ restarts: 1, status: 'completed' })
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

  it('treats wall-clock and output caps as terminal budget caps', async () => {
    const wallClockChild = new FakeChild(911)
    const outputChild = new FakeChild(912)
    const wallClockSleeper = deferredSleep()
    const outputSleeper = deferredSleep()
    let now = 0
    const wallClockSession = new WorkerSupervisorAdapter({
      kill: (_pid, signal) => { wallClockChild.exit(null, signal); },
      nowMs: () => now,
      sleep: wallClockSleeper.sleep,
      spawn: fakeSpawn([wallClockChild], []),
    }).start({
      command: 'agent',
      id: 'T11-wall',
      pollIntervalMs: 1,
      watchdog: { maxRestarts: 2, wallClockCapMs: 1_000 },
      worktree: await tempRoot(),
    })

    now = 1_000
    await wallClockSleeper.tick()
    await expect(wallClockSession.result).resolves.toMatchObject({
      detection: { capMs: 1_000, elapsedMs: 1_000, kind: 'wall-clock-cap' },
      restarts: 0,
      status: 'budget-cap',
    })

    const outputSession = new WorkerSupervisorAdapter({
      kill: (_pid, signal) => { outputChild.exit(null, signal); },
      sleep: outputSleeper.sleep,
      spawn: fakeSpawn([outputChild], []),
    }).start({
      command: 'agent',
      id: 'T11-output',
      pollIntervalMs: 1,
      watchdog: { maxRestarts: 2, outputCapBytes: 4 },
      worktree: await tempRoot(),
    })
    outputChild.stdout.write('12345')
    await outputSleeper.tick()

    await expect(outputSession.result).resolves.toMatchObject({
      detection: { capBytes: 4, kind: 'output-cap', outputBytes: 5 },
      restarts: 0,
      status: 'budget-cap',
      stdout: '12345',
      stdoutBytes: 5,
      stdoutLogPath: 'workers/T11-output/logs/stdout.log',
    })
  })

  it('retries attempt timeouts once then fails fast on the repeated fingerprint', async () => {
    const first = new FakeChild(921)
    const second = new FakeChild(922)
    const records: SpawnRecord[] = []
    const sleeper = deferredSleep()
    let now = 0
    const adapter = new WorkerSupervisorAdapter({
      kill: (pid, signal) => {
        if (pid === -921) {
          first.exit(null, signal)
        }
        if (pid === -922) {
          second.exit(null, signal)
        }
      },
      nowMs: () => now,
      sleep: sleeper.sleep,
      spawn: fakeSpawn([first, second], records),
    })
    const session = adapter.start({
      command: 'agent',
      id: 'T11-attempt',
      pollIntervalMs: 1,
      watchdog: { attemptTimeoutMs: 1_000, maxRestarts: 2 },
      worktree: await tempRoot(),
    })

    now = 1_000
    await sleeper.tick()
    expect(records).toHaveLength(2)

    now = 2_000
    await sleeper.tick()

    await expect(session.result).resolves.toMatchObject({
      detection: { elapsedMs: 1_000, kind: 'attempt-timeout', timeoutMs: 1_000 },
      restarts: 1,
      status: 'stalled',
    })
    expect(records).toHaveLength(2)
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

  it('reattaches to a real live pid through default liveness and stops it', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'council-reattach-'))
    const child = nodeSpawn(process.execPath, ['-e', 'setInterval(() => {}, 1_000)'], {
      detached: true,
      stdio: 'ignore',
    })
    if (child.pid === undefined) {
      throw new Error('expected child pid')
    }
    const snapshots: WorkerSupervisorSnapshot[] = []
    const adapter = new WorkerSupervisorAdapter({
      onSnapshot: (snapshot) => { snapshots.push(snapshot); },
      sleep: () => new Promise(() => undefined),
    })
    const session = adapter.reattach({
      command: 'agent',
      id: 'real-reattach',
      worktree: dir,
    }, supervisorSnapshot('real-reattach', { pid: child.pid }))

    await session.stop('test complete')

    await expect(session.result).resolves.toMatchObject({ status: 'stopped' })
    expect(snapshots).toContainEqual(expect.objectContaining({
      pid: child.pid,
      status: 'stopped',
      task_id: 'real-reattach',
    }))
    await rm(dir, { force: true, recursive: true })
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

async function tempRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), 'council-supervisor-'))
  tempRoots.push(root)
  return root
}

function tail(text: string): string {
  return text.slice(-PROCESS_TAIL_MAX_CHARS)
}

function supervisorSnapshot(
  taskId: string,
  overrides: Partial<WorkerSupervisorSnapshot> = {},
): WorkerSupervisorSnapshot {
  return {
    attempt_id: 2,
    logs: {
      stderr: `workers/${taskId}/logs/stderr.log`,
      stdout: `workers/${taskId}/logs/stdout.log`,
    },
    model_tier: 'cheap',
    offsets: {
      stderr: 0,
      stdout: 0,
    },
    pid: 4242,
    restart_count: 1,
    status: 'running',
    task_id: taskId,
    watchdog: {
      handling_detection: false,
      loop: { actions: [] },
      progress: {
        attemptStartedAtMs: 100,
        lastActionAtMs: 100,
        lastOutputAtMs: 100,
        lastProgressAtMs: 100,
        outputBytes: 0,
        startedAtMs: 0,
      },
      retry: {
        attempts: 0,
        failureFingerprints: [],
      },
    },
    ...overrides,
  }
}

function grandchildScript(marker: string): string {
  return [
    "const { writeFileSync } = require('node:fs')",
    `setInterval(() => writeFileSync(${JSON.stringify(marker)}, String(Date.now())), 20)`,
  ].join('\n')
}
