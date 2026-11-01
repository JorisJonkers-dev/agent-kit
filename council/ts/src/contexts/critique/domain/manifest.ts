import { createHash } from 'node:crypto'

import { readLimits, type CritiqueLimits } from './budget.js'
import { readDepthChoice, type DepthChoice } from './depth.js'
import { MODEL_FAMILIES, ORCHESTRATOR_FAMILIES, type ModelFamily, type OrchestratorFamily } from './family.js'
import { JsonFields } from './json.js'
import { readDroppedCritic, readRosterCritic, type DroppedCritic, type RosterCritic } from './routing.js'
import { CRITIQUE_STAGES, DEPTHS, type CritiqueStage, type Depth, type Round } from './stage.js'
import type { TicketCheckSummary } from './tickets.js'
import { TRACKER_KINDS } from './tracker.js'

export function sha256(content: string | Uint8Array): string {
  return createHash('sha256').update(content).digest('hex')
}

/**
 * What a packet file is. A document stage freezes its `artifact` and its
 * `reference`s. A code stage freezes the `tree` at the commit under review,
 * the `diff` against the base, the `ticket` or tickets the change delivers
 * and the `spec` they come from. The tickets stage freezes the `spec`, its
 * `ticket`s and the ticket checker's report (`check`). The decisions stage
 * adds the `feature` the decisions under review are for.
 */
export const PACKET_FILE_KINDS = ['artifact', 'reference', 'tree', 'diff', 'ticket', 'spec', 'check', 'feature'] as const
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
  /**
   * Only when a ticket review reviewed the staged change: its hash
   * (`stagedDiffHash`), so a commit gate can tell whether what it is about
   * to commit is what was reviewed.
   */
  readonly staged_diff?: string
}

/**
 * Where an artifact came from: a tracker, or `none` for the decisions
 * stage's decision docs or decision summary, which no tracker holds.
 */
export const ARTIFACT_ORIGINS = [...TRACKER_KINDS, 'none'] as const
export type ArtifactOrigin = (typeof ARTIFACT_ORIGINS)[number]

/** Where the artifact under critique came from. */
export interface ArtifactSource {
  readonly tracker: ArtifactOrigin
  /**
   * `host/owner/repo#<number>` for a GitHub issue, the repository path for a
   * markdown file, and `decision-docs` or `decision-summary` at the decisions stage.
   */
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
  /** Only for a code stage. */
  readonly code?: CodeSource
  /** Only for the tickets stage: what the ticket checker found when the packet was frozen. */
  readonly ticket_check?: TicketCheckSummary
  /** Whether no two critics of the stage may share a family; a fallback retry keeps to it too. */
  readonly distinct_families: boolean
  readonly critics: readonly RosterCritic[]
  readonly dropped: readonly DroppedCritic[]
}

/**
 * The reference of the spec a run belongs to: its artifact's, unless the
 * artifact is a ticket. A ticket review critiques a ticket, so its spec is
 * the source of the packet's spec file.
 */
export function specReference(manifest: Pick<RunManifest, 'stage' | 'artifact' | 'packet_files'>): string {
  const spec = manifest.stage === 'ticket-review' ? manifest.packet_files.find((file) => file.kind === 'spec') : undefined
  return spec?.source ?? manifest.artifact.reference
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
    distinct_families: file.boolean('distinct_families'),
    depth_choice: readDepthChoice(file.object('depth_choice')),
    dropped: file.list('dropped', readDroppedCritic),
    flow: file.string('flow'),
    limits: readLimits(file.object('limits')),
    orchestrator_family: file.oneOf('orchestrator_family', ORCHESTRATOR_FAMILIES),
    packet_dir: file.string('packet_dir'),
    ...(file.has('code') ? { code: readCodeSource(file.object('code')) } : {}),
    packet_files: file.list('packet_files', (packetFile) => ({
      kind: packetFile.oneOf('kind', PACKET_FILE_KINDS),
      path: packetFile.string('path'),
      sha256: packetFile.string('sha256'),
      source: packetFile.string('source'),
    })),
    ...readRound(file),
    ...(file.has('ticket_check') ? { ticket_check: readTicketCheck(file.object('ticket_check')) } : {}),
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

function readTicketCheck(check: JsonFields): TicketCheckSummary {
  const reason = check.optionalString('coverage_reason')
  return {
    cycles: check.stringListList('cycles'),
    dangling_blockers: check.list('dangling_blockers', (blocker) => ({ blocker: blocker.string('blocker'), ticket: blocker.string('ticket') })),
    status: check.oneOf('status', ['passed', 'failed']),
    ticket_ids: check.strings('ticket_ids'),
    uncited_stories: reason === undefined ? check.numbers('uncited_stories') : null,
    ...(reason === undefined ? {} : { coverage_reason: reason }),
  }
}

function readCodeSource(code: JsonFields): CodeSource {
  return {
    base: code.string('base'),
    base_commit: code.string('base_commit'),
    changed_files: code.strings('changed_files'),
    head: code.string('head'),
    head_commit: code.string('head_commit'),
    ...(code.has('staged_diff') ? { staged_diff: code.string('staged_diff') } : {}),
  }
}

function readArtifactSource(artifact: JsonFields): ArtifactSource {
  return {
    labels: artifact.strings('labels'),
    reference: artifact.string('reference'),
    tracker: artifact.oneOf('tracker', ARTIFACT_ORIGINS),
  }
}

/** `2026-10-30T12:00:00.000Z` becomes `20261030T120000Z`. */
export function compactTimestamp(at: Date): string {
  return at.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '')
}
