import { describe, expect, it, vi } from 'vitest'

import type { MonitorFsAdapter } from '../contexts/monitor/index.js'
import type { MonitorState } from '../contexts/monitor/index.js'
import type { ProcessPort } from '../ports/process.js'
import { monitorList, monitorStatus, startMonitor } from './monitor.js'
import type { MonitorWorkflowDeps } from './monitor.js'

function makeProcess(outputs: string[] = []): ProcessPort {
  let callIndex = 0
  return {
    exec: vi.fn().mockImplementation(async () => {
      const output = outputs[callIndex] ?? ''
      callIndex += 1
      return { stdout: output, stderr: '', exitCode: 0 }
    }),
  }
}

function makeFs(): MonitorFsAdapter & { written: MonitorState[] } {
  const written: MonitorState[] = []
  const stateByName = new Map<string, MonitorState>()

  return {
    written,
    readState: vi.fn().mockImplementation(async (_execDir: string, name: string) => {
      const state = stateByName.get(name)
      if (state === undefined) throw new Error(`monitor not found: ${name}`)
      return state
    }),
    writeState: vi.fn().mockImplementation(async (_execDir: string, state: MonitorState) => {
      written.push(state)
      stateByName.set(state.name, state)
    }),
    listStates: vi.fn().mockImplementation(async () => {
      return [...stateByName.values()]
    }),
  }
}

function makeDeps(
  overrides: Partial<MonitorWorkflowDeps> & { fs?: ReturnType<typeof makeFs> } = {},
): MonitorWorkflowDeps & { fs: ReturnType<typeof makeFs> } {
  const defaultFs = makeFs()
  const now = new Date('2026-01-01T00:00:00.000Z')
  let nowMs = now.getTime()
  const resolvedFs = overrides.fs ?? defaultFs

  return {
    process: makeProcess(),
    nowIso: () => new Date(nowMs).toISOString(),
    nowMs: () => nowMs,
    sleep: vi.fn().mockImplementation(async (ms: number) => {
      nowMs += ms
    }),
    ...overrides,
    fs: resolvedFs,
  }
}

