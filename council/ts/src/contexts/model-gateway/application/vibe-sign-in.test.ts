import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { FakeFiles, FakeProcess } from '../../../../test/model-gateway/harness.js'
import type { ProcessResult } from '../../../ports/index.js'
import { keyringQuery, vibeSignedIn } from './vibe-sign-in.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

/** No real key: the fakes answer the keyring lookups, and the `.env` holds a placeholder. */
async function signedIn(
  platform: NodeJS.Platform,
  listings: Readonly<Record<string, ProcessResult>>,
  dotenv?: string,
): Promise<{ readonly answer: boolean; readonly process: FakeProcess }> {
  const dir = await mkdtemp(join(tmpdir(), 'vibe-sign-in-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  const home = join(dir, '.vibe')
  const files = new FakeFiles()
  if (dotenv !== undefined) await files.writeText(join(home, '.env'), dotenv)
  const process = new FakeProcess([], listings)
  return { answer: await vibeSignedIn({ files, process }, home, platform), process }
}

const found = (stdout: string): ProcessResult => ({ exitCode: 0, stderr: '', stdout })
const NO_ITEMS = found('(@ao [], @ao [])\n')

describe("Vibe's sign-in", () => {
  it('counts the Keychain item on macOS, asked without printing it', async () => {
    const { answer, process } = await signedIn('darwin', { security: found('keychain: "/Users/me/Library/Keychains/login.keychain-db"\n') })

    expect(answer).toBe(true)
    expect(process.ran).toEqual([
      { args: ['find-generic-password', '-s', 'ai.mistral.vibe', '-a', 'MISTRAL_API_KEY'], command: 'security' },
    ])
    expect(process.ran[0]?.args).not.toContain('-w')
  })

  it('falls back to the .env on macOS when the Keychain has no item', async () => {
    const missing = { exitCode: 44, stderr: 'security: The specified item could not be found in the keychain.\n', stdout: '' }

    expect((await signedIn('darwin', { security: missing })).answer).toBe(false)
    expect((await signedIn('darwin', { security: missing }, "MISTRAL_API_KEY='placeholder'\n")).answer).toBe(true)
  })

  it('counts an unlocked Secret Service item on Linux', async () => {
    const { answer, process } = await signedIn('linux', { gdbus: found("([objectpath '/org/freedesktop/secrets/collection/login/3'], @ao [])\n") })

    expect(answer).toBe(true)
    expect(process.ran.map((command) => command.command)).toEqual(['gdbus'])
    expect(process.ran[0]?.args).toContain('org.freedesktop.Secret.Service.SearchItems')
    expect(process.ran[0]?.args).toContain("{'service': 'ai.mistral.vibe', 'username': 'MISTRAL_API_KEY'}")
  })

  it('counts a locked Secret Service item on Linux: it exists, and Vibe unlocks it', async () => {
    expect((await signedIn('linux', { gdbus: found("(@ao [], [objectpath '/org/freedesktop/secrets/collection/login/3'])\n") })).answer).toBe(true)
  })

  it('is signed out on Linux when the Secret Service has no item and there is no .env', async () => {
    expect((await signedIn('linux', { gdbus: NO_ITEMS })).answer).toBe(false)
  })

  it('is signed out on Linux when the .env sets the key to nothing', async () => {
    expect((await signedIn('linux', { gdbus: NO_ITEMS }, 'MISTRAL_API_KEY=\nOTHER=x\n')).answer).toBe(false)
  })

  it('counts the .env on Linux when the Secret Service has no item', async () => {
    expect((await signedIn('linux', { gdbus: NO_ITEMS }, 'MISTRAL_API_KEY=placeholder\n')).answer).toBe(true)
  })

  it('counts the .env on Linux when there is no Secret Service to ask', async () => {
    const noService = { exitCode: 1, stderr: 'Error: GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown\n', stdout: '' }

    expect((await signedIn('linux', { gdbus: noService })).answer).toBe(false)
    expect((await signedIn('linux', {}, 'MISTRAL_API_KEY="placeholder"\n')).answer).toBe(true)
  })

  it('times the Secret Service lookup, and leaves the Keychain untimed', () => {
    expect(keyringQuery('linux')?.timeoutMs).toBeGreaterThan(0)
    expect(keyringQuery('darwin')?.timeoutMs).toBeUndefined()
  })

  it('asks no keyring where Vibe uses none, so only the .env counts', async () => {
    const { answer, process } = await signedIn('win32', {}, 'MISTRAL_API_KEY=placeholder\n')

    expect(answer).toBe(true)
    expect(process.ran).toEqual([])
  })
})
