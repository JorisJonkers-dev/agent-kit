import { readdirSync, readFileSync } from 'node:fs'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

import { describe, expect, it } from 'vitest'

import { CRITIQUE_STAGES, isCodeStage, isReviewStage } from './domain/stage.js'
import { STAGE_TABLE, type StageTable } from './domain/stage-table.js'

const CONTEXT_DIR = dirname(fileURLToPath(import.meta.url))

describe('the critique stage table', () => {
  it('gives each stage its per-stage behaviour', () => {
    expect(STAGE_TABLE).toEqual({
      'adr-review': { doNotStart: null, evidence: 'adrs', flowKey: 'run', packet: 'adrs', removal: 'runs', secondRound: true },
      decisions: { doNotStart: '/to-tickets', evidence: 'decisions', flowKey: 'draft', packet: 'decisions', removal: 'features', secondRound: false },
      'feature-review': { doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
      spec: { doNotStart: '/to-tickets', evidence: 'spec', flowKey: 'spec', packet: 'document', removal: 'features', secondRound: true },
      'ticket-review': { doNotStart: '/to-tickets', evidence: 'code', flowKey: 'spec', packet: 'code', removal: 'features', secondRound: true },
      tickets: { doNotStart: '/implement', evidence: 'tickets', flowKey: 'spec', packet: 'tickets', removal: 'features', secondRound: true },
    })
  })

  it('builds a code packet and applies the code evidence rule exactly at the code stages', () => {
    for (const stage of CRITIQUE_STAGES) {
      expect({ evidence: STAGE_TABLE[stage].evidence === 'code', packet: STAGE_TABLE[stage].packet === 'code', stage }).toEqual({
        evidence: isCodeStage(stage),
        packet: isCodeStage(stage),
        stage,
      })
    }
  })

  it('keys a run by its run id, judges it by its runs and names no command not to start exactly at the review stages', () => {
    for (const stage of CRITIQUE_STAGES) {
      const { doNotStart, flowKey, removal } = STAGE_TABLE[stage]
      expect({ flowKey: flowKey === 'run', noCommand: doNotStart === null, removal: removal === 'runs', stage }).toEqual({
        flowKey: isReviewStage(stage),
        noCommand: isReviewStage(stage),
        removal: isReviewStage(stage),
        stage,
      })
    }
  })

  it('refuses at compile time a table that leaves a stage out', () => {
    // @ts-expect-error -- tickets has no entry, so this is not a stage table
    const partial: StageTable = {
      'adr-review': STAGE_TABLE['adr-review'],
      decisions: STAGE_TABLE.decisions,
      'feature-review': STAGE_TABLE['feature-review'],
      spec: STAGE_TABLE.spec,
      'ticket-review': STAGE_TABLE['ticket-review'],
    }
    expect(Object.keys(partial)).not.toContain('tickets')
  })

  it('leaves no module outside the table branching on a literal stage name for the table behaviours', () => {
    const branches = sourceFiles(CONTEXT_DIR)
      .filter((path) => relative(CONTEXT_DIR, path) !== join('domain', 'stage-table.ts'))
      .flatMap((path) =>
        [...readFileSync(path, 'utf8').matchAll(STAGE_COMPARISON)].map((match) => `${relative(CONTEXT_DIR, path)}: ${match[0]}`),
      )
      .sort()
    // Each branch left is a stage-specific behaviour the table does not hold:
    // which tracker item a code packet freezes, which stage takes a staged
    // diff, the tickets stage's prompt wording, which spec a ticket review
    // belongs to, and the verdict fields and gates of particular stages.
    expect(branches).toEqual([
      "application/code-packet.ts: stage === 'ticket-review'",
      "application/code-packet.ts: stage === 'ticket-review'",
      "application/record.ts: stage === 'feature-review'",
      "application/record.ts: stage === 'ticket-review'",
      "application/staged-change.ts: stage !== 'ticket-review'",
      "domain/manifest.ts: stage === 'ticket-review'",
      "domain/prompt.ts: stage === 'tickets'",
      "domain/verdicts.ts: stage !== 'feature-review'",
      "domain/verdicts.ts: stage !== 'ticket-review'",
      "domain/verdicts.ts: stage !== 'ticket-review'",
      "domain/verdicts.ts: stage === 'decisions'",
      "domain/verdicts.ts: stage === 'ticket-review'",
    ])
  })
})

/** A comparison of something named `stage` (or `run.stage`, `manifest.stage`) with a stage name, either way round. */
const STAGE_COMPARISON = new RegExp(
  String.raw`\bstage\s*[!=]==?\s*'(?:${CRITIQUE_STAGES.join('|')})'|'(?:${CRITIQUE_STAGES.join('|')})'\s*[!=]==?\s*[\w.]*\bstage\b`,
  'g',
)

function sourceFiles(dir: string): readonly string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return sourceFiles(path)
    return entry.name.endsWith('.ts') && !entry.name.endsWith('.test.ts') ? [path] : []
  })
}
