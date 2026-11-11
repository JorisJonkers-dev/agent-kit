import { join } from 'node:path'

import { rewriteCheckStatus, rewriteFailures, type RewriteFailure, type RewriteCheckStatus } from '../domain/rewrite-check.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import { readInput } from './decode.js'
import { insideRepository } from './paths.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'

export interface CheckRewriteResult {
  /** `failed` while any rewrite check failed; the failures say which and how. */
  readonly status: RewriteCheckStatus
  readonly failures: readonly RewriteFailure[]
}

/**
 * The rewrite check (#228), run after `/critique-adrs` applies its editorial
 * edits and before anything is recorded. It takes the decision sentences the
 * orchestrator quoted from the frozen packet, the rewritten files as they now
 * sit in the working tree, and the decision sentences the orchestrator labels
 * in each rewritten file; for a split it also takes the record that was split
 * and the files created from it. It confirms each original sentence appears
 * byte-identical in exactly one rewritten file, that no rewritten file labels
 * a decision sentence that is not an original one, and that the split marks
 * the original record superseded by the split files. It reads the working
 * tree and writes nothing, so it needs no family allowlist entry; a
 * repository whose remotes are refused is still refused.
 */
export class CheckRewriteCommand implements CritiqueCommand<CheckRewriteResult | Refusal> {
  private readonly repositories: RepositoryGate

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
  }

  async run(raw: unknown): Promise<CheckRewriteResult | Refusal> {
    const input = readInput(raw, 'check-rewrite', (fields) => {
      const split = fields.has('split') ? fields.object('split') : undefined
      return {
        originalSentences: fields.strings('original_sentences'),
        rewritten: fields.list('rewritten', (item) => ({ decisionSentences: item.strings('decision_sentences'), path: item.string('path') })),
        repo: fields.string('repo'),
        split: split === undefined ? undefined : { files: split.strings('files'), original: split.string('original') },
      }
    })
    const sentences = [...input.originalSentences, ...input.rewritten.flatMap((file) => file.decisionSentences)]
    if (sentences.some((sentence) => sentence === '')) throw new CritiqueError('invalid-input', 'check-rewrite input: a decision sentence is empty')
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const rewritten = await Promise.all(
      input.rewritten.map(async (file) => ({
        decision_sentences: file.decisionSentences,
        path: file.path,
        text: await this.read(repository.root, file.path),
      })),
    )
    const split =
      input.split === undefined
        ? undefined
        : { files: input.split.files, original: input.split.original, text: await this.read(repository.root, input.split.original) }
    const failures = rewriteFailures({ original_sentences: input.originalSentences, rewritten, split })
    return { failures, status: rewriteCheckStatus(failures) }
  }

  /** A rewritten file as it sits in the working tree; one missing there is input the command cannot check. */
  private async read(root: string, path: string): Promise<string> {
    const full = join(root, insideRepository(root, path))
    if (!(await this.deps.files.exists(full))) throw new CritiqueError('invalid-input', `check-rewrite input: ${path} does not exist in the repository`)
    return this.deps.files.readText(full)
  }
}
