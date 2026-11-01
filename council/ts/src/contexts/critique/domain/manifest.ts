import { createHash } from 'node:crypto'

import { MODEL_FAMILIES, ORCHESTRATOR_FAMILIES, type ModelFamily, type OrchestratorFamily } from './family.js'
import { JsonFields } from './json.js'
import { readDroppedCritic, readRosterCritic, type DroppedCritic, type RosterCritic } from './routing.js'
import { CRITIQUE_STAGES, DEPTHS, type CritiqueStage, type Depth } from './stage.js'
import { TRACKER_KINDS, type TrackerKind } from './tracker.js'

export function sha256(content: string | Uint8Array): string {
  return createHash('sha256').update(content).digest('hex')
}

/**
 * What a packet file is. A document stage freezes its `artifact` and its
 * `reference`s. A code stage freezes the `tree` at the commit under review,
 * the `diff` against the base, the `ticket` or tickets the change delivers
 * and the `spec` they come from.
 */
export const PACKET_FILE_KINDS = ['artifact', 'reference', 'tree', 'diff', 'ticket', 'spec'] as const
export type PacketFileKind = (typeof PACKET_FILE_KINDS)[number]

export interface PacketFile {
  /** Path inside the packet, which is the critic's working directory. */
  readonly path: string
  /**
   * Where it was frozen from: a repository path, an issue reference for a
   * file read from GitHub, or `<base commit>..<head commit>` for a diff.
   */
  readonly source: string
  readonly kind: PacketFileKind
  readonly sha256: string
}

/** The commits a code stage reviewed. */
export interface CodeSource {
  /** The base as the caller named it. */
  readonly base: string
  /** Where the change left the base: the merge base of the base and the head. */
  readonly base_commit: string
  /** The commit under review as the caller named it. */
  readonly head: string
  readonly head_commit: string
  /** The packet path, under `tree/`, of every file the change touched that is still in the tree. */
  readonly changed_files: readonly string[]
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
  readonly orchestrator_family: OrchestratorFamily
  readonly repository: string
  readonly repo_root: string
  readonly run_dir: string
  readonly packet_dir: string
  readonly allowed_families: readonly ModelFamily[]
  readonly artifact: ArtifactSource
  readonly packet_files: readonly PacketFile[]
  /** Only for a code stage. */
  readonly code?: CodeSource
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

/**
 * The packet files a critic's prompt names. A document packet names every
 * file. A code packet names the diff, the files the change touched, the
 * tickets and the spec, and leaves the rest of the tree for the critic to
 * read by path.
 */
export function listedFiles(manifest: Pick<RunManifest, 'packet_files' | 'code'>): readonly PacketFile[] {
  const { code, packet_files: files } = manifest
  if (code === undefined) return files
  const changed = new Set(code.changed_files)
  const ofKind = (kind: PacketFileKind) => files.filter((file) => file.kind === kind)
  return [...ofKind('diff'), ...ofKind('tree').filter((file) => changed.has(file.path)), ...ofKind('ticket'), ...ofKind('spec')]
}

export function parseManifest(text: string): RunManifest {
  const file = JsonFields.parse(text)
  return {
    allowed_families: file.oneOfList('allowed_families', MODEL_FAMILIES),
    artifact: readArtifactSource(file.object('artifact')),
    created_at: file.string('created_at'),
    critics: file.list('critics', readRosterCritic),
    depth: file.oneOf('depth', DEPTHS),
    dropped: file.list('dropped', readDroppedCritic),
    orchestrator_family: file.oneOf('orchestrator_family', ORCHESTRATOR_FAMILIES),
    packet_dir: file.string('packet_dir'),
    ...(file.has('code') ? { code: readCodeSource(file.object('code')) } : {}),
    packet_files: file.list('packet_files', (packetFile) => ({
      kind: packetFile.oneOf('kind', PACKET_FILE_KINDS),
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

function readCodeSource(code: JsonFields): CodeSource {
  return {
    base: code.string('base'),
    base_commit: code.string('base_commit'),
    changed_files: code.strings('changed_files'),
    head: code.string('head'),
    head_commit: code.string('head_commit'),
  }
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
