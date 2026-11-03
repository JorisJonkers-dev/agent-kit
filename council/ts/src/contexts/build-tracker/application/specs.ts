import { basename, dirname, resolve } from 'node:path'

import { localSpecTitle, localTickets } from '../domain/local-tickets.js'
import type { IssueTrackerSnapshot, SpecTicket } from '../domain/seed.js'
import {
  issueReferenceText,
  localRepositoryName,
  parseSpecArgument,
  sameRepository,
  ticketFiles,
  type RepositoryName,
  type ResolvedSpec,
  type TicketFiles,
} from '../domain/spec.js'
import { BuildTrackerError } from '../domain/state.js'
import { firstLine } from '../domain/text.js'
import type { BuildTrackerFilesPort, BuildTrackerIssuesPort, BuildTrackerRepositoryPort, IssueRecord, RepositoryLocation } from './ports.js'

export interface SpecSourceDependencies {
  readonly files: BuildTrackerFilesPort
  readonly issues: BuildTrackerIssuesPort
  readonly repositories: BuildTrackerRepositoryPort
  /** The directory a relative `--spec` path and a bare issue number are read against. */
  readonly cwd: string
}

/** A resolved spec and how to read its tickets from its issue tracker. */
export interface SpecSource {
  readonly spec: ResolvedSpec
  /** Where the page links a local spec's ticket files; undefined for an issue spec, whose tickets link to their issues. */
  readonly files: TicketFiles | undefined
  snapshot(): Promise<IssueTrackerSnapshot>
}

/** Finds the spec a `--spec` argument names: a GitHub issue, or a local markdown file in a git repository. */
export async function resolveSpec(argument: string, deps: SpecSourceDependencies): Promise<SpecSource> {
  const parsed = parseSpecArgument(argument)
  if (parsed.kind === 'issue') {
    const repository = parsed.repository ?? (await currentRepository(argument, deps))
    const spec: ResolvedSpec = { kind: 'github', number: parsed.number, repository }
    return { files: undefined, snapshot: () => readIssueSpec(repository, parsed.number, deps.issues), spec }
  }
  return resolveLocalSpec(parsed.path, deps)
}

/** The repository a local spec in this git repository belongs to, and where its ticket files are shown. */
export function localSpecPlace(location: RepositoryLocation): { readonly repository: RepositoryName; readonly files: TicketFiles } {
  return { files: ticketFiles(location.root, location.remote), repository: location.remote ?? localRepositoryName(location.mainRoot) }
}

async function currentRepository(argument: string, deps: SpecSourceDependencies): Promise<RepositoryName> {
  const location = await deps.repositories.locate(deps.cwd)
  if (location?.remote === undefined) {
    throw new BuildTrackerError(
      'invalid-spec',
      `${argument} names an issue of the current repository, but ${deps.cwd} is in no repository with a remote; name it as owner/repo#<number>`,
    )
  }
  return location.remote
}

/** The spec issue's title and its sub-issues, each with its native blocked-by links. */
async function readIssueSpec(repository: RepositoryName, number: number, issues: BuildTrackerIssuesPort): Promise<IssueTrackerSnapshot> {
  const reference = issueReferenceText(repository, number)
  const unreachable = (error: unknown): BuildTrackerError =>
    new BuildTrackerError(
      'tracker-unreachable',
      `cannot read the tickets of ${reference}: ${firstLine(error instanceof Error ? error.message : String(error)) ?? 'the tracker failed'}`,
    )
  const [specIssue, subs] = await Promise.all([issues.issue(repository, number), issues.subIssues(repository, number)]).catch((error: unknown) => {
    throw unreachable(error)
  })
  const outside = subs.find((sub) => !sameRepository(sub.repository, repository))
  if (outside !== undefined) {
    throw new BuildTrackerError(
      'unsupported-ticket',
      `${issueReferenceText(outside.repository, outside.number)} is a sub-issue of ${reference} in another repository; the build tracker keys tickets by issue number in the spec's repository`,
    )
  }
  const blockersOfEach = await Promise.all(subs.map((sub) => issues.blockedBy(sub.repository, sub.number))).catch((error: unknown) => {
    throw unreachable(error)
  })
  const numbers = new Set(subs.map((sub) => sub.number))
  const inside = (blocker: IssueRecord): boolean => sameRepository(blocker.repository, repository) && numbers.has(blocker.number)
  const tickets = subs.map((sub, index): SpecTicket => {
    const blockers = blockersOfEach[index] ?? []
    return {
      blockedBy: blockers.filter(inside).map((blocker) => blocker.number),
      closed: sub.state === 'closed',
      external: blockers
        .filter((blocker) => !inside(blocker))
        .map((blocker) => ({ ref: issueReferenceText(blocker.repository, blocker.number), state: blocker.state })),
      number: sub.number,
      title: sub.title,
    }
  })
  return { tickets, title: specIssue.title }
}

async function resolveLocalSpec(path: string, deps: SpecSourceDependencies): Promise<SpecSource> {
  const file = resolve(deps.cwd, path)
  let text: string
  try {
    text = await deps.files.readText(file)
  } catch {
    throw new BuildTrackerError(
      'invalid-spec',
      `${path} is neither an issue reference (#<number>, owner/repo#<number> or an issue URL) nor a readable spec file`,
    )
  }
  const location = await deps.repositories.locate(dirname(file))
  if (location === undefined) throw new BuildTrackerError('invalid-spec', `${path} is in no git repository`)
  const relativePath = `${location.prefix}${basename(file)}`
  const place = localSpecPlace(location)
  return {
    files: place.files,
    snapshot: async () => ({
      tickets: await readLocalTickets(dirname(file), location.prefix, relativePath, deps.files),
      title: localSpecTitle(text, relativePath),
    }),
    spec: { kind: 'local', path: relativePath, repository: place.repository },
  }
}

/** The `.md` files in the `issues/` folder beside the spec, as `/to-tickets` writes them. */
async function readLocalTickets(specDir: string, prefix: string, specPath: string, files: BuildTrackerFilesPort): Promise<readonly SpecTicket[]> {
  const folder = resolve(specDir, 'issues')
  let names: readonly string[]
  try {
    names = await files.listFiles(folder)
  } catch {
    throw new BuildTrackerError('invalid-ticket-file', `there is no issues/ folder beside ${specPath}`)
  }
  const texts = await Promise.all(
    names
      .filter((name) => name.toLowerCase().endsWith('.md'))
      .sort()
      .map(async (name) => ({ file: `${prefix}issues/${name}`, text: await files.readText(resolve(folder, name)) })),
  )
  return localTickets(texts)
}
