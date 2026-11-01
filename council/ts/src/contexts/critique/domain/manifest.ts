import { createHash } from 'node:crypto'

import { readLimits, type CritiqueLimits } from './budget.js'
import { readDepthChoice, type DepthChoice } from './depth.js'
import { MODEL_FAMILIES, ORCHESTRATOR_FAMILIES, type ModelFamily, type OrchestratorFamily } from './family.js'
import { JsonFields } from './json.js'
import { readDroppedCritic, readRosterCritic, type DroppedCritic, type RosterCritic } from './routing.js'
import { CRITIQUE_STAGES, DEPTHS, type CritiqueStage, type Depth, type Round } from './stage.js'
import { TRACKER_KINDS, type TrackerKind } from './tracker.js'

export function sha256(content: string | Uint8Array): string {
  return createHash('sha256').update(content).digest('hex')
}

export interface PacketFile {
  /** Path inside the packet, which is the critic's working directory. */
  readonly path: string
  /** Repository-relative path it was frozen from, or the issue reference for an artifact read from GitHub. */
  readonly source: string
  readonly kind: 'artifact' | 'reference'
  readonly sha256: string
}

/** Where the artifact under critique came from. */
export interface ArtifactSource {
  readonly tracker: TrackerKind
  /** `host/owner/repo#<number>` for a GitHub issue, the repository path for a markdown file. */
  readonly reference: string
  /** The issue's labels when it was read; empty for a markdown file. */
  readonly labels: readonly string[]
}

/** Everything fan-out and record need to know about a prepared run. */
export interface RunManifest {
  readonly version: 1
  readonly run_id: string
  readonly created_at: string
  readonly stage: CritiqueStage
  readonly depth: Depth
  readonly round: Round
  /** The first round's run id, on a second round. */
  readonly previous_run_id?: string
  /** How the depth was chosen, and the signals that decided it. */
  readonly depth_choice: DepthChoice
  readonly orchestrator_family: OrchestratorFamily
  readonly repository: string
  readonly repo_root: string
  readonly run_dir: string
  readonly packet_dir: string
  readonly allowed_families: readonly ModelFamily[]
  readonly artifact: ArtifactSource
  /**
   * The flow this run belongs to: the spend of every run of one flow of one
   * repository counts against one Vibe cap.
   */
  readonly flow: string
  readonly limits: CritiqueLimits
  readonly packet_files: readonly PacketFile[]
  /** Whether no two critics of the stage may share a family; a fallback retry keeps to it too. */
  readonly distinct_families: boolean
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

export function parseManifest(text: string): RunManifest {
  const file = JsonFields.parse(text)
  return {
    allowed_families: file.oneOfList('allowed_families', MODEL_FAMILIES),
    artifact: readArtifactSource(file.object('artifact')),
    created_at: file.string('created_at'),
    critics: file.list('critics', readRosterCritic),
    depth: file.oneOf('depth', DEPTHS),
    distinct_families: file.boolean('distinct_families'),
    depth_choice: readDepthChoice(file.object('depth_choice')),
    dropped: file.list('dropped', readDroppedCritic),
    flow: file.string('flow'),
    limits: readLimits(file.object('limits')),
    orchestrator_family: file.oneOf('orchestrator_family', ORCHESTRATOR_FAMILIES),
    packet_dir: file.string('packet_dir'),
    packet_files: file.list('packet_files', (packetFile) => ({
      kind: packetFile.oneOf('kind', ['artifact', 'reference']),
      path: packetFile.string('path'),
      sha256: packetFile.string('sha256'),
      source: packetFile.string('source'),
    })),
    ...readRound(file),
    repo_root: file.string('repo_root'),
    repository: file.string('repository'),
    run_dir: file.string('run_dir'),
    run_id: file.string('run_id'),
    stage: file.oneOf('stage', CRITIQUE_STAGES),
    version: file.literal('version', 1),
  }
}

/** A manifest written before rounds existed is a first round. */
function readRound(file: JsonFields): Pick<RunManifest, 'round' | 'previous_run_id'> {
  if (!file.has('round') || file.number('round') !== 2) return { round: file.has('round') ? file.literal('round', 1) : 1 }
  return { previous_run_id: file.string('previous_run_id'), round: 2 }
}

function readArtifactSource(artifact: JsonFields): ArtifactSource {
  return {
    labels: artifact.strings('labels'),
    reference: artifact.string('reference'),
    tracker: artifact.oneOf('tracker', TRACKER_KINDS),
  }
}

/** `2026-10-30T12:00:00.000Z` becomes `20261030T120000Z`. */
export function compactTimestamp(at: Date): string {
  return at.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '')
}
