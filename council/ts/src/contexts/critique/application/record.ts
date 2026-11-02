import { join } from 'node:path'

import type { GateRecord } from '../domain/feature-review.js'
import { SEVERITIES, type RunFinding } from '../domain/findings.js'
import type { JsonFields } from '../domain/json.js'
import { ledgerLines } from '../domain/ledger.js'
import { parseManifest, specReference } from '../domain/manifest.js'
import { CritiqueError } from '../domain/outcomes.js'
import { parseRefuteFile, type RefuteRecord } from '../domain/refute.js'
import { renderReport, renderSummary } from '../domain/report.js'
import type { Round } from '../domain/stage.js'
import type { TicketReviewRecord } from '../domain/ticket-review.js'
import type { TicketCheckStatus } from '../domain/tickets.js'
import {
  countVerdicts,
  secondRoundDue,
  severityOverrideProblem,
  stageVerdictError,
  verdictProblem,
  VERDICTS,
  type AdjudicatedFinding,
  type AdjudicatedRun,
  type Verdict,
  type VerdictCounts,
} from '../domain/verdicts.js'
import { readInput, readJsonFile } from './decode.js'
import { parseFanOutResult } from './fan-out.js'
import { FeatureGate } from './feature-gate.js'
import { findingsPath, ledgerPath, manifestPath, refutePath, reportPath as reportPathOf } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { TicketReviewLog } from './ticket-review.js'

export interface RecordResult {
  readonly status: 'recorded'
  readonly run_id: string
  /** True when no critic produced a usable review. */
  readonly degraded: boolean
  readonly round: Round
  /** More than three critical findings were accepted in a first round: prepare again with `previous_run_dir`. */
  readonly second_round_due: boolean
  readonly counts: VerdictCounts
  readonly report_path: string
  readonly summary_path: string
  readonly ledger_path: string
  readonly summary: string
  /** Only for the tickets stage: `failed` while the checker found a dependency cycle, so `/implement` must not start. */
  readonly ticket_check?: TicketCheckStatus
  /** A feature review's gate: its line, the head it holds for and the follow-ups that hold it. */
  readonly gate?: GateRecord
  /** A ticket review's record: whether it passed, and the staged change it vouches for. */
  readonly ticket_review?: TicketReviewRecord
}

const REASON_CODE = /^[a-z][a-z0-9-]{0,39}$/

/**
 * Takes the orchestrator's verdict on every finding, then writes the report
 * and the summary into the run directory and appends the run's metadata to
 * the user's ledger. A run is recorded once.
 */
export class RecordCommand implements CritiqueCommand<RecordResult> {
  constructor(private readonly deps: CritiqueDeps) {}

