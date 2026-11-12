import { join } from 'node:path'

import { ADR_DIRECTORY, GLOSSARIES } from '../domain/decision-docs.js'
import type { ArtifactSource } from '../domain/manifest.js'
import { refusal, type Refusal } from '../domain/outcomes.js'
import { ADR_CHECK_PATH } from '../domain/adr-log.js'
import { rankedPaths, rankingNote } from '../domain/ranking.js'
import { isSourcePath } from '../domain/recovery.js'
import { FACTS_INDEX_PATH, MANIFEST_PATH, NO_FACTS, sampleRepository, type PacketLimits, type TrackedFile } from '../domain/repository-packet.js'
import { checkLog, logRecords, type CheckAdrsResult } from './check-adrs.js'
import { TREE_DIR, type UnfrozenFile } from './code-packet.js'
import { insideRepository } from './paths.js'
import type { CritiqueDeps } from './ports.js'
import { readTrackedTree } from './tracked-tree.js'

/** The reference an ADR review's artifact carries: the repository's decision records, not a tracker item. */
export const ADR_ARTIFACT = 'adrs'

export interface AdrPacketRequest {
  /** The records under review; undefined means every record in `docs/adr/`. */
  readonly adrs: readonly string[] | undefined
  /** The reference paths the caller names; undefined means the glossary and the records not under review. */
  readonly references: readonly string[] | undefined
  /**
   * Only when the repository allows code: the commit whose tree is sampled,
   * and routing's packet limits. Without it the packet holds documents only.
   */
  readonly code?: { readonly head: string; readonly limits: PacketLimits }
}

/** What prepare reports about a repository packet's tree. */
export interface PreparedRepository {
  readonly head_commit: string
  /** How many sampled files of the tree were sent, in whole or in part. */
  readonly tree_files: number
  /** How many sampled files the cap left out, in whole or in part. */
  readonly dropped_files: number
  /** The bytes of every packet file. */
  readonly packet_bytes: number
  readonly cap_bytes: number
}

export interface AdrPacket {
  readonly status: 'read'
  /** What depth is chosen from: the records under review, as one text, and how many there are. */
  readonly artifact: ArtifactSource & { readonly text: string }
  readonly adrs: number
  readonly files: readonly UnfrozenFile[]
  /** Only when the code permission let the tree be sampled. */
  readonly repository?: PreparedRepository
}

/**
 * Reads what an ADR review freezes. The records under review go to
 * `artifact/` at their repository paths, and the glossary and every other
 * record go to `references/`. Every record is read whatever its status,
 * since a superseded record is part of the log under review. No tracker is
 * read: a review stage belongs to no tracker item.
 *
 * With the code permission on, the packet is a repository packet: beside
 * the documents it holds `tree/MANIFEST.tsv`, a sample of the tree at the
 * named commit chosen by rule (the files the records name first), and
 * `facts/` with its index of what was sent and what the cap dropped.
 * Vendored, generated and lock files are never read.
 */
export class AdrPacketReader {
  constructor(private readonly deps: Pick<CritiqueDeps, 'files' | 'git'>) {}

  async read(root: string, request: AdrPacketRequest): Promise<AdrPacket | Refusal> {
    const { files } = this.deps
    const records = await logRecords(files, root)
    const reviewed = request.adrs === undefined ? records : request.adrs.map((path) => insideRepository(root, path))
    if (reviewed.length === 0) return refusal('no-adrs', `found no decision records in ${ADR_DIRECTORY}/ to review`)
    const references = request.references?.map((path) => insideRepository(root, path)) ?? [
      ...(await this.glossary(root)),
      ...records.filter((path) => !reviewed.includes(path)),
    ]
    const read = async (folder: 'artifact' | 'reference', source: string): Promise<UnfrozenFile> => ({
      kind: folder,
      path: join(folder === 'artifact' ? 'artifact' : 'references', source),
      source,
      text: await files.readText(join(root, source)),
    })
    const artifact = await Promise.all(reviewed.map((source) => read('artifact', source)))
    const check = await checkLog(this.deps, root, records)
    const documents = [...artifact, ...(await Promise.all(references.map((source) => read('reference', source)))), checkReportFile(check)]
    const text = artifact.map((file) => file.text).join('\n\n')
    const packet: AdrPacket = {
      adrs: artifact.length,
      artifact: { labels: [], reference: ADR_ARTIFACT, text, tracker: 'none' },
      files: documents,
      status: 'read',
    }
    return request.code === undefined ? packet : this.withTree(root, packet, request.code)
  }

