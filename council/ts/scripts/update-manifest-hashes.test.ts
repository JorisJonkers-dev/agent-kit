import { mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

import { pinnedPathEntries, updateManifestHashes } from './update-manifest-hashes.js'

describe('update manifest hashes', () => {
  it('finds every mapping with string path and sha256', () => {
    const manifest = [
      'version: 2',
      'commands:',
      '  - name: alpha',
      '    targets:',
      '      - agent: codex',
      '        path: fixtures/a.txt',
      '        sha256: 0000000000000000000000000000000000000000000000000000000000000000',
      'nested:',
      '  child:',
      '    path: "fixtures/b.txt"',
      "    sha256: '1111111111111111111111111111111111111111111111111111111111111111'",
      'not_pinned:',
      '  path: fixtures/c.txt',
    ].join('\n')

    expect(pinnedPathEntries(manifest).map((entry) => entry.path)).toEqual([
      'fixtures/a.txt',
      'fixtures/b.txt',
    ])
  })

  it('rewrites only sha256 scalar values in place', async () => {
    const root = await mkdtemp(join(tmpdir(), 'manifest-hashes-'))
    await mkdir(join(root, 'fixtures'))
    await writeFile(join(root, 'fixtures/a.txt'), 'alpha\n')
    await writeFile(join(root, 'fixtures/b.txt'), 'beta\n')

    const manifestPath = join(root, 'manifest.yaml')
    await writeFile(
      manifestPath,
      [
        'version: 2',
        'commands:',
        '  - name: alpha',
        '    targets:',
        '      - agent: codex',
        '        path: fixtures/a.txt',
        '        sha256: 0000000000000000000000000000000000000000000000000000000000000000 # keep',
        'groups:',
        '  beta:',
        '    path: "fixtures/b.txt"',
        "    sha256: '1111111111111111111111111111111111111111111111111111111111111111'",
        '',
      ].join('\n'),
    )

    expect(updateManifestHashes(manifestPath, root)).toBe(2)
    const updated = await readFile(manifestPath, 'utf8')

    expect(updated).toContain(
      '        sha256: b6a98d9ce9a2d9149288fa3df42d377c3e42737afdcdaf714e33c0a100b51060 # keep',
    )
    expect(updated).toContain(
      "    sha256: 'f2c82decdd7181cf98945929a62598db7e6b477e11f6e0eb0ae97020eff151ad'",
    )
    expect(updateManifestHashes(manifestPath, root)).toBe(0)
  })
})
