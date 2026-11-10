import { JsonFields } from './json.js'
import { isWorkClient, type CriticClient } from './routing.js'

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
  /** sha256 of the local files a hook check reads: the agents' hook files, Codex's trust, the critique configuration. */
  readonly files_hash?: string
}

export const SETUP_INPUTS = [
  'cli_version',
  'agent_hash',
  'routing_hash',
  'files_hash',
] as const satisfies readonly (keyof SetupInputs)[]

/**
 * `unproven` is a read-only proof that cannot be shown without a live prompt
 * this machine does not spend on a proof: it is recorded with its reason, and
 * it never counts as a pass.
 */
export type SetupOutcome = 'pass' | 'fail' | 'unproven'

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

/** The checks about a critic client's CLI, and the two about the critique hooks on this machine. */
export type ClientCheckKind = 'vibe-signed-in' | 'agy-signed-in' | 'critic' | 'agy-quota' | 'read-only'
export type MachineCheckKind = 'hooks' | 'kill-switch'
export type SetupCheckKind = ClientCheckKind | MachineCheckKind

interface SetupCheckBase {
  readonly id: string
  readonly title: string
  /** One sentence on why the item matters. */
  readonly why: string
  /** What to do when the check fails. */
  readonly fix: string
  readonly cost: SetupCost
}

/** One item of the checklist about a critic client's CLI, before anything is checked. */
export interface ClientCheck extends SetupCheckBase {
  readonly kind: ClientCheckKind
  /** The routing client the item is about; absent for a sign-in or the quota. */
  readonly client?: string
  /** The CLI the check reads: the item's client, or for a sign-in or the quota, the first routing client of that CLI. */
  readonly client_config: CriticClient
}

/** One item of the checklist about the critique hooks on this machine; it reads local files only. */
export interface MachineCheck extends SetupCheckBase {
  readonly kind: MachineCheckKind
  readonly client?: undefined
}

/**
 * The read-only proof of a client that has no critic configuration (Claude
 * Code, Codex, Hermes, OpenCode). No critic runs there, so the proof is
 * recorded as unproven with its reason, and nothing is launched.
 */
export interface UnprovenReadOnlyCheck extends SetupCheckBase {
  readonly kind: 'read-only'
  readonly client: string
  readonly client_config?: undefined
  /** Why the client's read-only mode is not proven on this machine, in one sentence. */
  readonly reason: string
}

/** One item of the checklist, before anything is checked. */
export type SetupCheck = ClientCheck | MachineCheck | UnprovenReadOnlyCheck

/** The clients without a critic configuration, in the order setup lists their proofs. */
export const UNPROVEN_READ_ONLY_CLIENTS: readonly { readonly client: string; readonly reason: string }[] = [
  {
    client: 'claude',
    reason: "Claude Code's read-only mode is a permission mode, and showing that it refuses writes needs a live prompt, which setup does not spend on a proof",
  },
  {
    client: 'codex',
    reason: "Codex's read-only mode is a sandbox setting, and showing that it refuses writes needs a live prompt, which setup does not spend on a proof",
  },
  {
    client: 'hermes',
    reason: "Hermes has no read-only flag, so only its toolsets limit a run, and showing that they refuse writes needs a live prompt, which setup does not spend on a proof",
  },
  {
    client: 'opencode',
    reason: "OpenCode's read-only use is an agent whose edit permissions are denied in its own configuration, and showing that it refuses writes needs a live prompt, which setup does not spend on a proof",
  },
]

