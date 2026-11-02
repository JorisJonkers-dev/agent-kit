import { isCheckId } from './checks.js'
import { readCriticRun, type CriticRun } from './critic.js'
import { checkEvidence, type PacketIndex } from './evidence.js'
import type { ModelFamily, OrchestratorFamily } from './family.js'
import {
  CHECK_HINT_CODES,
  HINT_CODES,
  readFindingBody,
  SEVERITIES,
  type CriticFinding,
  type FindingBody,
  type RunFinding,
  type Severity,
} from './findings.js'
import { isOneOf, JsonFields } from './json.js'
import { CritiqueError } from './outcomes.js'
import type { CriticPrompt } from './prompt.js'
import { stageRouting, type CriticClient, type Routing, type RosterCritic, type RosterSlot } from './routing.js'
import { CRITIQUE_STAGES, type CritiqueStage, type Round } from './stage.js'

/**
 * What a refuter may answer. It answers in the finding format: an empty
 * findings list is no objection, and one finding whose `check` names the
 * refuter role's `refute` or `downgrade` rule is the objection.
 */
export const REFUTE_ANSWERS = ['no-objection', 'refute', 'downgrade'] as const
export type RefuteAnswer = (typeof REFUTE_ANSWERS)[number]

/** The two answers that object, each a check id of the refuter role. */
const OBJECTIONS = ['refute', 'downgrade'] as const satisfies readonly RefuteAnswer[]

/**
 * What the round did to one accepted critical finding: the refuter's answer
 * when it holds, `stands` when the refuter objected without evidence the
 * packet supports or gave no usable answer, and `skipped` when no client
 * could refute it.
 */
export const REFUTE_OUTCOMES = [...REFUTE_ANSWERS, 'stands', 'skipped'] as const
export type RefuteOutcome = (typeof REFUTE_OUTCOMES)[number]

/**
 * Why a finding stands or was skipped. `no-eligible-client`: no routed
 * client could run. `no-review`: the refuter failed, wrote to what it may
 * only read, or was never launched. `invalid-answer`: an answer outside the
 * three. Otherwise the evidence check's own code, as for any finding.
 */
export const REFUTE_REASONS = [
  'no-eligible-client',
  'no-review',
  'invalid-answer',
  'fabricated-evidence',
  ...HINT_CODES,
  ...CHECK_HINT_CODES,
] as const
export type RefuteReason = (typeof REFUTE_REASONS)[number]

/** The refute round's result for one accepted critical finding. */
export interface RefuteRecord {
  readonly finding_id: string
  readonly outcome: RefuteOutcome
  /** What the refuter answered, when it gave one answer the runner could read. */
  readonly answer?: RefuteAnswer
  readonly reason_code?: RefuteReason
  /** Free text for the report; never written to the ledger. */
  readonly detail?: string
  /** On a downgrade that holds: the severity the finding now has. */
  readonly severity?: Severity
  /** The refuter's objection as it wrote it, for the report. */
  readonly reply?: FindingBody
  /** The refuter's run; absent when the round was skipped. */
  readonly refuter?: CriticRun
}

/** `refute.json`, which refute writes beside the run and record reads. */
export interface RefuteFile {
  readonly status: 'checked'
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly refute_path: string
  readonly results: readonly RefuteRecord[]
  readonly counts: Readonly<Record<RefuteOutcome, number>>
}

/** One client a stage routes, with the slot whose caps a refuter through it takes. */
export interface RefuterCandidate {
  readonly client: string
  readonly client_config: CriticClient
  readonly slot: RosterSlot
}

/**
 * Every client a stage routes, in the order a refuter is chosen: the base
 * slots, then the deep slots whatever the run's depth, each slot's client
 * followed by its fallbacks. A client named twice keeps its first place.
 */
export function refuterCandidates(routing: Routing, stage: CritiqueStage): readonly RefuterCandidate[] {
  const routes = stageRouting(routing, { stage })
  const candidates: RefuterCandidate[] = []
  for (const slot of [...routes.base, ...routes.deep]) {
    for (const client of [slot.client, ...(routing.fallbacks[slot.client] ?? [])]) {
      const config = routing.clients[client]
      if (config === undefined) throw new CritiqueError('invalid-routing', `routing stage ${stage} names unknown client ${client}`)
      if (!candidates.some((candidate) => candidate.client === client)) candidates.push({ client, client_config: config, slot })
    }
  }
  return candidates
}

