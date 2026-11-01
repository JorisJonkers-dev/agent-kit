import { JsonFields } from './json.js'
import type { CriticClient } from './routing.js'

/**
 * What running a check spends: nothing, Vibe money on the personal
 * account, or one Antigravity invocation of today's quota.
 */
export type SetupCost = 'free' | 'vibe' | 'agy'

/**
 * What a check's result depends on. A result stays good while these are
 * unchanged: the CLI's version, the critic agent file a Vibe critic runs
 * through, and the part of routing the check reads.
 */
export interface SetupInputs {
  /** The first line `<binary> --version` prints; null when it cannot be read. */
  readonly cli_version?: string | null
  /** sha256 of the critic agent file; null when it is not installed. */
  readonly agent_hash?: string | null
  /** sha256 of the part of the merged routing the check reads. */
  readonly routing_hash?: string
}

export const SETUP_INPUTS = ['cli_version', 'agent_hash', 'routing_hash'] as const satisfies readonly (keyof SetupInputs)[]

export type SetupOutcome = 'pass' | 'fail'

/** One check's last run on this machine. */
export interface SetupCheckResult {
  readonly checked_at: string
  readonly result: SetupOutcome
  /** What the check found, or why it failed, in one line. */
  readonly detail: string
  readonly inputs: SetupInputs
}

/** This machine's setup results, one per check id. */
export interface SetupResults {
  readonly version: 1
  readonly checks: Readonly<Record<string, SetupCheckResult>>
}

export const EMPTY_SETUP_RESULTS: SetupResults = { checks: {}, version: 1 }

export type SetupCheckKind = 'vibe-signed-in' | 'agy-signed-in' | 'critic' | 'agy-quota' | 'read-only'

/** One item of the checklist, before anything is checked. */
export interface SetupCheck {
  readonly id: string
  readonly kind: SetupCheckKind
  readonly title: string
  /** One sentence on why the item matters. */
  readonly why: string
  /** What to do when the check fails. */
  readonly fix: string
  readonly cost: SetupCost
  /** The routing client the item is about; absent for a sign-in or the quota. */
  readonly client?: string
  /** The CLI the check reads: the item's client, or for a sign-in or the quota, the first routing client of that CLI. */
  readonly client_config: CriticClient
}

/** Why an item is due: it never ran here, it last failed, or an input it depends on changed. */
export type DueReason = 'never-checked' | 'failed' | 'inputs-changed'

export interface SetupItem {
  readonly id: string
  readonly kind: SetupCheckKind
  readonly title: string
  readonly why: string
  readonly fix: string
  readonly cost: SetupCost
  readonly client?: string
  /**
   * `current`: it last passed and nothing it depends on changed, so it is not
   * re-run. `due`: it should run. `not-available`: no check exists yet.
   */
  readonly status: 'current' | 'due' | 'not-available'
  readonly due_because?: DueReason
  /** The inputs that changed since the last run, for `inputs-changed`. */
  readonly changed?: readonly (keyof SetupInputs)[]
  /** Why a not-available item cannot be checked. */
  readonly detail?: string
  readonly inputs?: SetupInputs
  readonly last?: SetupCheckResult
}

/** The read-only proof per client is #178's live smoke check, which does not exist yet. */
export const READ_ONLY_PENDING = 'not available until agent-kit#178 lands: the read-only proof reuses its live smoke check'

/**
 * The machine checklist, in the order setup walks it: Vibe's sign-in and each
 * Vibe critic, Antigravity's sign-in, its critic and its quota, then the
 * read-only proof per critic client. Clients come from routing, in its order.
 */
export function setupChecklist(clients: Readonly<Record<string, CriticClient>>): readonly SetupCheck[] {
  const entries = Object.entries(clients)
  const vibe = entries.filter(([, config]) => config.kind === 'vibe')
  const agy = entries.filter(([, config]) => config.kind === 'agy')
  const [firstVibe] = vibe
  const [firstAgy] = agy
  return [
    ...(firstVibe === undefined
      ? []
      : [{ ...VIBE_SIGNED_IN, client_config: firstVibe[1] }, ...vibe.map(([client, config]) => criticCheck(client, config))]),
    ...(firstAgy === undefined
      ? []
      : [
          { ...AGY_SIGNED_IN, client_config: firstAgy[1] },
          ...agy.map(([client, config]) => criticCheck(client, config)),
          { ...AGY_QUOTA, client_config: firstAgy[1] },
        ]),
    ...entries.map(([client, config]) => readOnlyCheck(client, config)),
  ]
}