  async run(raw: unknown): Promise<RecordResult> {
    const input = readInput(raw, 'record', (fields) => ({
      runDir: fields.string('run_dir'),
      verdicts: fields.list('verdicts', readVerdict),
    }))
    const manifest = await readJsonFile(this.deps.files, manifestPath(input.runDir), 'invalid-run', parseManifest)
    const merged = await readJsonFile(this.deps.files, findingsPath(input.runDir), 'invalid-run', parseFanOutResult)
    for (const verdict of input.verdicts) {
      const error = stageVerdictError(manifest.stage, verdict) ?? verdictProblem(manifest.stage, verdict, manifest.packet_files)
      if (error !== undefined) throw new CritiqueError('invalid-input', error)
    }
    const verdicts = verdictsByFinding(input.verdicts, new Set(merged.findings.map((finding) => finding.id)))
    for (const finding of merged.findings) {
      const given = verdicts.get(finding.id)
      const error = given === undefined ? undefined : severityOverrideProblem(finding, given)
      if (error !== undefined) throw new CritiqueError('invalid-input', error)
    }
    const reportPath = reportPathOf(input.runDir)
    if (await this.deps.files.exists(reportPath)) throw new CritiqueError('already-recorded', `run ${manifest.run_id} is already recorded`)

    const refutes = await this.refutes(input.runDir, merged.findings)
    const adjudicated = applyRefutes(
      merged.findings.map((finding) => adjudicate(verdicts, finding)),
      refutes ?? [],
    )
    const gates = new FeatureGate(this.deps)
    const gate = manifest.stage === 'feature-review' ? await gates.compute(manifest, adjudicated) : undefined
    const run: AdjudicatedRun = {
      critics: merged.critics,
      degraded: merged.degraded,
      depth: manifest.depth,
      depth_choice: manifest.depth_choice,
      dropped: manifest.dropped,
      findings: adjudicated,
      flow: manifest.flow,
      ...(gate === undefined ? {} : { gate }),
      repository: manifest.repository,
      round: manifest.round,
      ...(manifest.previous_run_id === undefined ? {} : { previous_run_id: manifest.previous_run_id }),
      ...(refutes === undefined ? {} : { refutes }),
      run_id: manifest.run_id,
      spec: specReference(manifest),
      stage: manifest.stage,
      ...(manifest.ticket_check === undefined ? {} : { ticket_check: manifest.ticket_check }),
    }
    const reviews = new TicketReviewLog(this.deps)
    const ticketReview = manifest.stage === 'ticket-review' ? await reviews.compute(manifest, run) : undefined
    const summary = renderSummary(run, reportPath)
    const summaryPath = join(input.runDir, 'summary.md')
    const ledger = ledgerPath(this.deps.env)
    await this.deps.files.writeText(reportPath, renderReport(run))
    await this.deps.files.writeText(summaryPath, summary)
    await this.deps.files.appendText(ledger, ledgerLines(run, this.deps.clock.now()).map((line) => `${line}\n`).join(''))
    if (gate !== undefined) await gates.save(manifest, gate)
    if (ticketReview !== undefined) await reviews.save(manifest, ticketReview)
    return {
      counts: countVerdicts(run),
      degraded: run.degraded,
      ...(gate === undefined ? {} : { gate }),
      ledger_path: ledger,
      report_path: reportPath,
      round: run.round,
      run_id: manifest.run_id,
      second_round_due: secondRoundDue(run),
      status: 'recorded',
      summary,
      summary_path: summaryPath,
      ...(run.ticket_check === undefined ? {} : { ticket_check: run.ticket_check.status }),
      ...(ticketReview === undefined ? {} : { ticket_review: ticketReview }),
    }
  }

  /** The refute round's results, when the run had one; each must name a finding of the run. */
  private async refutes(runDir: string, findings: readonly RunFinding[]): Promise<readonly RefuteRecord[] | undefined> {
    const path = refutePath(runDir)
    if (!(await this.deps.files.exists(path))) return undefined
    const { results } = await readJsonFile(this.deps.files, path, 'invalid-run', parseRefuteFile)
    const unknown = results.find((result) => !findings.some((finding) => finding.id === result.finding_id))
    if (unknown !== undefined) throw new CritiqueError('invalid-run', `${path} refutes ${unknown.finding_id}, which the run does not have`)
    return results
  }
}

/**
 * Applies the refute round to the orchestrator's verdicts, so the
 * second-round count sees only the criticals still accepted. A refutation
 * that holds rejects the finding (`refuted`); a downgrade that holds lowers
 * its severity and keeps it accepted. Every other outcome leaves the verdict
 * standing, and a finding the orchestrator no longer accepts is left as it
 * ruled.
 */
function applyRefutes(findings: readonly AdjudicatedFinding[], refutes: readonly RefuteRecord[]): readonly AdjudicatedFinding[] {
  return findings.map((finding) => {
    const result = refutes.find((refute) => refute.finding_id === finding.id)
    if (result === undefined || finding.verdict.verdict !== 'accepted') return finding
    if (result.outcome === 'downgrade' && result.severity !== undefined) return { ...finding, severity: result.severity }
    if (result.outcome !== 'refute') return finding
    const reason = `Refuted by ${result.refuter?.critic_id ?? 'the refuter'}: ${result.reply?.claim ?? ''}`
    const { severity, severity_reason: severityReason } = finding.verdict
    const override = { ...(severity === undefined ? {} : { severity }), ...(severityReason === undefined ? {} : { severity_reason: severityReason }) }
    return { ...finding, verdict: { finding_id: finding.id, reason, reason_code: 'refuted', verdict: 'rejected', ...override } }
  })
}

