import { createHash } from 'node:crypto'

import { MODEL_FAMILIES, ORCHESTRATOR_FAMILIES, type ModelFamily, type OrchestratorFamily } from './family.js'
import { JsonFields } from './json.js'
import { readDroppedCritic, readRosterCritic, type DroppedCritic, type RosterCritic } from './routing.js'
import { CRITIQUE_STAGES, DEPTHS, type CritiqueStage, type Depth } from './stage.js'

export function sha256(text: string): string {
  return createHash('sha256').update(text).digest('hex')
}

export interface PacketFile {
  /** Path inside the packet, which is the critic's working directory. */
  readonly path: string
  /** Repository-relative path it was frozen from. */
  readonly source: string
  readonly kind: 'artifact' | 'reference'
  readonly sha256: string
}

/** Everything fan-out and record need to know about a prepared run. */
export interface RunManifest {
  readonly version: 1
  readonly run_id: string
  readonly created_at: string
  readonly stage: CritiqueStage
  readonly depth: Depth
  readonly orchestrator_family: OrchestratorFamily
  readonly repository: string
  readonly repo_root: string
  readonly run_dir: string
  readonly packet_dir: string
  readonly allowed_families: readonly ModelFamily[]
  readonly packet_files: readonly PacketFile[]
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

export function parseManifest(text: string): RunManifest {
  const file = JsonFields.parse(text)
  return {
    allowed_families: file.oneOfList('allowed_families', MODEL_FAMILIES),
    created_at: file.string('created_at'),
    critics: file.list('critics', readRosterCritic),
    depth: file.oneOf('depth', DEPTHS),
    dropped: file.list('dropped', readDroppedCritic),
    orchestrator_family: file.oneOf('orchestrator_family', ORCHESTRATOR_FAMILIES),
    packet_dir: file.string('packet_dir'),
    packet_files: file.list('packet_files', (packetFile) => ({
      kind: packetFile.oneOf('kind', ['artifact', 'reference']),
      path: packetFile.string('path'),
      sha256: packetFile.string('sha256'),
      source: packetFile.string('source'),
    })),
    repo_root: file.string('repo_root'),
    repository: file.string('repository'),
    run_dir: file.string('run_dir'),
    run_id: file.string('run_id'),
    stage: file.oneOf('stage', CRITIQUE_STAGES),
    version: file.literal('version', 1),
  }
}

/** `2026-10-30T12:00:00.000Z` becomes `20261030T120000Z`. */
export function compactTimestamp(at: Date): string {
  return at.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '')
}
