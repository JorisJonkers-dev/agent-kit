import { join } from 'node:path'

import type { JsonFields } from '../domain/json.js'
import { ledgerLines } from '../domain/ledger.js'
import { parseManifest } from '../domain/manifest.js'
import { CritiqueError } from '../domain/outcomes.js'
import { renderReport, renderSummary } from '../domain/report.js'
import { countVerdicts, VERDICTS, type AdjudicatedRun, type Verdict, type VerdictCounts } from '../domain/verdicts.js'
import { readInput, readJsonFile } from './decode.js'
import { parseFanOutResult } from './fan-out.js'
import { findingsPath, ledgerPath, manifestPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'

export interface RecordResult {
  readonly status: 'recorded'
  readonly run_id: string
  /** True when no critic produced a usable review. */
  readonly degraded: boolean
  readonly counts: VerdictCounts
  readonly report_path: string
  readonly summary_path: string
  readonly ledger_path: string
  readonly summary: string
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
    const verdicts = verdictsByFinding(input.verdicts, new Set(merged.findings.map((finding) => finding.id)))
    const reportPath = join(input.runDir, 'report.md')
    if (await this.deps.files.exists(reportPath)) throw new CritiqueError('already-recorded', `run ${manifest.run_id} is already recorded`)

    const run: AdjudicatedRun = {
      critics: merged.critics,
      degraded: merged.degraded,
      depth: manifest.depth,
      dropped: manifest.dropped,
      findings: merged.findings.map((finding) => ({ ...finding, verdict: verdictFor(verdicts, finding.id) })),
      repository: manifest.repository,
      run_id: manifest.run_id,
      stage: manifest.stage,
    }
    const summary = renderSummary(run, reportPath)
    const summaryPath = join(input.runDir, 'summary.md')
    const ledger = ledgerPath(this.deps.env)
    await this.deps.files.writeText(reportPath, renderReport(run))
    await this.deps.files.writeText(summaryPath, summary)
    await this.deps.files.appendText(ledger, ledgerLines(run, this.deps.clock.now()).map((line) => `${line}\n`).join(''))
    return {
      counts: countVerdicts(run),
      degraded: run.degraded,
      ledger_path: ledger,
      report_path: reportPath,
      run_id: manifest.run_id,
      status: 'recorded',
      summary,
      summary_path: summaryPath,
    }
  }
}

function verdictFor(verdicts: ReadonlyMap<string, Verdict>, id: string): Verdict {
  const verdict = verdicts.get(id)
  if (verdict === undefined) throw new CritiqueError('invalid-input', `no verdict for ${id}`)
  return verdict
}

function verdictsByFinding(list: readonly Verdict[], known: ReadonlySet<string>): ReadonlyMap<string, Verdict> {
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

function readVerdict(fields: JsonFields): Verdict {
  const verdict = fields.oneOf('verdict', VERDICTS)
  const reason = verdict === 'rejected' || fields.has('reason') ? fields.string('reason') : undefined
  const code = fields.optionalMatching('reason_code', isReasonCode, 'a short lowercase slug')
  return {
    finding_id: fields.string('finding_id'),
    verdict,
    ...(reason === undefined ? {} : { reason }),
    ...(code === undefined ? {} : { reason_code: code }),
  }
}

function isReasonCode(value: string): value is string {
  return REASON_CODE.test(value)
}
