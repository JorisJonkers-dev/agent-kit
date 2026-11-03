import type { RepositoryName } from '../domain/spec.js'

/** The files the build tracker reads and writes. */
export interface BuildTrackerFilesPort {
  /** Rejects with the platform's error (its `code` is `ENOENT` for a missing file). */
  readText(path: string): Promise<string>
  /** Replaces the file whole: a temporary file beside it, then a rename. */
  writeAtomic(path: string, content: string): Promise<void>
  /** The names of the files in a directory; rejects with the platform's error when it cannot be listed. */
  listFiles(dir: string): Promise<readonly string[]>
}

export interface BuildTrackerClockPort {
  now(): Date
}

/** An issue as the issue tracker reports it. */
export interface IssueRecord {
  readonly repository: RepositoryName
  readonly number: number
  readonly title: string
  readonly state: 'open' | 'closed'
}

/** The issue tracker (GitHub), read only. Any failure rejects with an error that says what went wrong. */
export interface BuildTrackerIssuesPort {
  issue(repository: RepositoryName, number: number): Promise<IssueRecord>
  /** The issue's sub-issues, in the tracker's order. */
  subIssues(repository: RepositoryName, number: number): Promise<readonly IssueRecord[]>
  /** The issues the issue's native blocked-by links name. */
  blockedBy(repository: RepositoryName, number: number): Promise<readonly IssueRecord[]>
}

/** Where a directory sits in its git repository. */
export interface RepositoryLocation {
  /** The working tree's root. */
  readonly root: string
  /** The directory's path under the root, ending in `/`, or empty at the root. */
  readonly prefix: string
  /** The root of the main working tree, the same for every worktree of the repository. */
  readonly mainRoot: string
  /** Where `origin` (or the first remote) points, when the repository has a remote. */
  readonly remote: RepositoryName | undefined
}

export interface BuildTrackerRepositoryPort {
  /** Undefined when the directory is in no git repository. */
  locate(dir: string): Promise<RepositoryLocation | undefined>
}
