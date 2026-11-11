import { afterEach, describe, expect, it } from 'vitest'

import { harness, seedGatewayLedger, SyntheticLedger, twoCriticRouting, type Harness } from '../../../test/critique/harness.js'
import type { BudgetResult } from './index.js'

/**
 * What `/flow budget` reads through the `budget` command: what the next
 * costly stage would draw on, from the ledger's invocation lines and the
 * routing (with this machine's override), by the rules the stages enforce.
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SPEC = '.scratch/widgets/spec.md'
const DRAFT = 'draft:widgets'

async function repository(options: Parameters<typeof harness>[0] = {}): Promise<Harness> {
  const h = await harness(options)
  open.push(h)
  return h
}

async function budgetOf(h: Harness, input: Record<string, unknown> = {}): Promise<BudgetResult> {
  const result = await h.commands.budget.run({ repo: h.repo, spec: SPEC, ...input })
  if (result.status === 'refused') throw new Error(`budget refused: ${result.message}`)
  return result
}

/** The ledger line state writes when a spec's flow is linked to the draft it grew from. */
function draftLink(): Record<string, unknown> {
  return {
    draft: 'widgets',
    event: 'draft',
    flow: SPEC,
    recorded_at: '2026-10-30T10:00:00.000Z',
    repository: 'github.com/jorisjonkers-dev/agent-kit',
    type: 'flow',
  }
}

describe("the flow's Vibe spend", () => {
  it('adds up what the Vibe critics of the flow and its draft were charged', async () => {
    const h = await repository()
    await new SyntheticLedger()
      .raw(draftLink())
      .invocation({ charged_usd: 1.25, flow: SPEC, kind: 'vibe' })
      .invocation({ charged_usd: 0.5, flow: DRAFT, kind: 'vibe', stage: 'decisions' })
      .invocation({ charged_usd: 3, flow: 'github.com/jorisjonkers-dev/agent-kit#9', kind: 'vibe' })
      .invocation({ charged_usd: 2, flow: SPEC, kind: 'vibe', repository: 'github.com/jorisjonkers-dev/other' })
      .invocation({ flow: SPEC, kind: 'agy' })
      .writeTo(h)

    const result = await budgetOf(h)

    expect(result.vibe).toEqual({
      flows: [
        { flow: SPEC, spent_usd: 1.25 },
        { flow: DRAFT, spent_usd: 0.5 },
      ],
      plan_limits: [],
      spent_usd: 1.75,
      warnings: [],
    })
  })
})

/** This machine's gateway file: Vibe's plan allows $5 a day. */
const VIBE_PLAN = { clients: { vibe: { plan_limits: [{ limit_usd: 5, window: 'day' }] } } }
const PLAN_SPENT = 'at 100% of its day plan limit ($5.25 of $5.00)'

describe('the stages whose Vibe critics would be dropped now', () => {
  it("drops them on every stage once Vibe is at 100% of its plan, since the plan is the machine's", async () => {
    const h = await repository({ gatewayOverride: VIBE_PLAN })
    await new SyntheticLedger().raw(draftLink()).writeTo(h)
    await seedGatewayLedger(h, [{ cost_usd: 5.25 }])

    const result = await budgetOf(h)

    expect(result.stages.map((stage) => [stage.stage, stage.flow, stage.vibe])).toEqual([
      ['decisions', DRAFT, PLAN_SPENT],
      ['spec', SPEC, PLAN_SPENT],
      ['tickets', SPEC, PLAN_SPENT],
      ['ticket-review', SPEC, PLAN_SPENT],
      ['feature-review', SPEC, PLAN_SPENT],
      ['adr-review', null, PLAN_SPENT],
      ['decision-recovery', null, PLAN_SPENT],
      ['architecture-review', null, PLAN_SPENT],
      ['plan', DRAFT, PLAN_SPENT],
    ])
  })

  it('leaves them alone while Vibe is under its plan, whatever the flow has spent', async () => {
    const h = await repository({ gatewayOverride: VIBE_PLAN })
    await new SyntheticLedger().invocation({ charged_usd: 5.25, flow: SPEC, kind: 'vibe' }).writeTo(h)

    const result = await budgetOf(h)

    expect(result.stages.map((stage) => stage.vibe)).toEqual([null, null, null, null, null, null, null, null, null])
  })

  it("charges an adr-review run to its own run id, never to the feature's spec or draft", async () => {
    const h = await repository()
    await new SyntheticLedger()
      .raw(draftLink())
      .invocation({ charged_usd: 5.25, flow: SPEC, kind: 'vibe' })
      .invocation({ charged_usd: 5.25, flow: DRAFT, kind: 'vibe', stage: 'decisions' })
      .invocation({ charged_usd: 4, flow: '20261030T110000Z-adr-review', kind: 'vibe', stage: 'adr-review' })
      .writeTo(h)

    const result = await budgetOf(h)

    expect(result.stages.find((stage) => stage.stage === 'adr-review')).toMatchObject({ flow: null, vibe: null })
    expect(result.vibe.flows).toEqual([
      { flow: SPEC, spent_usd: 5.25 },
      { flow: DRAFT, spent_usd: 5.25 },
    ])
  })

  it('drops them on the decisions stage of a draft, before there is a spec', async () => {
    const h = await repository({ gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 5.25 }])

    const result = await budgetOf(h, { draft: 'widgets', spec: undefined })

    expect(result.stages.slice(0, 2).map((stage) => [stage.stage, stage.flow, stage.vibe])).toEqual([
      ['decisions', DRAFT, PLAN_SPENT],
      ['spec', null, PLAN_SPENT],
    ])
  })
})

