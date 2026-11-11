import { resolve } from 'node:path'

import { BuildTrackerError } from '../domain/state.js'
import { parseTicketsFile, stateFromTickets } from '../domain/tickets-file.js'

import type { Counts } from '../domain/board.js'
import { parseIssueReference, repositoryName, stateBase, stateLocation, type RepositoryName, type TicketFiles } from '../domain/spec.js'
import type { BuildTrackerState } from '../domain/state.js'
import type { BuildTrackerCommand, BuildTrackerDependencies, IssueRecord } from './ports.js'
import { localSpecPlace } from './specs.js'
import { loadState, writePage } from './state-file.js'

/** Names the page's source: a state file, or a ticket file with GitHub's state of its issues. */
export type RenderSource =
  | { readonly state: string; readonly tickets?: undefined; readonly repo?: undefined }
  | { readonly state?: undefined; readonly tickets: string; readonly repo?: string | undefined }

export type RenderInput = RenderSource & { readonly out: string }

/** The issues read from GitHub at once when a ticket file is rendered. */
const ISSUE_READS_AT_ONCE = 8

export interface RenderResult {
  readonly status: 'rendered'
  readonly out: string
  readonly counts: Counts
}

/** Writes the build tracker page for a state file. */
export class RenderCommand implements BuildTrackerCommand<RenderInput, RenderResult> {
  constructor(private readonly deps: BuildTrackerDependencies) {}

  async run(input: RenderInput): Promise<RenderResult> {
    if (input.tickets !== undefined) return this.runTickets(input.tickets, input.repo, input.out)
    const { state } = await loadState(this.deps.files, input.state)
    const counts = await writePage(this.deps, input.out, state, await this.ticketFiles(input.state, state))
    return { counts, out: input.out, status: 'rendered' }
  }

  /**
   * One page for every ticket of a spec, grouped by branch, from its ticket
   * file and the state of each ticket's issue on GitHub. It reads and writes
   * no state file: the ticket file and the issue tracker are the source.
   */
  private async runTickets(path: string, repo: string | undefined, out: string): Promise<RenderResult> {
    const file = await this.readTickets(path)
    const repository = await this.repository(repo)
    const spec = await this.issue(repository, file.spec)
    const states = new Map<number, 'open' | 'closed'>()
    for (let start = 0; start < file.tickets.length; start += ISSUE_READS_AT_ONCE) {
      const batch = file.tickets.slice(start, start + ISSUE_READS_AT_ONCE)
      const issues = await Promise.all(batch.map((ticket) => this.issue(repository, ticket.issue)))
      issues.forEach((issue, index) => states.set(batch[index]?.issue ?? 0, issue.state))
    }
    const state = stateFromTickets(file, repository, spec.title, states)
    const counts = await writePage(this.deps, out, state, undefined)
    return { counts, out, status: 'rendered' }
  }

  private async readTickets(path: string): Promise<ReturnType<typeof parseTicketsFile>> {
    let text: string
    try {
      text = await this.deps.files.readText(resolve(this.deps.cwd, path))
    } catch (error) {
      throw new BuildTrackerError('invalid-tickets', `the ticket file at ${path} cannot be read: ${error instanceof Error ? error.message : String(error)}`)
    }
    return parseTicketsFile(text)
  }

  /** The repository the issues are in: `--repo owner/repo`, else the working directory's remote. */
  private async repository(repo: string | undefined): Promise<RepositoryName> {
    if (repo !== undefined) {
      const named = parseIssueReference(`${repo}#1`)
      if (named === undefined) throw new BuildTrackerError('invalid-spec', `--repo must be owner/repo, not ${repo}`)
      return repositoryName(named.repository.host, named.repository.owner, named.repository.repo)
    }
    const remote = (await this.deps.repositories.locate(this.deps.cwd))?.remote
    if (remote === undefined) throw new BuildTrackerError('invalid-spec', 'the working directory has no git remote; name the repository with --repo owner/repo')
    return remote
  }

  private async issue(repository: RepositoryName, number: number): Promise<IssueRecord> {
    try {
      return await this.deps.issues.issue(repository, number)
    } catch (error) {
      throw new BuildTrackerError('tracker-unreachable', `the issue tracker could not be read: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  /**
   * Where a local spec's ticket files are shown, read from the repository
   * the command runs in. Only the spec's own repository will do: its state
   * file is the one that repository's spec names, so a state rendered from
   * anywhere else gets no links rather than links into the wrong tree.
   */
  private async ticketFiles(statePath: string, state: BuildTrackerState): Promise<TicketFiles | undefined> {
    if (state.spec.kind !== 'local') return undefined
    const base = stateBase(this.deps.env)
    const location = await this.deps.repositories.locate(this.deps.cwd)
    if (base === undefined || location === undefined) return undefined
    const place = localSpecPlace(location)
    const own = stateLocation(base, { kind: 'local', path: state.spec.path, repository: place.repository }).state
    return own === resolve(this.deps.cwd, statePath) ? place.files : undefined
  }
}
