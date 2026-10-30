import { createHash } from 'node:crypto'

import type { ModelFamily, OrchestratorFamily } from './family.js'
import type { DroppedCritic, RosterCritic } from './routing.js'
import type { CritiqueStage, Depth } from './stage.js'

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

/** `2026-10-30T12:00:00.000Z` becomes `20261030T120000Z`. */
export function compactTimestamp(at: Date): string {
  return at.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '')
}
