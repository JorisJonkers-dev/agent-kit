import { join } from 'node:path'

import { ADR_DIRECTORY, GLOSSARIES } from '../domain/decision-docs.js'
import type { ArtifactSource } from '../domain/manifest.js'
import { refusal, type Refusal } from '../domain/outcomes.js'
import type { UnfrozenFile } from './code-packet.js'
import { insideRepository } from './paths.js'
import type { CritiqueFilesPort } from './ports.js'

/** The reference an ADR review's artifact carries: the repository's decision records, not a tracker item. */
export const ADR_ARTIFACT = 'adrs'

export interface AdrPacketRequest {
  /** The records under review; undefined means every record in `docs/adr/`. */
  readonly adrs: readonly string[] | undefined
  /** The reference paths the caller names; undefined means the glossary and the records not under review. */
  readonly references: readonly string[] | undefined
}

export interface AdrPacket {
  readonly status: 'read'
  /** What depth is chosen from: the records under review, as one text, and how many there are. */
  readonly artifact: ArtifactSource & { readonly text: string }
  readonly adrs: number
  readonly files: readonly UnfrozenFile[]
}

/**
 * Reads what an ADR review freezes, as documents only: the records under
 * review go to `artifact/` at their repository paths, and the glossary and
 * every other record go to `references/`. Every record is read whatever its
 * status, since a superseded record is part of the log under review. No
 * tracker is read: a review stage belongs to no tracker item.
 *
 * This is the documents-only packet a review needs to be prepared at all;
 * the repository packet (#221) adds the sampled tree and facts.
 */
export class AdrPacketReader {
  constructor(private readonly files: CritiqueFilesPort) {}

  async read(root: string, request: AdrPacketRequest): Promise<AdrPacket | Refusal> {
    const records = (await this.files.listFiles(join(root, ADR_DIRECTORY))).filter((name) => name.endsWith('.md')).map((name) => join(ADR_DIRECTORY, name))
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
      text: await this.files.readText(join(root, source)),
    })
    const artifact = await Promise.all(reviewed.map((source) => read('artifact', source)))
    return {
      adrs: artifact.length,
      artifact: { labels: [], reference: ADR_ARTIFACT, text: artifact.map((file) => file.text).join('\n\n'), tracker: 'none' },
      files: [...artifact, ...(await Promise.all(references.map((source) => read('reference', source))))],
      status: 'read',
    }
  }

  /** The first of `GLOSSARY.md` and `CONTEXT.md` the repository has, as decision docs pick it. */
  private async glossary(root: string): Promise<readonly string[]> {
    const present = await this.files.listFiles(root)
    const found = GLOSSARIES.find((name) => present.includes(name))
    return found === undefined ? [] : [found]
  }
}
