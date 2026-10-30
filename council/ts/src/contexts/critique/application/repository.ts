import {
  EMPTY_ALLOWLIST,
  identifyRepository,
  parseAllowlist,
  type FamilyAllowlist,
} from '../domain/data-policy.js'
import { allowlistPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

export interface Refusal {
  readonly status: 'refused'
  readonly reason: 'no-remote' | 'nedap' | 'not-opted-in' | 'no-critic-allowed'
  readonly message: string
}

export interface ResolvedRepository {
  readonly root: string
  readonly commonDir: string
  readonly key: string
}

export async function resolveRepository(
  deps: CritiqueDeps,
  cwd: string,
): Promise<ResolvedRepository | Refusal> {
  const decision = identifyRepository(await deps.git.remotes(cwd))
  if (decision.kind === 'refused') {
    return { message: decision.message, reason: decision.reason, status: 'refused' }
  }
  return {
    commonDir: await deps.git.commonDir(cwd),
    key: decision.key,
    root: await deps.git.root(cwd),
  }
}

export async function readAllowlist(deps: CritiqueDeps): Promise<FamilyAllowlist> {
  const path = allowlistPath(deps.env)
  if (!(await deps.files.exists(path))) return EMPTY_ALLOWLIST
  return parseAllowlist(await deps.files.readText(path))
}

export function isRefusal(value: object): value is Refusal {
  return 'status' in value && value.status === 'refused'
}
