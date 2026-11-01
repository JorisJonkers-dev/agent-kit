import { basename, join } from 'node:path'

import { CritiqueError, refusal, type Refusal } from '../domain/outcomes.js'
import {
  defaultTracker,
  parseIssueReference,
  renderIssue,
  TRACKER_DOC,
  trackerFromDoc,
  type TrackerKind,
} from '../domain/tracker.js'
import { insideRepository } from './paths.js'
import type { CritiqueDeps } from './ports.js'
import type { ResolvedRepository } from './repository.js'

/** An artifact as its tracker holds it, read once so the packet can freeze it. */
export interface TrackedArtifact {
  readonly status: 'read'
  readonly tracker: TrackerKind
  /** Where the artifact lives: `host/owner/repo#<number>` for an issue, its repository path for a file. */
  readonly reference: string
  /** Its file name under the packet's `artifact/`. */
  readonly name: string
  readonly text: string
  /** The issue's labels; a markdown file has none. */
  readonly labels: readonly string[]
}

export interface ArtifactRequest {
  /** What the caller named: `#167`, an issue URL, or a repository path. */
  readonly reference: string
  /** Whether an issue's comments are frozen with its body. */
  readonly comments: boolean
}

/**
 * One repository's issue tracker. Prepare reads the artifact under critique
 * through it. A later reader of a spec's tickets (#184) belongs here too, as
 * a second method beside `artifact`, so every caller finds tickets the way it
 * finds specs.
 */
export interface Tracker {
  readonly status: 'resolved'
  readonly kind: TrackerKind
  /** The artifact a reference names, or the refusal the tracker gives when it cannot be read. */
  artifact(request: ArtifactRequest): Promise<TrackedArtifact | Refusal>
}

/** Finds a repository's tracker from its issue-tracker doc, or from its remote when there is none. */
export class TrackerResolver {
  constructor(private readonly deps: Pick<CritiqueDeps, 'files' | 'gh'>) {}

  async resolve(repository: ResolvedRepository): Promise<Tracker | Refusal> {
    const doc = join(repository.root, TRACKER_DOC)
    const choice = (await this.deps.files.exists(doc))
      ? trackerFromDoc(await this.deps.files.readText(doc))
      : ({ kind: defaultTracker(repository.key), status: 'known' } as const)
    if (choice.status === 'unsupported') {
      return refusal(
        'unsupported-tracker',
        `${TRACKER_DOC} names ${choice.name}; the critique reads specs from GitHub issues or local markdown only`,
      )
    }
    return choice.kind === 'github'
      ? new GithubTracker(this.deps, repository)
      : new LocalMarkdownTracker(this.deps, repository)
  }
}

class GithubTracker implements Tracker {
  readonly status = 'resolved'
  readonly kind = 'github'

  constructor(
    private readonly deps: Pick<CritiqueDeps, 'gh'>,
    private readonly repository: ResolvedRepository,
  ) {}

  async artifact(request: ArtifactRequest): Promise<TrackedArtifact | Refusal> {
    const issue = parseIssueReference(request.reference, this.repository.key)
    if (issue === undefined) {
      throw new CritiqueError(
        'invalid-input',
        `${request.reference} is not an issue reference; this repository tracks specs as GitHub issues, so name one as #<number> or by its URL`,
      )
    }
    if (issue.repository !== this.repository.key) {
      throw new CritiqueError('outside-repository', `${request.reference} is an issue of ${issue.repository}, not of ${this.repository.key}`)
    }
    const reference = `${issue.repository}#${String(issue.number)}`
    try {
      const read = await this.deps.gh.viewIssue({
        comments: request.comments,
        cwd: this.repository.root,
        number: issue.number,
        repo: issue.repository,
      })
      return {
        labels: read.labels,
        name: `issue-${String(issue.number)}.md`,
        reference,
        status: 'read',
        text: renderIssue(read),
        tracker: this.kind,
      }
    } catch (error) {
      return refusal('tracker-unreachable', `cannot read issue ${reference} through gh: ${firstLine(error)}`)
    }
  }
}

class LocalMarkdownTracker implements Tracker {
  readonly status = 'resolved'
  readonly kind = 'local-markdown'

  constructor(
    private readonly deps: Pick<CritiqueDeps, 'files'>,
    private readonly repository: ResolvedRepository,
  ) {}

  async artifact(request: ArtifactRequest): Promise<TrackedArtifact> {
    const path = insideRepository(this.repository.root, request.reference)
    const absolute = join(this.repository.root, path)
    if (!(await this.deps.files.exists(absolute))) {
      throw new CritiqueError(
        'invalid-input',
        `${request.reference} is not a file in the repository; this repository tracks specs as local markdown, so name one by its path`,
      )
    }
    return {
      labels: [],
      name: basename(path),
      reference: path,
      status: 'read',
      text: await this.deps.files.readText(absolute),
      tracker: this.kind,
    }
  }
}

function firstLine(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  return message.split('\n').find((line) => line.trim().length > 0)?.trim() ?? 'gh failed'
}
