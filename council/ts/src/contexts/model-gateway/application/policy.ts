import { identifyRepository } from '../../../shared-kernel/index.js'
import { EMPTY_ALLOWLIST, parseAllowlist, type FamilyAllowlist } from '../domain/allowlist.js'
import { ModelGatewayError } from '../domain/config.js'
import { decidePolicy, type DataScope, type PolicyDecision } from '../domain/policy.js'
import { loadConfig } from './config.js'
import { fileExists, messageOf, readJsonFile } from './json-file.js'
import { allowlistPath, overridePath } from './locations.js'
import type { ModelGatewayDependencies, ModelGatewayFilesPort } from './ports.js'

/** What the scope reads: git for the repository, and the files and environment for its allowlist. */
export type ScopeDependencies = Pick<ModelGatewayDependencies, 'env' | 'files' | 'git'>

/**
 * The data policy for a delegation made from `repo`: per client model,
 * whether it is permitted and what may be sent. Every run and every batch
 * asks this before it hands anything to a client.
 */
export async function decidePolicyFor(deps: ModelGatewayDependencies, repo: string | undefined): Promise<PolicyDecision> {
  const config = await loadConfig(deps.files, deps.defaultsPath, overridePath(deps.env))
  return decidePolicy(config, await resolveScope(deps, repo))
}

/**
 * What a delegation from `repo` would carry: a repository's content, or only
 * what the caller declares. No `repo`, and a path in no repository, both mean
 * the caller declares its content itself; a path git cannot read is an error,
 * never a permissive scope.
 */
export async function resolveScope(deps: ScopeDependencies, repo: string | undefined): Promise<DataScope> {
  if (repo === undefined) return { kind: 'outside' }
  const root = await gitOrFail(repo, () => deps.git.root(repo))
  if (root === undefined) return { kind: 'outside' }
  const identity = identifyRepository(await gitOrFail(repo, () => deps.git.remotes(root)))
  if (identity.status === 'denied-owner') return { kind: 'nedap' }
  if (identity.status === 'no-remote') return { kind: 'no-remote' }
  const allowlist = await loadAllowlist(deps.files, allowlistPath(deps.env))
  return { entry: allowlist.repositories[identity.key], key: identity.key, kind: 'repository' }
}

/** A git that cannot read the path is an error naming the path and git's message. */
async function gitOrFail<T>(repo: string, call: () => Promise<T>): Promise<T> {
  try {
    return await call()
  } catch (error) {
    throw new ModelGatewayError('repository-unreadable', `${repo}: ${messageOf(error)}`)
  }
}

/** This machine's family allowlist, the one critique writes; a missing file is an empty one. */
export async function loadAllowlist(files: ModelGatewayFilesPort, path: string): Promise<FamilyAllowlist> {
  if (!(await fileExists(files, path, 'the family allowlist'))) return EMPTY_ALLOWLIST
  return readJsonFile(files, path, 'the family allowlist', parseAllowlist)
}
