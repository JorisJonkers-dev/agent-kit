import { join } from 'node:path'

import { sampleCodeTree, sliceReason } from '../domain/code-sample.js'
import { contextSlices, type ChangedFile } from '../domain/context-slices.js'
import type { CodeSource, PacketFileKind, PacketWindow } from '../domain/manifest.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import { buildRepoMap, renderRepoMap, REPO_MAP_PATH, SourceGraph } from '../domain/repo-map.js'
import { FACTS_INDEX_PATH, type PacketLimits } from '../domain/repository-packet.js'
import type { CritiqueStage } from '../domain/stage.js'
import type { CodeChange } from '../domain/triage.js'
import { CodeChangeReader } from './code-change.js'
import type { CritiqueDeps } from './ports.js'
import type { ResolvedRepository } from './repository.js'
import { readTrackedTree } from './tracked-tree.js'
import type { TrackedArtifact, Tracker } from './tracker.js'

/** One file to freeze into a packet, with its text. */
export interface UnfrozenFile {
  readonly kind: PacketFileKind
  readonly path: string
  readonly source: string
  readonly text: string
  /** Only for a tree file the packet cap cut short. */
  readonly window?: PacketWindow
}

export interface CodePacketRequest {
  readonly stage: CritiqueStage
  /** The spec, named the way its tracker names it. */
  readonly spec: string
  /** The ticket a ticket review delivers; a feature review reads every ticket of the spec. */
  readonly ticket: string | undefined
  /** The commit the change starts from, or a branch it left. */
  readonly base: string | undefined
  /** The commit under review. */
  readonly head: string
  readonly comments: boolean
  /** Routing's packet limits: the cap, the window size, the repo map's cap and the paths never sent. */
  readonly limits: PacketLimits
}

/** What of the tree the cap let in. */
export interface CodeTreeSample {
  /** How many files of the tree the cap left out in whole or in part. */
  readonly dropped_files: number
  readonly cap_bytes: number
}

export interface CodePacket {
  readonly status: 'read'
  readonly files: readonly UnfrozenFile[]
  readonly code: CodeSource
  /** The change as the risk rules read it, for the stage's depth signals. */
  readonly change: CodeChange
  /** What the stage critiques: the ticket for a ticket review, the spec for a feature review. */
  readonly artifact: TrackedArtifact
  /** The spec's reference, which a ticket review's flow defaults to. */
  readonly specReference: string
  readonly sample: CodeTreeSample
}

/** The packet folder of the tree at the commit under review. */
export const TREE_DIR = 'tree'
const DIFF_PATH = join('diff', 'change.diff')

/**
 * Reads what a code stage freezes, computed at the commit under review by
 * rule (ADR-0004): the diff from where the change left its base, without
 * the hunks of generated, vendored and lock files; the ticket or the spec's
 * tickets, and the spec; `tree/REPO-MAP.md`, every source file's top-level
 * symbols ranked by references; and a capped `tree/` read from git's object
 * store (so no `.git` and no worktree metadata), filled with the context
 * slices around the change first, then the changed files, their importers
 * and the rest of the tree, with `facts/INDEX.md` recording every drop.
 * Each goes into its own packet folder, so the repository's own
 * `AGENTS.md`, `.vibe/` or `.agents/` lands under `tree/`, never at the
 * packet root.
 */
export class CodePacketReader {
  private readonly changes: CodeChangeReader

  constructor(private readonly deps: Pick<CritiqueDeps, 'git'>) {
    this.changes = new CodeChangeReader(deps)
  }

