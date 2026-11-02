import { join } from 'node:path'

import { budgetReached, quotaReached } from '../domain/budget.js'
import { declaredChecks } from '../domain/checks.js'
import { indexPacket, type PacketIndex } from '../domain/evidence.js'
import type { ModelFamily } from '../domain/family.js'
import type { RunFinding } from '../domain/findings.js'
import { listedFiles, parseManifest, specReference, type RunManifest } from '../domain/manifest.js'
import { CritiqueError } from '../domain/outcomes.js'
import { assemblePrompt, type CriticPrompt } from '../domain/prompt.js'
import {
  chooseRefuter,
  countOutcomes,
  familyBar,
  judgeReply,
  refuteLedgerLine,
  refuterCandidates,
  withFinding,
  type RefuteFile,
  type RefuteRecord,
  type RefuterCandidate,
} from '../domain/refute.js'
import { severityOverrideProblem } from '../domain/verdicts.js'
import { FlowBudget } from './budget.js'
import { FINDING_SCHEMA, promptFields, readFrozenPacket, runCritic } from './critic-run.js'
import { readInput, readJsonFile } from './decode.js'
import { parseFanOutResult, type FanOutResult } from './fan-out.js'
import { findingsPath, ledgerPath, manifestPath, refutePath, reportPath } from './locations.js'
import { clientReadiness, type CritiqueCommand, type CritiqueDeps } from './ports.js'
import { readVerdict, verdictsByFinding } from './record.js'
import { RoutingSource } from './routing.js'
import { ReadOnlyTripwire } from './tripwire.js'

export type RefuteResult = RefuteFile

/** What every refuter of one round shares: the run, its packet and tripwire, and the refuter role's prompt. */
interface Round {
  readonly manifest: RunManifest
  readonly merged: FanOutResult
  readonly tripwire: ReadOnlyTripwire
  readonly packet: PacketIndex
  readonly candidates: readonly RefuterCandidate[]
  /** Why each candidate client may not run now, by client name; absent while it may. */
  readonly blocked: ReadonlyMap<string, string>
  readonly prompt: CriticPrompt
  /** The check ids the refuter role declares: the answers that object. */
  readonly declared: readonly string[]
}

/**
 * The refute round (CONTEXT.md). Takes a run directory and the
 * orchestrator's verdicts, after fan-out and before record. Each accepted
 * critical finding (critical as the verdict's severity override has it, else
 * as fan-out fixed it) goes to the first routed client, base before deep, whose
 * family neither raised it nor is the orchestrator's, run with the refuter
 * role on the original packet plus the finding. Its answer counts only with
 * evidence the packet holds; otherwise the verdict stands. With no eligible
 * client the finding is skipped. Results go to `refute.json`, which record
 * applies, and one line each to the ledger; a refuter's launch is logged
 * like any critic's, so its spend counts in the run's flow cap. A run is
 * refuted once, and only before it is recorded.
 */
export class RefuteCommand implements CritiqueCommand<RefuteResult> {
  constructor(private readonly deps: CritiqueDeps) {}

  async run(raw: unknown): Promise<RefuteResult> {
    const input = readInput(raw, 'refute', (fields) => ({
      runDir: fields.string('run_dir'),
      verdicts: fields.list('verdicts', readVerdict),
    }))
    const manifest = await readJsonFile(this.deps.files, manifestPath(input.runDir), 'invalid-run', parseManifest)
    const merged = await readJsonFile(this.deps.files, findingsPath(input.runDir), 'invalid-run', parseFanOutResult)
    if (await this.deps.files.exists(reportPath(input.runDir))) {
      throw new CritiqueError('already-recorded', `run ${manifest.run_id} is already recorded; refute comes before record`)
    }
    const path = refutePath(input.runDir)
    if (await this.deps.files.exists(path)) throw new CritiqueError('already-refuted', `run ${manifest.run_id} already had its refute round`)
    const verdicts = verdictsByFinding(input.verdicts, new Set(merged.findings.map((finding) => finding.id)))
    const targets = merged.findings.flatMap((finding) => {
      const given = verdicts.get(finding.id)
      const problem = given === undefined ? undefined : severityOverrideProblem(finding, given)
      if (problem !== undefined) throw new CritiqueError('invalid-input', problem)
      const accepted = given?.verdict === 'accepted'
      if (accepted && finding.pre_verdict?.verdict === 'rejected') {
        throw new CritiqueError('invalid-input', `${finding.id} was rejected as fabricated-evidence, which a verdict cannot overturn`)
      }
      // The severity that counts is the orchestrator's override, else the one fan-out fixed.
      const severity = given?.severity ?? finding.severity
      return accepted && severity === 'critical' ? [{ ...finding, severity }] : []
    })
    const results = targets.length === 0 ? [] : await this.round(manifest, merged, targets)
    const file: RefuteFile = {
      counts: countOutcomes(results),
      refute_path: path,
      results,
      run_id: manifest.run_id,
      stage: manifest.stage,
      status: 'checked',
    }
    const recordedAt = this.deps.clock.now().toISOString()
    const context = {
      flow: manifest.flow,
      recorded_at: recordedAt,
      repository: manifest.repository,
      round: manifest.round,
      run_id: manifest.run_id,
      spec: specReference(manifest),
      stage: manifest.stage,
    }
    const lines = results.flatMap((result) => {
      const finding = targets.find((target) => target.id === result.finding_id)
      return finding === undefined ? [] : [refuteLedgerLine(result, finding, context)]
    })
    if (lines.length > 0) await this.deps.files.appendText(ledgerPath(this.deps.env), lines.map((line) => `${line}\n`).join(''))
    await this.deps.files.writeText(path, `${JSON.stringify(file, null, 2)}\n`)
    return file
  }

