import { parseModelFamilies, withEntry } from '../domain/family-allowlist.js'
import type { ModelFamily } from '../domain/family.js'
import { asObject, requireString, requireStringList } from './input.js'
import { allowlistPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'
import type { Refusal } from '../domain/outcomes.js'
import { readAllowlist, resolveRepository } from './repository.js'

export interface AllowResult {
  readonly status: 'allowed'
  readonly repository: string
  readonly families: readonly ModelFamily[]
  readonly allowlist_path: string
}

/** Records which model families this machine lets a repository send content to. */
export async function allow(deps: CritiqueDeps, raw: unknown): Promise<AllowResult | Refusal> {
  const input = asObject(raw, 'allow input')
  const families = parseModelFamilies(requireStringList(input, 'families'))
  const repository = await resolveRepository(deps, requireString(input, 'repo'))
  if (repository.status === 'refused') return repository
  const path = allowlistPath(deps.env)
  const next = withEntry(await readAllowlist(deps), repository.key, families, deps.clock.now())
  await deps.files.writeText(path, `${JSON.stringify(next, null, 2)}\n`)
  return { allowlist_path: path, families, repository: repository.key, status: 'allowed' }
}