const VIBE_SIGNED_IN: Omit<SetupCheck, 'client_config'> = {
  cost: 'free',
  fix: 'Run vibe and sign in with your personal Mistral account, then check again.',
  id: 'vibe-signed-in',
  kind: 'vibe-signed-in',
  title: 'Vibe is signed in on the personal account',
  why: 'Every Vibe critic runs on the personal Mistral account, so while it is signed out each one drops out of a run.',
}

const AGY_SIGNED_IN: Omit<SetupCheck, 'client_config'> = {
  cost: 'free',
  fix: 'Run agy and sign in with your Google account, then check again.',
  id: 'agy-signed-in',
  kind: 'agy-signed-in',
  title: 'Antigravity is signed in',
  why: 'The Gemini critic runs through agy on your Google account, so while it is signed out its slots fall back to Vibe.',
}

const AGY_QUOTA: Omit<SetupCheck, 'client_config'> = {
  cost: 'free',
  fix: "Give your Antigravity plan's daily request limit and the reserve to keep for the spec and feature reviews.",
  id: 'agy-quota',
  kind: 'agy-quota',
  title: 'The Antigravity daily cap and reserve are set',
  why: 'agy reports no quota, so the runner counts its own launches against a daily cap, which should be your plan’s real limit.',
}

function criticCheck(client: string, config: CriticClient): SetupCheck {
  const through = config.kind === 'vibe' ? `the ${config.agent} agent` : 'agy'
  return {
    client,
    client_config: config,
    cost: config.kind,
    fix:
      config.kind === 'vibe'
        ? `Re-run agent-kit setup to reinstall ${config.agent}, then check again; if it still fails, read the detail.`
        : 'Check that agy runs and offers the model, then check again; if it still fails, read the detail.',
    id: `critic:${client}`,
    kind: 'critic',
    title: `${client} answers a trivial prompt`,
    why: `${config.model} runs through ${through}; a critic that cannot answer a trivial prompt fails every review it is routed to.`,
  }
}

function readOnlyCheck(client: string, config: CriticClient): SetupCheck {
  return {
    client,
    client_config: config,
    cost: config.kind,
    fix: 'Wait for agent-kit#178.',
    id: `read-only:${client}`,
    kind: 'read-only',
    title: `${client} is proven read-only`,
    why: 'A critic may only read its packet; this proves the client refuses to change files on this machine.',
  }
}

/**
 * Whether an item runs: a check that never ran here, last failed, or whose
 * inputs changed is due; one that passed with the same inputs is current.
 */
export function setupItem(check: SetupCheck, inputs: SetupInputs, last: SetupCheckResult | undefined): SetupItem {
  const shown = shownCheck(check)
  if (check.kind === 'read-only') return { ...shown, detail: READ_ONLY_PENDING, status: 'not-available' }
  const base = { ...shown, inputs, ...(last === undefined ? {} : { last }) }
  if (last === undefined) return { ...base, due_because: 'never-checked', status: 'due' }
  const changed = SETUP_INPUTS.filter((key) => last.inputs[key] !== inputs[key])
  if (changed.length > 0) return { ...base, changed, due_because: 'inputs-changed', status: 'due' }
  if (last.result === 'fail') return { ...base, due_because: 'failed', status: 'due' }
  return { ...base, status: 'current' }
}

/** The check as the skill sees it: the client's routing config stays with the runner. */
function shownCheck(check: SetupCheck): Omit<SetupCheck, 'client_config'> {
  return {
    cost: check.cost,
    fix: check.fix,
    id: check.id,
    kind: check.kind,
    title: check.title,
    why: check.why,
    ...(check.client === undefined ? {} : { client: check.client }),
  }
}

export function parseSetupResults(text: string): SetupResults {
  const file = JsonFields.parse(text)
  return {
    checks: file.entries('checks', (entry) => {
      const inputs = entry.object('inputs')
      return {
        checked_at: entry.string('checked_at'),
        detail: entry.string('detail'),
        inputs: {
          ...(inputs.has('cli_version') ? { cli_version: inputs.nullableString('cli_version') } : {}),
          ...(inputs.has('agent_hash') ? { agent_hash: inputs.nullableString('agent_hash') } : {}),
          ...(inputs.has('routing_hash') ? { routing_hash: inputs.string('routing_hash') } : {}),
        },
        result: entry.oneOf('result', ['pass', 'fail'] as const),
      }
    }),
    version: file.literal('version', 1),
  }
}
