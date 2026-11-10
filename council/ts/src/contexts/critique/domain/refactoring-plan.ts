import { withoutFences } from './decision-docs.js'
import type { TicketSize } from './tickets.js'

/** The sections a refactoring plan holds, in the order the format lists them. */
export const PLAN_SECTIONS = ['Goal', 'Target', 'Safety net', 'Steps', 'Not doing', 'Open decisions'] as const

/** The fields every step states, as `- **Field**: value` lines. */
export const PLAN_FIELDS = [
  'Kind',
  'Technique',
  'Anchor',
  'Touches',
  'Safety net',
  'Main stays green because',
  'Rollback',
  'Transitional element',
  'Blocked by',
  'Done when',
] as const

/** The one kind a step has. */
export const PLAN_KINDS = ['characterize', 'tidy', 'structure', 'behaviour', 'expand', 'migrate', 'contract', 'remove-transitional'] as const

export type PlanKind = (typeof PLAN_KINDS)[number]

export interface PlanError {
  readonly step?: string
  /** A short kebab-case id, one per rule. */
  readonly rule: string
  readonly message: string
}

export interface PlanStep {
  readonly id: string
  readonly title: string
  readonly kind: string
  readonly blocked_by: readonly string[]
}

/** What the decision log says about the ADRs a plan may anchor to; absent when no repository was given. */
export interface PlanAdrs {
  /** The ADR numbers the log holds. */
  readonly present: ReadonlySet<number>
  /** The numbers of the records the log marks superseded. */
  readonly superseded: ReadonlySet<number>
}

export interface PlanCheck {
  readonly errors: readonly PlanError[]
  /** The migrate and contract steps that touch stored data, for the depth signal. */
  readonly stored_data_steps: readonly string[]
  readonly steps: readonly PlanStep[]
  /** What was not checked, and why. */
  readonly notes: readonly string[]
}

export type PlanCheckStatus = 'passed' | 'failed'

export function planCheckStatus(check: Pick<PlanCheck, 'errors'>): PlanCheckStatus {
  return check.errors.length > 0 ? 'failed' : 'passed'
}

interface RawStep {
  readonly id: string
  readonly title: string
  readonly fields: ReadonlyMap<string, Field>
  /** The non-field lines under the heading. */
  readonly prose: readonly string[]
  /** Fields given more than once. */
  readonly duplicates: readonly string[]
}

interface Field {
  /** The field's line and its continuation lines, then its sub-bullets, joined: what a rule reads. */
  readonly value: string
  readonly text: string
  /** The sub-bullets under the field line; empty when the value is a single line. */
  readonly items: readonly string[]
}

const STEP_HEADING = /^###[ \t]+(S\d+)[ \t]*:[ \t]*(.*?)[ \t]*$/
const FIELD_LINE = /^[-*][ \t]+\*\*([^*]+?)(:)?\*\*[ \t]*(:)?[ \t]*(.*)$/
const STEP_ID = /\bS\d+\b/g
const NONE = /^\W*(?:none|n\/a|nothing)\W*$/i
const UNTESTED = /^\W*none\b|\b(?:untested|not tested|no (?:existing )?tests?|nothing covers)\b/i
const STORED_DATA = /\b(?:tables?|columns?|migrations?|schemas?|databases?|stored data)\b/i
const URL = /\b[a-z][a-z0-9+.-]*:\/\/\S+/gi
const EXTENSIONS = 'ts|tsx|js|mjs|cjs|jsx|py|kt|java|go|rs|rb|sh|md|json|ya?ml|toml|sql|css|html'
const PATH_LIKE = new RegExp(
  `(?:[\\w@.-]+/)+[\\w@-]+\\.[A-Za-z]\\w{0,5}\\b|(?:[\\w@.-]+/){2,}[\\w@.-]+|(?<![\\w.-])[a-z_][\\w-]*\\.(?:${EXTENSIONS})\\b`,
  'g',
)
const FACT_ANCHOR = new RegExp(`\`([^\`]+)\``, 'g')
const ADR_ID = /\bADR[- ]?(\d{1,5})\b/gi
const PREREQUISITE = /\bprerequisite-of:[ \t]*(S\d+)\b/gi

/** The path-like tokens of a text, outside URLs. */
export function pathsIn(text: string): readonly string[] {
  return text.replace(URL, ' ').match(PATH_LIKE) ?? []
}

