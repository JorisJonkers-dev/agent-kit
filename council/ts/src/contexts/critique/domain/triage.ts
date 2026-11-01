import type { JsonFields } from './json.js'

/**
 * What decides, without any model, whether a ticket's change is risky:
 * repository paths that hold risky code, a size above which any change is
 * risky, and tracker labels that mark a ticket risky.
 */
export interface RiskRules {
  /** Globs over repository paths, as git prints them; see `matchesGlob`. */
  readonly risk_paths: readonly string[]
  /** Changed lines (added plus removed) above which a change is risky. */
  readonly changed_lines_threshold: number
  /** Tracker labels, in any case, that make a ticket risky. */
  readonly labels: readonly string[]
}

/** The cheap model asked, only when no rule fired, whether the change warrants a review. */
export interface TriageModel {
  /** A Vibe client of routing's `clients`. */
  readonly client: string
  readonly max_price_usd: number
  readonly time_limit_s: number
}

/** Routing's `triage` as written: the risk rules and the triage model. */
export interface TriageRules extends RiskRules {
  readonly model: TriageModel
}

export function readTriageRules(fields: JsonFields): TriageRules {
  const model = fields.object('model')
  return {
    changed_lines_threshold: fields.number('changed_lines_threshold'),
    labels: fields.strings('labels'),
    model: {
      client: model.string('client'),
      max_price_usd: model.number('max_price_usd'),
      time_limit_s: model.number('time_limit_s'),
    },
    risk_paths: fields.strings('risk_paths'),
  }
}

/** What a change amounts to, as the risk rules read it. */
export interface CodeChange {
  /** Every repository path the change touches, deleted files included. */
  readonly paths: readonly string[]
  /** Lines added plus lines removed. */
  readonly changed_lines: number
}

/** The change a diff and its touched paths describe. */
export function codeChange(diff: string, paths: readonly string[]): CodeChange {
  return { changed_lines: countChangedLines(diff), paths }
}

/** A rule that fired. */
export type RiskSignal =
  | { readonly kind: 'risk-path'; readonly glob: string; readonly path: string }
  | { readonly kind: 'size'; readonly changed_lines: number; readonly threshold: number }
  | { readonly kind: 'label'; readonly label: string }

/**
 * Every rule the change and the ticket's labels fire, in a fixed order: each
 * touched path that matches a risk path (with the first glob it matches),
 * then the size, then each risk label. Pure, so a hook can call it with no
 * model and no network.
 */
export function riskSignals(change: CodeChange, labels: readonly string[], rules: RiskRules): readonly RiskSignal[] {
  const carried = new Set(labels.map((label) => label.toLowerCase()))
  return [
    ...change.paths.flatMap((path): RiskSignal[] => {
      const glob = rules.risk_paths.find((candidate) => matchesGlob(path, candidate))
      return glob === undefined ? [] : [{ glob, kind: 'risk-path', path }]
    }),
    ...(change.changed_lines > rules.changed_lines_threshold
      ? [{ changed_lines: change.changed_lines, kind: 'size' as const, threshold: rules.changed_lines_threshold }]
      : []),
    ...rules.labels.filter((label) => carried.has(label.toLowerCase())).map((label): RiskSignal => ({ kind: 'label', label })),
  ]
}

/** How many touched paths match a risk path. */
export function riskPathHits(change: CodeChange, rules: Pick<RiskRules, 'risk_paths'>): number {
  return change.paths.filter((path) => rules.risk_paths.some((glob) => matchesGlob(path, glob))).length
}

/**
 * Whether a repository path matches a glob. `**` stands for any number of
 * whole path segments (none included), `*` for any characters but `/`, `?`
 * for one character but `/`, and `{a,b}` for either alternative. The glob
 * must match the whole path, case-sensitively, as git prints it.
 */
export function matchesGlob(path: string, glob: string): boolean {
  return globPattern(glob).test(path)
}

function globPattern(glob: string): RegExp {
  let pattern = ''
  let braces = 0
  for (let i = 0; i < glob.length; i += 1) {
    const char = glob.charAt(i)
    if (glob.startsWith('**/', i)) {
      pattern += '(?:.*/)?'
      i += 2
    } else if (glob.startsWith('/**', i) && i + 3 === glob.length) {
      pattern += '(?:/.*)?'
      i += 2
    } else if (glob.startsWith('**', i)) {
      pattern += '.*'
      i += 1
    } else if (char === '*') pattern += '[^/]*'
    else if (char === '?') pattern += '[^/]'
    else if (char === '{') {
      braces += 1
      pattern += '(?:'
    } else if (char === '}' && braces > 0) {
      braces -= 1
      pattern += ')'
    } else if (char === ',' && braces > 0) pattern += '|'
    else pattern += char.replace(/[.+^${}()|[\]\\]/g, '\\$&')
  }
  return new RegExp(`^${pattern}$`)
}