/** The families no refuter of a run may come from, beyond those that raised the finding. */
export interface RefuterRules {
  readonly allowedFamilies: readonly ModelFamily[]
  readonly orchestratorFamily: OrchestratorFamily
}

/** Why a candidate's family may not refute this finding, if it may not. */
export function familyBar(family: ModelFamily, rules: RefuterRules, raisers: readonly ModelFamily[]): string | undefined {
  if (raisers.includes(family)) return `${family} raised the finding`
  if (family === rules.orchestratorFamily) return `${family} is the orchestrator's family`
  if (!rules.allowedFamilies.includes(family)) return `the allowlist does not permit ${family}`
  return undefined
}

/**
 * The first candidate whose family neither raised the finding nor is the
 * orchestrator's, that the allowlist permits, and that `blocked` lets run
 * now; or, with none, why each candidate was passed over.
 */
export function chooseRefuter(
  finding: Pick<RunFinding, 'id'>,
  candidates: readonly RefuterCandidate[],
  bars: { readonly family: (family: ModelFamily) => string | undefined; readonly blocked: (client: string) => string | undefined },
): { readonly refuter: RosterCritic } | { readonly skipped: string } {
  const passed: string[] = []
  for (const candidate of candidates) {
    const bar = bars.family(candidate.client_config.family) ?? bars.blocked(candidate.client)
    if (bar !== undefined) {
      passed.push(`${candidate.client}: ${bar}`)
      continue
    }
    return {
      refuter: {
        ...candidate.slot,
        client: candidate.client,
        client_config: candidate.client_config,
        critic_id: `r-${finding.id}-${candidate.client}`,
        fallbacks: [],
        lenses: [],
        role: 'refuter',
      },
    }
  }
  return { skipped: `no routed client can refute it (${passed.length === 0 ? 'the stage routes none' : passed.join('; ')})` }
}

/**
 * The refuter's prompt: the stage's prompt for the refuter role, with the
 * finding under refute after the file list. The finding goes in the prompt,
 * never into the packet, so a quote of it is no evidence.
 */
export function withFinding(prompt: CriticPrompt, finding: RunFinding): CriticPrompt {
  const body: FindingBody = {
    category: finding.category,
    ...(finding.check === undefined ? {} : { check: finding.check }),
    claim: finding.claim,
    evidence: finding.evidence,
    location: finding.location,
    severity: finding.severity,
    ...(finding.suggestion === undefined ? {} : { suggestion: finding.suggestion }),
  }
  const section = [
    '# The finding',
    '',
    'This is the finding you check, as its reviewer reported it. It is not one of the files: your evidence comes from the files.',
    '',
    '```json',
    JSON.stringify(body, null, 2),
    '```',
  ].join('\n')
  return { ...prompt, text: `${prompt.text}\n---\n\n${section}\n` }
}

/** What one refuter launch gave back. */
export interface RefuterReply {
  readonly run: CriticRun
  readonly findings: readonly CriticFinding[]
}

/**
 * Judges a refuter's reply. An objection counts only as one finding that
 * names a check the refuter's prompt declares (`refute` or `downgrade`); a
 * downgrade must leave the finding below critical. Its evidence is then
 * checked as the stage checks any finding's, with the original finding's
 * category and location, against the packet as prepare froze it: anything the
 * check would reject or call a hint leaves the verdict standing.
 */
export function judgeReply(
  finding: RunFinding,
  reply: RefuterReply,
  context: { readonly declared: readonly string[]; readonly stage: CritiqueStage; readonly packet: PacketIndex },
): Omit<RefuteRecord, 'finding_id' | 'refuter'> {
  const { run, findings } = reply
  if (run.status !== 'ok') return { detail: run.detail ?? run.status, outcome: 'stands', reason_code: 'no-review' }
  const [first] = findings
  if (first === undefined) {
    return { answer: 'no-objection', outcome: 'no-objection', ...(run.no_findings_reason === undefined ? {} : { detail: run.no_findings_reason }) }
  }
  const objection: FindingBody = first
  if (findings.length > 1) return invalid(`answered with ${String(findings.length)} entries; a refuter gives one`)
  const answer = objection.check
  if (!isOneOf(OBJECTIONS, answer) || !context.declared.includes(answer)) {
    return invalid(`names the check ${answer ?? '(none)'}; a refuter answers refute or downgrade`, objection)
  }
  if (answer === 'downgrade' && objection.severity === 'critical') return invalid('a downgrade that leaves the finding critical', objection, answer)
  const pre = checkEvidence({ ...objection, category: finding.category, location: finding.location }, context.stage, context.packet)
  if (pre !== undefined) return { answer, detail: pre.reason, outcome: 'stands', reason_code: pre.reason_code, reply: objection }
  return { answer, outcome: answer, reply: objection, ...(answer === 'downgrade' ? { severity: objection.severity } : {}) }
}

