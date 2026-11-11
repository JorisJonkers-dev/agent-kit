import { join } from 'node:path'

import { CONTEXT_MAP, GLOSSARIES, inForce } from '../domain/decision-docs.js'
import type { ArtifactSource } from '../domain/manifest.js'
import type { Refusal } from '../domain/outcomes.js'
import {
  citedIssues,
  HISTORY_ISSUES_DIR,
  HISTORY_LOG_PATH,
  HISTORY_MAX_COMMITS,
  HISTORY_MAX_PATHS,
  renderHistoryLog,
  sourceLines,
  sourceSample,
} from '../domain/recovery.js'
import type { PacketLimits } from '../domain/repository-packet.js'
import { sampleTree, type PreparedRepository } from './adr-packet.js'
import { logRecords } from './check-adrs.js'
import type { UnfrozenFile } from './code-packet.js'
import type { CritiqueDeps } from './ports.js'
import type { TrackedArtifact, Tracker } from './tracker.js'

/** The reference a recovery run's artifact carries: the repository itself, not a tracker item. */
export const RECOVERY_ARTIFACT = 'recovery'

export interface RecoveryPacketRequest {
  /** The commit whose tree is sampled. */
  readonly head: string
  /** Routing's packet limits. */
  readonly limits: PacketLimits
  /**
   * The tracker to read the issues the commit messages cite from, only when
   * the allowlist entry allows history; undefined when it does not, or when
   * the repository's tracker cannot be read.
   */
  readonly history: { readonly tracker: Tracker | undefined } | undefined
}

export interface RecoveryPacket {
  readonly status: 'read'
  /** What depth is chosen from: the decision docs the packet holds, as one text. */
  readonly artifact: ArtifactSource & { readonly text: string }
  readonly files: readonly UnfrozenFile[]
  readonly repository: PreparedRepository
  /** How many lines of source the repository tracks, and how many context maps it holds. */
  readonly source_lines: number
  readonly context_maps: number
  /** How many ADRs in force went in as references. */
  readonly adrs: number
}

/**
 * Reads what a decision-recovery run freezes: the repository's tree
 * sampled by rule, `tree/MANIFEST.tsv` and `facts/`, every ADR in force,
 * `CONTEXT.md` and a `CONTEXT-MAP.md`, the last three under `references/`.
 * There is no artifact under review: the decisions are what the critics
 * find. Only when the allowlist entry allows history does the packet also
 * hold `history/log.txt`, the commit messages for the sampled files, and
 * `history/issues/`, the issues those messages cite where the tracker can
 * be read. Without that permission no commit message or issue text is read.
 */
export class RecoveryPacketReader {
  constructor(private readonly deps: Pick<CritiqueDeps, 'files' | 'git'>) {}

  async read(root: string, request: RecoveryPacketRequest): Promise<RecoveryPacket | Refusal> {
    const { files } = this.deps
    const present = await files.listFiles(root)
    const records = await this.records(root)
    const glossary = GLOSSARIES.find((name) => present.includes(name))
    const maps = present.filter((path) => path.split('/').at(-1) === CONTEXT_MAP)
    const references = [...(glossary === undefined ? [] : [glossary]), ...maps, ...records.map((record) => record.source)]
    const documents: UnfrozenFile[] = await Promise.all(
      references.map(async (source) => ({
        kind: 'reference' as const,
        path: join('references', source),
        source,
        text: await files.readText(join(root, source)),
      })),
    )
    const sampled = await sampleTree(this.deps, root, { artifactText: documents.map((file) => file.text).join('\n\n'), files: documents }, request, sourceSample)
    if (sampled.status === 'refused') return sampled
    const history = request.history === undefined ? [] : await this.history(root, sampled, request.history.tracker)
    return {
      adrs: records.length,
      artifact: { labels: [], reference: RECOVERY_ARTIFACT, text: documents.map((file) => file.text).join('\n\n'), tracker: 'none' },
      context_maps: maps.length,
      files: [...sampled.files, ...history],
      repository: { ...sampled.repository, packet_bytes: [...sampled.files, ...history].reduce((sum, file) => sum + Buffer.byteLength(file.text), 0) },
      source_lines: sourceLines(sampled.tracked),
      status: 'read',
    }
  }

  /** The ADRs in force: a record that is proposed, rejected, superseded or deprecated is no decision the code can be checked against. */
  private async records(root: string): Promise<readonly { readonly source: string }[]> {
    const paths = await logRecords(this.deps.files, root)
    const read = await Promise.all(paths.map(async (source) => ({ source, text: await this.deps.files.readText(join(root, source)) })))
    return read.filter((record) => inForce(record.text))
  }

  /** An issue a commit message cites, or nothing when the tracker cannot read it: a number that is no issue, or a tracker of local files. */
  private async issue(tracker: Tracker, number: number): Promise<TrackedArtifact | undefined> {
    try {
      const read = await tracker.artifact({ comments: false, reference: `#${String(number)}` })
      return read.status === 'read' ? read : undefined
    } catch {
      return undefined
    }
  }

  /** The commit log of the sampled files, and the issues it cites. */
  private async history(root: string, sampled: { readonly files: readonly UnfrozenFile[]; readonly repository: PreparedRepository }, tracker: Tracker | undefined): Promise<readonly UnfrozenFile[]> {
    const paths = sampled.files.filter((file) => file.kind === 'tree').map((file) => file.source).slice(0, HISTORY_MAX_PATHS)
    const commit = sampled.repository.head_commit
    const entries = await this.deps.git.commitLog(root, commit, paths, HISTORY_MAX_COMMITS)
    const log: UnfrozenFile = { kind: 'history', path: HISTORY_LOG_PATH, source: `git log ${commit}`, text: renderHistoryLog(entries, commit).text }
    if (tracker === undefined) return [log]
    const issues = await Promise.all(
      citedIssues(entries).map(async (number) => this.issue(tracker, number)),
    )
    return [
      log,
      ...issues.flatMap((issue): UnfrozenFile[] =>
        issue === undefined ? [] : [{ kind: 'history', path: join(HISTORY_ISSUES_DIR, issue.name), source: issue.reference, text: issue.text }],
      ),
    ]
  }
}
