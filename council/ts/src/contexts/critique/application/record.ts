import { join } from 'node:path'

import { ledgerLines } from '../domain/ledger.js'
import type { RunManifest } from '../domain/manifest.js'
import { renderReport, renderSummary } from '../domain/report.js'
import { countVerdicts, VERDICTS, type AdjudicatedRun, type Verdict, type VerdictCounts, type VerdictKind } from '../domain/verdicts.js'
import type { FanOutResult } from './fan-out.js'
import { asObject, requireString, type JsonObject } from './input.js'
import { findingsPath, ledgerPath, manifestPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

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
export async function record(deps: CritiqueDeps, raw: unknown): Promise<RecordResult> {
  const input = asObject(raw, 'record input')
  const runDir = requireString(input, 'run_dir')
  const manifest = JSON.parse(await deps.files.readText(manifestPath(runDir))) as RunManifest
  const fan = JSON.parse(await deps.files.readText(findingsPath(runDir))) as FanOutResult
  const verdicts = readVerdicts(input, fan)
  const reportPath = join(runDir, 'report.md')
  if (await deps.files.exists(reportPath)) throw new Error(`run ${manifest.run_id} is already recorded`)

  const run: AdjudicatedRun = {
    critics: fan.critics,
    degraded: fan.degraded,
    depth: manifest.depth,
    dropped: manifest.dropped,
    findings: fan.findings.map((finding) => ({ ...finding, verdict: verdictFor(verdicts, finding.id) })),
    repository: manifest.repository,
    run_id: manifest.run_id,
    stage: manifest.stage,
  }
  const summary = renderSummary(run, reportPath)
  const summaryPath = join(runDir, 'summary.md')
  const ledger = ledgerPath(deps.env)
  await deps.files.writeText(reportPath, renderReport(run))
  await deps.files.writeText(summaryPath, summary)
  await deps.files.appendText(ledger, ledgerLines(run, deps.clock.now()).map((line) => `${line}\n`).join(''))
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

function verdictFor(verdicts: ReadonlyMap<string, Verdict>, id: string): Verdict {
  const verdict = verdicts.get(id)
  if (verdict === undefined) throw new Error(`no verdict for ${id}`)
  return verdict
}

function readVerdicts(input: JsonObject, fan: FanOutResult): ReadonlyMap<string, Verdict> {
  const list = input.verdicts
  if (!Array.isArray(list)) throw new Error('verdicts must be a list')
  const known = new Set(fan.findings.map((finding) => finding.id))
  const verdicts = new Map<string, Verdict>()
  for (const item of list) {
    const verdict = readVerdict(asObject(item, 'a verdict'))
    if (!known.has(verdict.finding_id)) throw new Error(`unknown finding ${verdict.finding_id}`)
    if (verdicts.has(verdict.finding_id)) throw new Error(`more than one verdict for ${verdict.finding_id}`)
    verdicts.set(verdict.finding_id, verdict)
  }
  return verdicts
}

function readVerdict(item: JsonObject): Verdict {
  const kind = requireString(item, 'verdict')
  if (!(VERDICTS as readonly string[]).includes(kind)) throw new Error(`verdict must be one of ${VERDICTS.join(', ')}`)
  const reason = kind === 'rejected' || item.reason !== undefined ? requireString(item, 'reason') : undefined
  const code = item.reason_code === undefined ? undefined : requireString(item, 'reason_code')
  if (code !== undefined && !REASON_CODE.test(code)) throw new Error('reason_code must be a short lowercase slug')
  return {
    finding_id: requireString(item, 'finding_id'),
    verdict: kind as VerdictKind,
    ...(reason === undefined ? {} : { reason }),
    ...(code === undefined ? {} : { reason_code: code }),
  }
}
