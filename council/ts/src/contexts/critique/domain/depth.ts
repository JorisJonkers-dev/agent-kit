import { JsonShapeError, type JsonFields } from './json.js'
import type { CritiqueStage, Depth } from './stage.js'

/**
 * What a stage's depth signals are read from. The spec stage needs the
 * artifact's text and its tracker labels; a later stage may add what it
 * measures (a diff, a ticket list) here.
 */
export interface DepthEvidence {
  /** The artifact as the packet freezes it. */
  readonly text: string
  /** The artifact's tracker labels; empty for a markdown file. */
  readonly labels: readonly string[]
  /** At the tickets stage: how many tickets the spec has. */
  readonly tickets?: number
}

type Measure = (evidence: DepthEvidence) => number

/**
 * The size measures each stage knows, by the name routing gives their
 * thresholds. A stage gains depth signals by adding a measure here; the
 * routing shape stays the same. Routing that names a measure its stage does
 * not know is rejected.
 */
const STAGE_MEASURES: Readonly<Record<CritiqueStage, Readonly<Record<string, Measure>>>> = {
  decisions: { decisions: ({ text }) => countDecisions(text), words: ({ text }) => countWords(text) },
  'feature-review': {},
  spec: { user_stories: ({ text }) => countUserStories(text), words: ({ text }) => countWords(text) },
  'ticket-review': {},
  tickets: { tickets: ({ tickets = 0 }) => tickets },
}

/** One stage's depth signals as routing configures them. Any one that fires selects deep. */
export interface DepthSignalsConfig {
  /** Per measure, the value above which the stage goes deep. */
  readonly thresholds: Readonly<Record<string, number>>
  /** Terms whose presence in the artifact, as a whole word and in any case, selects deep. */
  readonly risk_terms: readonly string[]
  /** Tracker labels, in any case, that select deep. */
  readonly labels: readonly string[]
}

export const NO_DEPTH_SIGNALS: DepthSignalsConfig = { labels: [], risk_terms: [], thresholds: {} }

/** A signal that fired. Every field is configuration or a count, never artifact text, so the ledger may keep it. */
export type DepthSignal =
  | { readonly kind: 'threshold'; readonly measure: string; readonly value: number; readonly threshold: number }
  | { readonly kind: 'risk-term'; readonly term: string }
  | { readonly kind: 'label'; readonly label: string }

export interface MeasuredValue {
  readonly value: number
  readonly threshold: number
}

/** How the depth of a run was chosen, beside the depth itself. */
export interface DepthChoice {
  /** `argument` when the caller named the depth, which always wins; `signals` otherwise. */
  readonly chosen_by: 'argument' | 'signals'
  /** Every configured measure with its value, fired or not, so the choice can be checked. */
  readonly measures: Readonly<Record<string, MeasuredValue>>
  /** The signals that fired; with `signals`, deep exactly when there is one. */
  readonly signals: readonly DepthSignal[]
}

export function readDepthSignalsConfig(fields: JsonFields, stage: string): DepthSignalsConfig {
  const known = measuresOf(stage)
  const thresholds = fields.has('thresholds') ? fields.numberMap('thresholds') : {}
  for (const measure of Object.keys(thresholds)) {
    if (known[measure] === undefined) {
      const names = Object.keys(known)
      throw new JsonShapeError(
        `stages.${stage}.depth_signals.thresholds.${measure} is not a measure of stage ${stage}; it knows ${names.length === 0 ? 'none' : names.join(', ')}`,
      )
    }
  }
  return {
    labels: fields.has('labels') ? fields.strings('labels') : [],
    risk_terms: fields.has('risk_terms') ? fields.strings('risk_terms') : [],
    thresholds,
  }
}

/**
 * Chooses a run's depth. An explicit depth always wins; the signals are
 * still read, so the summary can show what they would have chosen.
 */
