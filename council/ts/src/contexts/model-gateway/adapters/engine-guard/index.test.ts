import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { FakeEnv, FixedClock } from '../../../../../test/model-gateway/harness.js'
import { ledgerPath } from '../../application/locations.js'
import { nodeEngineGuard } from './index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

const SHIPPED_DEFAULTS = join(import.meta.dirname, '../../../../../../../skills/workstation/model-gateway/defaults.json')

describe('nodeEngineGuard', () => {
  it('guards and records on the machine’s own ledger, under the home it is given and nowhere else', async () => {
    const home = await mkdtemp(join(tmpdir(), 'model-gateway-engine-guard-home-'))
    cleanups.push(() => rm(home, { force: true, recursive: true }))
    const env = new FakeEnv({ HOME: home })
    const guard = nodeEngineGuard({ clock: new FixedClock(), defaultsPath: SHIPPED_DEFAULTS, env })

    const refused = await guard.authorize({ client: 'codex', model: 'gpt-6.1-sol', prompt_chars: 100, workflow: 'run-1' })
      .then(() => undefined, (error: unknown) => (error as { code: string }).code)
    expect(refused).toBe('policy-refused')

    const { reservation } = await guard.authorize({ carries_history: false, client: 'claude', model: 'haiku', prompt_chars: 100, workflow: 'run-1' })
    await guard.record({ duration_s: 2, outcome: 'ok', reservation, run_id: reservation.run_id })

    const lines = (await readFile(ledgerPath(env), 'utf8')).trim().split('\n').map((line) => JSON.parse(line) as Record<string, unknown>)
    expect(lines).toHaveLength(1)
    expect(lines[0]).toMatchObject({ client: 'claude', mode: 'write', model: 'claude-haiku-5-5', run_id: reservation.run_id, source: 'council', workflow: 'run-1' })
    expect(ledgerPath(env).startsWith(home)).toBe(true)
  })
})