function invalid(detail: string, reply?: FindingBody, answer?: RefuteAnswer): Omit<RefuteRecord, 'finding_id' | 'refuter'> {
  return {
    ...(answer === undefined ? {} : { answer }),
    detail,
    outcome: 'stands',
    reason_code: 'invalid-answer',
    ...(reply === undefined ? {} : { reply }),
  }
}

export function countOutcomes(results: readonly RefuteRecord[]): Readonly<Record<RefuteOutcome, number>> {
  const counts = { downgrade: 0, 'no-objection': 0, refute: 0, skipped: 0, stands: 0 }
  for (const result of results) counts[result.outcome] += 1
  return counts
}

/** What every refute ledger line says about its run. */
export interface RefuteLedgerContext {
  readonly recorded_at: string
  readonly repository: string
  readonly flow: string
  readonly spec: string
  readonly run_id: string
  readonly stage: CritiqueStage
  readonly round: Round
}

/**
 * One ledger line per result, metadata only like every ledger line: the
 * finding, the check it named, the outcome and answer, and the refuter's
 * client and cost. The refuter's claim and evidence stay in `refute.json`.
 */
export function refuteLedgerLine(result: RefuteRecord, finding: RunFinding, context: RefuteLedgerContext): string {
  const refuter = result.refuter
  return JSON.stringify({
    type: 'refute',
    ...context,
    answer: result.answer ?? null,
    charged_usd: refuter?.charged_usd ?? null,
    check: isCheckId(finding.check) ? finding.check : null,
    client: refuter?.client ?? null,
    cost_usd: refuter?.cost_usd ?? null,
    critic_id: refuter?.critic_id ?? null,
    family: refuter?.family ?? null,
    finding_id: result.finding_id,
    model: refuter?.model ?? null,
    outcome: result.outcome,
    prompt_hashes: refuter?.prompt_hashes ?? null,
    reason_code: result.reason_code ?? null,
    role: 'refuter',
    severity: result.severity ?? null,
    severity_before: finding.severity,
    status: refuter?.status ?? null,
  })
}

export function parseRefuteFile(text: string): RefuteFile {
  const file = JsonFields.parse(text)
  return {
    counts: countOutcomes(file.list('results', readRefuteRecord)),
    refute_path: file.string('refute_path'),
    results: file.list('results', readRefuteRecord),
    run_id: file.string('run_id'),
    stage: file.oneOf('stage', CRITIQUE_STAGES),
    status: file.literal('status', 'checked'),
  }
}

function readRefuteRecord(fields: JsonFields): RefuteRecord {
  const answer = fields.has('answer') ? fields.oneOf('answer', REFUTE_ANSWERS) : undefined
  const reason = fields.has('reason_code') ? fields.oneOf('reason_code', REFUTE_REASONS) : undefined
  const detail = fields.optionalString('detail')
  const severity = fields.has('severity') ? fields.oneOf('severity', SEVERITIES) : undefined
  return {
    finding_id: fields.string('finding_id'),
    outcome: fields.oneOf('outcome', REFUTE_OUTCOMES),
    ...(answer === undefined ? {} : { answer }),
    ...(reason === undefined ? {} : { reason_code: reason }),
    ...(detail === undefined ? {} : { detail }),
    ...(severity === undefined ? {} : { severity }),
    ...(fields.has('reply') ? { reply: readFindingBody(fields.object('reply')) } : {}),
    ...(fields.has('refuter') ? { refuter: readCriticRun(fields.object('refuter')) } : {}),
  }
}
