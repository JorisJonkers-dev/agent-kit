import { EMPTY_ALLOWLIST, identifyRepository, parseAllowlist, type FamilyAllowlist } from '../domain/family-allowlist.js'
import type { Refusal } from '../domain/outcomes.js'
import { allowlistPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

export interface ResolvedRepository {
  readonly status: 'resolved'
  readonly root: string
  readonly commonDir: string
  readonly key: string
}

export async function resolveRepository(deps: CritiqueDeps, cwd: string): Promise<ResolvedRepository | Refusal> {
  const decision = identifyRepository(await deps.git.remotes(cwd))
  if (decision.status === 'refused') return decision
  return {
    commonDir: await deps.git.commonDir(cwd),
    key: decision.key,
    root: await deps.git.root(cwd),
    status: 'resolved',
  }
}

export async function readAllowlist(deps: CritiqueDeps): Promise<FamilyAllowlist> {
  const path = allowlistPath(deps.env)
  if (!(await deps.files.exists(path))) return EMPTY_ALLOWLIST
  return parseAllowlist(await deps.files.readText(path))
}
