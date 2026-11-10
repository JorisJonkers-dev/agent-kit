import { codeFacts, historyFacts, type CodeFacts, type FactsIndex, type HistoryFact } from '../domain/code-facts.js'
import { refusal, type Refusal } from '../domain/outcomes.js'
import { readInput } from './decode.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'

export interface FactsResult extends CodeFacts, FactsIndex {
  readonly status: 'reported'
  /** The commit every fact is read at. */
  readonly commit: string
  readonly history: readonly HistoryFact[]
}

/**
 * The deterministic history and import-graph facts (ADR-0004): for the named
 * files, the last commit that changed each and how many commits did; for the
 * named modules, what each imports and how the named set depends on itself.
 * Everything is read at HEAD's commit and its own committer dates, so the same
 * commit gives byte-identical facts. The facts hold no author and no commit
 * message. They count as code under the family allowlist, so the command only
 * prints them: it sends nothing to any critic and writes nothing.
 */
export class FactsCommand implements CritiqueCommand<FactsResult | Refusal> {
  private readonly repositories: RepositoryGate

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
  }

  async run(raw: unknown): Promise<FactsResult | Refusal> {
    const input = readInput(raw, 'facts', (fields) => ({
      files: fields.has('files') ? fields.strings('files') : [],
      modules: fields.has('modules') ? fields.strings('modules') : [],
      repo: fields.string('repo'),
    }))
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const { git } = this.deps
    const root = repository.root
    const commit = await git.resolveCommit(root, 'HEAD').catch(() => undefined)
    if (commit === undefined) return refusal('no-commit', 'the repository has no commit to read facts at')
    const files = [...new Set(input.files)].sort()
    const modules = [...new Set(input.modules)].sort()
    const histories = await Promise.all(files.map(async (path) => ({ commits: await git.fileCommits(root, commit, path), path })))
    const tree = new Set(await git.treeFiles(root, commit))
    const sources = await Promise.all(
      modules.map(async (path) => ({ path, text: tree.has(path) ? await git.fileAt(root, commit, path) : null })),
    )
    return { commit, history: historyFacts(histories), status: 'reported', ...codeFacts(sources, tree) }
  }
}
