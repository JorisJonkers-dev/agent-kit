import { isAbsolute, relative, resolve } from 'node:path'

import { CritiqueError } from '../domain/outcomes.js'

/** The repository-relative form of `path`; a path outside the repository is an error. */
export function insideRepository(root: string, path: string): string {
  const rel = relative(root, isAbsolute(path) ? path : resolve(root, path))
  if (rel.startsWith('..') || isAbsolute(rel)) throw new CritiqueError('outside-repository', `${path} is outside the repository`)
  return rel
}
