import { join } from 'node:path'

import { driftAndStale, namedBy, type CodeFacts, type NamedCode } from '../domain/adr-drift.js'
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
export async function checkLog(deps: Pick<CritiqueDeps, 'files' | 'git'>, root: string, paths: readonly string[]): Promise<CheckAdrsResult> {
  const { files } = deps
  const sources = await Promise.all(paths.map(async (path) => ({ path, text: await files.readText(join(root, path)) })))
  const linked = linkedPaths(sources)
  const found = await Promise.all(linked.map(async (path) => ((await files.exists(join(root, path))) ? [path] : [])))
  const structure = checkAdrLog(sources, new Set(found.flat()))
  const inForce = new Set(structure.records.filter((record) => record.in_force && record.number !== null).map((record) => record.path))
  const checked = namedBy(sources.filter((source) => inForce.has(source.path)))
  const facts = await codeFacts(deps, root, checked)
  const { drifted, stale } = facts === undefined ? { drifted: [], stale: [] } : driftAndStale(checked, facts)
  return { status: adrCheckStatus(structure), ...structure, drifted, stale }
}

/** How many tracked files are read at once. */
const READ_CONCURRENCY = 32

/** A tracked file a symbol is searched in: not the decision log itself, and not vendored dependencies. */
const searched = (path: string): boolean => !path.startsWith(`${ADR_DIRECTORY}/`) && !path.split('/').includes('node_modules')

/**
 * What the repository holds, for the records that name code: the paths in
 * the working tree, the symbols in the tracked and untracked files outside the log, and
 * the last change of each existing path. Undefined when the repository has
 * no commit yet, since there is then no tree to judge a record against.
 */
async function codeFacts(
  { files, git }: Pick<CritiqueDeps, 'files' | 'git'>,
  root: string,
  records: readonly NamedCode[],
): Promise<CodeFacts | undefined> {
  const head = await git.resolveCommit(root, 'HEAD').catch(() => undefined)
  if (head === undefined) return undefined
  const names = [...new Set(records.flatMap((record) => record.paths))]
  const existing = new Set((await Promise.all(names.map(async (path) => ((await files.exists(join(root, path))) ? [path] : [])))).flat())
  const changes = new Map<string, { readonly commit: string; readonly date: string }>()
  for (const path of existing) {
    const change = await git.lastChange(root, path)
    if (change !== null) changes.set(path, change)
  }
  const wanted = new Set(records.flatMap((record) => record.symbols))
  const present = new Set<string>()
  let incomplete = false
  if (wanted.size > 0) {
    const tracked = [...(await git.treeFiles(root, head)), ...(await git.untrackedPaths(root))].filter(searched)
    const queue = [...tracked]
    const worker = async (): Promise<void> => {
      for (let path = queue.shift(); path !== undefined; path = queue.shift()) {
        try {
          const text = await files.readText(join(root, path))
          for (const word of text.match(/[A-Za-z_]\w*/g) ?? []) if (wanted.has(word)) present.add(word)
        } catch (error) {
          // A tracked file deleted from the working tree holds no symbol; any other failure leaves the search incomplete.
          if ((error as NodeJS.ErrnoException).code !== 'ENOENT') incomplete = true
        }
      }
    }
    await Promise.all(Array.from({ length: Math.min(READ_CONCURRENCY, tracked.length) }, worker))
  }
  return { lastChange: (path) => changes.get(path), pathExists: (path) => existing.has(path), symbolExists: (symbol) => incomplete || present.has(symbol) }
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
    return checkLog(this.deps, repository.root, paths)
  }
}
