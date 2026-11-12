import { join } from 'node:path'

import { BUILD_CONFIG_MAX_BYTES, READMES, buildConfigs } from '../domain/architecture.js'
import type { ArtifactSource } from '../domain/manifest.js'
import type { Refusal } from '../domain/outcomes.js'
import { sourceLines, sourceSample } from '../domain/recovery.js'
import type { PacketLimits } from '../domain/repository-packet.js'
import { sampleTree, type PreparedRepository } from './adr-packet.js'
import type { UnfrozenFile } from './code-packet.js'
import type { CritiqueDeps } from './ports.js'
import { decisionReferences } from './recovery-packet.js'

/** The reference an architecture run's artifact carries: the repository itself, not a tracker item. */
export const ARCHITECTURE_ARTIFACT = 'architecture'

export interface ArchitecturePacketRequest {
  /** The commit whose tree is sampled. */
  readonly head: string
  /** Routing's packet limits. */
  readonly limits: PacketLimits
}

export interface ArchitecturePacket {
  readonly status: 'read'
  /** What depth is chosen from: the documents the packet holds, as one text. */
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
 * Reads what an architecture review freezes: the repository's tree sampled
 * by rule, `tree/MANIFEST.tsv` and `facts/`, and under `references/` every
 * ADR in force, `CONTEXT.md`, a `CONTEXT-MAP.md`, the README and the build
 * configuration at the repository root, whose layer-rule configs a finding
 * about dependency direction needs. There is no artifact under review and
 * no history: the critics review the design against what the project
 * declared and what the packet shows.
 */
export class ArchitecturePacketReader {
  constructor(private readonly deps: Pick<CritiqueDeps, 'files' | 'git' | 'repoMap'>) {}

  async read(root: string, request: ArchitecturePacketRequest): Promise<ArchitecturePacket | Refusal> {
    const { adrs, documents: records, maps } = await decisionReferences(this.deps, root)
    const documents = [...records, ...(await this.configuration(root))]
    const text = documents.map((file) => file.text).join('\n\n')
    const sampled = await sampleTree(this.deps, root, { artifactText: text, files: documents }, request, sourceSample, true)
    if (sampled.status === 'refused') return sampled
    return {
      adrs,
      artifact: { labels: [], reference: ARCHITECTURE_ARTIFACT, text, tracker: 'none' },
      context_maps: maps,
      files: sampled.files,
      repository: sampled.repository,
      source_lines: sourceLines(sampled.tracked),
      status: 'read',
    }
  }

  /** The README and the root build configuration, each cut to its byte cap. */
  private async configuration(root: string): Promise<readonly UnfrozenFile[]> {
    const present = await this.deps.files.listFiles(root)
    const readme = READMES.find((name) => present.includes(name))
    const sources = [...(readme === undefined ? [] : [readme]), ...buildConfigs(present)]
    return Promise.all(
      sources.map(async (source): Promise<UnfrozenFile> => ({
        kind: 'reference',
        path: join('references', source),
        source,
        text: Buffer.from(await this.deps.files.readText(join(root, source))).subarray(0, BUILD_CONFIG_MAX_BYTES).toString('utf8'),
      })),
    )
  }
}
