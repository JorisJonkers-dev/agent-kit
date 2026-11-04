import { join } from 'node:path'

import { parseManifest } from '../domain/manifest.js'
import type { Refusal } from '../domain/outcomes.js'
import { reviewRuns, type ReviewRun, type ReviewRunSource } from '../domain/review-runs.js'
import { readInput, readJsonFile } from './decode.js'
import { LedgerSource } from './ledger-source.js'
import { manifestPath, reportPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate, type ResolvedRepository } from './repository.js'

export interface ReviewsResult {
  readonly status: 'read'
  /** The repository's review runs, newest first. */
  readonly runs: readonly ReviewRun[]
}

/** How many runs `/flow` lists unless it asks for another number. */
const DEFAULT_LIMIT = 10

/**
 * The review entry of `/flow`: a repository's recent review runs, each a
 * flow of its own, with its stage, run id, verdict counts and spend. It
 * reads the run directories and the ledgers and writes nothing; it never
 * says a review is due, since a review runs only when someone asks for one.
 * It sends nothing anywhere, so it needs no family allowlist entry, but a
 * repository whose remotes refuse a critique is refused here too.
 */
export class ReviewsCommand implements CritiqueCommand<ReviewsResult | Refusal> {
  private readonly repositories: RepositoryGate
  private readonly source: LedgerSource

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.source = new LedgerSource(deps)
  }

  async run(raw: unknown): Promise<ReviewsResult | Refusal> {
    const input = readInput(raw, 'reviews', (fields) => ({
      limit: fields.has('limit') ? fields.positiveInteger('limit') : DEFAULT_LIMIT,
      repo: fields.string('repo'),
    }))
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const { invocations, ledger, recorded_runs } = await this.source.read()
    const ours = <T extends { readonly repository: string }>(lines: readonly T[]) => lines.filter((line) => line.repository === repository.key)
    const runs = reviewRuns(
      {
        directories: await this.directories(repository),
        findings: ours(ledger.findings),
        invocations: ours(invocations),
        ledgerRuns: ours(recorded_runs),
      },
      input.limit,
    )
    return { runs, status: 'read' }
  }

  /** The runs the repository's run directories hold, recorded or not. */
  private async directories(repository: ResolvedRepository): Promise<readonly ReviewRunSource[]> {
    const critiqueDir = join(repository.commonDir, 'critique')
    const runs: ReviewRunSource[] = []
    for (const name of await this.deps.files.listDirectories(critiqueDir)) {
      const runDir = join(critiqueDir, name)
      if (!(await this.deps.files.exists(manifestPath(runDir)))) continue
      const manifest = await readJsonFile(this.deps.files, manifestPath(runDir), 'invalid-run', parseManifest)
      runs.push({ recorded: await this.deps.files.exists(reportPath(runDir)), round: manifest.round, run_id: manifest.run_id, stage: manifest.stage })
    }
    return runs
  }
}