  /** Adds the sampled tree, its manifest and the facts to a documents-only packet. */
  private async withTree(root: string, packet: AdrPacket, code: NonNullable<AdrPacketRequest['code']>): Promise<AdrPacket | Refusal> {
    const sampled = await sampleTree(this.deps, root, { artifactText: packet.artifact.text, files: packet.files }, code)
    if (sampled.status === 'refused') return sampled
    return { ...packet, files: sampled.files, repository: sampled.repository }
  }

  /** The first of `GLOSSARY.md` and `CONTEXT.md` the repository has, as decision docs pick it. */
  private async glossary(root: string): Promise<readonly string[]> {
    const present = await this.deps.files.listFiles(root)
    const found = GLOSSARIES.find((name) => present.includes(name))
    return found === undefined ? [] : [found]
  }
}

/** What check-adrs reports on the whole log, settled before any critic reads the records. */
function checkReportFile(check: CheckAdrsResult): UnfrozenFile {
  return { kind: 'check', path: ADR_CHECK_PATH, source: `check-adrs ${ADR_DIRECTORY}`, text: `${JSON.stringify(check, null, 2)}\n` }
}

/** A packet of documents, and the text that names the files of the tree to sample first. */
export interface DocumentPacket {
  readonly artifactText: string
  readonly files: readonly UnfrozenFile[]
}

/** A documents packet with a sample of the repository's tree beside it. */
export interface SampledTree {
  readonly status: 'sampled'
  readonly files: readonly UnfrozenFile[]
  readonly repository: PreparedRepository
  /** Every tracked path that may be sent, with its text, for what a stage measures of the repository. */
  readonly tracked: readonly TrackedFile[]
}

/**
 * Adds a repository packet's tree to a packet of documents: `tree/MANIFEST.tsv`,
 * a sample of the tree at the commit chosen by rule (the files the documents
 * name first), and `facts/INDEX.md` with what was sent and what the cap
 * dropped. Vendored, generated and lock files are never read.
 */
export async function sampleTree(
  deps: Pick<CritiqueDeps, 'git' | 'repoMap'>,
  root: string,
  packet: DocumentPacket,
  code: { readonly head: string; readonly limits: PacketLimits },
  rest?: (tracked: readonly TrackedFile[]) => readonly string[],
  /** Whether code-nav's repository map chooses and orders the files after the named ones, entry files and cycles. */
  ranked = false,
): Promise<SampledTree | Refusal> {
  const commit = await deps.git.resolveCommit(root, code.head)
  const { excluded, tracked } = await readTrackedTree(deps, root, commit, code.limits)
  const ranking = ranked ? await deps.repoMap?.rank(root) : undefined
  const sendable = new Set(tracked.filter((file) => file.text !== null).map((file) => file.path))
  // The ranked source files first, then the stage's own order for the rest; a file is sampled once, in its first place.
  const restPaths = ranking === undefined ? rest?.(tracked) : [...rankedPaths(ranking).filter((path) => sendable.has(path) && isSourcePath(path)), ...(rest?.(tracked) ?? [])]
  const sample = sampleRepository({
    artifactText: packet.artifactText,
    commit,
    documentBytes: packet.files.reduce((sum, file) => sum + Buffer.byteLength(file.text), 0),
    documents: packet.files.map((file) => file.source),
    excluded,
    // No scan computes facts yet: #232 and #233 compute them here, and
    // freeze each of `sample.facts` as `facts/<name>`, kind `fact`.
    facts: NO_FACTS,
    limits: code.limits,
    ...(ranking === undefined ? {} : { rankingNote: rankingNote(ranking) }),
    ...(restPaths === undefined ? {} : { rest: restPaths }),
    tracked,
  })
  if (sample.status === 'refused') return sample
  const source = commit
  const files: UnfrozenFile[] = [
    ...packet.files,
    { kind: 'fact', path: MANIFEST_PATH, source, text: sample.manifest },
    { kind: 'fact', path: FACTS_INDEX_PATH, source, text: sample.index },
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
    files,
    repository: {
      cap_bytes: code.limits.cap_bytes,
      dropped_files: sample.dropped,
      head_commit: commit,
      packet_bytes: files.reduce((sum, file) => sum + Buffer.byteLength(file.text), 0),
      tree_files: sample.tree.length,
    },
    status: 'sampled',
    tracked,
  }
}
