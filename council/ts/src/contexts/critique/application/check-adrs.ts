import { join } from 'node:path'

import { adrCheckStatus, checkAdrLog, linkedPaths, type AdrCheck, type AdrCheckStatus } from '../domain/adr-log.js'
import { ADR_DIRECTORY } from '../domain/decision-docs.js'
import { refusal, type Refusal } from '../domain/outcomes.js'
import { readInput } from './decode.js'
import type { CritiqueCommand, CritiqueDeps, CritiqueFilesPort } from './ports.js'
import { RepositoryGate } from './repository.js'

export interface CheckAdrsResult extends AdrCheck {
  /** `failed` while the log holds a structural error; unnumbered files alone never fail it. */
  readonly status: AdrCheckStatus
}

/** Every record of the log at `docs/adr/`, as the ADR review's packet lists them. */
export async function logRecords(files: CritiqueFilesPort, root: string): Promise<readonly string[]> {
  return (await files.listFiles(join(root, ADR_DIRECTORY))).filter((name) => name.endsWith('.md')).map((name) => join(ADR_DIRECTORY, name))
}

/**
 * Reads the decision log's records, as `logRecords` lists them, and checks its structure. Both the
 * check-adrs command and the adr-review stage's prepare check it this way,
 * so the packet's report is what check-adrs prints.
 */
export async function checkLog(files: CritiqueFilesPort, root: string, paths: readonly string[]): Promise<CheckAdrsResult> {
  const sources = await Promise.all(paths.map(async (path) => ({ path, text: await files.readText(join(root, path)) })))
  const linked = linkedPaths(sources)
  const found = await Promise.all(linked.map(async (path) => ((await files.exists(join(root, path))) ? [path] : [])))
  const check = checkAdrLog(sources, new Set(found.flat()))
  return { status: adrCheckStatus(check), ...check }
}

/**
 * The deterministic decision-log checker: reads every record in
 * `docs/adr/` and reports what a script can judge. It sends nothing to any
 * critic and writes nothing, so it needs no family allowlist entry; a
 * repository whose remotes are refused is still refused.
 */
export class CheckAdrsCommand implements CritiqueCommand<CheckAdrsResult | Refusal> {
  private readonly repositories: RepositoryGate

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
  }

  async run(raw: unknown): Promise<CheckAdrsResult | Refusal> {
    const input = readInput(raw, 'check-adrs', (fields) => ({ repo: fields.string('repo') }))
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const paths = await logRecords(this.deps.files, repository.root)
    if (paths.length === 0) return refusal('no-adrs', `found no decision records in ${ADR_DIRECTORY}/ to review`)
    return checkLog(this.deps.files, repository.root, paths)
  }
}
