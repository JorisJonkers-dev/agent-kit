import { basename, join } from 'node:path'

import { answeredFile, parseColdReadReply, unavailableFile, type ColdReadFile, type ColdReadResult } from '../domain/cold-read.js'
import { READ_ONLY_TOOLSET, toolsSection } from '../domain/critic-tools.js'
import { parseManifest, specReference, type RunManifest } from '../domain/manifest.js'
import { CritiqueError } from '../domain/outcomes.js'
import type { ColdReadRouting } from '../domain/routing.js'
import { FlowBudget } from './budget.js'
import { readInput, readJsonFile } from './decode.js'
import { manifestPath } from './locations.js'
import { insideRepository } from './paths.js'
import { clientReadiness, type CritiqueCommand, type CritiqueDeps } from './ports.js'
import { RoutingSource } from './routing.js'

/** The role the cold reader runs, under the shipped assets. */
const COLD_READER_ROLE = 'roles/cold-reader.md'

/** `cold-read.json`, which the command writes beside the run and returns. */
export function coldReadPath(runDir: string): string {
  return join(runDir, 'cold-read.json')
}

/**
 * The cold read (#476), run after `/critique-adrs` rewrote records and
 * check-rewrite confirmed the decisions survived. It takes the run directory
 * and the rewritten records, and for each one builds a one-file packet under
 * the run directory holding only that record: no glossary, no other records,
 * no tree, no spec. The `cold-reader` role runs on it through the client
 * routing's `cold_read` section names, capped at its price and time, and each
 * launch is logged like any critic's, so its spend counts in the run's flow.
 * The reader answers what was decided, what was rejected and why, and when to
 * revisit; the command does not judge the answers, it only reports them, and
 * an empty answer is an omission. A client that may not run, fails, or gives
 * no readable answer leaves its record `unavailable` with a reason, never a
 * pass. A run is cold-read once.
 */
export class ColdReadCommand implements CritiqueCommand<ColdReadResult> {
  private readonly budget: FlowBudget
  private readonly routing: RoutingSource

  constructor(private readonly deps: CritiqueDeps) {
    this.budget = new FlowBudget(deps)
    this.routing = new RoutingSource(deps)
  }

  async run(raw: unknown): Promise<ColdReadResult> {
    const input = readInput(raw, 'cold-read', (fields) => ({ files: fields.strings('files'), runDir: fields.string('run_dir') }))
    if (input.files.length === 0) throw new CritiqueError('invalid-input', 'cold-read input: files names no record')
    const manifest = await readJsonFile(this.deps.files, manifestPath(input.runDir), 'invalid-run', parseManifest)
    const path = coldReadPath(manifest.run_dir)
    if (await this.deps.files.exists(path)) {
      throw new CritiqueError('already-cold-read', `run ${manifest.run_id} was already cold-read`)
    }
    const coldRead = (await this.routing.load()).cold_read
    if (coldRead === undefined) throw new CritiqueError('invalid-routing', 'routing has no cold_read, so a rewrite cannot be cold-read')
    const blocked = await this.blocked(manifest, coldRead)
    // One launch at a time, in the order the caller named the records, so the ledger reads in that order.
    // Every record is read before the first launch, so a bad path costs nothing and a retry re-charges nothing.
    const texts = await Promise.all(input.files.map((file) => this.workingTreeText(manifest, file)))
    const files: ColdReadFile[] = []
    const ledgerWarnings: string[] = []
    for (const [index, file] of input.files.entries()) {
      files.push(await this.read(manifest, coldRead, file, texts[index] ?? '', index + 1, blocked, ledgerWarnings))
    }
    const result: ColdReadResult = {
      files,
      ...(ledgerWarnings.length === 0 ? {} : { ledger_warnings: ledgerWarnings }),
      run_id: manifest.run_id,
      status: 'checked',
    }
    await this.deps.files.writeText(path, `${JSON.stringify(result, null, 2)}\n`)
    return result
  }