export function isMachineCheck(check: SetupCheck): check is MachineCheck {
  return check.kind === 'hooks' || check.kind === 'kill-switch'
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

/**
 * agy's read-only proof is #178's live smoke check, which does not exist
 * yet. A Vibe client's is proven now: its search reach, which needs no
 * model, and one live prompt that shows `run_typescript` refusing all but
 * the search tools (ADR-0006, amended).
 */
export const READ_ONLY_PENDING =
  'not available until agent-kit#178 lands: the read-only proof reuses its live smoke check'

/** What a passed Vibe read-only proof leaves to #178. */
export const WRITE_PROOF_PENDING = "agent-kit#178's live smoke check adds the rest of the read-only proof"

/**
 * The machine checklist, in the order setup walks it: Vibe's sign-in and each
 * Vibe critic, Antigravity's sign-in, its critic and its quota, the
 * read-only proof per critic client, then the hooks (installed, and the kill
 * switch). Clients come from routing, in its order.
 */
export function setupChecklist(
  clients: Readonly<Record<string, CriticClient>>,
): readonly SetupCheck[] {
  // The work clients have no setup check yet: only a nedap repository's roster names them.
  const entries = Object.entries(clients).filter((entry): entry is [string, SetupClient] => !isWorkClient(entry[1]))
  const vibe = entries.filter(([, config]) => config.kind === 'vibe')
  const agy = entries.filter(([, config]) => config.kind === 'agy')
  const [firstVibe] = vibe
  const [firstAgy] = agy
  return [
    ...(firstVibe === undefined
      ? []
      : [
          { ...VIBE_SIGNED_IN, client_config: firstVibe[1] },
          ...vibe.map(([client, config]) => criticCheck(client, config)),
        ]),
    ...(firstAgy === undefined
      ? []
      : [
          { ...AGY_SIGNED_IN, client_config: firstAgy[1] },
          ...agy.map(([client, config]) => criticCheck(client, config)),
          { ...AGY_QUOTA, client_config: firstAgy[1] },
        ]),
    ...entries.map(([client, config]) => readOnlyCheck(client, config)),
    ...UNPROVEN_READ_ONLY_CLIENTS.map(unprovenReadOnlyCheck),
    HOOKS_INSTALLED,
    KILL_SWITCH,
  ]
}

const HOOKS_INSTALLED: MachineCheck = {
  cost: 'free',
  fix: 'Re-run agent-kit setup to install the hooks; in Codex, open /hooks and trust critique-commit-gate, critique-pr-gate and critique-session-start; then check again.',
  id: 'hooks-installed',
  kind: 'hooks',
  title: 'The critique hooks are installed in Claude Code and Codex',
  why: 'Without the commit and PR gate hooks every gate is advisory, and Codex skips a hook the owner has not trusted in /hooks.',
}

const KILL_SWITCH: MachineCheck = {
  cost: 'free',
  fix: 'Turn the kill switch off or on here, or fix the critique configuration file the detail names.',
  id: 'kill-switch',
  kind: 'kill-switch',
  title: 'The kill switch turns the hooks off',
  why: 'CRITIQUE_OFF=1 or the disabled flag in the per-machine critique configuration must make every hook a no-op at once.',
}

const VIBE_SIGNED_IN: Omit<ClientCheck, 'client_config'> = {
  cost: 'free',
  fix: 'Run vibe and sign in with your personal Mistral account, then check again.',
  id: 'vibe-signed-in',
  kind: 'vibe-signed-in',
  title: 'Vibe is signed in on the personal account',
  why: 'Every Vibe critic runs on the personal Mistral account, so while it is signed out each one drops out of a run.',
}

const AGY_SIGNED_IN: Omit<ClientCheck, 'client_config'> = {
  cost: 'free',
  fix: 'Run agy and sign in with your Google account, then check again.',
  id: 'agy-signed-in',
  kind: 'agy-signed-in',
  title: 'Antigravity is signed in',
  why: 'The Gemini critic runs through agy on your Google account, so while it is signed out its slots fall back to Vibe.',
}

const AGY_QUOTA: Omit<ClientCheck, 'client_config'> = {
  cost: 'free',
  fix: "Give your Antigravity plan's daily request limit and the reserve to keep for the spec and feature reviews.",
  id: 'agy-quota',
  kind: 'agy-quota',
  title: 'The Antigravity daily cap and reserve are set',
  why: 'agy reports no quota, so the runner counts its own launches against a daily cap, which should be your plan’s real limit.',
}

/** A critic client setup checks: the work clients are not among them. */
type SetupClient = Exclude<CriticClient, { readonly kind: 'codex' | 'claude' }>

function criticCheck(client: string, config: SetupClient): ClientCheck {
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

function unprovenReadOnlyCheck({ client, reason }: { readonly client: string; readonly reason: string }): UnprovenReadOnlyCheck {
  return {
    client,
    cost: 'free',
    fix: 'Nothing to fix: this client has no critic, and its read-only delegations run in a throwaway worktree until a live proof exists.',
    id: `read-only:${client}`,
    kind: 'read-only',
    reason,
    title: `${client} has no read-only proof yet`,
    why: `${reason}.`,
  }
}

function readOnlyCheck(client: string, config: SetupClient): ClientCheck {
  const vibe = config.kind === 'vibe'
  return {
    client,
    client_config: config,
    // A Vibe client's proof is one trivial prompt, after a search-reach check that calls no model.
    cost: config.kind,
    fix: vibe
      ? 'Re-run agent-kit setup so the critique runner is current, then check again; until it passes, this client reads its packet without searching.'
      : 'Wait for agent-kit#178.',
    id: `read-only:${client}`,
    kind: 'read-only',
    title: `${client} is proven read-only`,
    why: vibe
      ? 'A critic may only read and search its packet; this proves the search server cannot reach past the packet, and that run_typescript, the only way into the search tools, refuses shell commands, file writes and process starts.'
      : "A critic may only read and search its packet; this proves the client refuses to change files and its search tools cannot reach past the packet on this machine.",
  }
}

/**
 * Whether an item runs: a check that never ran here, last failed, or whose
 * inputs changed is due; one that passed with the same inputs is current.
 */
export function setupItem(
  check: SetupCheck,
  inputs: SetupInputs,
  last: SetupCheckResult | undefined,
): SetupItem {
  const shown = shownCheck(check)
  if (check.kind === 'read-only' && check.client_config?.kind === 'agy')
    return { ...shown, detail: READ_ONLY_PENDING, status: 'not-available' }
  const base = { ...shown, inputs, ...(last === undefined ? {} : { last }) }
  if (last === undefined) return { ...base, due_because: 'never-checked', status: 'due' }
  const changed = SETUP_INPUTS.filter((key) => last.inputs[key] !== inputs[key])
  if (changed.length > 0) return { ...base, changed, due_because: 'inputs-changed', status: 'due' }
  if (last.result === 'fail') return { ...base, due_because: 'failed', status: 'due' }
  return { ...base, status: 'current' }
}

/** The check as the skill sees it: the client's routing config stays with the runner. */
function shownCheck(check: SetupCheck): Omit<SetupItem, 'status'> {
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
          ...(inputs.has('cli_version')
            ? { cli_version: inputs.nullableString('cli_version') }
            : {}),
          ...(inputs.has('agent_hash') ? { agent_hash: inputs.nullableString('agent_hash') } : {}),
          ...(inputs.has('routing_hash') ? { routing_hash: inputs.string('routing_hash') } : {}),
          ...(inputs.has('files_hash') ? { files_hash: inputs.string('files_hash') } : {}),
        },
        result: entry.oneOf('result', ['pass', 'fail', 'unproven'] as const),
      }
    }),
    version: file.literal('version', 1),
  }
}