describe('the figures the stages enforce', () => {
  it('shows the Vibe spend the flow state holds', async () => {
    const h = await repository()
    await new SyntheticLedger()
      .raw(draftLink())
      .invocation({ charged_usd: 1.25, flow: SPEC, kind: 'vibe' })
      .invocation({ charged_usd: 0.5, flow: DRAFT, kind: 'vibe', stage: 'decisions' })
      .writeTo(h)
    const state = await h.commands.state.run({ repo: h.repo, spec: SPEC })
    if (state.status === 'refused') throw new Error(state.message)

    const result = await budgetOf(h)

    expect([result.vibe.spent_usd, state.state.vibe_spend_usd]).toEqual([1.75, 1.75])
  })

  it('names the Antigravity drop prepare makes for a stage past the reserve', async () => {
    const h = await repository({ routingOverride: { antigravity_quota: { reserved_stages: ['feature-review'] } } })
    await agyLaunches(h, 10)
    await h.commands.allow.run({ families: ['mistral', 'google', 'zhipu'], repo: h.repo })
    const outlook = (await budgetOf(h)).stages.find((stage) => stage.stage === 'spec')

    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec' })

    expect(run).toMatchObject({ dropped: [{ client: 'agy-gemini-flash-high', detail: outlook?.antigravity, reason: 'quota', replaced_by: 'c2-vibe-glm' }] })
    expect(outlook?.falls_back).toEqual([{ client: 'agy-gemini-flash-high', to: 'vibe-glm' }])
  })

  it('names the quota drop prepare makes for a stage past Vibe\'s plan', async () => {
    const h = await repository({ gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 5.25 }])
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const outlook = (await budgetOf(h)).stages.find((stage) => stage.stage === 'spec')
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'spec' })

    expect(run.status === 'ready' ? run.dropped[0] : undefined).toMatchObject({ client: 'vibe-mistral-large', detail: outlook?.vibe, reason: 'quota' })
  })
})

describe('a budget the runner refuses', () => {
  it('needs the feature named by its spec or a draft name', async () => {
    const h = await repository()

    await expect(h.commands.budget.run({ repo: h.repo })).rejects.toMatchObject({ code: 'invalid-input' })
  })

  it('refuses a repository owned by nedap, as every command does', async () => {
    const h = await repository({ remotes: [{ name: 'origin', url: 'git@github.com:nedap/widgets.git' }] })

    expect(await h.commands.budget.run({ repo: h.repo, spec: SPEC })).toMatchObject({ reason: 'nedap', status: 'refused' })
  })

  it("refuses when this machine's gateway file cannot be read, naming the gateway's problem", async () => {
    const h = await repository({ gatewayOverride: '{ not json' })

    expect(await h.commands.budget.run({ repo: h.repo, spec: SPEC })).toMatchObject({
      reason: 'invalid-gateway-config',
      status: 'refused',
    })
  })

  it('reports a ledger line it cannot read', async () => {
    const h = await repository()
    await new SyntheticLedger().raw({ type: 'flow' }).writeTo(h)

    await expect(budgetOf(h)).rejects.toMatchObject({ code: 'invalid-ledger' })
  })
})

describe("the caps this machine's routing sets", () => {
  it("reads Vibe's plan limit from the machine's gateway file, and warns at the warning ratio", async () => {
    const h = await repository({ gatewayOverride: VIBE_PLAN })
    await seedGatewayLedger(h, [{ cost_usd: 4.2 }])

    const result = await budgetOf(h)

    expect(result.vibe.plan_limits).toEqual([{ limit_usd: 5, ratio: 0.84, spent_usd: 4.2, state: 'warning', window: 'day' }])
    expect(result.vibe.warnings).toEqual(['vibe has used 84% of its day plan limit ($4.20 of $5.00)'])
  })

  it('holds Antigravity to the daily cap and reserve the per-machine override names', async () => {
    const h = await repository({ routingOverride: { antigravity_quota: { daily_cap: 8, reserve_threshold: 4, reserved_stages: ['spec'] } } })

    const result = await budgetOf(h)

    expect(result.antigravity).toMatchObject({ daily_cap: 8, reserve_threshold: 4, reserved_stages: ['spec'] })
  })
})