  /**
   * Cold-reads one record: its own one-file packet, the reader's prompt beside
   * it, and one launch, logged whether or not its answer is readable.
   */
  private async read(
    manifest: RunManifest,
    coldRead: ColdReadRouting,
    path: string,
    text: string,
    n: number,
    blocked: string | undefined,
    ledgerWarnings: string[],
  ): Promise<ColdReadFile> {
    if (blocked !== undefined) return unavailableFile(path, blocked)
    const dir = join(manifest.run_dir, 'cold-read', String(n))
    const packetDir = join(dir, 'packet')
    const packetPath = join('artifact', basename(path))
    await this.deps.files.writeText(join(packetDir, packetPath), text)
    const promptPath = join(dir, 'prompt.md')
    await this.deps.files.writeText(promptPath, await this.prompt(packetPath))
    const started = this.deps.clock.monotonicMs()
    let report: Awaited<ReturnType<CritiqueDeps['clients']['vibe']['ask']>>
    try {
      report = await this.deps.clients.vibe.ask(
        { max_price_usd: coldRead.max_price_usd, outputDir: dir, promptPath, time_limit_s: coldRead.time_limit_s, workDir: packetDir },
        coldRead.client_config,
      )
    } catch (error) {
      // A launch that throws must not lose the records already read, or a retry would charge them again.
      return unavailableFile(path, `the launch failed: ${error instanceof Error ? error.message : String(error)}`)
    }
    const durationMs = this.deps.clock.monotonicMs() - started
    // A launch with no readable answer is a failed launch in the ledger, as for any critic.
    const answers = report.answer.status === 'answered' ? parseColdReadReply(report.answer.text) : undefined
    await this.deps.files.writeText(join(dir, 'stdout.json'), report.stdout)
    await this.deps.files.writeText(join(dir, 'stderr.txt'), report.stderr)
    const ledgerWarning = await this.budget.logInvocation(
      manifest,
      {
        charged_usd: report.chargedUsd,
        client: coldRead.client,
        cost_usd: report.answer.cost_usd,
        critic_id: `cold-read-${String(n)}`,
        duration_ms: durationMs,
        exit_code: report.exitCode,
        family: coldRead.client_config.family,
        model: coldRead.client_config.model,
        role: 'cold-reader',
        status: answers === undefined ? 'failed' : 'ok',
        vibe_cost_usd: report.vibeCostUsd,
      },
      coldRead.client_config,
      specReference(manifest),
    )
    if (ledgerWarning !== undefined) ledgerWarnings.push(ledgerWarning)
    if (report.answer.status === 'failed') return unavailableFile(path, `${report.answer.failure}: ${report.answer.detail}`)
    return answers === undefined ? unavailableFile(path, 'the cold reader gave no readable answer') : answeredFile(path, answers)
  }

  /**
   * Why the cold reader may not run now, or undefined while it may: the
   * repository's allowlist does not permit its family, Vibe is at its plan
   * limit, or the client is not ready on this machine. Read once, before the
   * records, so a client that cannot run costs nothing.
   */
  private async blocked(manifest: RunManifest, coldRead: ColdReadRouting): Promise<string | undefined> {
    const config = coldRead.client_config
    if (!manifest.allowed_families.includes(config.family)) return `the allowlist does not permit the ${config.family} family`
    const plan = (await this.deps.plan.read('vibe')).unable_to_start
    if (plan !== undefined) return plan
    const ready = await clientReadiness(this.deps.clients, config)
    return ready.ready ? undefined : ready.detail
  }

  /** The rewritten record as it sits in the working tree; one missing there is input the command cannot read. */
  private async workingTreeText(manifest: RunManifest, path: string): Promise<string> {
    const full = join(manifest.repo_root, insideRepository(manifest.repo_root, path))
    if (!(await this.deps.files.exists(full))) throw new CritiqueError('invalid-input', `cold-read input: ${path} does not exist in the repository`)
    return this.deps.files.readText(full)
  }

  /** The reader's prompt: its role, the one tool it has, and the one file it reads. */
  private async prompt(packetPath: string): Promise<string> {
    const role = await this.deps.files.readText(join(this.deps.assetsRoot, COLD_READER_ROLE))
    const files = [
      '# Files',
      '',
      'Your working directory holds one file and no others. Read it with `read_file`, using the path exactly as written here:',
      '',
      `- \`${packetPath}\`: the decision record you answer about`,
    ].join('\n')
    return `${[role.trim(), toolsSection(READ_ONLY_TOOLSET), files].join('\n\n')}\n`
  }
}
