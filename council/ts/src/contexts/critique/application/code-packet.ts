import { join } from 'node:path'

import type { CodeSource, PacketFileKind } from '../domain/manifest.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import type { CritiqueStage } from '../domain/stage.js'
import type { CodeChange } from '../domain/triage.js'
import { CodeChangeReader } from './code-change.js'
import type { CritiqueDeps } from './ports.js'
import type { ResolvedRepository } from './repository.js'
import type { TrackedArtifact, Tracker } from './tracker.js'

/** One file to freeze into a packet, with its text. */
export interface UnfrozenFile {
  readonly kind: PacketFileKind
  readonly path: string
  readonly source: string
  readonly text: string
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
}

/** The packet folder of the tree at the commit under review. */
export const TREE_DIR = 'tree'
const DIFF_PATH = join('diff', 'change.diff')
/** How many files of the tree are read from git at once. */
const READ_CONCURRENCY = 16

/**
 * Reads what a code stage freezes: every file of the commit under review
 * (from git's object store, so no `.git` and no worktree metadata), the
 * diff from where the change left its base, the ticket or the spec's
 * tickets, and the spec. Each goes into its own packet folder, so the
 * repository's own `AGENTS.md`, `.vibe/` or `.agents/` lands under `tree/`,
 * never at the packet root.
 */
export class CodePacketReader {
  private readonly changes: CodeChangeReader

  constructor(private readonly deps: Pick<CritiqueDeps, 'git'>) {
    this.changes = new CodeChangeReader(deps)
  }

  async read(repository: ResolvedRepository, tracker: Tracker, request: CodePacketRequest): Promise<CodePacket | Refusal> {
    const { stage } = request
    if (request.base === undefined) {
      throw new CritiqueError('invalid-input', `${stage} needs base: the commit or branch the change is diffed against`)
    }
    if (stage === 'ticket-review' && request.ticket === undefined) {
      throw new CritiqueError('invalid-input', 'ticket-review needs ticket: the ticket the change delivers')
    }
    const root = repository.root
    const read = await this.changes.read(root, request.base, request.head)
    const { base_commit: base, head_commit: head } = read

    const spec = await tracker.artifact({ comments: request.comments, reference: request.spec })
    if (spec.status === 'refused') return spec
    const tickets = await this.tickets(tracker, request, stage === 'ticket-review' ? request.ticket : undefined)
    if (tickets.status === 'refused') return tickets

    const tree = await this.tree(root, head)
    const inTree = new Set(tree.map((file) => file.source))
    const changed = read.change.paths.filter((path) => inTree.has(path))
    const files: UnfrozenFile[] = [
      { kind: 'diff', path: DIFF_PATH, source: `${base}..${head}`, text: read.diff },
      ...tickets.files,
      { kind: 'spec', path: join('spec', spec.name), source: spec.reference, text: spec.text },
      ...tree,
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
      },
      files,
      specReference: spec.reference,
      status: 'read',
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

  /** Every text file of the commit's tree; a file holding a NUL byte is binary and left out. */
  private async tree(root: string, commit: string): Promise<readonly UnfrozenFile[]> {
    const paths = await this.deps.git.treeFiles(root, commit)
    const files: UnfrozenFile[] = []
    for (let start = 0; start < paths.length; start += READ_CONCURRENCY) {
      const batch = paths.slice(start, start + READ_CONCURRENCY)
      const texts = await Promise.all(batch.map((path) => this.deps.git.fileAt(root, commit, path)))
      batch.forEach((path, i) => {
        const text = texts[i] ?? ''
        if (!text.includes('\u0000')) files.push({ kind: 'tree', path: join(TREE_DIR, path), source: path, text })
      })
    }
    return files
  }
}

export function ticketFile(name: string, reference: string, text: string): UnfrozenFile {
  return { kind: 'ticket', path: join('tickets', name), source: reference, text }
}
