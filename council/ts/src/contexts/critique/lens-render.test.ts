import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { ASSETS_ROOT, fenced, finding, harness, isolatedExport, vibeStdout, type Harness } from '../../../test/critique/harness.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const LENSES = ['dependency-direction', 'domain-language', 'aggregates', 'coupling', 'test-seams', 'quality-scenarios'] as const

/** Every check id a lens file declares, from the bullets that open with an id in backticks. */
function checkIds(text: string): string[] {
  return [...text.matchAll(/^- `([a-z0-9]+(?:-[a-z0-9]+)*)`:/gm)].map((m) => m[1] ?? '')
}

/** One spec-stage critic routed with `lens` alone, whose reply names `check`. */
async function routedWith(lens: string, check: string): Promise<{ findings: readonly { pre_verdict?: unknown }[]; prompt: string }> {
  const h = await harness({
    routing: {
      clients: {
        'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' },
      },
      stages: {
        spec: {
          base: [{ client: 'vibe-mistral-large', lenses: [lens], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 }],
          distinct_families: true,
        },
      },
      version: 1,
    },
    scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [finding({ check })] })) }],
  })
  open.push(h)
  await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
  const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
  if (run.status !== 'ready') throw new Error('not ready')
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  const args = h.process.commands[0]?.args ?? []
  const prompt = await readFile(args[args.indexOf('--prompt-file') + 1] ?? '', 'utf8')
  return { findings: result.findings, prompt }
}

describe('the architecture lenses render into a critic prompt', () => {
  it.each(LENSES)('the %s lens puts every check it declares in the prompt', async (lens) => {
    const ids = checkIds(await readFile(join(ASSETS_ROOT, 'lenses', `${lens}.md`), 'utf8'))
    expect(ids.length).toBeGreaterThan(0)

    const { findings, prompt } = await routedWith(lens, ids[0] ?? '')

    for (const id of ids) expect(prompt).toContain(`- \`${id}\`:`)
    // The prompt declares the check, so a finding naming it is not an unknown-check hint.
    expect(findings).toHaveLength(1)
    expect(findings[0]?.pre_verdict).toBeUndefined()
  })
})