describe("today's Antigravity invocations", () => {
  it("counts this machine's agy launches of today, in every repository and flow", async () => {
    const h = await repository()
    await new SyntheticLedger()
      .invocation({ flow: SPEC, kind: 'agy' })
      .invocation({ flow: 'github.com/jorisjonkers-dev/agent-kit#9', kind: 'agy' })
      .invocation({ flow: SPEC, kind: 'agy', repository: 'github.com/jorisjonkers-dev/other' })
      .invocation({ flow: SPEC, kind: 'agy', recorded_at: '2026-10-29T23:59:59.000Z' })
      .invocation({ charged_usd: 0.5, flow: SPEC, kind: 'vibe' })
      .writeTo(h)

    const result = await budgetOf(h)

    expect(result.antigravity).toEqual({
      daily_cap: 20,
      day: '2026-10-30',
      invocations: 3,
      reserve_threshold: 10,
      reserved_stages: ['spec', 'feature-review'],
    })
  })
})

/** A ledger holding `count` agy launches of today. */
async function agyLaunches(h: Harness, count: number): Promise<void> {
  const ledger = new SyntheticLedger()
  for (let n = 0; n < count; n += 1) ledger.invocation({ flow: SPEC, kind: 'agy' })
  await ledger.writeTo(h)
}

const RESERVE = 'antigravity: 10 invocations today have reached the reserve threshold of 10; the rest is kept for spec and feature-review'

describe('the stages that would fall back now', () => {
  it('lets every stage launch its Antigravity critic below the reserve', async () => {
    const h = await repository()
    await agyLaunches(h, 9)

    const result = await budgetOf(h)

    expect(result.stages.map((stage) => [stage.stage, stage.antigravity, stage.falls_back])).toEqual([
      ['decisions', null, []],
      ['spec', null, []],
      ['tickets', null, []],
      ['ticket-review', null, []],
      ['feature-review', null, []],
      ['adr-review', null, []],
      ['decision-recovery', null, []],
      ['architecture-review', null, []],
      ['plan', null, []],
    ])
  })

  it('falls back from Antigravity on the stages the reserve does not keep it for', async () => {
    const h = await repository()
    await agyLaunches(h, 10)

    const result = await budgetOf(h)

    expect(result.stages.map((stage) => [stage.stage, stage.antigravity, stage.falls_back])).toEqual([
      ['decisions', RESERVE, [{ client: 'agy-gemini-flash-high', to: 'vibe-glm' }]],
      ['spec', null, []],
      ['tickets', null, []],
      ['ticket-review', RESERVE, [{ client: 'agy-gemini-flash-high', to: 'vibe-glm' }]],
      ['feature-review', null, []],
      ['adr-review', RESERVE, [{ client: 'agy-gemini-flash-high', to: 'vibe-glm' }]],
      ['decision-recovery', null, []],
      ['architecture-review', RESERVE, [{ client: 'agy-gemini-flash-high', to: 'vibe-glm' }]],
      ['plan', null, []],
    ])
  })

  it('falls back from Antigravity on every stage that routes it once the daily cap is reached', async () => {
    const h = await repository()
    await agyLaunches(h, 20)
    const cap = 'antigravity: 20 invocations today have reached the daily cap of 20'

    const result = await budgetOf(h)

    expect(result.stages.map((stage) => [stage.stage, stage.antigravity])).toEqual([
      ['decisions', cap],
      ['spec', cap],
      ['tickets', null],
      ['ticket-review', cap],
      ['feature-review', cap],
      ['adr-review', cap],
      ['decision-recovery', null],
      ['architecture-review', cap],
      ['plan', null],
    ])
  })

  it('lists only the stages routing defines', async () => {
    const h = await repository({ routing: twoCriticRouting() })

    const result = await budgetOf(h)

    expect(result.stages.map((stage) => stage.stage)).toEqual(['spec'])
  })

  it('says a held slot stays empty when routing names no other client for it', async () => {
    const h = await repository({ routingOverride: { fallbacks: { 'agy-gemini-flash-high': null } } })
    await agyLaunches(h, 20)

    const result = await budgetOf(h)

    expect(result.stages.find((stage) => stage.stage === 'spec')?.falls_back).toEqual([{ client: 'agy-gemini-flash-high', to: null }])
  })

  it('names a held slot of the deep roster as deep', async () => {
    const slot = { client: 'agy-gemini-flash-high', lenses: ['failure-modes'], max_price_usd: 0, role: 'ticket-linter', time_limit_s: 600 }
    const h = await repository({ routingOverride: { stages: { tickets: { deep: [slot] } } } })
    await agyLaunches(h, 20)

    const result = await budgetOf(h)

    expect(result.stages.find((stage) => stage.stage === 'tickets')?.falls_back).toEqual([
      { client: 'agy-gemini-flash-high', deep: true, to: 'vibe-glm' },
    ])
  })
})
