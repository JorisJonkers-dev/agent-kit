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
  /** Whether the repository may send code to the code stages. */
  readonly code: boolean
  readonly allowlist_path: string
}

/**
 * Records which model families this machine lets a repository send content
 * to, and whether that content may be code. `code` left out keeps the
 * entry's current permission, which is off for a new entry.
 */
export class AllowCommand implements CritiqueCommand<AllowResult | Refusal> {
  private readonly repositories: RepositoryGate

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
  }

  async run(raw: unknown): Promise<AllowResult | Refusal> {
    const input = readInput(raw, 'allow', (fields) => ({
      code: fields.has('code') ? fields.boolean('code') : undefined,
      families: fields.oneOfList('families', MODEL_FAMILIES),
      repo: fields.string('repo'),
    }))
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const path = allowlistPath(this.deps.env)
    const allowlist = await this.repositories.allowlist()
    const code = input.code ?? allowlist.repositories[repository.key]?.code ?? false
    const next = withEntry(allowlist, repository.key, { code, families: input.families }, this.deps.clock.now())
    await this.deps.files.writeText(path, `${JSON.stringify(next, null, 2)}\n`)
    return { allowlist_path: path, code, families: input.families, repository: repository.key, status: 'allowed' }
  }
}
