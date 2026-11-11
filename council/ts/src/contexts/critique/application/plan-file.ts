import { isAbsolute, join, relative, resolve } from 'node:path'

import { CritiqueError } from '../domain/outcomes.js'
import type { TrackerKind } from '../domain/tracker.js'
import type { ResolvedRepository } from './repository.js'

/** The plan's file name: the same in every place a plan lives. */
export const PLAN_FILE = 'refactoring-plan.md'

/**
 * Where a draft's refactoring plan lives, by the repository's tracker
 * convention (ADR-0005). A repository that tracks specs as local markdown
 * keeps it under `.scratch/<draft>/`, beside the specs. Any other, a GitHub
 * repository, keeps it beside the flow state in the git common directory,
 * which no commit holds, so nothing under the tracked tree is ever written.
 */
export function planFilePath(repository: Pick<ResolvedRepository, 'root' | 'commonDir'>, tracker: TrackerKind, draft: string): string {
  const path = planFilePathOrNull(repository, tracker, draft)
  if (path === null) throw new CritiqueError('invalid-input', `the draft name ${draft} has an empty or dot segment, so it names no folder`)
  return path
}

/** `planFilePath`, or null for a draft name that names no folder. */
export function planFilePathOrNull(repository: Pick<ResolvedRepository, 'root' | 'commonDir'>, tracker: TrackerKind, draft: string): string | null {
  if (draft.split('/').some((part) => part === '..' || part === '.' || part === '')) return null
  return tracker === 'local-markdown'
    ? join(repository.root, '.scratch', draft, PLAN_FILE)
    : join(repository.commonDir, 'critique', 'plans', draft.replaceAll('/', '~'), PLAN_FILE)
}

/**
 * A plan path the caller names, made absolute against the repository's root.
 * It must lie in the working tree or in the git common directory, the only
 * two places a plan lives, so a run never freezes a file from elsewhere.
 */
export function namedPlanPath(repository: Pick<ResolvedRepository, 'root' | 'commonDir'>, path: string): string {
  const absolute = isAbsolute(path) ? path : resolve(repository.root, path)
  const inside = (base: string): boolean => {
    const rel = relative(base, absolute)
    return rel !== '..' && !rel.startsWith('../') && !isAbsolute(rel)
  }
  if (!inside(repository.root) && !inside(repository.commonDir)) {
    throw new CritiqueError('outside-repository', `${path} is outside the repository and its git directory, where a plan lives`)
  }
  return absolute
}

/** The draft a flow key names: `draft:<name>`. Undefined for any other key. */
export function draftOf(flow: string | undefined): string | undefined {
  return flow?.startsWith('draft:') === true ? flow.slice('draft:'.length) : undefined
}
