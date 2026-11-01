import { createHash } from 'node:crypto'
import { join } from 'node:path'

import { agyInvocationsOn, invocationLines } from '../domain/budget.js'
import { isJsonObject, type JsonFields } from '../domain/json.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import type { ModelFamily } from '../domain/family.js'
import type { CriticClient, RosterCritic, Routing } from '../domain/routing.js'
import {
  EMPTY_SETUP_RESULTS,
  parseSetupResults,
  setupChecklist,
  setupItem,
  type SetupCheck,
  type SetupCheckResult,
  type SetupInputs,
  type SetupItem,
  type SetupOutcome,
  type SetupResults,
} from '../domain/setup.js'
import { readInput, readJsonFile } from './decode.js'
import { ledgerPath, routingOverridePath, setupProbeDir, setupResultsPath } from './locations.js'
import { clientReadiness, clientSignIn, clientVersion, reviewWith, type CritiqueCommand, type CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'
import { RoutingSource } from './routing.js'

/** Where the repository the owner is in stands with the family allowlist. */
export type RepositorySetup =
  | Refusal
  | {
      readonly status: 'not-opted-in' | 'opted-in'
      readonly key: string
      /** The entry's families; empty while the repository is not opted in. */
      readonly families: readonly ModelFamily[]
      /** Whether the entry lets the code stages send code. */
      readonly code: boolean
      /** The families routing's critic clients come from, in routing order: what an opt-in would usually allow. */
      readonly routed_families: readonly ModelFamily[]
    }

export interface SetupListResult {
  readonly status: 'listed'
  readonly results_path: string
  readonly items: readonly SetupItem[]
  /** Present when the input named a repository. */
  readonly repository?: RepositorySetup
}

export interface SetupCheckOutput {
  /**
   * `checked`: the check ran and its result is recorded. `current`: it last
   * passed with the same inputs, so it did not run. `needs-confirmation`: it
   * costs Vibe money or Antigravity quota, and the input did not confirm
   * that. `not-available`: there is no check to run yet.
   */
  readonly status: 'checked' | 'current' | 'needs-confirmation' | 'not-available'
  readonly results_path: string
  readonly item: SetupItem
}

export interface SetupQuotaOutput {
  readonly status: 'quota-set'
  readonly override_path: string
  readonly antigravity_quota: { readonly daily_cap: number; readonly reserve_threshold: number }
  /** The quota check, run again on the new values. */
  readonly item: SetupItem
}

export type SetupResult = SetupListResult | SetupCheckOutput | SetupQuotaOutput

type SetupInput =
  | { readonly action: 'list'; readonly repo: string | undefined }
  | { readonly action: 'check'; readonly check: string; readonly confirmed: boolean; readonly force: boolean }
  | { readonly action: 'quota'; readonly daily_cap: number; readonly reserve_threshold: number }

/** Setup's trivial prompt is a one-turn answer: a few cents at most, and never long. */
const PROBE_PRICE_USD = 0.05
const PROBE_TIME_LIMIT_S = 120

const PROBE_NOTE = "This note is agent-kit's setup check. It has nothing in it to review.\n"

const PROBE_PROMPT = `This is agent-kit's setup check, not a review. Your working directory holds one file, note.md, which has nothing in it to review. Reply with exactly this findings object in a json code block:

\`\`\`json
{"findings": [], "no_findings_reason": "setup check: nothing to review"}
\`\`\`
`

/** The ledger's repository and flow for a setup check, which belongs to no repository and no flow. */
const SETUP_LEDGER_SCOPE = 'machine-setup'

/**
 * The bookkeeping of `/flow setup`'s machine checklist. `list` says, per
 * item, whether it is current or due; `check` runs one item and records its
 * result per machine, with the inputs it depended on; `quota` stores the
 * owner's Antigravity plan limit in the per-machine routing override. A
 * check that costs Vibe money or Antigravity quota runs only when the input
 * confirms it. The one-item-at-a-time walk with the owner is the skill's.
 */
export class SetupCommand implements CritiqueCommand<SetupResult> {
  private readonly routing: RoutingSource
  private readonly repositories: RepositoryGate

  constructor(private readonly deps: CritiqueDeps) {
    this.routing = new RoutingSource(deps)
    this.repositories = new RepositoryGate(deps)
  }

  async run(raw: unknown): Promise<SetupResult> {
    const input = readInput(raw, 'setup', readSetupInput)
    if (input.action === 'list') return this.list(input.repo)
    if (input.action === 'check') return this.check(input.check, input.confirmed, input.force)
    return this.setQuota(input.daily_cap, input.reserve_threshold)
  }

  private async list(repo: string | undefined): Promise<SetupListResult> {
    const routing = await this.routing.load()
    const results = await this.results()
    const items: SetupItem[] = []
    for (const check of setupChecklist(routing.clients)) items.push(setupItem(check, await this.inputs(check, routing), results.checks[check.id]))
    return {
      items,
      results_path: setupResultsPath(this.deps.env),
      status: 'listed',
      ...(repo === undefined ? {} : { repository: await this.repository(repo, routing) }),
    }
  }

  private async check(id: string, confirmed: boolean, force: boolean): Promise<SetupCheckOutput> {
    const routing = await this.routing.load()
    const check = setupChecklist(routing.clients).find((entry) => entry.id === id)
    if (check === undefined) throw new CritiqueError('invalid-input', `setup input: check ${id} is not on this machine's checklist`)
    const inputs = await this.inputs(check, routing)
    const results = await this.results()
    const item = setupItem(check, inputs, results.checks[id])
    const output = (status: SetupCheckOutput['status'], shown: SetupItem): SetupCheckOutput => ({
      item: shown,
      results_path: setupResultsPath(this.deps.env),
      status,
    })
    if (item.status === 'not-available') return output('not-available', item)
    if (item.status === 'current' && !force) return output('current', item)
    const outcome = await this.runCheck(check, routing, confirmed)
    if (outcome === 'needs-confirmation') return output('needs-confirmation', item)
    return output('checked', await this.record(check, inputs, outcome))
  }

  private async setQuota(dailyCap: number, reserve: number): Promise<SetupQuotaOutput> {
    if (reserve > dailyCap) {
      throw new CritiqueError('invalid-input', `setup input: the reserve of ${String(reserve)} is above the daily cap of ${String(dailyCap)}`)
    }
    await this.routing.load()
    const path = routingOverridePath(this.deps.env)
    const override = await this.override()
    const quota = isJsonObject(override.antigravity_quota) ? override.antigravity_quota : {}
    const next = { ...override, antigravity_quota: { ...quota, daily_cap: dailyCap, reserve_threshold: reserve } }
    await this.deps.files.writeText(path, `${JSON.stringify(next, null, 2)}\n`)
    const output = await this.check('agy-quota', false, true)
    return {
      antigravity_quota: { daily_cap: dailyCap, reserve_threshold: reserve },
      item: output.item,
      override_path: path,
      status: 'quota-set',
    }
  }

  /** What the check's result depends on, read without launching a model. */
  private async inputs(check: SetupCheck, routing: Routing): Promise<SetupInputs> {
    if (check.kind === 'read-only') return {}
    if (check.kind === 'agy-quota') {
      const override = await this.override()
      return { routing_hash: sha256({ override: override.antigravity_quota ?? null, quota: routing.limits.antigravity_quota }) }
    }
    const config = check.client_config
    const cli_version = await clientVersion(this.deps.clients, config)
    if (check.kind !== 'critic') return { cli_version }
    return {
      cli_version,
      ...(config.kind === 'vibe' ? { agent_hash: await this.agentHash(config.agent) } : {}),
      routing_hash: sha256(config),
    }
  }

  private async runCheck(
    check: SetupCheck,
    routing: Routing,
    confirmed: boolean,
  ): Promise<{ readonly result: SetupOutcome; readonly detail: string } | 'needs-confirmation'> {
    if (check.kind === 'agy-quota') return this.quotaCheck(routing)
    const config = check.client_config
    if (check.kind !== 'critic') {
      const signIn = await clientSignIn(this.deps.clients, config)
      return signIn.ready ? { detail: `${config.binary} is installed and signed in`, result: 'pass' } : { detail: signIn.detail, result: 'fail' }
    }
    const readiness = await clientReadiness(this.deps.clients, config)
    if (!readiness.ready) return { detail: readiness.detail, result: 'fail' }
    if (!confirmed) return 'needs-confirmation'
    return this.probe(check.client ?? '', config)
  }

  /** Antigravity reports no quota, so the check passes once the owner has stored the plan's limit in the override. */
  private async quotaCheck(routing: Routing): Promise<{ readonly result: SetupOutcome; readonly detail: string }> {
    const { daily_cap, reserve_threshold } = routing.limits.antigravity_quota
    const used = await this.agyToday()
    const override = await this.override()
    const stored = isJsonObject(override.antigravity_quota) && typeof override.antigravity_quota.daily_cap === 'number'
    const figures = `daily cap ${String(daily_cap)}, reserve threshold ${String(reserve_threshold)}, ${String(used)} used today`
    return stored
      ? { detail: `${figures}; set in ${routingOverridePath(this.deps.env)}`, result: 'pass' }
      : { detail: `Antigravity cannot report its quota, so routing's default applies (${figures}); give your plan's daily limit`, result: 'fail' }
  }

  /** Puts the trivial prompt to one critic client, exactly as a critic is launched, and logs the launch to the ledger. */
  private async probe(client: string, config: CriticClient): Promise<{ readonly result: SetupOutcome; readonly detail: string }> {
    const dir = setupProbeDir(this.deps.env, client)
    const packetDir = join(dir, 'packet')
    const criticDir = join(dir, 'critic')
    const promptPath = join(dir, 'prompt.md')
    await this.deps.files.writeText(join(packetDir, 'note.md'), PROBE_NOTE)
    await this.deps.files.writeText(promptPath, PROBE_PROMPT)
    // A previous check's output must never be read as this one's.
    await this.deps.files.remove(join(criticDir, 'export', 'export.json'))
    await this.deps.files.remove(join(criticDir, 'result.json'))
    const critic: RosterCritic = {
      client,
      client_config: config,
      critic_id: `setup-${client}`,
      fallbacks: [],
      lenses: [],
      max_price_usd: PROBE_PRICE_USD,
      role: 'setup-check',
      time_limit_s: PROBE_TIME_LIMIT_S,
    }
    const report = await reviewWith(this.deps.clients, {
      critic,
      criticDir,
      packetDir,
      promptPath,
      schemaPath: join(this.deps.assetsRoot, 'schemas', 'finding.schema.json'),
    })
    await this.logInvocation(client, config, report.outcome.status, report.chargedUsd)
    const charged = config.kind === 'vibe' ? ` (charged $${report.chargedUsd.toFixed(4)})` : ''
    return report.outcome.status === 'ok'
      ? { detail: `${client} answered the trivial prompt${charged}`, result: 'pass' }
      : { detail: `${report.outcome.failure}: ${report.outcome.detail}${charged}`, result: 'fail' }
  }

  /**
   * An invocation line, as fan-out writes one per launch, so a setup check
   * counts against today's Antigravity quota and its Vibe spend stays on
   * record. It belongs to no repository, flow, run or stage.
   */
  private async logInvocation(client: string, config: CriticClient, status: string, charged: number): Promise<void> {
    const line = {
      charged_usd: charged,
      client,
      critic_id: `setup-${client}`,
      family: config.family,
      flow: SETUP_LEDGER_SCOPE,
      kind: config.kind,
      model: config.model,
      recorded_at: this.deps.clock.now().toISOString(),
      repository: SETUP_LEDGER_SCOPE,
      status,
      type: 'invocation',
    }
    await this.deps.files.appendText(ledgerPath(this.deps.env), `${JSON.stringify(line)}\n`)
  }

  private async record(check: SetupCheck, inputs: SetupInputs, outcome: { readonly result: SetupOutcome; readonly detail: string }): Promise<SetupItem> {
    const results = await this.results()
    const entry: SetupCheckResult = { checked_at: this.deps.clock.now().toISOString(), detail: outcome.detail, inputs, result: outcome.result }
    const next: SetupResults = { checks: { ...results.checks, [check.id]: entry }, version: 1 }
    await this.deps.files.writeText(setupResultsPath(this.deps.env), `${JSON.stringify(next, null, 2)}\n`)
    return setupItem(check, inputs, entry)
  }

  private async repository(repo: string, routing: Routing): Promise<RepositorySetup> {
    const resolved = await this.repositories.resolve(repo)
    if (resolved.status === 'refused') return resolved
    const entry = (await this.repositories.allowlist()).repositories[resolved.key]
    const routed_families = [...new Set(Object.values(routing.clients).map((client) => client.family))]
    return entry === undefined
      ? { code: false, families: [], key: resolved.key, routed_families, status: 'not-opted-in' }
      : { code: entry.code, families: entry.families, key: resolved.key, routed_families, status: 'opted-in' }
  }

  private async agentHash(agent: string): Promise<string | null> {
    const path = join(this.deps.env.require('HOME'), '.vibe', 'agents', `${agent}.toml`)
    if (!(await this.deps.files.exists(path))) return null
    return createHash('sha256').update(await this.deps.files.readBytes(path)).digest('hex')
  }

  private async agyToday(): Promise<number> {
    const path = ledgerPath(this.deps.env)
    const lines = (await this.deps.files.exists(path)) ? invocationLines(await this.deps.files.readText(path)) : []
    return agyInvocationsOn(lines, this.deps.clock.now())
  }

  /** This machine's routing override as it is on disk; none is an empty one. RoutingSource has already checked it is an object. */
  private async override(): Promise<Readonly<Record<string, unknown>>> {
    const path = routingOverridePath(this.deps.env)
    if (!(await this.deps.files.exists(path))) return {}
    const value = JSON.parse(await this.deps.files.readText(path)) as unknown
    return isJsonObject(value) ? value : {}
  }

  private async results(): Promise<SetupResults> {
    const path = setupResultsPath(this.deps.env)
    if (!(await this.deps.files.exists(path))) return EMPTY_SETUP_RESULTS
    return readJsonFile(this.deps.files, path, 'invalid-setup', parseSetupResults)
  }
}

function sha256(value: unknown): string {
  return createHash('sha256').update(JSON.stringify(value)).digest('hex')
}

function readSetupInput(fields: JsonFields): SetupInput {
  const action = fields.has('action') ? fields.oneOf('action', ['list', 'check', 'quota'] as const) : 'list'
  if (action === 'list') return { action, repo: fields.optionalString('repo') }
  if (action === 'check') {
    return {
      action,
      check: fields.string('check'),
      confirmed: fields.has('confirmed') ? fields.boolean('confirmed') : false,
      force: fields.has('force') ? fields.boolean('force') : false,
    }
  }
  return { action, daily_cap: fields.positiveInteger('daily_cap'), reserve_threshold: fields.positiveInteger('reserve_threshold') }
}
