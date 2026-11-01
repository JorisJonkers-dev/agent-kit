import type { JsonFields } from '../domain/json.js'
import { EMPTY_LEDGER, parseLedger, type Ledger } from '../domain/ledger.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import type { StatsThresholds } from '../domain/routing.js'
import {
  critiqueStats,
  renderStats,
  type CritiqueStats,
  type StatsFilter,
} from '../domain/stats.js'
import { readInput, readJsonFile } from './decode.js'
import { ledgerPath } from './locations.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'
import { RoutingSource } from './routing.js'

export interface StatsResult extends CritiqueStats {
  readonly status: 'reported'
  readonly ledger_path: string
  readonly filter: StatsFilter
  /** Read from routing, which stats never writes. */
  readonly thresholds: StatsThresholds
  /** The figures, removal candidates and suggestions as the skill shows them. */
  readonly report: string
}

interface StatsInput {
  /** A working directory whose repository the figures are limited to. */
  readonly repo: string | undefined
  readonly filter: StatsFilter
}

const DAY = /^\d{4}-\d{2}-\d{2}$/

/**
 * Reads the user's ledger, and the thresholds and routed clients from
 * routing (the shipped file with this machine's override), and reports how each critic client does per stage. It
 * writes nothing: the suggestions are sentences for the owner, and routing
 * changes only when the owner edits it. A missing ledger is an empty one.
 */
export class StatsCommand implements CritiqueCommand<StatsResult | Refusal> {
  private readonly repositories: RepositoryGate
  private readonly routing: RoutingSource

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.routing = new RoutingSource(deps)
  }

  async run(raw: unknown): Promise<StatsResult | Refusal> {
    const input = readInput(raw, 'stats', readStatsInput)
    let filter = input.filter
    if (input.repo !== undefined) {
      const repository = await this.repositories.resolve(input.repo)
      if (repository.status === 'refused') return repository
      filter = { ...filter, repository: repository.key }
    }
    const routing = await this.routing.load()
    const thresholds = routing.stats
    if (thresholds === undefined)
      throw new CritiqueError('invalid-routing', 'routing has no stats thresholds')
    const path = ledgerPath(this.deps.env)
    const ledger = await this.readLedger(path)
    const stats = critiqueStats(
      ledger,
      filter,
      thresholds,
      (stage) => routing.stages[stage]?.base.map((slot) => slot.client) ?? [],
    )
    return {
      ...stats,
      filter,
      ledger_path: path,
      report: renderStats(stats, { filter, ledgerPath: path, thresholds }),
      status: 'reported',
      thresholds,
    }
  }

  private async readLedger(path: string): Promise<Ledger> {
    if (!(await this.deps.files.exists(path))) return EMPTY_LEDGER
    return readJsonFile(this.deps.files, path, 'invalid-ledger', parseLedger)
  }
}

function readStatsInput(fields: JsonFields): StatsInput {
  if (fields.has('repo') && fields.has('repository')) {
    throw new CritiqueError(
      'invalid-input',
      'stats input: name the repository by repo or by repository, not both',
    )
  }
  const since = fields.optionalMatching('since', isDay, 'a date, YYYY-MM-DD')
  const until = fields.optionalMatching('until', isDay, 'a date, YYYY-MM-DD')
  if (since !== undefined && until !== undefined && since > until) {
    throw new CritiqueError('invalid-input', `stats input: since ${since} is after until ${until}`)
  }
  const repository = fields.optionalString('repository')
  return {
    filter: {
      ...(repository === undefined ? {} : { repository }),
      ...(since === undefined ? {} : { since }),
      ...(until === undefined ? {} : { until }),
    },
    repo: fields.optionalString('repo'),
  }
}

function isDay(value: string): value is string {
  if (!DAY.test(value)) return false
  const day = new Date(`${value}T00:00:00Z`)
  return !Number.isNaN(day.getTime()) && day.toISOString().startsWith(value)
}