/**
 * A finding with the orchestrator's verdict. A severity override replaces
 * the severity the finding's check fixed, and stays on the verdict even
 * where a pre-verdict stands in for the rest of it.
 */
function adjudicate(verdicts: ReadonlyMap<string, Verdict>, finding: RunFinding): AdjudicatedFinding {
  const given = verdicts.get(finding.id)
  const verdict = verdictFor(verdicts, finding)
  if (given?.severity === undefined) return { ...finding, verdict }
  return {
    ...finding,
    severity: given.severity,
    verdict: { ...verdict, severity: given.severity, ...(given.severity_reason === undefined ? {} : { severity_reason: given.severity_reason }) },
  }
}

/**
 * The orchestrator's verdict, weighed against fan-out's pre-verdict. A
 * fabricated-evidence rejection stands whatever the orchestrator says, and
 * only a rejection may restate it. A hint stands unless the orchestrator
 * accepts or rejects the finding. A finding without a pre-verdict needs a
 * verdict.
 */
function verdictFor(verdicts: ReadonlyMap<string, Verdict>, finding: RunFinding): Verdict {
  const given = verdicts.get(finding.id)
  const pre = finding.pre_verdict
  if (pre === undefined) {
    if (given === undefined) throw new CritiqueError('invalid-input', `no verdict for ${finding.id}`)
    return given
  }
  const mechanical: Verdict = { finding_id: finding.id, reason: pre.reason, reason_code: pre.reason_code, verdict: pre.verdict }
  if (pre.verdict === 'rejected') {
    if (given !== undefined && given.verdict !== 'rejected') {
      throw new CritiqueError('invalid-input', `${finding.id} was rejected as fabricated-evidence, which a verdict cannot overturn`)
    }
    return mechanical
  }
  if (given?.verdict === 'needs-human') {
    throw new CritiqueError('invalid-input', `${finding.id} is a hint: its verdict may only be accepted, rejected or hint`)
  }
  return given ?? mechanical
}

export function verdictsByFinding(list: readonly Verdict[], known: ReadonlySet<string>): ReadonlyMap<string, Verdict> {
  const verdicts = new Map<string, Verdict>()
  for (const verdict of list) {
    if (!known.has(verdict.finding_id)) throw new CritiqueError('invalid-input', `unknown finding ${verdict.finding_id}`)
    if (verdicts.has(verdict.finding_id)) {
      throw new CritiqueError('invalid-input', `more than one verdict for ${verdict.finding_id}`)
    }
    verdicts.set(verdict.finding_id, verdict)
  }
  return verdicts
}

export function readVerdict(fields: JsonFields): Verdict {
  const verdict = fields.oneOf('verdict', VERDICTS)
  const reason = verdict === 'rejected' || fields.has('reason') ? fields.string('reason') : undefined
  const code = fields.optionalMatching('reason_code', isReasonCode, 'a short lowercase slug')
  const findingId = fields.string('finding_id')
  const answered = fields.has('answered') ? fields.boolean('answered') : undefined
  const followUp = fields.optionalString('follow_up')
  const traceTickets = fields.has('trace_tickets') ? fields.strings('trace_tickets') : undefined
  const regressionTest = fields.optionalString('regression_test')
  const fixed = fields.has('fixed') ? fields.boolean('fixed') : undefined
  const severity = fields.has('severity') ? fields.oneOf('severity', SEVERITIES) : undefined
  const severityReason = fields.optionalMatching('severity_reason', isReasonCode, 'a short lowercase slug')
  return {
    finding_id: findingId,
    verdict,
    ...(reason === undefined ? {} : { reason }),
    ...(code === undefined ? {} : { reason_code: code }),
    ...(answered === undefined ? {} : { answered }),
    ...(followUp === undefined ? {} : { follow_up: followUp }),
    ...(traceTickets === undefined ? {} : { trace_tickets: traceTickets }),
    ...(regressionTest === undefined ? {} : { regression_test: regressionTest }),
    ...(fixed === undefined ? {} : { fixed }),
    ...(severity === undefined ? {} : { severity }),
    ...(severityReason === undefined ? {} : { severity_reason: severityReason }),
  }
}

function isReasonCode(value: string): value is string {
  return REASON_CODE.test(value)
}
