import { join } from 'node:path'

import { ADR_DIRECTORY, DOMAIN_DOC, GLOSSARIES } from '../domain/decision-docs.js'
import { DECISION_ARTIFACTS, decisionSummary, FEATURE_PATH, featureStatement, SUMMARY_NAME } from '../domain/decisions.js'
import type { ArtifactSource } from '../domain/manifest.js'
import { CritiqueError, refusal, type Refusal } from '../domain/outcomes.js'
import type { UnfrozenFile } from './code-packet.js'
import { DecisionDocs } from './decision-docs.js'
import { insideRepository } from './paths.js'
import type { CritiqueFilesPort } from './ports.js'

export interface DecisionsPacketRequest {
  /** The orchestrator's short statement of the feature being grilled. */
  readonly feature: string | undefined
  /** The orchestrator's summary of the decisions, after `/grill-me`; undefined after `/grill-with-docs`. */
  readonly summary: string | undefined
  /** The decision docs the caller names; undefined means the repository's own. */
  readonly references: readonly string[] | undefined
}

export interface DecisionsPacket {
  readonly status: 'read'
  /** What depth is chosen from: the decisions under review, as one text. */
  readonly artifact: ArtifactSource & { readonly text: string }
  readonly files: readonly UnfrozenFile[]
  readonly code: undefined
  /** A decisions packet has no code change for the risk rules to read. */
  readonly change: undefined
}

/**
 * Reads what the decisions stage freezes. After `/grill-with-docs` the
 * decision docs are the artifact, under `artifact/` at their repository
 * paths. After `/grill-me` the orchestrator's summary is the artifact, and
 * the decision docs the repository already has are reference material. The
 * feature statement goes to `feature/`, so critics know which decisions bear
 * on the feature. No tracker is read: grilling comes before any spec.
 */
export class DecisionsPacketReader {
  private readonly decisionDocs: DecisionDocs

  constructor(private readonly files: CritiqueFilesPort) {
    this.decisionDocs = new DecisionDocs(files)
  }

  async read(root: string, request: DecisionsPacketRequest): Promise<DecisionsPacket | Refusal> {
    if (request.feature === undefined) {
      throw new CritiqueError('invalid-input', 'decisions needs feature: a short statement of the feature the decisions are for')
    }
    const feature: UnfrozenFile = { kind: 'feature', path: FEATURE_PATH, source: 'feature-statement', text: featureStatement(request.feature) }
    const docs =
      request.references === undefined
        ? await this.decisionDocs.resolve(root)
        : request.references.map((path) => insideRepository(root, path))
    const read = async (folder: 'artifact' | 'reference', source: string): Promise<UnfrozenFile> => ({
      kind: folder,
      path: join(folder === 'artifact' ? 'artifact' : 'references', source),
      source,
      text: await this.files.readText(join(root, source)),
    })
    if (request.summary !== undefined) {
      const summary: UnfrozenFile = {
        kind: 'artifact',
        path: join('artifact', SUMMARY_NAME),
        source: DECISION_ARTIFACTS.summary,
        text: decisionSummary(request.summary),
      }
      const references = await Promise.all(docs.map((source) => read('reference', source)))
      return packet(DECISION_ARTIFACTS.summary, [feature, summary, ...references])
    }
    if (docs.length === 0) {
      return refusal(
        'no-decision-docs',
        `found no decision docs (${DOMAIN_DOC}, ${GLOSSARIES.join(', ')} or ${ADR_DIRECTORY}/); after /grill-me, pass the decisions as decision_summary`,
      )
    }
    return packet(DECISION_ARTIFACTS.docs, [feature, ...(await Promise.all(docs.map((source) => read('artifact', source))))])
  }
}

function packet(reference: string, files: readonly UnfrozenFile[]): DecisionsPacket {
  const text = files
    .filter((file) => file.kind === 'artifact')
    .map((file) => file.text)
    .join('\n\n')
  return { artifact: { labels: [], reference, text, tracker: 'none' }, change: undefined, code: undefined, files, status: 'read' }
}