  /** Refutes every target at once, so the round takes as long as its slowest refuter. */
  private async round(manifest: RunManifest, merged: FanOutResult, targets: readonly RunFinding[]): Promise<readonly RefuteRecord[]> {
    const asset = (path: string) => this.deps.files.readText(join(this.deps.assetsRoot, path))
    const role = await asset('roles/refuter.md')
    const prompt = assemblePrompt(
      {
        contract: await asset('prompts/output-contract.md'),
        lenses: [],
        preamble: await asset('prompts/preamble.md'),
        role,
        schema: await asset(FINDING_SCHEMA),
      },
      listedFiles(manifest),
      manifest.stage,
      manifest.code?.slices,
    )
    const candidates = refuterCandidates(await new RoutingSource(this.deps).load(), manifest.stage)
    const round: Round = {
      blocked: await this.blocked(manifest, candidates),
      candidates,
      declared: declaredChecks([role]).map((check) => check.id),
      manifest,
      merged,
      packet: indexPacket(await readFrozenPacket(this.deps, manifest), manifest.ticket_check?.ticket_ids),
      prompt,
      tripwire: await ReadOnlyTripwire.load(this.deps, manifest),
    }
    return Promise.all(targets.map((finding) => this.refute(round, finding)))
  }

  private async refute(round: Round, finding: RunFinding): Promise<RefuteRecord> {
    const { manifest } = round
    const raisers: readonly ModelFamily[] = round.merged.critics.filter((c) => finding.raised_by.includes(c.critic_id)).map((c) => c.family)
    const rules = { allowedFamilies: manifest.allowed_families, orchestratorFamily: manifest.orchestrator_family }
    const choice = chooseRefuter(finding, round.candidates, {
      blocked: (client) => round.blocked.get(client),
      family: (family) => familyBar(family, rules, raisers),
    })
    if ('skipped' in choice) return { detail: choice.skipped, finding_id: finding.id, outcome: 'skipped', reason_code: 'no-eligible-client' }
    const { refuter } = choice
    const criticDir = join(manifest.run_dir, 'refute', finding.id)
    const prompt = withFinding(round.prompt, finding)
    await this.deps.files.writeText(promptFields(prompt, criticDir).prompt_path, prompt.text)
    const reply = await runCritic(this.deps, { critic: refuter, criticDir, prompt, tripwire: round.tripwire })
    await new FlowBudget(this.deps).logInvocation(manifest, reply.run, refuter.client_config.kind)
    const judged = judgeReply(finding, reply, { declared: round.declared, packet: round.packet, stage: manifest.stage })
    return { finding_id: finding.id, ...judged, refuter: reply.run }
  }

  /**
   * Why each candidate may not run now: not ready on this machine, an agy
   * client past today's quota, or a Vibe client of a flow that has spent its
   * cap. The spend and the count are read once, before the round.
   */
  private async blocked(manifest: RunManifest, candidates: readonly RefuterCandidate[]): Promise<ReadonlyMap<string, string>> {
    const budget = new FlowBudget(this.deps)
    const spent = budgetReached(await budget.vibeSpend(manifest), manifest.limits)
    const quota = quotaReached(await budget.agyInvocationsToday(), manifest.limits.antigravity_quota, manifest.stage)
    const eligible = candidates.filter(
      ({ client_config: config }) => manifest.allowed_families.includes(config.family) && config.family !== manifest.orchestrator_family,
    )
    const reasons = await Promise.all(
      eligible.map(async ({ client, client_config: config }) => {
        const limit = config.kind === 'vibe' ? spent : quota
        if (limit !== undefined) return [client, limit] as const
        const ready = await clientReadiness(this.deps.clients, config)
        return [client, ready.ready ? undefined : ready.detail] as const
      }),
    )
    return new Map(reasons.flatMap(([client, reason]) => (reason === undefined ? [] : [[client, reason] as const])))
  }
}