function sectionsOf(lines: readonly string[]): ReadonlyMap<string, readonly string[]> {
  const sections = new Map<string, string[]>()
  let current: string[] | undefined
  for (const line of lines) {
    const heading = /^##[ \t]+(.*?)[ \t]*$/.exec(line)
    if (heading !== null) {
      const name = PLAN_SECTIONS.find((section) => section.toLowerCase() === heading[1]?.toLowerCase())
      current = name === undefined || sections.has(name) ? undefined : []
      if (name !== undefined && current !== undefined) sections.set(name, current)
    } else current?.push(line)
  }
  return sections
}

function fieldsOf(lines: readonly string[]): Pick<RawStep, 'duplicates' | 'fields' | 'prose'> {
  const fields = new Map<string, { text: string[]; items: string[] }>()
  const duplicates: string[] = []
  const prose: string[] = []
  let current: { text: string[]; items: string[] } | undefined
  for (const line of lines) {
    const field = FIELD_LINE.exec(line)
    const name = field === null ? undefined : PLAN_FIELDS.find((known) => known.toLowerCase() === field[1]?.trim().toLowerCase())
    if (line.trim() === '') continue
    if (field !== null && name !== undefined && (field[2] !== undefined || field[3] !== undefined)) {
      current = { items: [], text: [field[4] ?? ''] }
      if (fields.has(name)) duplicates.push(name)
      else fields.set(name, current)
    } else if (current !== undefined && /^[ \t]+\S/.test(line)) {
      const bullet = /^[ \t]+[-*][ \t]+(.*)$/.exec(line)
      if (bullet === null) current.text.push(line.trim())
      else current.items.push((bullet[1] ?? '').trim())
    } else {
      current = undefined
      prose.push(line)
    }
  }
  return {
    duplicates,
    fields: new Map(
      [...fields].map(([name, { text, items }]) => {
        const joined = text.join(' ').trim()
        return [name, { items, text: joined, value: [joined, ...items].join(' ').trim() }]
      }),
    ),
    prose,
  }
}

function parseSteps(lines: readonly string[], errors: PlanError[]): readonly RawStep[] {
  const steps: { id: string; title: string; lines: string[] }[] = []
  for (const line of lines) {
    if (!line.startsWith('###')) {
      steps.at(-1)?.lines.push(line)
      continue
    }
    const heading = STEP_HEADING.exec(line)
    if (heading === null) {
      errors.push({ message: `the heading "${line.trim()}" is not "### S01: <title>"`, rule: 'bad-step-heading' })
      continue
    }
    const id = heading[1] ?? ''
    if (steps.some((step) => step.id === id)) errors.push({ message: `step ${id} is defined twice`, rule: 'duplicate-step', step: id })
    steps.push({ id, lines: [], title: heading[2] ?? '' })
  }
  return steps.map((step) => ({ ...fieldsOf(step.lines), id: step.id, title: step.title }))
}

/** The kinds a Kind value names: split on whitespace, commas, slashes, bars, plus signs, ampersands and "and". */
function kindsIn(value: string): readonly string[] {
  return value.split(/[\s,/|+&]+/).filter((kind) => kind !== '' && kind !== 'and')
}

const idsIn = (text: string): readonly string[] => (text.match(STEP_ID) ?? []).map((id) => id.toUpperCase())

/** How many things a field lists: its sub-bullets, else its comma or semicolon separated parts. */
function countOf(field: Field | undefined, separator: RegExp): number {
  if (field === undefined) return 0
  if (field.items.length > 0) return field.items.length
  return field.text.split(separator).filter((part) => part.trim() !== '').length
}

/** Whether the Goal says behaviour changes: a sentence that says so and does not say it does not. */
export function goalChangesBehaviour(goal: string): boolean {
  const says = /\bbehaviou?r\b[^.\n]*\b(?:chang\w*|differ\w*|alter\w*)|\b(?:chang\w*|alter\w*)\b[^.\n]*\bbehaviou?r\b/i
  const denied = /\b(?:no|not|never|without)\b(?:\W+\w+){0,3}?\W+behaviou?r\b|\bbehaviou?r\b(?:\W+\w+){0,2}?\W+(?:not|never)\W+(?:\w+\W+)?(?:chang|differ|alter)/i
  return goal.split(/(?<=[.!?])\s+|\n/).some((sentence) => says.test(sentence) && !denied.test(sentence))
}

