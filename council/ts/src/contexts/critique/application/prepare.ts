import { join } from 'node:path'

import { quotaReached } from '../domain/budget.js'
import { criticIdentity, type CriticIdentity } from '../domain/critic.js'
import { chooseDepth, type DepthChoice } from '../domain/depth.js'
import { ORCHESTRATOR_FAMILIES, type OrchestratorFamily } from '../domain/family.js'
import {
  compactTimestamp,
  listedFiles,
  parseManifest,
  sha256,
  type ArtifactSource,
  type CodeSource,
  type PacketFile,
  type RunManifest,
} from '../domain/manifest.js'
import { CritiqueError, refusal, type Refusal } from '../domain/outcomes.js'
import {
  candidateClients,
  resolveRoster,
  stageRouting,
  type ClientAvailability,
  type DroppedCritic,
  type Routing,
  type RosterRequest,
} from '../domain/routing.js'
import { CRITIQUE_STAGES, DEPTHS, isCodeStage, type CritiqueStage, type Depth, type Round } from '../domain/stage.js'
import { FlowBudget } from './budget.js'
import { CodePacketReader, type UnfrozenFile } from './code-packet.js'
import { readInput, readJsonFile } from './decode.js'
import { DecisionDocs } from './decision-docs.js'
import { manifestPath, reportPath } from './locations.js'
import { insideRepository } from './paths.js'
import { clientReadiness, type CritiqueCommand, type CritiqueDeps } from './ports.js'
import { RepositoryGate, type ResolvedRepository } from './repository.js'
import { RoutingSource } from './routing.js'
import { TrackerResolver, type TrackedArtifact, type Tracker } from './tracker.js'

interface PrepareInput {
  readonly stage: CritiqueStage
  readonly orchestratorFamily: OrchestratorFamily
  readonly repo: string
  /** The artifact as its tracker names it: an issue reference for GitHub, a path for local markdown. */
  readonly spec: string
  /** Whether a GitHub issue's comments are frozen with its body. */
  readonly comments: boolean
  /** The reference paths the caller names; undefined means the repository's decision docs. */
  readonly references: readonly string[] | undefined
  /**
   * The flow whose Vibe spend this run adds to; undefined means the first
   * round's flow on a second round, and the artifact's own reference otherwise.
   */
  readonly flow: string | undefined
  /** The recorded first round, when this run is the second round of the same stage over the same artifact. */
  readonly previousRunDir: string | undefined
  /** The depth the caller names, which always wins over the signals; undefined lets the signals choose. */
  readonly depth: Depth | undefined
  /** A code stage's base: the commit or branch the change is diffed against. */
  readonly base: string | undefined
  /** A code stage's commit under review. */
  readonly head: string
  /** The ticket a ticket review delivers. */
  readonly ticket: string | undefined
}

/** What a code stage's prepare reports about the commits it froze. */
export interface PreparedCode {
  readonly base_commit: string
  readonly head_commit: string
  readonly changed_files: readonly string[]
  /** How many files of the tree at the head were frozen. */
  readonly tree_files: number
}

type RoundOf = Pick<RunManifest, 'round' | 'previous_run_id'> & {
  /** A second round's first-round flow, so both rounds count against one Vibe cap. */
  readonly previousFlow?: string
}

export interface PrepareResult {
  readonly status: 'ready'
  readonly run_id: string
  readonly run_dir: string
  readonly manifest_path: string
  readonly packet_dir: string
  /** The packet files the critics' prompts name; a code packet's tree beyond the changed files is not listed. */
  readonly packet_files: readonly string[]
  /** Only for a code stage. */
  readonly code?: PreparedCode
  readonly artifact: ArtifactSource
  readonly depth: Depth
  readonly round: Round
  readonly depth_choice: DepthChoice
  readonly critics: readonly CriticIdentity[]
  readonly dropped: readonly DroppedCritic[]
}

/**
 * Resolves the repository and its family allowlist, reads the artifact from
 * the repository's tracker, chooses the depth from the artifact (or takes
 * the caller's), then resolves the roster, and only then freezes
 * the artifact and its decision docs into a packet. A refused repository, an
 * unsupported tracker and an issue gh cannot read all return before anything
 * is written. A roster that drops every
 * critic still prepares a run: fan-out and record then report it degraded,
 * so the flow goes on.
 */