  async read(repository: ResolvedRepository, tracker: Tracker, request: CodePacketRequest): Promise<CodePacket | Refusal> {
    const { limits, stage } = request
    if (request.base === undefined) {
      throw new CritiqueError('invalid-input', `${stage} needs base: the commit or branch the change is diffed against`)
    }
    if (stage === 'ticket-review' && request.ticket === undefined) {
      throw new CritiqueError('invalid-input', 'ticket-review needs ticket: the ticket the change delivers')
    }
    const root = repository.root
    const read = await this.changes.read(root, request.base, request.head, limits)
    const { base_commit: base, head_commit: head } = read

    const spec = await tracker.artifact({ comments: request.comments, reference: request.spec })
    if (spec.status === 'refused') return spec
    const tickets = await this.tickets(tracker, request, stage === 'ticket-review' ? request.ticket : undefined)
    if (tickets.status === 'refused') return tickets

    const { excluded, tracked } = await readTrackedTree(this.deps, root, head, limits)
    const texts = new Map(tracked.flatMap((file) => (file.text === null ? [] : [[file.path, file.text] as const])))
    const graph = SourceGraph.build([...texts].map(([path, text]) => ({ path, text })))
    const changed = read.change.paths.filter((path) => texts.has(path))
    const changedFiles = await Promise.all(
      changed.map(async (path): Promise<ChangedFile> => ({ before: await this.before(root, base, path), path, text: texts.get(path) ?? '' })),
    )
    const documents: UnfrozenFile[] = [...tickets.files, { kind: 'spec', path: join('spec', spec.name), source: spec.reference, text: spec.text }]
    const repoMap = renderRepoMap(buildRepoMap(graph), { capBytes: limits.repo_map_cap_bytes, commit: head })
    const sample = sampleCodeTree({
      changed,
      commit: head,
      diff: read.diff,
      documentBytes: documents.reduce((sum, file) => sum + Buffer.byteLength(file.text), 0),
      excluded,
      importers: [...new Set(changed.flatMap((path) => graph.importers(path)))].filter((path) => !changed.includes(path)).sort(),
      leftOut: read.left_out,
      limits,
      repoMap,
      slices: contextSlices(graph, changedFiles),
      tracked,
    })
    if (sample.status === 'refused') return sample
    const files: UnfrozenFile[] = [
      { kind: 'diff', path: DIFF_PATH, source: `${base}..${head}`, text: sample.diff },
      ...documents,
      { kind: 'fact', path: REPO_MAP_PATH, source: head, text: repoMap.text },
      { kind: 'fact', path: FACTS_INDEX_PATH, source: head, text: sample.index },
      ...sample.tree.map(
        (file): UnfrozenFile => ({
          kind: 'tree',
          path: join(TREE_DIR, file.path),
          source: file.path,
          text: file.text,
          ...(file.window === undefined ? {} : { window: file.window }),
        }),
      ),
    ]
    return {
      artifact: tickets.ticket ?? spec,
      change: read.change,
      code: {
        base: request.base,
        base_commit: base,
        changed_files: changed.map((path) => join(TREE_DIR, path)),
        head: request.head,
        head_commit: head,
        slices: sample.slices.map((slice) => ({ first: slice.first, last: slice.last, path: join(TREE_DIR, slice.path), reason: sliceReason(slice) })),
      },
      files,
      sample: { cap_bytes: limits.cap_bytes, dropped_files: sample.dropped },
      specReference: spec.reference,
      status: 'read',
    }
  }

  /** A file's text at the base, or nothing when the change added it. */
  private async before(root: string, base: string, path: string): Promise<string | undefined> {
    try {
      return await this.deps.git.fileAt(root, base, path)
    } catch {
      return undefined
    }
  }

  /** The one ticket a ticket review names, or else every ticket of the spec. */
  private async tickets(
    tracker: Tracker,
    request: CodePacketRequest,
    reference: string | undefined,
  ): Promise<Refusal | { readonly status: 'read'; readonly files: readonly UnfrozenFile[]; readonly ticket?: TrackedArtifact }> {
    if (reference !== undefined) {
      const ticket = await tracker.artifact({ comments: request.comments, reference })
      if (ticket.status === 'refused') return ticket
      return { files: [ticketFile(ticket.name, ticket.reference, ticket.text)], status: 'read', ticket }
    }
    const read = await tracker.tickets(request.spec)
    if (read.status === 'refused') return read
    return { files: read.tickets.map((ticket) => ticketFile(ticket.name, ticket.reference, ticket.text)), status: 'read' }
  }
}

export function ticketFile(name: string, reference: string, text: string): UnfrozenFile {
  return { kind: 'ticket', path: join('tickets', name), source: reference, text }
}