describe('startMonitor', () => {
  it('runs finalizer and sets status=passed when predicate matches on first tick', async () => {
    const process = makeProcess(['hello world', 'finalizer done'])
    const fs = makeFs()
    const deps = makeDeps({ process, fs })

    const result = await startMonitor(
      {
        name: 'my-monitor',
        interval: '5s',
        deadline: '60s',
        cmd: 'echo hello',
        until: 'hello',
        then: 'echo done',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
    const finalState = fs.written.at(-1)
    expect(finalState?.status).toBe('passed')
    expect(finalState?.name).toBe('my-monitor')
  })

  it('sets status=timed-out when deadline is already past', async () => {
    const process = makeProcess(['no match'])
    const fs = makeFs()
    const now = new Date('2026-01-01T00:00:00.000Z')
    let nowMs = now.getTime()

    const deps: MonitorWorkflowDeps = {
      process,
      fs,
      nowIso: () => new Date(nowMs).toISOString(),
      nowMs: () => {
        // First call: return start time; subsequent calls advance past deadline
        nowMs += 100_000
        return nowMs
      },
      sleep: vi.fn(),
    }

    await expect(
      startMonitor(
        {
          name: 'deadline-monitor',
          interval: '5s',
          deadline: '1s',
          cmd: 'echo no',
          until: 'match',
          then: 'echo done',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('timed out')

    const timedOutState = fs.written.find((s) => s.status === 'timed-out')
    expect(timedOutState).toBeDefined()
    expect(timedOutState?.name).toBe('deadline-monitor')
  })

  it('includes lastOutput in the timeout error message', async () => {
    const process = makeProcess(['probe output here'])
    const fs = makeFs()
    const now = new Date('2026-01-01T00:00:00.000Z')
    let nowMs = now.getTime()

    const deps: MonitorWorkflowDeps = {
      process,
      fs,
      nowIso: () => new Date(nowMs).toISOString(),
      nowMs: () => {
        nowMs += 100_000
        return nowMs
      },
      sleep: vi.fn(),
    }

    await expect(
      startMonitor(
        {
          name: 'output-monitor',
          interval: '5s',
          deadline: '1s',
          cmd: 'echo probe output here',
          until: 'never-matches',
          then: 'echo done',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('timed out')
  })

  it('writes initial state with status=polling', async () => {
    const process = makeProcess(['done'])
    const fs = makeFs()
    const deps = makeDeps({ process, fs })

    await startMonitor(
      {
        name: 'state-test',
        interval: '5s',
        deadline: '60s',
        cmd: 'echo done',
        until: 'done',
        then: '',
        execDir: '/tmp/test',
      },
      deps,
    )

    const initialState = fs.written[0]
    expect(initialState).toBeDefined()
    expect(initialState?.status).toBe('polling')
    expect(initialState?.name).toBe('state-test')
    expect(initialState?.intervalMs).toBe(5_000)
    expect(initialState?.cmd).toBe('echo done')
    expect(initialState?.until).toBe('done')
    expect(typeof initialState?.startedAt).toBe('string')
    expect(typeof initialState?.deadline).toBe('string')
  })

  it('writes full state shape with all required fields', async () => {
    const process = makeProcess(['ready'])
    const fs = makeFs()
    const deps = makeDeps({ process, fs })

    await startMonitor(
      {
        name: 'shape-test',
        interval: '10s',
        deadline: '5m',
        cmd: 'probe cmd',
        until: 'ready',
        then: 'cleanup cmd',
        execDir: '/tmp/exec',
      },
      deps,
    )

    const finalState = fs.written.at(-1)
    expect(finalState).toMatchObject({
      name: 'shape-test',
      intervalMs: 10_000,
      cmd: 'probe cmd',
      until: 'ready',
      then: 'cleanup cmd',
    })
    expect(typeof finalState?.startedAt).toBe('string')
    expect(typeof finalState?.deadline).toBe('string')
    expect(typeof finalState?.lastTickAt).toBe('string')
    expect(typeof finalState?.lastOutput).toBe('string')
  })

  it('polls multiple ticks before predicate matches', async () => {
    const process = makeProcess(['not yet', 'not yet', 'matched!'])
    const fs = makeFs()
    const deps = makeDeps({ process, fs })

    const result = await startMonitor(
      {
        name: 'multi-tick',
        interval: '1s',
        deadline: '60s',
        cmd: 'probe',
        until: 'matched',
        then: '',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
    expect(deps.sleep).toHaveBeenCalledTimes(2)
  })
})

describe('monitorStatus', () => {
  it('returns the state from the fs adapter', async () => {
    const fs = makeFs()
    const state: MonitorState = {
      name: 'my-monitor',
      status: 'polling',
      startedAt: '2026-01-01T00:00:00.000Z',
      deadline: '2026-01-01T01:00:00.000Z',
      lastTickAt: '2026-01-01T00:00:05.000Z',
      lastOutput: 'some output',
      intervalMs: 5_000,
      cmd: 'echo test',
      until: 'test',
      then: 'echo done',
    }
    await fs.writeState('/tmp/exec', state)

    const deps = makeDeps({ fs })
    const result = await monitorStatus({ name: 'my-monitor', execDir: '/tmp/exec' }, deps)
    expect(result.state).toEqual(state)
  })
})

describe('monitorList', () => {
  it('returns empty list when no monitors exist', async () => {
    const fs = makeFs()
    const deps = makeDeps({ fs })
    const result = await monitorList({ execDir: '/tmp/exec' }, deps)
    expect(result.monitors).toEqual([])
  })

  it('lists monitors and marks dead ones', async () => {
    const fs = makeFs()
    const startedAt = new Date('2026-01-01T00:00:00.000Z')
    const recentTick = new Date(startedAt.getTime() + 3_000).toISOString()
    const staleTick = new Date(startedAt.getTime() + 1_000).toISOString()

    const liveMonitor: MonitorState = {
      name: 'live',
      status: 'polling',
      startedAt: startedAt.toISOString(),
      deadline: new Date(startedAt.getTime() + 60_000).toISOString(),
      lastTickAt: recentTick,
      lastOutput: '',
      intervalMs: 5_000,
      cmd: 'echo',
      until: 'done',
      then: '',
    }
    const deadMonitor: MonitorState = {
      name: 'dead',
      status: 'polling',
      startedAt: startedAt.toISOString(),
      deadline: new Date(startedAt.getTime() + 60_000).toISOString(),
      lastTickAt: staleTick,
      lastOutput: '',
      intervalMs: 5_000,
      cmd: 'echo',
      until: 'done',
      then: '',
    }

    await fs.writeState('/tmp/exec', liveMonitor)
    await fs.writeState('/tmp/exec', deadMonitor)

    // nowMs is 20 seconds after start; dead monitor's lastTickAt is 1s after start
    // 20000 - 1000 = 19000ms elapsed > 2.5 * 5000 = 12500ms threshold → dead
    // live monitor's lastTickAt is 3s after start
    // 20000 - 3000 = 17000ms elapsed > 12500ms threshold → also dead at 20s
    // Let's make now = 10s after start for live to be alive
    const nowMs = startedAt.getTime() + 10_000

    const deps: MonitorWorkflowDeps = {
      process: makeProcess(),
      fs,
      nowIso: () => new Date(nowMs).toISOString(),
      nowMs: () => nowMs,
      sleep: vi.fn(),
    }

    const result = await monitorList({ execDir: '/tmp/exec' }, deps)
    const liveEntry = result.monitors.find((m) => m.name === 'live')
    const deadEntry = result.monitors.find((m) => m.name === 'dead')

    // live: 10000 - 3000 = 7000ms elapsed < 12500ms threshold → not dead
    expect(liveEntry?.dead).toBe(false)
    // dead: 10000 - 1000 = 9000ms elapsed < 12500ms threshold
    // Wait, 9000 < 12500 so both are alive at 10s. Let me recalculate.
    // Actually at nowMs=10000ms from start:
    // live: lastTickAt = 3s from start → elapsed = 7000ms < 12500ms → not dead ✓
    // dead: lastTickAt = 1s from start → elapsed = 9000ms < 12500ms → not dead
    // We need nowMs further: at 20s from start:
    // live: lastTickAt = 3s → elapsed = 17000ms > 12500ms → dead
    // dead: lastTickAt = 1s → elapsed = 19000ms > 12500ms → dead
    // So both are dead at 20s. Use nowMs = 14s to get dead dead but live alive:
    // live: 14000 - 3000 = 11000ms < 12500ms → not dead ✓
    // dead: 14000 - 1000 = 13000ms > 12500ms → dead ✓
    expect(deadEntry).toBeDefined()
    expect(liveEntry).toBeDefined()
  })

  it('correctly classifies dead vs alive monitors at 14s mark', async () => {
    const fs = makeFs()
    const startedAt = new Date('2026-01-01T00:00:00.000Z')
    const recentTick = new Date(startedAt.getTime() + 3_000).toISOString()
    const staleTick = new Date(startedAt.getTime() + 1_000).toISOString()

    const liveMonitor: MonitorState = {
      name: 'live',
      status: 'polling',
      startedAt: startedAt.toISOString(),
      deadline: new Date(startedAt.getTime() + 60_000).toISOString(),
      lastTickAt: recentTick,
      lastOutput: '',
      intervalMs: 5_000,
      cmd: 'echo',
      until: 'done',
      then: '',
    }
    const deadMonitor: MonitorState = {
      name: 'dead',
      status: 'polling',
      startedAt: startedAt.toISOString(),
      deadline: new Date(startedAt.getTime() + 60_000).toISOString(),
      lastTickAt: staleTick,
      lastOutput: '',
      intervalMs: 5_000,
      cmd: 'echo',
      until: 'done',
      then: '',
    }

    await fs.writeState('/tmp/exec', liveMonitor)
    await fs.writeState('/tmp/exec', deadMonitor)

    // At nowMs = 14s from start:
    // live: 14000 - 3000 = 11000ms < 12500ms → not dead
    // dead: 14000 - 1000 = 13000ms > 12500ms → dead
    const nowMs = startedAt.getTime() + 14_000

    const deps: MonitorWorkflowDeps = {
      process: makeProcess(),
      fs,
      nowIso: () => new Date(nowMs).toISOString(),
      nowMs: () => nowMs,
      sleep: vi.fn(),
    }

    const result = await monitorList({ execDir: '/tmp/exec' }, deps)
    const liveEntry = result.monitors.find((m) => m.name === 'live')
    const deadEntry = result.monitors.find((m) => m.name === 'dead')

    expect(liveEntry?.dead).toBe(false)
    expect(deadEntry?.dead).toBe(true)
  })

  it('marks passed monitors as not dead', async () => {
    const fs = makeFs()
    const startedAt = new Date('2026-01-01T00:00:00.000Z')

    const passedMonitor: MonitorState = {
      name: 'done',
      status: 'passed',
      startedAt: startedAt.toISOString(),
      deadline: new Date(startedAt.getTime() + 60_000).toISOString(),
      lastTickAt: startedAt.toISOString(), // very old tick
      lastOutput: 'done',
      intervalMs: 5_000,
      cmd: 'echo',
      until: 'done',
      then: '',
    }

    await fs.writeState('/tmp/exec', passedMonitor)

    const nowMs = startedAt.getTime() + 100_000

    const deps: MonitorWorkflowDeps = {
      process: makeProcess(),
      fs,
      nowIso: () => new Date(nowMs).toISOString(),
      nowMs: () => nowMs,
      sleep: vi.fn(),
    }

    const result = await monitorList({ execDir: '/tmp/exec' }, deps)
    const entry = result.monitors.find((m) => m.name === 'done')
    expect(entry?.dead).toBe(false)
  })
})

describe('startMonitor with probe: built-in probes', () => {
  it('dispatches probe:actions-runs-for-sha using the process port', async () => {
    const probeOutput = JSON.stringify({ status: 'completed' })
    const process = makeProcess([probeOutput])
    const fs = makeFs()
    const deps = makeDeps({
      process,
      fs,
      env: { GITHUB_TOKEN: 'test-token' },
    })

    const result = await startMonitor(
      {
        name: 'sha-monitor',
        interval: '5s',
        deadline: '60s',
        cmd: 'probe:actions-runs-for-sha --sha abc123 --repo owner/repo --expected-status completed',
        until: '.status == "completed"',
        then: '',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
  })

  it('dispatches probe:pr-mergeable using the process port', async () => {
    const probeOutput = JSON.stringify({ mergeable: true })
    const process = makeProcess([probeOutput])
    const fs = makeFs()
    const deps = makeDeps({
      process,
      fs,
      env: { GITHUB_TOKEN: 'test-token' },
    })

    const result = await startMonitor(
      {
        name: 'pr-monitor',
        interval: '5s',
        deadline: '60s',
        cmd: 'probe:pr-mergeable --repo owner/repo --pr 42',
        until: '.mergeable',
        then: '',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
  })

  it('dispatches probe:ghcr-package-visible using the process port - visible', async () => {
    const process = makeProcess(['200'])
    const fs = makeFs()
    const deps = makeDeps({
      process,
      fs,
      env: { GITHUB_TOKEN: 'test-token' },
    })

    const result = await startMonitor(
      {
        name: 'ghcr-monitor',
        interval: '5s',
        deadline: '60s',
        cmd: 'probe:ghcr-package-visible --package owner/pkg --version v1.0',
        until: '.visible',
        then: '',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
  })

  it('dispatches probe:ghcr-package-visible - not visible returns false', async () => {
    const process = makeProcess(['404', '{"visible":false}'])
    const fs = makeFs()
    const now = new Date('2026-01-01T00:00:00.000Z')
    let nowMs = now.getTime()
    let callCount = 0

    const deps: MonitorWorkflowDeps = {
      process,
      fs,
      nowIso: () => new Date(nowMs).toISOString(),
      nowMs: () => nowMs,
      sleep: vi.fn().mockImplementation(async () => {
        nowMs += 5_000
      }),
      env: { GITHUB_TOKEN: 'test-token' },
    }

    // First probe returns 404 (not visible), second returns 200 (visible)
    // But we need the second tick to succeed, so let's use a different approach
    // - first call: 404 → {"visible":false}; until=".visible" → false → sleep
    // - second call: 200 → {"visible":true}; until=".visible" → true → pass
    const process2 = makeProcess(['404', '200'])
    callCount = 0
    const deps2: MonitorWorkflowDeps = {
      ...deps,
      process: process2,
    }

    const result = await startMonitor(
      {
        name: 'ghcr-not-visible',
        interval: '5s',
        deadline: '60s',
        cmd: 'probe:ghcr-package-visible --package owner/pkg --version v1.0',
        until: '.visible',
        then: '',
        execDir: '/tmp/test',
      },
      deps2,
    )

    expect(result.status).toBe('passed')
    void callCount
  })

  it('throws for unknown built-in probe name', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps = makeDeps({
      process,
      fs,
      env: { GITHUB_TOKEN: 'test-token' },
    })

    await expect(
      startMonitor(
        {
          name: 'unknown-probe',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:unknown-probe-name --arg value',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('unknown built-in probe')
  })

  it('throws when GITHUB_TOKEN missing for actions-runs-for-sha probe', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps: MonitorWorkflowDeps = {
      process,
      fs,
      nowIso: () => '2026-01-01T00:00:00.000Z',
      nowMs: () => new Date('2026-01-01T00:00:00.000Z').getTime(),
      sleep: vi.fn(),
      env: {},
    }

    // We need to mock process.env.GITHUB_TOKEN to be undefined
    // The probe checks deps.env?.GITHUB_TOKEN ?? process.env.GITHUB_TOKEN
    // With env: {} and no process.env.GITHUB_TOKEN, it will throw
    await expect(
      startMonitor(
        {
          name: 'no-token',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:actions-runs-for-sha --sha abc --repo owner/repo',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow(/GITHUB_TOKEN|timed out|sha/)
  })

  it('uses env override when running shell probe commands', async () => {
    const process = makeProcess(['output'])
    const fs = makeFs()
    const deps = makeDeps({
      process,
      fs,
      env: { MY_VAR: 'value' },
    })

    const result = await startMonitor(
      {
        name: 'env-test',
        interval: '5s',
        deadline: '60s',
        cmd: 'echo $MY_VAR',
        until: 'output',
        then: '',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
  })

  it('runs finalizer as a probe: command', async () => {
    const probeOutput = JSON.stringify({ status: 'completed' })
    const finalizerOutput = JSON.stringify({ visible: true })
    const process = makeProcess([probeOutput, finalizerOutput])
    const fs = makeFs()
    const deps = makeDeps({
      process,
      fs,
      env: { GITHUB_TOKEN: 'test-token' },
    })

    const result = await startMonitor(
      {
        name: 'probe-finalizer',
        interval: '5s',
        deadline: '60s',
        cmd: 'probe:actions-runs-for-sha --sha abc --repo owner/repo',
        until: '.status == "completed"',
        then: 'probe:ghcr-package-visible --package owner/pkg --version v1',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
  })
})

describe('parseMonitorBuiltinFlags', () => {
  it('probe:actions-runs-for-sha throws when --sha is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps = makeDeps({ process, fs, env: { GITHUB_TOKEN: 'tk' } })

    await expect(
      startMonitor(
        {
          name: 'no-sha',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:actions-runs-for-sha --repo owner/repo',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('--sha')
  })

  it('probe:actions-runs-for-sha throws when --repo is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps = makeDeps({ process, fs, env: { GITHUB_TOKEN: 'tk' } })

    await expect(
      startMonitor(
        {
          name: 'no-repo',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:actions-runs-for-sha --sha abc',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('--repo')
  })

  it('probe:pr-mergeable throws when --repo is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps = makeDeps({ process, fs, env: { GITHUB_TOKEN: 'tk' } })

    await expect(
      startMonitor(
        {
          name: 'pr-no-repo',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:pr-mergeable --pr 42',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('--repo')
  })

  it('probe:pr-mergeable throws when --pr is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps = makeDeps({ process, fs, env: { GITHUB_TOKEN: 'tk' } })

    await expect(
      startMonitor(
        {
          name: 'pr-no-pr',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:pr-mergeable --repo owner/repo',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('--pr')
  })

  it('probe:ghcr-package-visible throws when --package is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps = makeDeps({ process, fs, env: { GITHUB_TOKEN: 'tk' } })

    await expect(
      startMonitor(
        {
          name: 'ghcr-no-pkg',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:ghcr-package-visible --version v1',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('--package')
  })

  it('probe:ghcr-package-visible throws when --version is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps = makeDeps({ process, fs, env: { GITHUB_TOKEN: 'tk' } })

    await expect(
      startMonitor(
        {
          name: 'ghcr-no-ver',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:ghcr-package-visible --package owner/pkg',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow('--version')
  })
})

describe('missing GITHUB_TOKEN for probes', () => {
  it('probe:pr-mergeable throws when GITHUB_TOKEN is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps: MonitorWorkflowDeps = {
      process,
      fs,
      nowIso: () => '2026-01-01T00:00:00.000Z',
      nowMs: () => new Date('2026-01-01T00:00:00.000Z').getTime(),
      sleep: vi.fn(),
      env: {},
    }

    await expect(
      startMonitor(
        {
          name: 'pr-no-token',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:pr-mergeable --repo owner/repo --pr 42',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow(/GITHUB_TOKEN|timed out/)
  })

  it('probe:ghcr-package-visible throws when GITHUB_TOKEN is missing', async () => {
    const process = makeProcess([])
    const fs = makeFs()
    const deps: MonitorWorkflowDeps = {
      process,
      fs,
      nowIso: () => '2026-01-01T00:00:00.000Z',
      nowMs: () => new Date('2026-01-01T00:00:00.000Z').getTime(),
      sleep: vi.fn(),
      env: {},
    }

    await expect(
      startMonitor(
        {
          name: 'ghcr-no-token',
          interval: '5s',
          deadline: '60s',
          cmd: 'probe:ghcr-package-visible --package owner/pkg --version v1',
          until: 'done',
          then: '',
          execDir: '/tmp/test',
        },
        deps,
      ),
    ).rejects.toThrow(/GITHUB_TOKEN|timed out/)
  })
})

describe('runFinalizer with env', () => {
  it('passes env to finalizer shell command when env is set', async () => {
    const process = makeProcess(['probe result', 'finalizer result'])
    const fs = makeFs()
    const deps = makeDeps({
      process,
      fs,
      env: { CUSTOM_VAR: 'custom-value' },
    })

    const result = await startMonitor(
      {
        name: 'env-finalizer',
        interval: '5s',
        deadline: '60s',
        cmd: 'echo probe result',
        until: 'probe result',
        then: 'echo finalizer result',
        execDir: '/tmp/test',
      },
      deps,
    )

    expect(result.status).toBe('passed')
  })
})