/** The members of every dependency cycle, one sorted list each. */
function cyclesOf(edges: ReadonlyMap<string, readonly string[]>): readonly (readonly string[])[] {
  const index = new Map<string, number>()
  const low = new Map<string, number>()
  const stack: string[] = []
  const onStack = new Set<string>()
  const cycles: string[][] = []
  const visit = (node: string): void => {
    index.set(node, index.size)
    low.set(node, index.get(node) ?? 0)
    stack.push(node)
    onStack.add(node)
    for (const next of edges.get(node) ?? []) {
      if (!index.has(next)) {
        visit(next)
        low.set(node, Math.min(low.get(node) ?? 0, low.get(next) ?? 0))
      } else if (onStack.has(next)) low.set(node, Math.min(low.get(node) ?? 0, index.get(next) ?? 0))
    }
    if (low.get(node) === index.get(node)) {
      const members: string[] = []
      for (let member = stack.pop(); member !== undefined; member = stack.pop()) {
        onStack.delete(member)
        members.push(member)
        if (member === node) break
      }
      if (members.length > 1 || (edges.get(node) ?? []).includes(node)) cycles.push(members.sort())
    }
  }
  for (const node of edges.keys()) if (!index.has(node)) visit(node)
  return cycles
}

/**
 * Checks a refactoring plan: its sections, the fields and kind of each step,
 * the step graph, and the rules of the format that a script can judge. `size`
 * is routing's ticket size; `adrs` is absent when no repository was given.
 */
export function checkPlan(text: string, size: TicketSize, adrs: PlanAdrs | undefined): PlanCheck {
  const errors: PlanError[] = []
  const notes: string[] = []
  const sections = sectionsOf(withoutFences(text).split(/\r?\n/))
  for (const name of PLAN_SECTIONS) {
    if (!sections.has(name)) errors.push({ message: `the plan has no "## ${name}" section`, rule: 'missing-section' })
  }
  const steps = parseSteps(sections.get('Steps') ?? [], errors)
  if (sections.has('Steps') && steps.length === 0) errors.push({ message: 'the plan has no steps', rule: 'no-steps' })
  const known = new Set(steps.map((step) => step.id))
  const valueOf = (step: RawStep, name: string): string => step.fields.get(name)?.value ?? ''
  const kindOf = (step: RawStep): string => valueOf(step, 'Kind').toLowerCase()
  const kindsOf = new Map(steps.map((step) => [step.id, kindOf(step)]))
  const edges = new Map(steps.map((step) => [step.id, idsIn(valueOf(step, 'Blocked by'))]))

  const goal = (sections.get('Goal') ?? []).join('\n')
  const behaviourGoal = goalChangesBehaviour(goal)
  const stepKinds = new Set(steps.flatMap((step) => kindsIn(kindOf(step))))

  for (const step of steps) {
    const fail = (rule: string, message: string): void => void errors.push({ message, rule, step: step.id })
    for (const name of PLAN_FIELDS) if (valueOf(step, name) === '') fail('missing-field', `step ${step.id} has no "${name}" field`)

    for (const name of new Set(step.duplicates)) fail('duplicate-field', `step ${step.id} gives "${name}" more than once`)
    for (const path of pathsIn(step.prose.join(' '))) fail('path-outside-anchor', `step ${step.id} names the path ${path} outside its fields; paths belong only in fact anchors`)
    const kinds = kindsIn(kindOf(step))
    if (kinds.length > 1) fail('multiple-kinds', `step ${step.id} has ${String(kinds.length)} kinds (${kinds.join(', ')}); a step has exactly one`)
    for (const kind of kinds) if (!(PLAN_KINDS as readonly string[]).includes(kind)) fail('unknown-kind', `step ${step.id} has the unknown kind "${kind}"`)

    const blocked = valueOf(step, 'Blocked by')
    if (blocked !== '' && !NONE.test(blocked) && idsIn(blocked).length === 0) fail('blocked-by-unknown', `step ${step.id} is blocked by "${blocked}", which names no step`)
    for (const id of edges.get(step.id) ?? []) if (!known.has(id)) fail('blocked-by-unknown', `step ${step.id} is blocked by ${id}, which is no step of the plan`)

    const transitional = valueOf(step, 'Transitional element')
    if (transitional !== '' && !NONE.test(transitional)) {
      const removers = idsIn(transitional).filter((id) => kindsOf.get(id) === 'remove-transitional')
      if (removers.length === 0) fail('transitional-no-removal', `step ${step.id} adds a transitional element that no remove-transitional step removes`)
    }

    const anchor = valueOf(step, 'Anchor')
    if (anchor !== '') checkAnchor(step.id, anchor, known, adrs, errors)

    if (kinds.includes('behaviour') && !behaviourGoal) fail('behaviour-not-in-goal', `step ${step.id} changes behaviour, but the Goal does not say behaviour changes`)

    if (kinds.includes('structure') && UNTESTED.test(valueOf(step, 'Safety net')) && !ancestorsOf(step.id, edges).some((id) => kindsOf.get(id) === 'characterize')) {
      fail('untested-structure', `step ${step.id} restructures untested code with no characterize step before it`)
    }

    for (const [name, field] of step.fields) {
      if (name === 'Anchor' || name === 'Touches') continue
      for (const path of pathsIn([field.text, ...field.items].join(' '))) fail('path-outside-anchor', `step ${step.id} names the path ${path} in "${name}"; paths belong only in fact anchors`)
    }

    const touches = countOf(step.fields.get('Touches'), /[,;]/)
    if (touches > size.max_modules) fail('too-many-touches', `step ${step.id} touches ${String(touches)} modules; the limit is ${String(size.max_modules)}`)
    const criteria = countOf(step.fields.get('Done when'), /;/)
    if (criteria > size.max_criteria) fail('too-many-criteria', `step ${step.id} has ${String(criteria)} done-when criteria; the limit is ${String(size.max_criteria)}`)

    if (kinds.includes('expand') && !(stepKinds.has('migrate') && stepKinds.has('contract'))) {
      const missing = ['migrate', 'contract'].filter((kind) => !stepKinds.has(kind))
      fail('expand-incomplete', `step ${step.id} expands, but the plan has no ${missing.join(' or ')} step`)
    }
  }

  for (const cycle of cyclesOf(edges)) {
    const message = cycle.length === 1 ? `step ${cycle[0] ?? ''} blocks itself` : `steps ${cycle.join(', ')} block each other in a cycle`
    errors.push({ message, rule: 'step-cycle' })
  }
  for (const [section, lines] of sections) {
    if (section === 'Goal' || section === 'Steps') continue
    for (const path of pathsIn(lines.join('\n'))) errors.push({ message: `the "${section}" section names the path ${path}; paths belong only in fact anchors`, rule: 'path-outside-anchor' })
  }
  if (adrs === undefined) notes.push('ADR ids were not checked against docs/adr: no repo was given')

  const stored = steps.filter((step) => {
    const kind = kindOf(step)
    return (kind === 'migrate' || kind === 'contract') && STORED_DATA.test(`${valueOf(step, 'Touches')} ${valueOf(step, 'Technique')} ${step.fields.get('Touches')?.items.join(' ') ?? ''}`)
  })
  return {
    errors,
    notes,
    steps: steps.map((step) => ({ blocked_by: edges.get(step.id) ?? [], id: step.id, kind: kindOf(step), title: step.title })),
    stored_data_steps: stored.map((step) => step.id),
  }
}

