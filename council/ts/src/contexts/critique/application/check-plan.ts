import { join, resolve } from 'node:path'

import { checkAdrLog } from '../domain/adr-log.js'
import { CritiqueError, type Refusal } from '../domain/outcomes.js'
import { checkPlan, planCheckStatus, type PlanAdrs, type PlanCheck, type PlanCheckStatus } from '../domain/refactoring-plan.js'
import { stageRouting } from '../domain/routing.js'
import { readInput } from './decode.js'
import { logRecords } from './check-adrs.js'
import type { CritiqueCommand, CritiqueDeps } from './ports.js'
import { RepositoryGate } from './repository.js'
import { RoutingSource } from './routing.js'

export interface CheckPlanResult extends PlanCheck {
  /** `failed` while the plan breaks any rule of the format. */
  readonly status: PlanCheckStatus
}

/**
 * The deterministic refactoring-plan checker: reads `refactoring-plan.md`
 * and reports what a script can judge. With a repository it also reads the
 * decision log, to find the ADRs a plan anchors to absent or superseded. It
 * sends nothing to any critic and writes nothing; a repository whose remotes
 * are refused is still refused.
 */
export class CheckPlanCommand implements CritiqueCommand<CheckPlanResult | Refusal> {
  private readonly repositories: RepositoryGate
  private readonly routing: RoutingSource

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.routing = new RoutingSource(deps)
  }

  async run(raw: unknown): Promise<CheckPlanResult | Refusal> {
    const input = readInput(raw, 'check-plan', (fields) => ({
      plan: fields.string('plan'),
      repo: fields.optionalString('repo'),
    }))
    const repository = input.repo === undefined ? undefined : await this.repositories.resolve(input.repo)
    if (repository?.status === 'refused') return repository
    const path = repository === undefined ? input.plan : resolve(repository.root, input.plan)
    if (!(await this.deps.files.exists(path))) throw new CritiqueError('invalid-input', `check-plan input: ${path} does not exist`)
    return this.checkFile(path, repository?.root)
  }

  /** The check of one plan file that exists, with the decision log of `root` when a repository is given. */
  async checkFile(path: string, root: string | undefined): Promise<CheckPlanResult> {
    const text = await this.deps.files.readText(path)
    const size = stageRouting(await this.routing.load(), { stage: 'tickets' }).ticket_size
    const check = checkPlan(text, size, root === undefined ? undefined : await this.adrs(root))
    return { status: planCheckStatus(check), ...check }
  }

  /** The numbers the decision log holds, and those of its records that are superseded. */
  private async adrs(root: string): Promise<PlanAdrs> {
    const sources = await Promise.all(
      (await logRecords(this.deps.files, root)).map(async (path) => ({ path, text: await this.deps.files.readText(join(root, path)) })),
    )
    const log = checkAdrLog(sources, new Set())
    const records = log.records.flatMap((record) => (record.number === null ? [] : [{ ...record, number: record.number }]))
    const superseded = new Set(log.superseded)
    return {
      present: new Set(records.map((record) => record.number)),
      superseded: new Set(records.filter((record) => superseded.has(record.path)).map((record) => record.number)),
    }
  }
}
