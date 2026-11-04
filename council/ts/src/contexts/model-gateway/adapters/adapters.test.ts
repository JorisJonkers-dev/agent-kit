import { chmod, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { NodeModelGatewayFiles } from './fs/index.js'
import { NodeModelGatewayProcess } from './process/index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

async function tempDir(): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'model-gateway-adapter-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  return dir
}

describe('NodeModelGatewayFiles', () => {
  it('reads a file as text', async () => {
    const dir = await tempDir()
    const path = join(dir, 'defaults.json')
    await writeFile(path, '{"version":1}\n')

    expect(await new NodeModelGatewayFiles().readText(path)).toBe('{"version":1}\n')
  })

  it('says whether a file exists', async () => {
    const dir = await tempDir()
    const path = join(dir, 'model-gateway.json')
    const files = new NodeModelGatewayFiles()

    expect(await files.exists(path)).toBe(false)
    await writeFile(path, '{}')
    expect(await files.exists(path)).toBe(true)
  })

  it('throws, rather than saying no, when it may not look', async () => {
    const dir = await tempDir()
    const locked = join(dir, 'locked')
    await mkdir(locked)
    await writeFile(join(locked, 'model-gateway.json'), '{}')
    await chmod(locked, 0o000)
    cleanups.unshift(() => chmod(locked, 0o755))

    await expect(new NodeModelGatewayFiles().exists(join(locked, 'model-gateway.json'))).rejects.toThrow()
  })
})

describe('NodeModelGatewayProcess', () => {
  it('finds a program that is on PATH', async () => {
    expect(await new NodeModelGatewayProcess().which('node')).toBe(true)
  })

  it('does not find a program that is not on PATH', async () => {
    expect(await new NodeModelGatewayProcess().which('agent-kit-no-such-binary')).toBe(false)
  })

  it('looks the program up on the PATH it is given, without running any other program', async () => {
    const dir = await tempDir()
    const bin = join(dir, 'bin')
    await mkdir(bin)
    await writeFile(join(bin, 'runnable'), '#!/bin/sh\n')
    await chmod(join(bin, 'runnable'), 0o755)
    await writeFile(join(bin, 'plain'), 'not a program\n')
    const lookup = new NodeModelGatewayProcess(`${join(dir, 'missing')}:${bin}`)

    expect(await lookup.which('runnable')).toBe(true)
    expect(await lookup.which('plain')).toBe(false)
    expect(await new NodeModelGatewayProcess('').which('runnable')).toBe(false)
  })
})