function ancestorsOf(id: string, edges: ReadonlyMap<string, readonly string[]>): readonly string[] {
  const seen = new Set<string>()
  const queue = [...(edges.get(id) ?? [])]
  for (let next = queue.shift(); next !== undefined; next = queue.shift()) {
    if (seen.has(next)) continue
    seen.add(next)
    queue.push(...(edges.get(next) ?? []))
  }
  return [...seen]
}

/** An anchor is a fact row (a backticked path, with an optional `#symbol`), an ADR id, or `prerequisite-of:Sn`. */
function checkAnchor(step: string, anchor: string, known: ReadonlySet<string>, adrs: PlanAdrs | undefined, errors: PlanError[]): void {
  const fail = (rule: string, message: string): void => void errors.push({ message, rule, step })
  const prerequisites = [...anchor.matchAll(PREREQUISITE)].map((match) => (match[1] ?? '').toUpperCase())
  const numbers = [...anchor.matchAll(ADR_ID)].map((match) => Number(match[1]))
  const facts = [...anchor.matchAll(FACT_ANCHOR)].filter((match) => pathsIn((match[1] ?? '').split('#')[0] ?? '').length > 0)
  if (prerequisites.length + numbers.length + facts.length === 0) fail('anchor-unresolved', `step ${step} is anchored to no fact row, ADR id or prerequisite-of:Sn`)
  for (const id of prerequisites) if (!known.has(id)) fail('anchor-unknown-step', `step ${step} is anchored to prerequisite-of:${id}, which is no step of the plan`)
  if (adrs === undefined) return
  for (const number of numbers) {
    const id = `ADR-${String(number).padStart(4, '0')}`
    if (!adrs.present.has(number)) fail('adr-absent', `step ${step} is anchored to ${id}, which is not in docs/adr`)
    else if (adrs.superseded.has(number)) fail('adr-superseded', `step ${step} is anchored to ${id}, which is superseded`)
  }
}
