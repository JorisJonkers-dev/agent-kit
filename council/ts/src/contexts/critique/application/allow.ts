import { withEntry } from '../domain/family-allowlist.js'
import { MODEL_FAMILIES, type ModelFamily } from '../domain/family.js'
import type { Refusal } from '../domain/outcomes.js'
import { readInput } from './decode.js'
import { allowlistPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'

export interface AllowResult {
  readonly status: 'allowed'
  readonly repository: string
  readonly families: readonly ModelFamily[]
  readonly allowlist_path: string
}

/** Records which model families this machine lets a repository send content to. */
export class AllowCommand implements CritiqueCommand<AllowResult | Refusal> {
  private readonly repositories: RepositoryGate

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
  }

  async run(raw: unknown): Promise<AllowResult | Refusal> {
    const input = readInput(raw, 'allow', (fields) => ({
      families: fields.oneOfList('families', MODEL_FAMILIES),
      repo: fields.string('repo'),
    }))
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const path = allowlistPath(this.deps.env)
    const next = withEntry(await this.repositories.allowlist(), repository.key, input.families, this.deps.clock.now())
    await this.deps.files.writeText(path, `${JSON.stringify(next, null, 2)}\n`)
    return { allowlist_path: path, families: input.families, repository: repository.key, status: 'allowed' }
  }
}
