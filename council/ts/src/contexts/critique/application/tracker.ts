import { basename, dirname, join } from 'node:path'

import { localTicketOpen } from '../domain/feature-review.js'
import { CritiqueError, refusal, type Refusal } from '../domain/outcomes.js'
import {
  defaultTracker,
  parseIssueReference,
  renderIssue,
  TRACKER_DOC,
  trackerFromDoc,
  type IssueReference,
  type TrackerKind,
} from '../domain/tracker.js'
import { localTickets, type Ticket } from '../domain/tickets.js'
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

/** A ticket as its tracker holds it, read once so a packet can freeze it. */
export interface TrackedTicket extends Ticket {
  /** Its file name under a packet folder. */
  readonly name: string
  /** False once the ticket is closed: a closed issue, or a local ticket whose status says it is done. */
  readonly open: boolean
}

export interface TrackedTickets {
  readonly status: 'read'
  /** In tracker order: GitHub's sub-issue order, or by file name. */
  readonly tickets: readonly TrackedTicket[]
}

/**
 * One repository's issue tracker. Prepare reads the artifact under critique
 * through it, and the ticket checker reads a spec's tickets through it, so
 * every caller finds tickets the way it finds specs.
 */
export interface Tracker {
  readonly status: 'resolved'
  readonly kind: TrackerKind
  /** The artifact a reference names, or the refusal the tracker gives when it cannot be read. */
  artifact(request: ArtifactRequest): Promise<TrackedArtifact | Refusal>
  /** The tickets of the spec a reference names, or the refusal the tracker gives when they cannot be read. */
  tickets(spec: string): Promise<TrackedTickets | Refusal>
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
    const issue = this.specIssue(request.reference)
    const reference = this.reference(issue)
    try {
      const read = await this.deps.gh.viewIssue({ ...this.locator(issue), comments: request.comments })
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

  /** The spec issue's sub-issues, each with its native blocked-by links. */
  async tickets(spec: string): Promise<TrackedTickets | Refusal> {
    const issue = this.specIssue(spec)
    try {
      const issues = await this.deps.gh.subIssues(this.locator(issue))
      const tickets = await Promise.all(
        issues.map(async (read): Promise<TrackedTicket> => {
          const ticket = this.issueAt(read.url, read.number, issue.repository)
          const blockers = await this.deps.gh.blockedBy(this.locator(ticket))
          return {
            blockedBy: blockers.map((blocker) => this.id(this.issueAt(blocker.url, blocker.number, ticket.repository))),
            id: this.id(ticket),
            name: `issue-${String(ticket.number)}.md`,
            open: read.state !== 'closed',
            reference: this.reference(ticket),
            text: renderIssue({ ...read, comments: [] }),
            title: read.title,
          }
        }),
      )
      return { status: 'read', tickets }
    } catch (error) {
      return refusal('tracker-unreachable', `cannot read the tickets of ${this.reference(issue)} through gh: ${firstLine(error)}`)
    }
  }

  /** The issue a spec reference names, which must be one of this repository's. */
  private specIssue(reference: string): IssueReference {
    const issue = parseIssueReference(reference, this.repository.key)
    if (issue === undefined) {
      throw new CritiqueError(
        'invalid-input',
        `${reference} is not an issue reference; this repository tracks specs as GitHub issues, so name one as #<number> or by its URL`,
      )
    }
    if (issue.repository !== this.repository.key) {
      throw new CritiqueError('outside-repository', `${reference} is an issue of ${issue.repository}, not of ${this.repository.key}`)
    }
    return issue
  }

  /** The issue a URL from gh names; one with no readable URL is taken to be in `repository`. */
  private issueAt(url: string, number: number, repository: string): IssueReference {
    return parseIssueReference(url, this.repository.key) ?? { number, repository }
  }

  private locator(issue: IssueReference): { readonly cwd: string; readonly number: number; readonly repo: string } {
    return { cwd: this.repository.root, number: issue.number, repo: issue.repository }
  }

  private reference(issue: IssueReference): string {
    return `${issue.repository}#${String(issue.number)}`
  }

  /** `#<number>` for an issue of this repository, its full reference for any other. */
  private id(issue: IssueReference): string {
    return issue.repository === this.repository.key ? `#${String(issue.number)}` : this.reference(issue)
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

  /** The `.md` files in the `issues/` folder beside the spec, as Pocock's `/to-tickets` writes them. */
  async tickets(spec: string): Promise<TrackedTickets> {
    const folder = join(dirname(insideRepository(this.repository.root, spec)), 'issues')
    const names = (await this.deps.files.listFiles(join(this.repository.root, folder))).filter((name) => name.endsWith('.md'))
    const files = await Promise.all(
      names.map(async (name) => ({
        reference: join(folder, name),
        text: await this.deps.files.readText(join(this.repository.root, folder, name)),
      })),
    )
    return { status: 'read', tickets: localTickets(files).map((ticket) => ({
        ...ticket,
        name: basename(ticket.reference),
        open: localTicketOpen(ticket.text),
      })) }
  }
}

function firstLine(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  return message.split('\n').find((line) => line.trim().length > 0)?.trim() ?? 'gh failed'
}