/**
 * Added plus removed lines of a unified diff. A file's header (from its
 * `diff --git` line to its `+++` line or first hunk) is not counted, so a
 * removed line that reads `-- x` is still one line.
 */
export function countChangedLines(diff: string): number {
  let header = false
  let count = 0
  for (const line of diff.split('\n')) {
    if (line.startsWith('diff --git ')) header = true
    else if (header && (line.startsWith('+++ ') || line.startsWith('@@'))) header = false
    else if (!header && (line.startsWith('+') || line.startsWith('-'))) count += 1
  }
  return count
}

/**
 * The triage model's answer, read as yes or no: one word, in any case, with
 * surrounding markup or a full stop allowed. Anything else is no answer.
 */
export function readTriageAnswer(text: string): 'yes' | 'no' | undefined {
  const word = text
    .trim()
    .toLowerCase()
    .replace(/^[\s*_`"']+|[\s*_`"'.!]+$/g, '')
  return word === 'yes' || word === 'no' ? word : undefined
}

/** What triage decides: a risky change is reviewed, a low-risk one skips the review. */
export type TriageDecision = 'risky' | 'low-risk'

/** Why the triage model gave no answer, so the change stays risky. */
export type UnavailableReason =
  | 'not-opted-in'
  | 'code-not-allowed'
  | 'family-not-allowed'
  | 'budget'
  | 'unavailable'
  | 'failed'
  | 'unclear-answer'

/** The signal that decided a triage: a risk rule, the model's answer, or the model's absence. */
export type TriageSignal =
  | RiskSignal
  | { readonly kind: 'model'; readonly answer: 'yes' | 'no' }
  | { readonly kind: 'model-unavailable'; readonly reason: UnavailableReason; readonly detail: string }

/** One line naming the decision and the signal behind it. */
export function triageSummary(decision: TriageDecision, signal: TriageSignal, client: string): string {
  const model = `no risk rule fired, and the triage model (${client})`
  switch (signal.kind) {
    case 'risk-path':
      return `Triage: risky: ${signal.path} matches the risk path ${signal.glob}.`
    case 'size':
      return `Triage: risky: ${String(signal.changed_lines)} changed lines, above the threshold of ${String(signal.threshold)}.`
    case 'label':
      return `Triage: risky: the ticket carries the label ${signal.label}.`
    case 'model':
      return decision === 'risky'
        ? `Triage: risky: ${model} answered yes.`
        : `Triage: low-risk: ${model} answered no, so the review is skipped.`
    case 'model-unavailable':
      return `Triage: risky: ${model} could not answer: ${signal.detail}.`
  }
}

/** What a triage's ledger line is made from. */
export interface TriageRecord {
  readonly status: TriageDecision
  readonly recorded_at: string
  readonly repository: string
  readonly flow: string
  readonly run_id: string
  readonly ticket: { readonly reference: string }
  readonly signal: TriageSignal
  readonly signals: readonly RiskSignal[]
  readonly change: { readonly changed_files: number; readonly changed_lines: number; readonly changed_lines_threshold: number }
  readonly model?: {
    readonly client: string
    readonly model: string
    readonly cost_usd: number | null
    readonly charged_usd: number
    readonly answer?: 'yes' | 'no'
  }
  /** The hash of the triage prompt, when the model was asked. */
  readonly prompt_hash: string | null
}

/**
 * A triage's ledger line. A low-risk change is a ticket-review run that
 * never ran, so it is a `run` line with status `skipped-low-risk`; a risky
 * one is a `triage` line, since the review that follows has run lines of its
 * own. Metadata only: a risk path hit keeps the glob, never the path, and an
 * unavailable model keeps its reason, never its detail.
 */
export function triageLedgerLine(record: TriageRecord): Readonly<Record<string, unknown>> {
  return {
    type: record.status === 'low-risk' ? 'run' : 'triage',
    recorded_at: record.recorded_at,
    repository: record.repository,
    flow: record.flow,
    run_id: record.run_id,
    stage: 'ticket-review',
    status: record.status === 'low-risk' ? 'skipped-low-risk' : 'risky',
    ticket: record.ticket.reference,
    triage_signal: ledgerSignal(record.signal),
    risk_signals: record.signals.map(ledgerSignal),
    changed_files: record.change.changed_files,
    changed_lines: record.change.changed_lines,
    changed_lines_threshold: record.change.changed_lines_threshold,
    client: record.model?.client ?? null,
    model: record.model?.model ?? null,
    answer: record.model?.answer ?? null,
    cost_usd: record.model?.cost_usd ?? null,
    charged_usd: record.model?.charged_usd ?? 0,
    prompt_hash: record.prompt_hash,
  }
}

function ledgerSignal(signal: TriageSignal): Readonly<Record<string, unknown>> {
  if (signal.kind === 'risk-path') return { glob: signal.glob, kind: signal.kind }
  if (signal.kind === 'model-unavailable') return { kind: signal.kind, reason: signal.reason }
  return signal
}