export class PrepareCommand implements CritiqueCommand<PrepareResult | Refusal> {
  private readonly repositories: RepositoryGate
  private readonly trackers: TrackerResolver
  private readonly decisionDocs: DecisionDocs
  private readonly codePackets: CodePacketReader
  private readonly routing: RoutingSource

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.trackers = new TrackerResolver(deps)
    this.decisionDocs = new DecisionDocs(deps.files)
    this.codePackets = new CodePacketReader(deps)
    this.routing = new RoutingSource(deps)
  }

  async run(raw: unknown): Promise<PrepareResult | Refusal> {
    const input = readInput(raw, 'prepare', (fields): PrepareInput => ({
      base: fields.optionalString('base'),
      comments: fields.has('comments') ? fields.boolean('comments') : false,
      flow: fields.optionalString('flow'),
      depth: fields.has('depth') ? fields.oneOf('depth', DEPTHS) : undefined,
      head: fields.optionalString('head') ?? 'HEAD',
      orchestratorFamily: fields.oneOf('orchestrator_family', ORCHESTRATOR_FAMILIES),
      previousRunDir: fields.has('previous_run_dir') ? fields.string('previous_run_dir') : undefined,
      references: fields.has('references') ? fields.strings('references') : undefined,
      repo: fields.string('repo'),
      spec: fields.string('spec'),
      stage: fields.oneOf('stage', CRITIQUE_STAGES),
      ticket: fields.optionalString('ticket'),
    }))
    const { orchestratorFamily, stage } = input
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const entry = (await this.repositories.allowlist()).repositories[repository.key]
    if (entry === undefined) {
      return refusal('not-opted-in', `${repository.key} has no family allowlist entry on this machine; record one with allow`)
    }
    if (isCodeStage(stage) && !entry.code) {
      return refusal(
        'code-not-allowed',
        `${repository.key} allows documents but not code; record the code permission with allow and "code": true`,
      )
    }
    const tracker = await this.trackers.resolve(repository)
    if (tracker.status === 'refused') return tracker
    const contents = isCodeStage(stage)
      ? await this.codePackets.read(repository, tracker, { ...input, stage })
      : await this.documentPacket(repository, tracker, input)
    if (contents.status === 'refused') return contents
    const { artifact, code } = contents
    const round = await this.round(input.previousRunDir, { reference: artifact.reference, repository: repository.key, stage })
    if ('status' in round) return round
    const { previousFlow, ...roundFields } = round
    const routing = await this.routing.load()
    const { choice, depth } = chooseDepth(
      stage,
      stageRouting(routing, { stage }).depth_signals,
      { labels: artifact.labels, text: artifact.text },
      input.depth,
    )
    const request = { allowedFamilies: entry.families, depth, orchestratorFamily, stage }
    const roster = resolveRoster(routing, request, await this.readiness(routing, request))
    const runId = await this.freshRunId(repository, stage)
    const runDir = join(repository.commonDir, 'critique', runId)
    const packetDir = join(runDir, 'packet')
    const packetFiles = await this.freezePacket(packetDir, contents.files)
    const source: ArtifactSource = { labels: artifact.labels, reference: artifact.reference, tracker: artifact.tracker }
    const manifest: RunManifest = {
      allowed_families: entry.families,
      artifact: source,
      ...(code === undefined ? {} : { code }),
      created_at: this.deps.clock.now().toISOString(),
      critics: roster.critics,
      depth: roster.depth,
      distinct_families: roster.distinct_families,
      depth_choice: choice,
      dropped: roster.dropped,
      flow: input.flow ?? previousFlow ?? artifact.reference,
      limits: routing.limits,
      orchestrator_family: orchestratorFamily,
      packet_dir: packetDir,
      packet_files: packetFiles,
      repo_root: repository.root,
      repository: repository.key,
      ...roundFields,
      run_dir: runDir,
      run_id: runId,
      stage,
      version: 1,
    }
    await this.deps.files.writeText(manifestPath(runDir), `${JSON.stringify(manifest, null, 2)}\n`)
    return {
      artifact: source,
      ...(code === undefined ? {} : { code: preparedCode(code, packetFiles) }),
      critics: roster.critics.map(criticIdentity),
      depth: roster.depth,
      depth_choice: choice,
      dropped: roster.dropped,
      manifest_path: manifestPath(runDir),
      packet_dir: packetDir,
      packet_files: listedFiles(manifest).map((file) => file.path),
      round: roundFields.round,
      run_dir: runDir,
      run_id: runId,
      status: 'ready',
    }
  }

  /**
   * A first round, or the second round after the recorded first round named
   * by `previous_run_dir`. A run that was itself a second round is refused, so
   * a stage never runs a third time over one artifact.
   */
  private async round(
    previousRunDir: string | undefined,
    current: { readonly repository: string; readonly stage: CritiqueStage; readonly reference: string },
  ): Promise<RoundOf | Refusal> {
    if (previousRunDir === undefined) return { round: 1 }
    const previous = await readJsonFile(this.deps.files, manifestPath(previousRunDir), 'invalid-run', parseManifest)
    if (previous.round === 2) {
      return refusal('round-limit', `run ${previous.run_id} was already the second round; a stage runs at most two rounds`)
    }
    if (!(await this.deps.files.exists(reportPath(previousRunDir)))) {
      throw new CritiqueError('invalid-run', `run ${previous.run_id} is not recorded yet; record it before its second round`)
    }
    const { reference, repository, stage } = current
    if (previous.repository !== repository || previous.stage !== stage || previous.artifact.reference !== reference) {
      throw new CritiqueError(
        'invalid-input',
        `run ${previous.run_id} critiqued ${previous.artifact.reference} at the ${previous.stage} stage, not ${reference}`,
      )
    }
    return { previousFlow: previous.flow, previous_run_id: previous.run_id, round: 2 }
  }

  /**
   * Asks each client the stage could run, and that the family rules permit,
   * whether it can run here, and holds an agy client that can to the day's
   * Antigravity quota. Counting reads the ledger and launches nothing, so an
   * agy that is not set up falls back without touching the counter.
   */
  private async readiness(routing: Routing, request: RosterRequest): Promise<ReadonlyMap<string, ClientAvailability>> {
    const agyToday = await new FlowBudget(this.deps).agyInvocationsToday()
    const checks = candidateClients(routing, request).map(async ([name, client]): Promise<readonly [string, ClientAvailability]> => {
      const ready = await clientReadiness(this.deps.clients, client)
      const quota = ready.ready && client.kind === 'agy' ? quotaReached(agyToday, routing.limits.antigravity_quota, request.stage) : undefined
      return [name, quota === undefined ? ready : { detail: quota, ready: false, reason: 'quota' }]
    })
    return new Map(await Promise.all(checks))
  }

  private async freshRunId(repository: ResolvedRepository, stage: CritiqueStage): Promise<string> {
    const base = `${compactTimestamp(this.deps.clock.now())}-${stage}`
    let candidate = base
    for (let n = 2; await this.deps.files.exists(manifestPath(join(repository.commonDir, 'critique', candidate))); n += 1) {
      candidate = `${base}-${String(n)}`
    }
    return candidate
  }

  /**
   * A document stage's packet: the artifact goes to `artifact/` and each
   * reference to `references/` under its repository path. Nothing else is
   * ever put at the packet root, so a `.vibe/`, `.agents/` or `AGENTS.md`
   * there can never become critic config.
   */
  private async documentPacket(
    repository: ResolvedRepository,
    tracker: Tracker,
    input: PrepareInput,
  ): Promise<
    Refusal | { readonly status: 'read'; readonly artifact: TrackedArtifact; readonly files: readonly UnfrozenFile[]; readonly code: undefined }
  > {
    const artifact = await tracker.artifact({ comments: input.comments, reference: input.spec })
    if (artifact.status === 'refused') return artifact
    const references =
      input.references === undefined
        ? await this.decisionDocs.resolve(repository.root)
        : input.references.map((path) => insideRepository(repository.root, path))
    const files: UnfrozenFile[] = [
      { kind: 'artifact', path: join('artifact', artifact.name), source: artifact.reference, text: artifact.text },
      ...(await Promise.all(
        references.map(async (source) => ({
          kind: 'reference' as const,
          path: join('references', source),
          source,
          text: await this.deps.files.readText(join(repository.root, source)),
        })),
      )),
    ]
    return { artifact, code: undefined, files, status: 'read' }
  }

  private async freezePacket(packetDir: string, files: readonly UnfrozenFile[]): Promise<readonly PacketFile[]> {
    const frozen: PacketFile[] = []
    for (const { text, ...file } of files) {
      await this.deps.files.writeText(join(packetDir, file.path), text)
      frozen.push({ ...file, sha256: sha256(text) })
    }
    return frozen
  }
}

function preparedCode(code: CodeSource, files: readonly PacketFile[]): PreparedCode {
  return {
    base_commit: code.base_commit,
    changed_files: code.changed_files,
    head_commit: code.head_commit,
    tree_files: files.filter((file) => file.kind === 'tree').length,
  }
}
