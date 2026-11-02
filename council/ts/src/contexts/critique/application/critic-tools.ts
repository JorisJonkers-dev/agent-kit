import { join } from 'node:path'

import { READ_ONLY_TOOLSET, type CriticToolset } from '../domain/critic-tools.js'
import { parseSetupResults } from '../domain/setup.js'
import { setupResultsPath } from './locations.js'
import { REACH_PROOF, reachProblems, reachProofCalls, type SearchLimits } from '../domain/packet-search.js'
import type { RosterCritic } from '../domain/routing.js'
import type { CritiqueDeps } from './ports.js'

/** The tools one critic gets, and why it reads alone when it could have searched. */
export interface CriticToolChoice {
  readonly toolset: CriticToolset
  readonly searchFailure?: string
}

/**
 * Proves that the runner's search server cannot see past a packet, in
 * `dir`: a packet with one file, a file outside it and two symlinks out of
 * it, then a session that tries every way out. Undefined when proven; else
 * why not.
 */
export async function proveSearchReach(
  deps: Pick<CritiqueDeps, 'files' | 'search'>,
  dir: string,
  limits: SearchLimits,
): Promise<string | undefined> {
  if (deps.search === undefined) return 'this runner has no search server'
  const packet = join(dir, 'packet')
  await deps.files.writeText(join(packet, REACH_PROOF.inside.path), REACH_PROOF.inside.text)
  await deps.files.writeText(join(dir, REACH_PROOF.outside.path), REACH_PROOF.outside.text)
  for (const link of REACH_PROOF.symlinks) await deps.files.symlink(link.target, join(packet, link.path))
  const session = await deps.search.session(packet, limits, reachProofCalls(join(dir, REACH_PROOF.outside.dir)))
  if (session.status === 'failed') return session.detail
  const problems = reachProblems(session.answers)
  return problems.length === 0 ? undefined : problems.join('; ')
}

/**
 * Chooses each critic's tools for one command (ADR-0006). An agy critic
 * uses agy's own read-only tools. A Vibe critic searches through the
 * runner's search server only when its installed agent file lets it, the
 * server proved, once per command, that it cannot see past a packet, and
 * the server then started on the critic's own copy; otherwise it reads
 * alone, and the run says why. All of this is settled before the critic is
 * launched, so no critic spends on a search it cannot have.
 */
export class CriticTools {
  private proof: Promise<string | undefined> | undefined

  constructor(
    private readonly deps: Pick<CritiqueDeps, 'clients' | 'env' | 'files' | 'search'>,
    private readonly limits: SearchLimits,
    /** Where the reach proof writes its packet, outside every critic's copy. */
    private readonly proofDir: string,
  ) {}

  /**
   * agy's search tools are its own, so only agy's read-only proof can show
   * they stay inside the packet, and that proof is #178's live check. Until
   * setup records it passing for this client, an agy critic reads alone.
   */
  private async agy(client: string): Promise<CriticToolChoice> {
    const path = setupResultsPath(this.deps.env)
    const results = (await this.deps.files.exists(path)) ? parseSetupResults(await this.deps.files.readText(path)) : undefined
    if (results?.checks[`read-only:${client}`]?.result === 'pass') return { toolset: { kind: 'agy', search: true } }
    return {
      searchFailure: `agy-search-unproven: ${client}'s read-only proof, which must show its search cannot reach past the packet, waits for agent-kit#178`,
      toolset: { kind: 'agy', search: false },
    }
  }

  async choose(critic: RosterCritic, packetDir: string): Promise<CriticToolChoice> {
    if (critic.client_config.kind === 'agy') return this.agy(critic.client)
    const search = this.deps.search
    if (search === undefined) return { toolset: READ_ONLY_TOOLSET }
    const blocked = await this.deps.clients.vibe.searchBlocked(critic.client_config)
    if (blocked !== undefined) return { searchFailure: blocked, toolset: READ_ONLY_TOOLSET }
    // A proof that throws (a symlink the file system refuses, say) is a proof
    // that failed, and is not kept: the next critic tries it again.
    this.proof ??= proveSearchReach(this.deps, this.proofDir, this.limits).catch((error: unknown) => {
      this.proof = undefined
      return error instanceof Error ? error.message : String(error)
    })
    const unproven = await this.proof
    if (unproven !== undefined) return { searchFailure: `search is not proven to stay inside the packet: ${unproven}`, toolset: READ_ONLY_TOOLSET }
    const session = await search.session(packetDir, this.limits, [])
    if (session.status === 'failed') return { searchFailure: session.detail, toolset: READ_ONLY_TOOLSET }
    return { toolset: { kind: 'vibe-search', limits: this.limits, search: session.tools, server: search.server(packetDir, this.limits) } }
  }
}
