import { dirname, join } from 'node:path'

import { gateLine, openCriticalFollowUps, parseGateRecord, type FollowUp, type GateRecord } from '../domain/feature-review.js'
import type { RunManifest } from '../domain/manifest.js'
import { CritiqueError } from '../domain/outcomes.js'
import { parseIssueReference } from '../domain/tracker.js'
import type { AdjudicatedFinding } from '../domain/verdicts.js'
import { decoded } from './decode.js'
import type { CritiqueDeps } from './ports.js'
import { TrackerResolver, type TrackedTicket } from './tracker.js'

/** The gate a feature-review run holds, in the run directory. */
export function gatePath(runDir: string): string {
  return join(runDir, 'gate.json')
}

/**
 * Every feature-review gate of the repository, one JSON line per recorded
 * run in the critique directory of the git common directory, oldest first:
 * the latest line for a head is that head's gate.
 */
export function gateIndexPath(critiqueDir: string): string {
  return join(critiqueDir, 'gates.jsonl')
}

/**
 * Computes a feature review's gate when its verdicts are recorded. Each
 * accepted finding must name a follow-up that is a ticket of the spec. The
 * gate fails while any critical follow-up, this run's or an earlier run's
 * of the same spec, is still open on the tracker, so a re-run on a new head
 * keeps failing until those tickets are closed, whether or not its critics
 * raise the finding again.
 */
export class FeatureGate {
  private readonly trackers: TrackerResolver

  constructor(private readonly deps: Pick<CritiqueDeps, 'clock' | 'files' | 'gh'>) {
    this.trackers = new TrackerResolver(deps)
  }

  async compute(manifest: RunManifest, findings: readonly AdjudicatedFinding[]): Promise<GateRecord> {
    const code = manifest.code
    if (code === undefined) throw new CritiqueError('invalid-run', `run ${manifest.run_id} is a feature review without the commits it reviewed`)
    const spec = manifest.artifact.reference
    const tickets = await this.tickets(manifest)
    const followUps = findings
      .filter((finding) => finding.verdict.verdict === 'accepted')
      .map((finding): FollowUp => {
        const given = finding.verdict.follow_up ?? ''
        const ticket = ticketNamed(tickets, given, manifest.repository)
        if (ticket === undefined) {
          throw new CritiqueError(
            'invalid-input',
            `${finding.id}'s follow-up ${given} is not a ticket of ${spec}; create it as one of the spec's tickets first`,
          )
        }
        return { finding_id: finding.id, open: ticket.open, reference: ticket.reference, severity: finding.severity, ticket: ticket.id }
      })
    const earlier = (await this.index(manifest)).filter((record) => record.repository === manifest.repository && record.spec === spec)
    // A follow-up that is no longer a ticket of the spec cannot be shown closed, so it still holds the gate.
    const isOpen = (reference: string) => tickets.find((ticket) => ticket.reference === reference)?.open ?? true
    const open = openCriticalFollowUps(manifest.run_id, followUps, earlier, isOpen)
    return {
      base_commit: code.base_commit,
      critical_open: open.length,
      follow_ups: followUps,
      gate: open.length === 0 ? 'pass' : 'fail',
      head_commit: code.head_commit,
      line: gateLine(open.length),
      open_critical: open,
      recorded_at: this.deps.clock.now().toISOString(),
      repository: manifest.repository,
      run_id: manifest.run_id,
      spec,
      stage: 'feature-review',
      version: 1,
    }
  }

  /** Writes the gate into the run and appends it to the repository's gate index. */
  async save(manifest: RunManifest, gate: GateRecord): Promise<void> {
    const line = JSON.stringify(gate)
    await this.deps.files.writeText(gatePath(manifest.run_dir), `${JSON.stringify(gate, null, 2)}\n`)
    await this.deps.files.appendText(gateIndexPath(dirname(manifest.run_dir)), `${line}\n`)
  }

  /** The spec's tickets as the tracker holds them now; a tracker that cannot be read leaves the gate unknown. */
  private async tickets(manifest: RunManifest): Promise<readonly TrackedTicket[]> {
    const repository = { commonDir: dirname(dirname(manifest.run_dir)), key: manifest.repository, root: manifest.repo_root, status: 'resolved' as const }
    const tracker = await this.trackers.resolve(repository)
    const read = tracker.status === 'refused' ? tracker : await tracker.tickets(manifest.artifact.reference)
    if (read.status === 'refused') throw new CritiqueError('tracker-unreachable', `the gate needs the spec's tickets: ${read.message}`)
    return read.tickets
  }

  private async index(manifest: RunManifest): Promise<readonly GateRecord[]> {
    const path = gateIndexPath(dirname(manifest.run_dir))
    if (!(await this.deps.files.exists(path))) return []
    const lines = (await this.deps.files.readText(path)).split('\n').filter((line) => line.trim().length > 0)
    return lines.map((line, n) => decoded('invalid-run', `${path} line ${String(n + 1)}`, () => parseGateRecord(line)))
  }
}

/** The ticket a follow-up names: by its id (`#301`, `03`), its reference, or any form of issue reference. */
function ticketNamed(tickets: readonly TrackedTicket[], given: string, repositoryKey: string): TrackedTicket | undefined {
  const issue = parseIssueReference(given, repositoryKey)
  const asReference = issue === undefined ? undefined : `${issue.repository}#${String(issue.number)}`
  return tickets.find((ticket) => ticket.id === given || ticket.reference === given || ticket.reference === asReference)
}
