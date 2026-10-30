import { withEntry } from '../domain/family-allowlist.js'
import { MODEL_FAMILIES, type ModelFamily } from '../domain/family.js'
import type { Refusal } from '../domain/outcomes.js'
import { readInput } from './decode.js'
import { allowlistPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'
import { readAllowlist, resolveRepository } from './repository.js'

export interface AllowResult {
  readonly status: 'allowed'
  readonly repository: string
  readonly families: readonly ModelFamily[]
  readonly allowlist_path: string
}

/** Records which model families this machine lets a repository send content to. */
export async function allow(deps: CritiqueDeps, raw: unknown): Promise<AllowResult | Refusal> {
  const input = readInput(raw, 'allow', (fields) => ({
    families: fields.oneOfList('families', MODEL_FAMILIES),
    repo: fields.string('repo'),
  }))
  const repository = await resolveRepository(deps, input.repo)
  if (repository.status === 'refused') return repository
  const path = allowlistPath(deps.env)
  const next = withEntry(await readAllowlist(deps), repository.key, input.families, deps.clock.now())
  await deps.files.writeText(path, `${JSON.stringify(next, null, 2)}\n`)
  return { allowlist_path: path, families: input.families, repository: repository.key, status: 'allowed' }
}
