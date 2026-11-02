import { readFile } from 'node:fs/promises'

import { afterEach, describe, expect, it } from 'vitest'

import { fenced, finding, harness, isolatedExport, vibeStdout, type Harness } from '../../../test/critique/harness.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

/** The check each of six lenses gained for the ADR and architecture stages, with the words its bullet opens with. */
const ADDITIONS = [
  ['fidelity', 'avoid-term', 'Look for a term the glossary marks _Avoid_'],
  ['testability', 'rule-at-port', 'Check that domain rules are tested at a port'],
  ['integration', 'unversioned-change', 'Look for a public interface (a flag, a format,'],
  ['security', 'trust-boundary', 'For each trust boundary, name the input that'],
  ['failure-modes', 'unbounded-call', 'Look for an outbound call with no timeout or'],
  ['simplicity', 'hypothetical-seam', 'Look for a port with one adapter and no test'],
] as const

/** One spec-stage critic routed with `lens` alone, whose reply names `check`. */
async function routedWith(lens: string, check: string): Promise<{ h: Harness; prompt: string; preVerdict: unknown }> {
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
  return { h, preVerdict: result.findings[0]?.pre_verdict, prompt }
}

describe('the checks added to six existing lenses', () => {
  it.each(ADDITIONS)('the %s lens puts `%s` in the prompt of a critic routed with it', async (lens, check, opening) => {
    const { prompt, preVerdict } = await routedWith(lens, check)

    expect(prompt).toContain(`- \`${check}\`: ${opening}`)
    // The critic's prompt declares the check, so a finding naming it is not an unknown-check hint.
    expect(preVerdict).toBeUndefined()
  })
})
