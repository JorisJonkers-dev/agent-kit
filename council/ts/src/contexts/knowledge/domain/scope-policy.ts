export const CURATED_DEFAULT_KNOWLEDGE_SCOPE = 'project:personal-stack'

export interface ResolveKnowledgeScopeInput {
  readonly requestedScope?: string | undefined
  readonly gitRemote?: string | undefined
  readonly allowAll?: boolean | undefined
  readonly curatedDefaultScope?: string | undefined
}

export function canonicalProjectScopeFromGitRemote(remote: string): string | undefined {
  const path = githubRemotePath(remote.trim())
  if (path === undefined) {
    return undefined
  }

  const [owner, repo, extra] = path.split('/')
  if (owner === undefined || repo === undefined || extra !== undefined) {
    return undefined
  }

  const normalizedRepo = stripGitSuffix(repo)
  if (owner.length === 0 || normalizedRepo.length === 0) {
    return undefined
  }

  return `project:${owner.toLowerCase()}/${normalizedRepo.toLowerCase()}`
}

export function resolveKnowledgeScope(input: ResolveKnowledgeScopeInput): string | undefined {
  const scope =
    trimToUndefined(input.requestedScope) ??
    (input.gitRemote === undefined ? undefined : canonicalProjectScopeFromGitRemote(input.gitRemote))

  if (scope === 'all' && input.allowAll !== true) {
    throw new Error('knowledge scope "all" requires explicit allowAll.')
  }

  if (scope === (input.curatedDefaultScope ?? CURATED_DEFAULT_KNOWLEDGE_SCOPE)) {
    return undefined
  }

  return scope
}

function githubRemotePath(remote: string): string | undefined {
  if (remote.length === 0) {
    return undefined
  }
  if (remote.startsWith('git@github.com:')) {
    return remote.slice('git@github.com:'.length)
  }
  if (remote.startsWith('https://github.com/')) {
    return remote.slice('https://github.com/'.length)
  }
  if (remote.startsWith('ssh://git@github.com/')) {
    return remote.slice('ssh://git@github.com/'.length)
  }
  return undefined
}

function stripGitSuffix(repo: string): string {
  return repo.endsWith('.git') ? repo.slice(0, -'.git'.length) : repo
}

function trimToUndefined(value: string | undefined): string | undefined {
  if (value === undefined) {
    return undefined
  }

  const trimmed = value.trim()
  return trimmed.length === 0 ? undefined : trimmed
}
