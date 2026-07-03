import { chmod } from 'node:fs/promises'
import { resolve } from 'node:path'

import { build } from 'esbuild'

const outFile = resolve(import.meta.dirname, '../../council.mjs')

await build({
  entryPoints: [resolve(import.meta.dirname, '../src/cli/index.ts')],
  outfile: outFile,
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  banner: {
    js: '#!/usr/bin/env node',
  },
  legalComments: 'none',
})

await chmod(outFile, 0o755)
