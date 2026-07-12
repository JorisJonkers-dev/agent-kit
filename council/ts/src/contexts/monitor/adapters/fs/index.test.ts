import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { createMonitorFsAdapter } from './index.js'
import type { MonitorState } from '../../domain/index.js'

const tempDirs: string[] = []

afterEach(async () => {
  await Promise.all(tempDirs.splice(0).map((dir) => rm(dir, { force: true, recursive: true })))
})

async function makeTempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'monitor-fs-test-'))
  tempDirs.push(dir)
  return dir
}

function makeState(overrides: Partial<MonitorState> = {}): MonitorState {
  return {
    name: 'test-monitor',
    status: 'polling',
    startedAt: '2026-01-01T00:00:00.000Z',
    deadline: '2026-01-01T01:00:00.000Z',
    lastTickAt: '2026-01-01T00:00:00.000Z',
    lastOutput: 'some output',
    intervalMs: 5_000,
    cmd: 'echo hello',
    until: 'hello',
    then: 'echo done',
    ...overrides,
  }
}

describe('MonitorFsAdapter', () => {
  it('writes and reads a monitor state', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const state = makeState()

    await adapter.writeState(execDir, state)
    const read = await adapter.readState(execDir, 'test-monitor')

    expect(read).toEqual(state)
  })

  it('overwrites existing state with updated values', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const initial = makeState({ status: 'polling' })
    const updated = makeState({ status: 'passed', lastOutput: 'done' })

    await adapter.writeState(execDir, initial)
    await adapter.writeState(execDir, updated)
    const read = await adapter.readState(execDir, 'test-monitor')

    expect(read.status).toBe('passed')
    expect(read.lastOutput).toBe('done')
  })

  it('lists all monitor states in the exec dir', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()

    await adapter.writeState(execDir, makeState({ name: 'monitor-a' }))
    await adapter.writeState(execDir, makeState({ name: 'monitor-b' }))

    const states = await adapter.listStates(execDir)
    const names = states.map((s) => s.name).sort()

    expect(names).toEqual(['monitor-a', 'monitor-b'])
  })

  it('returns empty list when monitors directory does not exist', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()

    const states = await adapter.listStates(execDir)
    expect(states).toEqual([])
  })

  it('throws when reading a non-existent monitor', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()

    await expect(adapter.readState(execDir, 'missing')).rejects.toThrow()
  })

  it('creates the monitors directory if it does not exist', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const state = makeState({ name: 'new-monitor' })

    await expect(adapter.writeState(execDir, state)).resolves.not.toThrow()
    const read = await adapter.readState(execDir, 'new-monitor')
    expect(read.name).toBe('new-monitor')
  })
})

describe('MonitorFsAdapter error cases', () => {
  it('skips monitor files that contain valid JSON but are not objects', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const { mkdir, writeFile } = await import('node:fs/promises')
    const { join } = await import('node:path')

    await mkdir(join(execDir, 'monitors'), { recursive: true })
    // Valid JSON but not an object
    await writeFile(join(execDir, 'monitors', 'array.json'), '["not", "an", "object"]', 'utf8')
    await writeFile(join(execDir, 'monitors', 'null.json'), 'null', 'utf8')
    await adapter.writeState(execDir, makeState({ name: 'good-monitor' }))

    const states = await adapter.listStates(execDir)
    expect(states).toHaveLength(1)
    expect(states[0]?.name).toBe('good-monitor')
  })

  it('skips non-JSON files in the monitors directory', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const { mkdir, writeFile } = await import('node:fs/promises')
    const { join } = await import('node:path')

    await mkdir(join(execDir, 'monitors'), { recursive: true })
    // Write a file that is not valid JSON
    await writeFile(join(execDir, 'monitors', 'broken.json'), 'not valid json', 'utf8')
    // Write a valid monitor state
    await adapter.writeState(execDir, makeState({ name: 'valid-monitor' }))

    const states = await adapter.listStates(execDir)
    expect(states).toHaveLength(1)
    expect(states[0]?.name).toBe('valid-monitor')
  })

  it('skips non-.json files in the monitors directory', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const { mkdir, writeFile } = await import('node:fs/promises')
    const { join } = await import('node:path')

    await mkdir(join(execDir, 'monitors'), { recursive: true })
    await writeFile(join(execDir, 'monitors', 'not-a-json-file.txt'), 'ignored', 'utf8')
    await adapter.writeState(execDir, makeState({ name: 'real-monitor' }))

    const states = await adapter.listStates(execDir)
    expect(states).toHaveLength(1)
    expect(states[0]?.name).toBe('real-monitor')
  })
})

describe('MonitorFsAdapter parseMonitorState validation', () => {
  it('skips state files with non-string name field', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const { mkdir, writeFile } = await import('node:fs/promises')
    const { join } = await import('node:path')

    await mkdir(join(execDir, 'monitors'), { recursive: true })
    const badState = {
      name: 123, // should be string
      status: 'polling',
      startedAt: '2026-01-01T00:00:00.000Z',
      deadline: '2026-01-01T01:00:00.000Z',
      lastTickAt: '2026-01-01T00:00:00.000Z',
      lastOutput: '',
      intervalMs: 5000,
      cmd: 'echo',
      until: 'done',
      then: '',
    }
    await writeFile(join(execDir, 'monitors', 'bad.json'), JSON.stringify(badState), 'utf8')

    const states = await adapter.listStates(execDir)
    expect(states).toHaveLength(0)
  })

  it('skips state files with non-number intervalMs field', async () => {
    const execDir = await makeTempDir()
    const adapter = createMonitorFsAdapter()
    const { mkdir, writeFile } = await import('node:fs/promises')
    const { join } = await import('node:path')

    await mkdir(join(execDir, 'monitors'), { recursive: true })
    const badState = {
      name: 'test',
      status: 'polling',
      startedAt: '2026-01-01T00:00:00.000Z',
      deadline: '2026-01-01T01:00:00.000Z',
      lastTickAt: '2026-01-01T00:00:00.000Z',
      lastOutput: '',
      intervalMs: 'not-a-number', // should be number
      cmd: 'echo',
      until: 'done',
      then: '',
    }
    await writeFile(join(execDir, 'monitors', 'bad-interval.json'), JSON.stringify(badState), 'utf8')

    const states = await adapter.listStates(execDir)
    expect(states).toHaveLength(0)
  })
})