export function chooseDepth(
  stage: CritiqueStage,
  config: DepthSignalsConfig,
  evidence: DepthEvidence,
  explicit: Depth | undefined,
): { readonly depth: Depth; readonly choice: DepthChoice } {
  const measures = Object.fromEntries(
    Object.entries(measuresOf(stage)).flatMap(([name, measure]) => {
      const threshold = config.thresholds[name]
      return threshold === undefined ? [] : [[name, { threshold, value: measure(evidence) }] as const]
    }),
  )
  const labels = new Set(evidence.labels.map((label) => label.toLowerCase()))
  const signals: DepthSignal[] = [
    ...Object.entries(measures)
      .filter(([, { threshold, value }]) => value > threshold)
      .map(([measure, { threshold, value }]): DepthSignal => ({ kind: 'threshold', measure, threshold, value })),
    ...config.risk_terms.filter((term) => containsTerm(evidence.text, term)).map((term): DepthSignal => ({ kind: 'risk-term', term })),
    ...config.labels.filter((label) => labels.has(label.toLowerCase())).map((label): DepthSignal => ({ kind: 'label', label })),
  ]
  const choice: DepthChoice = { chosen_by: explicit === undefined ? 'signals' : 'argument', measures, signals }
  return { choice, depth: explicit ?? (signals.length > 0 ? 'deep' : 'base') }
}

/** What the signals alone would choose. */
export function signalledDepth(choice: DepthChoice): Depth {
  return choice.signals.length > 0 ? 'deep' : 'base'
}

export function readDepthChoice(fields: JsonFields): DepthChoice {
  return {
    chosen_by: fields.oneOf('chosen_by', ['argument', 'signals']),
    measures: fields.entries('measures', (measure) => ({ threshold: measure.number('threshold'), value: measure.number('value') })),
    signals: fields.list('signals', readDepthSignal),
  }
}

function readDepthSignal(fields: JsonFields): DepthSignal {
  const kind = fields.oneOf('kind', ['threshold', 'risk-term', 'label'])
  if (kind === 'threshold') {
    return { kind, measure: fields.string('measure'), threshold: fields.number('threshold'), value: fields.number('value') }
  }
  return kind === 'risk-term' ? { kind, term: fields.string('term') } : { kind, label: fields.string('label') }
}

function measuresOf(stage: string): Readonly<Record<string, Measure>> {
  return (STAGE_MEASURES as Readonly<Record<string, Readonly<Record<string, Measure>>>>)[stage] ?? {}
}

/**
 * The numbered list items under the spec's `User Stories` heading, up to the
 * next heading of the same or a higher level. Subheadings inside it, such as
 * the groups #167 uses, do not end it.
 */
function countUserStories(text: string): number {
  let level: number | undefined
  let count = 0
  for (const line of text.split(/\r?\n/)) {
    const heading = /^(#{1,6})\s+(.*?)\s*#*\s*$/.exec(line)
    if (heading !== null) {
      const depth = (heading[1] ?? '').length
      if (level !== undefined && depth <= level) level = undefined
      if (level === undefined && /^user stories$/i.test(heading[2] ?? '')) level = depth
    } else if (level !== undefined && /^\s*\d+[.)]\s+\S/.test(line)) {
      count += 1
    }
  }
  return count
}

/**
 * The decisions a decisions-stage artifact records: each glossary term (a
 * line opening with a bold term and a colon, `**Term**:`) and each top-level
 * list item, the shape a decision summary takes. Nested items are details of
 * the decision above them.
 */
function countDecisions(text: string): number {
  return text.split(/\r?\n/).filter((line) => /^\*\*[^*\n]+(?::\*\*|\*\*\s*:)/.test(line) || /^(?:[-*+]|\d+[.)])\s+\S/.test(line)).length
}

function countWords(text: string): number {
  return text.split(/\s+/).filter((word) => word.length > 0).length
}

/** The term as a whole word or phrase, in any case; white space in it matches any run of white space. */
function containsTerm(text: string, term: string): boolean {
  const pattern = term
    .trim()
    .split(/\s+/)
    .map((word) => word.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
    .join('\\s+')
  return new RegExp(`(?<![\\p{L}\\p{N}_])${pattern}(?![\\p{L}\\p{N}_])`, 'iu').test(text)
}
